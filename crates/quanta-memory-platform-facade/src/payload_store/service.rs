use std::marker::PhantomData;
use std::sync::{Mutex, PoisonError};

use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, ManagedCharge, MemoryContextHandle, MemoryContextKind,
    MemoryError, MemoryResult, PublishedPayload, RetainedEnvelope, StoreBindingHandle,
    StoreDeleteError, StoreStats, StoreWriteError,
};

use crate::payload_store::PayloadStore;
#[cfg(feature = "unbudgeted-stores")]
use crate::shared::accounting::Unbudgeted;
use crate::shared::accounting::{Budgeted, StoreAccounting};
use crate::shared::lifecycle::{retire_client_value, BeginClose, DropTransition, StoreLifecycle};
use crate::shared::measure::{ByteSized, CanonicalKey};

fn canonical_digest(key: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug)]
struct PayloadEntry<PayloadRef, Bytes> {
    payload_ref: PayloadRef,
    canonical_digest: u64,
    charged_bytes: u64,
    pin_count: u64,
    charge: Option<ManagedCharge>,
    bytes: Bytes,
}

fn retire_entries<PayloadRef, Bytes>(entries: Vec<PayloadEntry<PayloadRef, Bytes>>) {
    for entry in entries {
        let PayloadEntry { payload_ref, bytes, charge, .. } = entry;
        drop(charge);
        retire_client_value(payload_ref);
        retire_client_value(bytes);
    }
}

#[derive(Debug)]
struct PayloadState<PayloadRef, Bytes> {
    entries: Vec<PayloadEntry<PayloadRef, Bytes>>,
    next_ref: u64,
    charged_total: u64,
}

/// Canonical payload store branded by its construction-time accounting mode.
///
/// Materialized stores expose neither raw construction nor late accounting
/// attachment:
///
/// ```compile_fail
/// # use quanta_memory_platform_facade::{Budgeted, SimplePayloadStore};
/// # use quanta_memory_platform_core::LiveProcessMemoryBroker;
/// # fn late_attach(store: SimplePayloadStore<String, Vec<u8>, Budgeted>, broker: LiveProcessMemoryBroker) {
/// let rebound = store.charged_against(broker);
/// # }
/// ```
///
/// ```compile_fail
/// # use quanta_memory_platform_facade::{Budgeted, SimplePayloadStore};
/// # use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
/// let binding = StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner".into(), None);
/// let store = SimplePayloadStore::<String, Vec<u8>, Budgeted>::new(binding, None);
/// ```
#[derive(Debug)]
pub struct SimplePayloadStore<PayloadRef, Bytes, Mode> {
    binding: StoreBindingHandle,
    context: Option<MemoryContextHandle>,
    accounting: StoreAccounting,
    lifecycle: StoreLifecycle,
    state: Mutex<PayloadState<PayloadRef, Bytes>>,
    _mode: PhantomData<fn() -> Mode>,
}

impl<PayloadRef, Bytes> SimplePayloadStore<PayloadRef, Bytes, Budgeted> {
    pub(crate) fn new_budgeted(
        binding: StoreBindingHandle,
        context: Option<MemoryContextHandle>,
        broker: LiveProcessMemoryBroker,
    ) -> Self {
        Self::with_accounting(binding, context, StoreAccounting::Budgeted(broker))
    }
}

#[cfg(feature = "unbudgeted-stores")]
impl<PayloadRef, Bytes> SimplePayloadStore<PayloadRef, Bytes, Unbudgeted> {
    pub(crate) fn new_unbudgeted(
        binding: StoreBindingHandle,
        context: Option<MemoryContextHandle>,
    ) -> Self {
        Self::with_accounting(binding, context, StoreAccounting::Unbudgeted)
    }
}

impl<PayloadRef, Bytes, Mode> SimplePayloadStore<PayloadRef, Bytes, Mode> {
    fn with_accounting(
        binding: StoreBindingHandle,
        context: Option<MemoryContextHandle>,
        accounting: StoreAccounting,
    ) -> Self {
        Self {
            binding,
            context,
            accounting,
            lifecycle: StoreLifecycle::new("payload store"),
            state: Mutex::new(PayloadState { entries: Vec::new(), next_ref: 0, charged_total: 0 }),
            _mode: PhantomData,
        }
    }

    fn lock(&self) -> MemoryResult<std::sync::MutexGuard<'_, PayloadState<PayloadRef, Bytes>>> {
        self.lifecycle.try_lock_state(&self.state)
    }

    fn lock_read(
        &self,
    ) -> MemoryResult<std::sync::MutexGuard<'_, PayloadState<PayloadRef, Bytes>>> {
        self.lifecycle.ensure_readable()?;
        let state = self.lock()?;
        self.lifecycle.ensure_readable()?;
        Ok(state)
    }

    fn context_kind(&self, value: &RetainedEnvelope<Bytes>) -> MemoryContextKind {
        value
            .context
            .as_ref()
            .or(self.context.as_ref())
            .map_or(MemoryContextKind::Request, |handle| handle.kind)
    }

    pub fn charged_bytes(&self) -> MemoryResult<u64> {
        Ok(self.lock_read()?.charged_total)
    }

    /// Release every live charge and enter the terminal closed state. A repeated
    /// close after success is an idempotent no-op.
    pub fn close(&self) -> MemoryResult<()> {
        let close = match self.lifecycle.begin_close()? {
            BeginClose::AlreadyClosed => return Ok(()),
            BeginClose::Acquired(close) => close,
        };

        let (indices, mut charges) = {
            let mut state = self.lock()?;
            let mut indices = Vec::new();
            let mut charges = Vec::new();
            for (index, entry) in state.entries.iter_mut().enumerate() {
                if let Some(charge) = entry.charge.take() {
                    indices.push(index);
                    charges.push(charge);
                }
            }
            (indices, charges)
        };

        if let Err(error) = self.accounting.release_many(&mut charges) {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            for (index, charge) in indices.into_iter().zip(charges) {
                state.entries[index].charge = Some(charge);
            }
            close.rollback();
            return Err(error);
        }

        let entries = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.charged_total = 0;
            std::mem::take(&mut state.entries)
        };
        close.commit();
        // User payload destructors run after both store and broker state reach
        // their terminal, mutually consistent postimage.
        retire_entries(entries);
        Ok(())
    }
}

impl<PayloadRef, Bytes, Mode> SimplePayloadStore<PayloadRef, Bytes, Mode>
where
    PayloadRef: Clone + From<String> + PartialEq,
    Bytes: Clone + ByteSized + CanonicalKey,
{
    pub fn pin(&self, payload_ref: &PayloadRef) -> MemoryResult<()> {
        let _permit = self.lifecycle.begin_mutation()?;
        let mut state = self.lock()?;
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| &entry.payload_ref == payload_ref)
            .ok_or_else(|| MemoryError::InvalidState("pin: payload not found".to_string()))?;
        entry.pin_count = entry
            .pin_count
            .checked_add(1)
            .ok_or_else(|| MemoryError::capacity("payload pin count overflow"))?;
        Ok(())
    }

    pub fn unpin(&self, payload_ref: &PayloadRef) -> MemoryResult<()> {
        let _permit = self.lifecycle.begin_mutation()?;
        let mut state = self.lock()?;
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| &entry.payload_ref == payload_ref)
            .ok_or_else(|| MemoryError::InvalidState("unpin: payload not found".to_string()))?;
        entry.pin_count = entry
            .pin_count
            .checked_sub(1)
            .ok_or_else(|| MemoryError::InvalidState("unpin: payload not pinned".to_string()))?;
        Ok(())
    }

    pub fn pin_count(&self, payload_ref: &PayloadRef) -> MemoryResult<u64> {
        Ok(self
            .lock_read()?
            .entries
            .iter()
            .find(|entry| &entry.payload_ref == payload_ref)
            .map_or(0, |entry| entry.pin_count))
    }
}

impl<PayloadRef, Bytes, Mode> PayloadStore<PayloadRef, Bytes>
    for SimplePayloadStore<PayloadRef, Bytes, Mode>
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
        let _permit = self.lifecycle.begin_mutation()?;
        let context_kind = self.context_kind(&value);
        let canonical_key = value.bytes.canonical_key();
        let digest = canonical_digest(&canonical_key);
        let charged_bytes = value.bytes.byte_size();

        let (payload_ref, next_ref, next_total) = {
            let state = self.lock()?;
            if let Some(existing) = state.entries.iter().find(|entry| {
                entry.canonical_digest == digest && entry.bytes.canonical_key() == canonical_key
            }) {
                return Ok(PublishedPayload::new(existing.payload_ref.clone()));
            }
            let next_ref = state
                .next_ref
                .checked_add(1)
                .ok_or_else(|| MemoryError::capacity("payload reference counter overflow"))?;
            let next_total = state
                .charged_total
                .checked_add(charged_bytes)
                .ok_or_else(|| MemoryError::capacity("payload byte counter overflow"))?;
            (PayloadRef::from(format!("payload-{}", state.next_ref)), next_ref, next_total)
        };

        let charge = self.accounting.charge(
            self.binding.class.clone(),
            self.binding.owner.clone(),
            context_kind,
            ChargeKind::RetainedPayload,
            charged_bytes,
        )?;

        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => {
                drop(charge);
                return Err(MemoryError::InvalidState(
                    "payload store lock poisoned after accounting".to_string(),
                ));
            }
        };
        state.entries.push(PayloadEntry {
            payload_ref: payload_ref.clone(),
            canonical_digest: digest,
            charged_bytes,
            pin_count: 0,
            charge,
            bytes: value.bytes,
        });
        state.next_ref = next_ref;
        state.charged_total = next_total;
        Ok(PublishedPayload::new(payload_ref))
    }

    fn delete_if_unpinned(&self, payload_ref: &PayloadRef) -> Result<(), StoreDeleteError> {
        let _permit = self.lifecycle.begin_mutation()?;
        let (index, next_total, mut charge) = {
            let mut state = self.lock()?;
            let index = state
                .entries
                .iter()
                .position(|entry| &entry.payload_ref == payload_ref)
                .ok_or_else(|| {
                    MemoryError::InvalidState("delete: payload not found".to_string())
                })?;
            if state.entries[index].pin_count > 0 {
                return Err(MemoryError::InvalidState(format!(
                    "delete rejected: payload still pinned ({} dependents)",
                    state.entries[index].pin_count
                )));
            }
            let next_total =
                state.charged_total.checked_sub(state.entries[index].charged_bytes).ok_or_else(
                    || MemoryError::InvalidState("payload byte counter underflow".to_string()),
                )?;
            (index, next_total, state.entries[index].charge.take())
        };

        if let Some(active) = charge.as_mut() {
            if let Err(error) = self.accounting.release_many(std::slice::from_mut(active)) {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.entries[index].charge = charge;
                return Err(error);
            }
        }

        let removed = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let removed = state.entries.remove(index);
            state.charged_total = next_total;
            removed
        };
        drop(removed);
        Ok(())
    }

    fn stats(&self) -> MemoryResult<StoreStats> {
        let state = self.lock_read()?;
        Ok(StoreStats { item_count: state.entries.len(), estimated_bytes: state.charged_total })
    }

    fn close(&self) -> MemoryResult<()> {
        SimplePayloadStore::close(self)
    }
}

impl<PayloadRef, Bytes, Mode> Drop for SimplePayloadStore<PayloadRef, Bytes, Mode> {
    fn drop(&mut self) {
        if self.lifecycle.transition_to_closed_for_drop() == DropTransition::AlreadyClosed {
            return;
        }
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        let entries = std::mem::take(&mut state.entries);
        state.charged_total = 0;
        retire_entries(entries);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, OnceLock, Weak};

    use quanta_memory_platform_core::{
        ExecutionBudget, MemoryBudgetManifest, MemoryClass, MemoryClassBudget, MemoryPreset,
        ReclaimPriority, ResidencyPolicy,
    };

    fn binding() -> StoreBindingHandle {
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-a".to_string(), None)
    }

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

    fn store_with(
        broker: &LiveProcessMemoryBroker,
    ) -> SimplePayloadStore<String, Vec<u8>, Budgeted> {
        SimplePayloadStore::new_budgeted(binding(), None, broker.clone())
    }

    fn store() -> (LiveProcessMemoryBroker, SimplePayloadStore<String, Vec<u8>, Budgeted>) {
        let broker = live_broker(1_000_000, 600_000);
        let store = store_with(&broker);
        (broker, store)
    }

    fn envelope(bytes: &[u8]) -> RetainedEnvelope<Vec<u8>> {
        RetainedEnvelope::new(bytes.to_vec())
    }

    #[derive(Clone)]
    struct PanickingBytes {
        bytes: Vec<u8>,
        drops: Arc<AtomicUsize>,
    }

    impl ByteSized for PanickingBytes {
        fn byte_size(&self) -> u64 {
            self.bytes.len() as u64
        }
    }

    impl CanonicalKey for PanickingBytes {
        fn canonical_key(&self) -> Vec<u8> {
            self.bytes.clone()
        }
    }

    impl Drop for PanickingBytes {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
            panic!("client payload destructor panic");
        }
    }

    #[derive(Clone, Debug)]
    struct ReentrantPayloadRef(String);

    type ReentrantRefStore = SimplePayloadStore<ReentrantPayloadRef, Vec<u8>, Budgeted>;

    impl From<String> for ReentrantPayloadRef {
        fn from(value: String) -> Self {
            Self(value)
        }
    }

    thread_local! {
        static REF_REENTRY_STORE: RefCell<Option<Weak<ReentrantRefStore>>> = const { RefCell::new(None) };
        static REF_REENTRY_HITS: Cell<usize> = const { Cell::new(0) };
    }

    impl PartialEq for ReentrantPayloadRef {
        fn eq(&self, other: &Self) -> bool {
            REF_REENTRY_STORE.with(|slot| {
                if let Some(store) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                    assert!(matches!(
                        store.stats(),
                        Err(MemoryError::OperationInProgress(resource)) if resource == "payload store"
                    ));
                    REF_REENTRY_HITS.with(|hits| hits.set(hits.get() + 1));
                }
            });
            self.0 == other.0
        }
    }

    #[derive(Clone)]
    struct ReentrantCanonicalBytes {
        bytes: Vec<u8>,
        probe: Option<ReentrantByteProbe>,
        hits: Arc<AtomicUsize>,
    }

    type ReentrantByteStore = SimplePayloadStore<String, ReentrantCanonicalBytes, Budgeted>;
    type ReentrantByteProbe = Arc<OnceLock<Weak<ReentrantByteStore>>>;

    impl ByteSized for ReentrantCanonicalBytes {
        fn byte_size(&self) -> u64 {
            self.bytes.len() as u64
        }
    }

    impl CanonicalKey for ReentrantCanonicalBytes {
        fn canonical_key(&self) -> Vec<u8> {
            if let Some(store) =
                self.probe.as_ref().and_then(|probe| probe.get()).and_then(Weak::upgrade)
            {
                assert!(matches!(
                    store.stats(),
                    Err(MemoryError::OperationInProgress(resource)) if resource == "payload store"
                ));
                self.hits.fetch_add(1, Ordering::SeqCst);
            }
            self.bytes.clone()
        }
    }

    fn context_handle(kind: MemoryContextKind) -> MemoryContextHandle {
        MemoryContextHandle::new("rt-test".to_string(), kind, "owner-a".to_string())
    }

    #[test]
    fn canonical_put_deduplicates_and_charges_once() {
        let (broker, store) = store();
        let first = store.put_canonical(envelope(b"same")).expect("first");
        let second = store.put_canonical(envelope(b"same")).expect("dedup");
        assert_eq!(first.payload_ref(), second.payload_ref());
        assert_eq!(store.stats().expect("stats").item_count, 1);
        assert_eq!(store.charged_bytes().expect("bytes"), 4);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 4);
    }

    #[test]
    fn metadata_equality_reentry_under_read_lock_is_typed_busy_and_preserves_charge() {
        let broker = live_broker(1_000_000, 600_000);
        let store =
            Arc::new(SimplePayloadStore::<ReentrantPayloadRef, Vec<u8>, Budgeted>::new_budgeted(
                binding(),
                None,
                broker.clone(),
            ));
        let published = store.put_canonical(envelope(b"metadata")).expect("put");
        REF_REENTRY_STORE.with(|slot| *slot.borrow_mut() = Some(Arc::downgrade(&store)));
        REF_REENTRY_HITS.with(|hits| hits.set(0));

        assert_eq!(store.pin_count(published.payload_ref()).expect("read metadata"), 0);
        REF_REENTRY_HITS.with(|hits| assert_eq!(hits.get(), 1));
        REF_REENTRY_STORE.with(|slot| *slot.borrow_mut() = None);
        assert_eq!(store.stats().expect("complete state").item_count, 1);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 8);
        store.close().expect("close");
        assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
    }

    #[test]
    fn duplicate_canonical_callback_reentry_under_lock_is_typed_busy_and_charges_once() {
        let broker = live_broker(1_000_000, 600_000);
        let store = Arc::new(
            SimplePayloadStore::<String, ReentrantCanonicalBytes, Budgeted>::new_budgeted(
                binding(),
                None,
                broker.clone(),
            ),
        );
        let probe = Arc::new(OnceLock::new());
        let hits = Arc::new(AtomicUsize::new(0));
        let first = store
            .put_canonical(RetainedEnvelope::new(ReentrantCanonicalBytes {
                bytes: b"same".to_vec(),
                probe: Some(Arc::clone(&probe)),
                hits: Arc::clone(&hits),
            }))
            .expect("first");
        assert!(probe.set(Arc::downgrade(&store)).is_ok());

        let replay = store
            .put_canonical(RetainedEnvelope::new(ReentrantCanonicalBytes {
                bytes: b"same".to_vec(),
                probe: None,
                hits: Arc::clone(&hits),
            }))
            .expect("deduplicated replay");
        assert_eq!(first.payload_ref(), replay.payload_ref());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(store.stats().expect("complete state").item_count, 1);
        assert_eq!(broker.ledger_snapshot().expect("charged once").total_owner_bytes(), 4);
        store.close().expect("close");
        assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
    }

    #[test]
    fn dedup_hit_preserves_first_context_and_stores_no_second_entry() {
        let (broker, store) = store();
        let first = store
            .put_canonical(
                envelope(b"same-bytes").with_context(context_handle(MemoryContextKind::Session)),
            )
            .expect("first");
        let bytes_before = store.charged_bytes().expect("bytes before");
        let second = store
            .put_canonical(
                envelope(b"same-bytes").with_context(context_handle(MemoryContextKind::Request)),
            )
            .expect("dedup");

        assert_eq!(second.payload_ref(), first.payload_ref());
        assert_eq!(store.stats().expect("stats").item_count, 1);
        assert_eq!(store.charged_bytes().expect("bytes after"), bytes_before);
        let snapshot = broker.ledger_snapshot().expect("ledger");
        assert_eq!(snapshot.total_owner_bytes(), bytes_before);
        assert_eq!(
            snapshot
                .context_usage
                .get(&MemoryContextKind::Session)
                .map_or(0, |usage| usage.total()),
            bytes_before
        );
        assert_eq!(
            snapshot
                .context_usage
                .get(&MemoryContextKind::Request)
                .map_or(0, |usage| usage.total()),
            0
        );
    }

    #[test]
    fn distinct_payloads_each_charge_once() {
        let (broker, store) = store();
        store.put_canonical(envelope(b"aa")).expect("a");
        store.put_canonical(envelope(b"bbbb")).expect("b");
        assert_eq!(store.charged_bytes().expect("bytes"), 6);
        assert_eq!(store.stats().expect("stats").item_count, 2);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 6);
    }

    #[test]
    fn delete_unpinned_releases_bytes_exactly_once() {
        let (broker, store) = store();
        let published = store.put_canonical(envelope(b"deleteme")).expect("put");
        store.delete_if_unpinned(published.payload_ref()).expect("delete");
        assert_eq!(store.charged_bytes().expect("bytes"), 0);
        assert_eq!(store.stats().expect("stats").item_count, 0);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
        assert!(matches!(
            store.delete_if_unpinned(published.payload_ref()),
            Err(MemoryError::InvalidState(_))
        ));
    }

    #[test]
    fn over_cap_put_is_fail_closed_and_does_not_consume_ref() {
        let broker = live_broker(1_000, 50);
        let store = store_with(&broker);
        assert!(matches!(store.put_canonical(envelope(&[0; 100])), Err(MemoryError::Capacity(_))));
        assert_eq!(store.stats().expect("stats").item_count, 0);
        let published = store.put_canonical(envelope(&[0; 40])).expect("within cap");
        assert_eq!(published.payload_ref(), &"payload-0".to_string());
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 40);
    }

    #[test]
    fn pin_is_metadata_only_and_pinned_delete_is_rejected() {
        let (broker, store) = store();
        let published = store.put_canonical(envelope(b"payload")).expect("put");
        store.pin(published.payload_ref()).expect("pin");
        store.pin(published.payload_ref()).expect("pin again");
        assert_eq!(store.pin_count(published.payload_ref()).expect("count"), 2);
        assert!(matches!(
            store.delete_if_unpinned(published.payload_ref()),
            Err(MemoryError::InvalidState(_))
        ));
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 7);
        store.unpin(published.payload_ref()).expect("unpin");
        store.unpin(published.payload_ref()).expect("unpin again");
        store.delete_if_unpinned(published.payload_ref()).expect("delete");
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn missing_delete_and_unpin_without_pin_are_invalid_state() {
        let (_broker, store) = store();
        assert!(matches!(
            store.delete_if_unpinned(&"payload-404".to_string()),
            Err(MemoryError::InvalidState(_))
        ));
        let published = store.put_canonical(envelope(b"x")).expect("put");
        assert!(matches!(store.unpin(published.payload_ref()), Err(MemoryError::InvalidState(_))));
    }

    #[test]
    fn put_and_delete_during_rebuild_are_fail_closed_then_resume() {
        let (broker, store) = store();
        let published = store.put_canonical(envelope(&[0; 80])).expect("seed");
        broker.begin_rebuild().expect("begin rebuild");

        assert!(matches!(
            store.put_canonical(envelope(&[1; 40])),
            Err(MemoryError::InvalidState(_))
        ));
        assert!(matches!(
            store.delete_if_unpinned(published.payload_ref()),
            Err(MemoryError::InvalidState(_))
        ));
        assert_eq!(store.stats().expect("preserved stats").item_count, 1);
        assert_eq!(store.charged_bytes().expect("preserved bytes"), 80);
        assert_eq!(broker.ledger_snapshot().expect("preserved ledger").total_owner_bytes(), 80);

        broker.finish_rebuild().expect("finish rebuild");
        store.delete_if_unpinned(published.payload_ref()).expect("delete resumes");
        assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
    }

    #[cfg(feature = "unbudgeted-stores")]
    #[test]
    fn explicit_unbudgeted_store_is_a_distinct_non_ledger_mode() {
        let store =
            SimplePayloadStore::<String, Vec<u8>, Unbudgeted>::new_unbudgeted(binding(), None);
        store.put_canonical(envelope(b"free")).expect("put");
        assert_eq!(store.charged_bytes().expect("bytes"), 4);
        assert_eq!(store.stats().expect("stats").item_count, 1);
        store.close().expect("close unbudgeted");
    }

    #[test]
    fn close_refunds_every_live_entry_exactly_once() {
        let (broker, store) = store();
        store.put_canonical(envelope(b"aa")).expect("a");
        store.put_canonical(envelope(b"bbbb")).expect("b");
        assert_eq!(broker.ledger_snapshot().expect("before").total_owner_bytes(), 6);
        store.close().expect("close");
        assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
        assert_eq!(store.stats().expect("stats").item_count, 0);
    }

    #[test]
    fn close_refund_failure_preserves_entries_and_charge_ownership() {
        let (broker, store) = store();
        store.put_canonical(envelope(b"owned")).expect("put");
        broker.begin_rebuild().expect("begin rebuild");
        assert!(matches!(store.close(), Err(MemoryError::InvalidState(_))));
        assert_eq!(store.stats().expect("preserved stats").item_count, 1);
        assert_eq!(store.charged_bytes().expect("preserved bytes"), 5);
        assert_eq!(broker.ledger_snapshot().expect("preserved ledger").total_owner_bytes(), 5);
        broker.finish_rebuild().expect("finish rebuild");
        store.close().expect("retry close");
        assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
    }

    #[test]
    fn drop_during_rebuild_defers_then_finish_drains_capacity() {
        let (broker, store) = store();
        store.put_canonical(envelope(b"owned")).expect("put");
        broker.begin_rebuild().expect("begin rebuild");
        drop(store);
        assert_eq!(broker.ledger_snapshot().expect("deferred").total_owner_bytes(), 5);
        broker.finish_rebuild().expect("finish drains");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }

    #[test]
    fn drop_during_outer_unwind_contains_two_client_panics_and_refunds_all_charges() {
        let broker = live_broker(1_000_000, 600_000);
        let drops = Arc::new(AtomicUsize::new(0));
        let store = SimplePayloadStore::<String, PanickingBytes, Budgeted>::new_budgeted(
            binding(),
            None,
            broker.clone(),
        );
        for bytes in [b"four".as_slice(), b"sixsix".as_slice()] {
            store
                .put_canonical(RetainedEnvelope::new(PanickingBytes {
                    bytes: bytes.to_vec(),
                    drops: Arc::clone(&drops),
                }))
                .expect("put before unwind");
        }
        assert_eq!(broker.ledger_snapshot().expect("before").total_owner_bytes(), 10);

        let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _store = store;
            panic!("outer unwind");
        }))
        .expect_err("outer panic must propagate");

        assert_eq!(outer.downcast_ref::<&str>(), Some(&"outer unwind"));
        assert_eq!(drops.load(Ordering::SeqCst), 2, "both client values must be retired");
        assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
    }

    #[test]
    fn mutation_after_close_is_typed_closed() {
        let (_broker, store) = store();
        store.close().expect("close");
        assert!(matches!(
            store.put_canonical(envelope(b"late")),
            Err(MemoryError::Closed(resource)) if resource == "payload store"
        ));
        assert!(matches!(
            store.pin(&"payload-0".to_string()),
            Err(MemoryError::Closed(resource)) if resource == "payload store"
        ));
    }

    #[test]
    fn double_close_is_idempotent_without_double_refund() {
        let (broker, store) = store();
        store.put_canonical(envelope(b"once")).expect("put");
        store.close().expect("first close");
        store.close().expect("second close");
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn concurrent_mutation_and_close_has_one_typed_linearized_outcome() {
        let broker = live_broker(1_000_000, 600_000);
        let store = Arc::new(store_with(&broker));
        let barrier = Arc::new(Barrier::new(3));
        let close_store = Arc::clone(&store);
        let close_barrier = Arc::clone(&barrier);
        let close = std::thread::spawn(move || {
            close_barrier.wait();
            close_store.close()
        });
        let put_store = Arc::clone(&store);
        let put_barrier = Arc::clone(&barrier);
        let put = std::thread::spawn(move || {
            put_barrier.wait();
            put_store.put_canonical(envelope(b"race"))
        });
        barrier.wait();

        let close = close.join().expect("close thread");
        let put = put.join().expect("put thread");
        assert!(matches!(&close, Ok(()) | Err(MemoryError::OperationInProgress(_))));
        assert!(matches!(
            put,
            Ok(_) | Err(MemoryError::Closed(_)) | Err(MemoryError::OperationInProgress(_))
        ));
        if close.is_err() {
            store.close().expect("retry close after winning mutation");
        }
        assert_eq!(broker.ledger_snapshot().expect("linearized ledger").total_owner_bytes(), 0);
        assert_eq!(store.stats().expect("linearized stats").item_count, 0);
    }

    #[test]
    fn counter_overflow_is_rejected_without_accounting_or_entry_mutation() {
        let (broker, store) = store();
        store.state.lock().expect("state").charged_total = u64::MAX;
        assert!(matches!(store.put_canonical(envelope(b"x")), Err(MemoryError::Capacity(_))));
        let state = store.state.lock().expect("state");
        assert_eq!(state.charged_total, u64::MAX);
        assert!(state.entries.is_empty());
        drop(state);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn poisoned_stats_is_error_not_empty() {
        let (_broker, store) = store();
        let store = Arc::new(store);
        let poison = Arc::clone(&store);
        assert!(std::thread::spawn(move || {
            let _guard = poison.state.lock().expect("lock");
            panic!("poison payload state");
        })
        .join()
        .is_err());
        assert!(matches!(store.stats(), Err(MemoryError::InvalidState(_))));
    }

    #[test]
    fn source_has_no_public_raw_constructor_or_late_attach() {
        let source = include_str!("service.rs");
        assert!(!source.lines().any(|line| line.trim_start().starts_with("pub fn new(")));
        assert!(!source
            .lines()
            .any(|line| line.trim_start().starts_with("pub fn charged_against(")));
        assert!(!source.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("pub struct SimplePayloadStore<") && line.contains("Mode = Budgeted")
        }));
        assert!(source.contains("pub(crate) fn new_budgeted"));
    }
}
