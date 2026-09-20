use std::sync::Mutex;

use quanta_memory_platform_core::{
    ChargeKind, MemoryContextHandle, MemoryContextKind, MemoryError, PublishedPayload,
    ReservationTicket, RetainedEnvelope, StoreBindingHandle, StoreDeleteError, StoreStats,
    StoreWriteError,
};

use crate::payload_store::PayloadStore;
use crate::shared::accounting::SharedAccountingSink;
use crate::shared::measure::{ByteSized, CanonicalKey};

/// A fixed-size digest of a canonical key, used as the dedup fast-path so the
/// store does not retain a full second copy of every payload's bytes (the old
/// `canonical_key: Vec<u8>` doubled resident memory). Digest collisions are
/// resolved exactly by re-deriving and comparing the colliding entry's canonical
/// key, so dedup stays correct, never hash-approximate.
fn canonical_digest(key: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

/// One retained canonical payload: its publish ref, charged bytes, canonical
/// identity digest, and current pin count (metadata only).
#[derive(Debug)]
struct PayloadEntry<PayloadRef, Bytes> {
    payload_ref: PayloadRef,
    canonical_digest: u64,
    charged_bytes: u64,
    pin_count: u64,
    /// Ledger receipt when the store is charged against a broker; `None` in
    /// non-budgeted mode. Refunded exactly once on delete.
    ticket: Option<ReservationTicket>,
    bytes: Bytes,
}

#[derive(Debug)]
struct PayloadState<PayloadRef, Bytes> {
    entries: Vec<PayloadEntry<PayloadRef, Bytes>>,
    next_ref: u64,
    /// Sum of charged payload bytes. Charged exactly once per canonical payload.
    charged_total: u64,
}

/// Portable canonical payload store.
///
/// Owns charge-once payload semantics (playbook 4.2, 5.1) and the pin-safe delete
/// path (playbook 4.3): canonical payload bytes are charged exactly once, a second
/// `put_canonical` of the same canonical key returns the existing ref without
/// re-charging, and a pinned payload cannot be deleted.
///
/// Pin counts are metadata only; they never re-charge payload bytes.
///
/// When built `charged_against` a broker, every distinct canonical payload is
/// reserved+committed through the broker's ledger (the accounting SSOT) and
/// refunded on delete, so global/owner/class caps gate real store bytes. Without
/// a sink the store self-accounts (explicit non-budgeted mode).
#[derive(Debug)]
pub struct SimplePayloadStore<PayloadRef, Bytes> {
    binding: StoreBindingHandle,
    context: Option<MemoryContextHandle>,
    charger: Option<SharedAccountingSink>,
    state: Mutex<PayloadState<PayloadRef, Bytes>>,
}

impl<PayloadRef, Bytes> SimplePayloadStore<PayloadRef, Bytes> {
    pub fn new(binding: StoreBindingHandle, context: Option<MemoryContextHandle>) -> Self {
        Self {
            binding,
            context,
            charger: None,
            state: Mutex::new(PayloadState { entries: Vec::new(), next_ref: 0, charged_total: 0 }),
        }
    }

    /// Charge this store's bytes against a broker ledger (the accounting SSOT).
    /// Without this, the store self-accounts and no cap controls its bytes.
    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }

    /// The context kind charges are attributed to (defaults to `Request`).
    fn context_kind(&self, value: &RetainedEnvelope<Bytes>) -> MemoryContextKind {
        value
            .context
            .as_ref()
            .or(self.context.as_ref())
            .map_or(MemoryContextKind::Request, |handle| handle.kind)
    }
}

impl<PayloadRef, Bytes> SimplePayloadStore<PayloadRef, Bytes>
where
    PayloadRef: Clone + From<String> + PartialEq,
    Bytes: Clone + ByteSized + CanonicalKey,
{
    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, PayloadState<PayloadRef, Bytes>>, MemoryError> {
        self.state
            .lock()
            .map_err(|_| MemoryError::InvalidState("payload store lock poisoned".to_string()))
    }

    /// Total payload bytes charged. Equals the sum of distinct canonical payloads.
    pub fn charged_bytes(&self) -> Result<u64, MemoryError> {
        Ok(self.lock()?.charged_total)
    }

    /// Pin a payload (metadata only; does not re-charge bytes).
    pub fn pin(&self, payload_ref: &PayloadRef) -> Result<(), MemoryError> {
        let mut state = self.lock()?;
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| &entry.payload_ref == payload_ref)
            .ok_or_else(|| MemoryError::InvalidState("pin: payload not found".to_string()))?;
        entry.pin_count = entry.pin_count.saturating_add(1);
        Ok(())
    }

    /// Release one pin. Errors if the payload has no live pins.
    pub fn unpin(&self, payload_ref: &PayloadRef) -> Result<(), MemoryError> {
        let mut state = self.lock()?;
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| &entry.payload_ref == payload_ref)
            .ok_or_else(|| MemoryError::InvalidState("unpin: payload not found".to_string()))?;
        if entry.pin_count == 0 {
            return Err(MemoryError::InvalidState("unpin: payload not pinned".to_string()));
        }
        entry.pin_count -= 1;
        Ok(())
    }

    /// Current pin count for a payload (0 if absent).
    pub fn pin_count(&self, payload_ref: &PayloadRef) -> Result<u64, MemoryError> {
        Ok(self
            .lock()?
            .entries
            .iter()
            .find(|entry| &entry.payload_ref == payload_ref)
            .map_or(0, |entry| entry.pin_count))
    }
}

impl<PayloadRef, Bytes> PayloadStore<PayloadRef, Bytes> for SimplePayloadStore<PayloadRef, Bytes>
where
    PayloadRef: Clone + From<String> + PartialEq,
    Bytes: Clone + ByteSized + CanonicalKey,
{
    fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    fn put_canonical(
        &self,
        value: RetainedEnvelope<Bytes>,
    ) -> Result<PublishedPayload<PayloadRef>, StoreWriteError> {
        let context_kind = self.context_kind(&value);
        let mut state = self.lock()?;
        let canonical_key = value.bytes.canonical_key();
        let digest = canonical_digest(&canonical_key);

        // Charge-once: if the canonical payload already exists, return the existing
        // ref WITHOUT re-charging (playbook 4.2 step 4 / 5.1). The digest is the
        // fast path; on a digest hit the full canonical key is re-derived and
        // compared so dedup is exact, never collision-approximate.
        if let Some(existing) = state.entries.iter().find(|entry| {
            entry.canonical_digest == digest && entry.bytes.canonical_key() == canonical_key
        }) {
            return Ok(PublishedPayload::new(existing.payload_ref.clone()));
        }

        let charged_bytes = value.bytes.byte_size();

        // Charge the ledger BEFORE allocating a ref or mutating state, so a cap
        // denial fails closed: nothing stored, no ref consumed (fail-closed).
        let ticket = match &self.charger {
            Some(sink) => Some(sink.charge(
                self.binding.class.clone(),
                self.binding.owner.clone(),
                context_kind,
                ChargeKind::RetainedPayload,
                charged_bytes,
            )?),
            None => None,
        };

        let payload_ref = PayloadRef::from(format!("payload-{}", state.next_ref));
        state.next_ref = state.next_ref.saturating_add(1);
        state.charged_total = state.charged_total.saturating_add(charged_bytes);
        state.entries.push(PayloadEntry {
            payload_ref: payload_ref.clone(),
            canonical_digest: digest,
            charged_bytes,
            pin_count: 0,
            ticket,
            bytes: value.bytes,
        });
        Ok(PublishedPayload::new(payload_ref))
    }

    fn delete_if_unpinned(&self, payload_ref: &PayloadRef) -> Result<(), StoreDeleteError> {
        let mut state = self.lock()?;
        let index =
            state.entries.iter().position(|entry| &entry.payload_ref == payload_ref).ok_or_else(
                || MemoryError::InvalidState("delete: payload not found".to_string()),
            )?;

        // Pin-safe delete (playbook 4.3 step 4): a pinned payload is never removed.
        if state.entries[index].pin_count > 0 {
            return Err(MemoryError::InvalidState(format!(
                "delete rejected: payload still pinned ({} dependents)",
                state.entries[index].pin_count
            )));
        }

        // Refund the ledger BEFORE removing the entry: a refund error leaves the
        // entry intact (fail-closed). The ticket is single-use, so removing the
        // entry right after prevents any replay.
        if let Some(sink) = &self.charger {
            if let Some(ticket) = state.entries[index].ticket.clone() {
                sink.refund(&ticket)?;
            }
        }

        let removed = state.entries.remove(index);
        // Release bytes exactly once, matching the single charge.
        state.charged_total = state.charged_total.saturating_sub(removed.charged_bytes);
        drop(removed.bytes);
        Ok(())
    }

    fn stats(&self) -> StoreStats {
        match self.lock() {
            Ok(state) => {
                StoreStats { item_count: state.entries.len(), estimated_bytes: state.charged_total }
            }
            // A poisoned lock cannot fabricate usage; report empty fail-closed.
            Err(_) => StoreStats::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};

    fn binding() -> StoreBindingHandle {
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-a".to_string(), None)
    }

    fn store() -> SimplePayloadStore<String, Vec<u8>> {
        SimplePayloadStore::new(binding(), None)
    }

    fn envelope(bytes: &[u8]) -> RetainedEnvelope<Vec<u8>> {
        RetainedEnvelope::new(bytes.to_vec())
    }

    #[test]
    fn put_canonical_charges_payload_bytes_exactly_once() {
        let store = store();
        let published = store.put_canonical(envelope(b"hello-payload")).expect("first put");
        // Bytes charged once == exact byte size of the payload.
        assert_eq!(store.charged_bytes().expect("charged"), b"hello-payload".len() as u64);
        assert_eq!(store.stats().item_count, 1);

        // NEGATIVE-PAIR / NEGATIVE-PATH 1: canonical payload already exists.
        // A second put of the same canonical bytes must NOT re-charge and must
        // return the SAME ref.
        let again = store.put_canonical(envelope(b"hello-payload")).expect("dedup put");
        assert_eq!(
            again.payload_ref(),
            published.payload_ref(),
            "duplicate canonical put must return the existing ref"
        );
        assert_eq!(
            store.charged_bytes().expect("charged after dedup"),
            b"hello-payload".len() as u64,
            "duplicate canonical payload must be charged exactly once"
        );
        assert_eq!(store.stats().item_count, 1, "dedup must not create a second entry");
    }

    #[test]
    fn dedup_hit_returns_existing_ref_without_storing_a_second_entry() {
        // scenario F (dedup-hit no re-clone / context preserved): a second
        // put_canonical of the same canonical bytes returns the FIRST entry's ref
        // and stores nothing new — the second envelope (and its distinct context)
        // is dropped, leaving the original entry and its accounting untouched.
        let store = store();
        let first = store
            .put_canonical(
                envelope(b"same-bytes").with_context(context_handle(MemoryContextKind::Session)),
            )
            .expect("first put");
        let item_count_before = store.stats().item_count;
        let bytes_before = store.charged_bytes().expect("bytes before");

        // Second put: identical canonical bytes, but a DIFFERENT context. The dedup
        // hit must not create a new entry, not re-charge, and not mutate state.
        let second = store
            .put_canonical(
                envelope(b"same-bytes").with_context(context_handle(MemoryContextKind::Request)),
            )
            .expect("dedup put");

        assert_eq!(second.payload_ref(), first.payload_ref(), "dedup returns the existing ref");
        assert_eq!(store.stats().item_count, item_count_before, "no second entry on dedup");
        assert_eq!(store.charged_bytes().expect("bytes after"), bytes_before, "no re-charge");
    }

    fn context_handle(kind: MemoryContextKind) -> MemoryContextHandle {
        MemoryContextHandle::new("rt-test".to_string(), kind, "owner-a".to_string())
    }

    #[test]
    fn distinct_payloads_each_charge_once() {
        let store = store();
        store.put_canonical(envelope(b"aa")).expect("a");
        store.put_canonical(envelope(b"bbbb")).expect("b");
        assert_eq!(store.charged_bytes().expect("total"), (2 + 4) as u64);
        assert_eq!(store.stats().item_count, 2);
    }

    #[test]
    fn delete_unpinned_releases_bytes_once() {
        let store = store();
        let published = store.put_canonical(envelope(b"deleteme")).expect("put");
        assert_eq!(store.charged_bytes().expect("charged"), 8);
        store.delete_if_unpinned(published.payload_ref()).expect("delete unpinned");
        assert_eq!(store.charged_bytes().expect("after delete"), 0);
        assert_eq!(store.stats().item_count, 0);
    }

    #[test]
    fn delete_while_pinned_is_rejected_with_invalid_state() {
        let store = store();
        let published = store.put_canonical(envelope(b"pinned")).expect("put");
        store.pin(published.payload_ref()).expect("pin");
        assert_eq!(store.pin_count(published.payload_ref()).expect("count"), 1);

        // NEGATIVE-PATH 2: delete attempted while pinned must fail-closed.
        let err = store
            .delete_if_unpinned(published.payload_ref())
            .expect_err("pinned delete must be rejected");
        assert!(
            matches!(err, MemoryError::InvalidState(_)),
            "pinned delete must be InvalidState, got {err:?}"
        );
        // Payload and its bytes survive the rejected delete.
        assert_eq!(store.charged_bytes().expect("charged"), b"pinned".len() as u64);
        assert_eq!(store.stats().item_count, 1);

        // Positive correction: after unpinning, delete succeeds and releases bytes.
        store.unpin(published.payload_ref()).expect("unpin");
        store.delete_if_unpinned(published.payload_ref()).expect("delete after unpin");
        assert_eq!(store.charged_bytes().expect("charged"), 0);
    }

    #[test]
    fn pin_is_metadata_only_and_does_not_recharge_bytes() {
        let store = store();
        let published = store.put_canonical(envelope(b"meta")).expect("put");
        let before = store.charged_bytes().expect("before");
        store.pin(published.payload_ref()).expect("pin");
        store.pin(published.payload_ref()).expect("pin again");
        // Invariant 5.1: pin counts are metadata; bytes unchanged.
        assert_eq!(store.charged_bytes().expect("after pins"), before);
        assert_eq!(store.pin_count(published.payload_ref()).expect("count"), 2);
    }

    #[test]
    fn delete_missing_payload_is_invalid_state() {
        let store = store();
        let err = store
            .delete_if_unpinned(&"payload-404".to_string())
            .expect_err("missing delete must fail");
        assert!(matches!(err, MemoryError::InvalidState(_)), "got {err:?}");
    }

    #[test]
    fn unpin_unpinned_payload_is_invalid_state() {
        let store = store();
        let published = store.put_canonical(envelope(b"x")).expect("put");
        let err = store.unpin(published.payload_ref()).expect_err("unpin without pin must fail");
        assert!(matches!(err, MemoryError::InvalidState(_)), "got {err:?}");
    }

    // ---- #3: a charged store routes real bytes through the broker ledger ----

    #[cfg(feature = "core-live-accounting")]
    mod charged {
        use super::*;
        use std::sync::Arc;

        use crate::shared::accounting::SharedAccountingSink;
        use quanta_memory_platform_core::{
            ExecutionBudget, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClassBudget,
            MemoryPreset, ReclaimPriority, ResidencyPolicy,
        };

        fn live_broker(global: u64, class_hard: u64) -> LiveProcessMemoryBroker {
            let residency = ResidencyPolicy::new(global).with_class_budget(
                MemoryClass::ArtifactCache,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: class_hard,
                    hard_cap_bytes: class_hard,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            );
            quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
                .with_manifest(MemoryBudgetManifest::new(
                    MemoryPreset::Production,
                    ExecutionBudget::default(),
                    residency,
                ))
                .build()
                .expect("valid broker")
        }

        fn charged_store(broker: &LiveProcessMemoryBroker) -> SimplePayloadStore<String, Vec<u8>> {
            let sink: SharedAccountingSink = Arc::new(broker.clone());
            SimplePayloadStore::<String, Vec<u8>>::new(binding(), None).charged_against(sink)
        }

        #[test]
        fn charged_store_routes_bytes_through_the_ledger() {
            let broker = live_broker(1_000_000, 600_000);
            let store = charged_store(&broker);

            let published = store.put_canonical(envelope(&[7u8; 100])).expect("put within cap");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                100,
                "the ledger (SSOT) must see the store's bytes"
            );

            // dedup-hit: same canonical bytes -> no second charge against the ledger.
            store.put_canonical(envelope(&[7u8; 100])).expect("dedup put");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                100,
                "a canonical dedup must not double-charge the ledger"
            );

            // delete refunds the ledger exactly once.
            store.delete_if_unpinned(published.payload_ref()).expect("delete");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                0,
                "delete must refund the ledger"
            );
        }

        #[test]
        fn charged_store_put_over_cap_is_denied_and_fails_closed() {
            let broker = live_broker(1_000, 50); // ArtifactCache hard cap 50 bytes
            let store = charged_store(&broker);

            let denied = store.put_canonical(envelope(&[0u8; 100])); // 100 > 50
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "an over-cap put must be a typed capacity error, got {denied:?}"
            );
            // Fail-closed: nothing stored, store counter untouched, ledger untouched.
            assert_eq!(store.stats().item_count, 0, "denied put must not store an entry");
            assert_eq!(store.charged_bytes().expect("charged"), 0);
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 0);

            // A denied put must not consume a payload ref (no ref leak).
            let ok = store.put_canonical(envelope(&[0u8; 40])).expect("within cap");
            assert_eq!(
                ok.payload_ref(),
                &"payload-0".to_string(),
                "a denied put must not advance the ref counter"
            );
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 40);
        }

        #[test]
        fn charged_store_put_and_delete_during_rebuild_are_fail_closed() {
            // Parity with the other charged stores: a charged put/delete while the
            // broker is rebuilding must fail closed (the charge/refund hits the
            // rebuild gate), leaving the ledger and store byte-for-byte unchanged,
            // and resume cleanly once the rebuild window closes. Exercises both the
            // put charge path and delete_if_unpinned's refund-during-rebuild branch.
            let broker = live_broker(1_000_000, 600_000);
            let store = charged_store(&broker);
            let published = store.put_canonical(envelope(&[0u8; 80])).expect("seed");
            let before = broker.ledger_snapshot().expect("snap").total_owner_bytes();
            assert_eq!(before, 80);

            broker.begin_rebuild().expect("begin rebuild");
            let put_denied = store.put_canonical(envelope(&[1u8; 40]));
            assert!(put_denied.is_err(), "put during rebuild must fail closed, got {put_denied:?}");
            let del_denied = store.delete_if_unpinned(published.payload_ref());
            assert!(
                del_denied.is_err(),
                "delete during rebuild must fail closed, got {del_denied:?}"
            );
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                before,
                "neither the denied put nor the denied delete moved the ledger"
            );

            broker.finish_rebuild().expect("finish");
            store
                .delete_if_unpinned(published.payload_ref())
                .expect("delete resumes after rebuild");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
        }

        #[test]
        fn uncharged_store_is_explicit_non_budgeted_mode() {
            // No sink: the store self-accounts and never touches a ledger.
            let store = SimplePayloadStore::<String, Vec<u8>>::new(binding(), None);
            store.put_canonical(envelope(b"free")).expect("put");
            assert_eq!(store.charged_bytes().expect("charged"), 4);
        }
    }
}
