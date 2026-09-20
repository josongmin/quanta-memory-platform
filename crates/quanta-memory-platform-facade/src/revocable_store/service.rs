use std::sync::Mutex;

use quanta_memory_platform_core::{
    ChargeKind, MemoryContextKind, MemoryError, ReservationTicket, StoreBindingHandle, StoreStats,
    StoreWriteError, SweepStats,
};

use crate::revocable_store::RevocableStore;
use crate::shared::accounting::SharedAccountingSink;
use crate::shared::measure::ByteSized;

#[derive(Debug)]
struct RevocableEntry<Key, Value> {
    key: Key,
    value: Value,
    charged_bytes: u64,
    /// Ledger receipt when charged against a broker; refunded on reclaim.
    ticket: Option<ReservationTicket>,
}

#[derive(Debug)]
struct RevocableState<Key, Value> {
    entries: Vec<RevocableEntry<Key, Value>>,
    committed_bytes: u64,
}

/// Portable revocable / spillable store.
///
/// Owns reclaim hooks (playbook 1.2 step 4, 4.3) and reports reclaimed bytes via
/// [`SweepStats`] for host telemetry.
///
/// When built `charged_against` a broker, insert reserves+commits the entry's
/// bytes through the ledger (the accounting SSOT, fail-closed on a cap breach)
/// and every reclaim path refunds them — so revocable bytes are gated by caps and
/// returned automatically, never left to a host that might forget to release.
/// Without a sink the store self-accounts (non-budgeted reporting mode).
#[derive(Debug)]
pub struct SimpleRevocableStore<Key, Value> {
    binding: StoreBindingHandle,
    charger: Option<SharedAccountingSink>,
    state: Mutex<RevocableState<Key, Value>>,
}

impl<Key, Value> SimpleRevocableStore<Key, Value> {
    pub fn new(binding: StoreBindingHandle) -> Self {
        Self {
            binding,
            charger: None,
            state: Mutex::new(RevocableState { entries: Vec::new(), committed_bytes: 0 }),
        }
    }

    /// Charge this store's bytes against a broker ledger (the accounting SSOT).
    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }
}

impl<Key, Value> SimpleRevocableStore<Key, Value>
where
    Value: ByteSized,
{
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, RevocableState<Key, Value>>, MemoryError> {
        self.state
            .lock()
            .map_err(|_| MemoryError::InvalidState("revocable store lock poisoned".to_string()))
    }

    /// Currently retained (reclaimable) bytes.
    pub fn committed_bytes(&self) -> Result<u64, MemoryError> {
        Ok(self.lock()?.committed_bytes)
    }

    /// Reclaim entries (oldest first) until at least `target_bytes` are freed.
    /// Returns the reclaim outcome so the caller can release the broker bytes.
    /// This reports reclaimed bytes only; it does not touch quota authority.
    pub fn reclaim_until(&self, target_bytes: u64) -> Result<SweepStats, MemoryError> {
        let mut state = self.lock()?;
        let mut reclaimed_items = 0usize;
        let mut reclaimed_bytes = 0u64;
        let mut refunds: Vec<ReservationTicket> = Vec::new();
        // Indices to evict, collected oldest-first (FIFO). Ascending order, so we
        // remove them in REVERSE below to keep the lower indices valid.
        let mut evict_indices: Vec<usize> = Vec::new();
        for (idx, entry) in state.entries.iter().enumerate() {
            if reclaimed_bytes >= target_bytes {
                break;
            }
            // A zero-byte entry frees no memory toward a BYTE target; evicting it is
            // pure data loss that never advances `reclaimed_bytes`. Skip it so a
            // positive target can never wipe a store of zero-byte entries
            // (over-eviction). Mirrors SimpleRetainedMapStore::sweep.
            if entry.charged_bytes == 0 {
                continue;
            }
            reclaimed_items += 1;
            reclaimed_bytes = reclaimed_bytes.saturating_add(entry.charged_bytes);
            if let Some(ticket) = &entry.ticket {
                refunds.push(ticket.clone());
            }
            evict_indices.push(idx);
        }
        if let Some(sink) = &self.charger {
            sink.refund_many(&refunds)?;
        }
        for idx in evict_indices.iter().rev() {
            state.entries.remove(*idx);
        }
        state.committed_bytes = state.committed_bytes.saturating_sub(reclaimed_bytes);
        Ok(SweepStats { reclaimed_items, reclaimed_bytes })
    }
}

impl<Key, Value> RevocableStore<Key, Value> for SimpleRevocableStore<Key, Value>
where
    Value: ByteSized,
{
    fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError> {
        let charged_bytes = value.byte_size();
        let mut state = self.lock()?;
        // Charge the ledger BEFORE recording the entry so a cap denial fails
        // closed: nothing stored, counter untouched.
        let ticket = match &self.charger {
            Some(sink) => Some(sink.charge(
                self.binding.class.clone(),
                self.binding.owner.clone(),
                MemoryContextKind::Runtime,
                ChargeKind::LiveScratch,
                charged_bytes,
            )?),
            None => None,
        };
        state.entries.push(RevocableEntry { key, value, charged_bytes, ticket });
        state.committed_bytes = state.committed_bytes.saturating_add(charged_bytes);
        Ok(())
    }

    fn reclaim_one(&self) -> Result<Option<(Key, Value)>, MemoryError> {
        let mut state = self.lock()?;
        if state.entries.is_empty() {
            return Ok(None);
        }
        if let (Some(sink), Some(ticket)) = (&self.charger, state.entries[0].ticket.as_ref()) {
            sink.refund(ticket)?;
        }
        let removed = state.entries.remove(0);
        state.committed_bytes = state.committed_bytes.saturating_sub(removed.charged_bytes);
        Ok(Some((removed.key, removed.value)))
    }

    fn stats(&self) -> StoreStats {
        match self.lock() {
            Ok(state) => StoreStats {
                item_count: state.entries.len(),
                estimated_bytes: state.committed_bytes,
            },
            Err(_) => StoreStats::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};

    fn store() -> SimpleRevocableStore<String, Vec<u8>> {
        let binding = StoreBindingHandle::new(
            MemoryClass::ExecutionScratchRevocable,
            "owner-rev".to_string(),
            None,
        );
        SimpleRevocableStore::new(binding)
    }

    #[test]
    fn insert_tracks_reclaimable_bytes() {
        let store = store();
        store.insert_checked("a".to_string(), vec![0u8; 4]).expect("a");
        store.insert_checked("b".to_string(), vec![0u8; 6]).expect("b");
        assert_eq!(store.committed_bytes().expect("committed"), 10);
        assert_eq!(store.stats(), StoreStats { item_count: 2, estimated_bytes: 10 });
    }

    #[test]
    fn reclaim_one_is_fifo_and_releases_its_bytes() {
        let store = store();
        store.insert_checked("first".to_string(), vec![0u8; 3]).expect("first");
        store.insert_checked("second".to_string(), vec![0u8; 5]).expect("second");
        let (key, value) = store.reclaim_one().expect("reclaim one").expect("entry present");
        assert_eq!(key, "first", "reclaim must be FIFO (oldest first)");
        assert_eq!(value.len(), 3);
        assert_eq!(store.committed_bytes().expect("after reclaim"), 5);
    }

    #[test]
    fn reclaim_one_on_empty_returns_none() {
        let store = store();
        assert!(store.reclaim_one().expect("reclaim empty").is_none());
        assert_eq!(store.committed_bytes().expect("empty"), 0);
    }

    #[test]
    fn reclaim_until_frees_at_least_target_and_reports_reclaimed_bytes() {
        let store = store();
        store.insert_checked("a".to_string(), vec![0u8; 4]).expect("a");
        store.insert_checked("b".to_string(), vec![0u8; 4]).expect("b");
        store.insert_checked("c".to_string(), vec![0u8; 4]).expect("c");
        assert_eq!(store.committed_bytes().expect("before"), 12);

        // Reports reclaimed bytes for the caller to release against the broker; it
        // never charges or redefines quota itself.
        let outcome = store.reclaim_until(5).expect("reclaim until");
        assert_eq!(outcome.reclaimed_items, 2, "two 4-byte entries cover a 5-byte target");
        assert_eq!(outcome.reclaimed_bytes, 8);
        assert_eq!(store.committed_bytes().expect("after"), 4);
    }

    #[test]
    fn reclaim_until_on_empty_store_reports_zero() {
        let store = store();
        let outcome = store.reclaim_until(100).expect("reclaim empty");
        assert_eq!(outcome.reclaimed_items, 0);
        assert_eq!(outcome.reclaimed_bytes, 0);
    }

    #[test]
    fn reclaim_until_does_not_over_evict_zero_byte_entries() {
        // Mirror of the retained_map sweep regression: a byte-target reclaim must
        // skip zero-byte entries (they free no memory) rather than draining them.
        let store = store();
        store.insert_checked("a".to_string(), vec![]).expect("a"); // 0 bytes (FIFO oldest)
        store.insert_checked("b".to_string(), vec![]).expect("b"); // 0 bytes
        store.insert_checked("c".to_string(), vec![0u8; 8]).expect("c"); // 8 bytes
        assert_eq!(store.committed_bytes().expect("before"), 8);

        let outcome = store.reclaim_until(5).expect("reclaim");
        assert_eq!(outcome.reclaimed_items, 1, "only the byte-bearing entry is reclaimed");
        assert_eq!(outcome.reclaimed_bytes, 8);
        assert_eq!(store.stats().item_count, 2, "leading zero-byte entries are NOT over-evicted");
    }

    #[test]
    fn reclaim_until_of_all_zero_byte_entries_reclaims_nothing() {
        // The pathological case: a store of only zero-byte entries can never reach a
        // POSITIVE byte target, so the reclaim must free nothing — not wipe the store.
        let store = store();
        for k in ["a", "b", "c"] {
            store.insert_checked(k.to_string(), vec![]).expect("zero-byte insert");
        }
        let outcome = store.reclaim_until(5).expect("reclaim");
        assert_eq!(outcome.reclaimed_items, 0, "zero-byte entries free no bytes; none reclaimed");
        assert_eq!(store.stats().item_count, 3, "a positive byte target must not wipe the store");
    }

    #[test]
    fn revoke_makes_entry_unretrievable_and_decrements_accounting() {
        // scenario F: revocable revoke -> entry gone (get(None)-equivalent) and the
        // store's reclaimable-byte accounting decrements by exactly that entry's
        // charge. The store exposes no `get`, so absence is proven by reclaiming the
        // ONLY remaining entry and by the byte/item counters.
        let store = store();
        store.insert_checked("only".to_string(), vec![0u8; 9]).expect("insert");
        assert_eq!(store.committed_bytes().expect("before"), 9);
        assert_eq!(store.stats().item_count, 1);

        // Revoke (reclaim) the entry.
        let (key, value) = store.reclaim_one().expect("revoke").expect("entry present");
        assert_eq!(key, "only");
        assert_eq!(value.len(), 9);

        // Accounting decremented to zero; the entry is no longer present.
        assert_eq!(store.committed_bytes().expect("after revoke"), 0);
        assert_eq!(store.stats(), StoreStats { item_count: 0, estimated_bytes: 0 });
        // A second revoke finds nothing (the equivalent of get -> None) and does
        // not drive accounting negative.
        assert!(
            store.reclaim_one().expect("reclaim after revoke").is_none(),
            "revoked entry must not be retrievable again"
        );
        assert_eq!(store.committed_bytes().expect("still zero"), 0);
    }

    #[test]
    fn empty_reclaim_leaves_state_byte_for_byte_unchanged() {
        // scenario F (fail-closed): a no-op reclaim on an empty store must not
        // mutate any observable counter. Capture state BEFORE and assert it is
        // identical AFTER.
        let store = store();
        store.insert_checked("keep".to_string(), vec![0u8; 7]).expect("insert");
        let bytes_before = store.committed_bytes().expect("before");
        let stats_before = store.stats();

        // reclaim_one on a store whose only entry we will NOT target: instead drain
        // it, then prove a further reclaim is inert.
        store.reclaim_one().expect("drain the one entry").expect("drain present");
        let bytes_drained = store.committed_bytes().expect("drained");
        let stats_drained = store.stats();
        assert_eq!(bytes_drained, 0);

        // Now the inert reclaim must leave the drained state untouched.
        assert!(store.reclaim_one().expect("inert reclaim").is_none());
        assert_eq!(store.committed_bytes().expect("after inert"), bytes_drained);
        assert_eq!(store.stats(), stats_drained);

        // Sanity: the pre-drain state was the 7-byte single entry.
        assert_eq!(bytes_before, 7);
        assert_eq!(stats_before, StoreStats { item_count: 1, estimated_bytes: 7 });
    }

    // ---- #3: a charged revocable store gates and refunds via the ledger ----
    #[cfg(feature = "core-live-accounting")]
    mod charged {
        use super::*;
        use crate::shared::accounting::SharedAccountingSink;
        use quanta_memory_platform_core::{
            ExecutionBudget, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClassBudget,
            MemoryPreset, ReclaimPriority, ResidencyPolicy,
        };
        use std::sync::Arc;

        fn live_broker(class_hard: u64) -> LiveProcessMemoryBroker {
            let residency = ResidencyPolicy::new(10_000_000).with_class_budget(
                MemoryClass::ExecutionScratchRevocable,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: class_hard,
                    hard_cap_bytes: class_hard,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::First,
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

        fn charged(broker: &LiveProcessMemoryBroker) -> SimpleRevocableStore<String, Vec<u8>> {
            let sink: SharedAccountingSink = Arc::new(broker.clone());
            let binding = StoreBindingHandle::new(
                MemoryClass::ExecutionScratchRevocable,
                "owner-rev".to_string(),
                None,
            );
            SimpleRevocableStore::new(binding).charged_against(sink)
        }

        #[test]
        fn insert_charges_and_reclaim_refunds_the_ledger() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("a".to_string(), vec![0u8; 4]).expect("a");
            store.insert_checked("b".to_string(), vec![0u8; 6]).expect("b");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 10);

            // reclaim_one refunds the oldest entry (4 bytes).
            store.reclaim_one().expect("reclaim").expect("entry present");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 6);

            // reclaim_until refunds the rest.
            store.reclaim_until(100).expect("reclaim until");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
        }

        #[test]
        fn insert_over_cap_is_denied_and_fails_closed() {
            let broker = live_broker(5);
            let store = charged(&broker);
            let denied = store.insert_checked("big".to_string(), vec![0u8; 9]); // 9 > 5
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "over-cap insert must be a typed capacity error, got {denied:?}"
            );
            assert_eq!(store.stats().item_count, 0);
            assert_eq!(store.committed_bytes().expect("committed"), 0);
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
        }

        #[test]
        fn reclaim_one_during_rebuild_is_fail_closed() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("a".to_string(), vec![0u8; 4]).expect("a");
            store.insert_checked("b".to_string(), vec![0u8; 6]).expect("b");
            broker.begin_rebuild().expect("begin rebuild");

            let denied = store.reclaim_one();
            assert!(
                matches!(denied, Err(MemoryError::InvalidState(_))),
                "reclaim_one during rebuild must fail closed, got {denied:?}"
            );
            assert_eq!(store.committed_bytes().expect("committed"), 10);
            assert_eq!(store.stats().item_count, 2);
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 10);
        }

        #[test]
        fn reclaim_until_during_rebuild_is_fail_closed() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("a".to_string(), vec![0u8; 4]).expect("a");
            store.insert_checked("b".to_string(), vec![0u8; 4]).expect("b");
            store.insert_checked("c".to_string(), vec![0u8; 4]).expect("c");
            broker.begin_rebuild().expect("begin rebuild");

            let denied = store.reclaim_until(5);
            assert!(
                matches!(denied, Err(MemoryError::InvalidState(_))),
                "reclaim_until during rebuild must fail closed, got {denied:?}"
            );
            assert_eq!(store.committed_bytes().expect("committed"), 12);
            assert_eq!(store.stats().item_count, 3);
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 12);
        }
    }

    #[test]
    fn reclaim_eviction_order_is_deterministic_fifo_at_capacity_boundary() {
        // scenario F (eviction boundary): when reclaiming down to a byte target, the
        // store evicts in a deterministic order (FIFO / oldest-first) so a host
        // releasing bytes against the broker gets a reproducible reclaim sequence.
        let store = store();
        store.insert_checked("oldest".to_string(), vec![0u8; 4]).expect("1");
        store.insert_checked("middle".to_string(), vec![0u8; 4]).expect("2");
        store.insert_checked("newest".to_string(), vec![0u8; 4]).expect("3");

        // Reclaim just enough to free 4 bytes: the single OLDEST entry must go.
        let outcome = store.reclaim_until(1).expect("reclaim one entry worth");
        assert_eq!(outcome.reclaimed_items, 1);
        assert_eq!(outcome.reclaimed_bytes, 4);
        // The survivor reclaimed next must be "middle" (FIFO), proving "oldest"
        // was evicted first at the boundary.
        let (next_key, _) = store.reclaim_one().expect("next oldest").expect("entry present");
        assert_eq!(next_key, "middle", "eviction order must be deterministic FIFO");
    }
}
