use std::sync::Mutex;

use quanta_memory_platform_core::{
    ChargeKind, MemoryContextKind, MemoryError, ReservationTicket, StoreBindingHandle, StoreStats,
    StoreWriteError, SweepHint, SweepStats,
};

use crate::retained_map::RetainedMapStore;
use crate::shared::accounting::SharedAccountingSink;
use crate::shared::measure::ByteSized;

#[derive(Debug)]
struct MapEntry<Key, Value> {
    key: Key,
    value: Value,
    charged_bytes: u64,
    /// Ledger receipt when the store is charged against a broker; `None` in
    /// non-budgeted mode. Refunded exactly once when the entry leaves the map
    /// (remove / overwrite / sweep).
    ticket: Option<ReservationTicket>,
}

#[derive(Debug)]
struct MapState<Key, Value> {
    entries: Vec<MapEntry<Key, Value>>,
    committed_bytes: u64,
}

/// Portable retained map store.
///
/// Owns the reserve -> mutate -> commit / release choreography (playbook 4.1):
/// envelope bytes are estimated and tentatively reserved, the map is mutated, and
/// only a successful mutation commits the reservation; a failed mutation releases
/// the tentative bytes so accounting never drifts.
///
/// When built `charged_against` a broker, every live entry is reserved+committed
/// through the broker's ledger (the accounting SSOT) on insert and refunded when
/// the entry leaves the map (remove / overwrite-in-place / sweep), so
/// global/owner/class caps gate real map bytes and a denied insert fails closed.
/// Without a sink the store self-accounts (explicit non-budgeted mode) and never
/// touches a ledger (invariant 2: it never requires full broker mutation access).
#[derive(Debug)]
pub struct SimpleRetainedMapStore<Key, Value> {
    binding: StoreBindingHandle,
    charger: Option<SharedAccountingSink>,
    state: Mutex<MapState<Key, Value>>,
}

impl<Key, Value> SimpleRetainedMapStore<Key, Value> {
    pub fn new(binding: StoreBindingHandle) -> Self {
        Self {
            binding,
            charger: None,
            state: Mutex::new(MapState { entries: Vec::new(), committed_bytes: 0 }),
        }
    }

    /// Charge this store's bytes against a broker ledger (the accounting SSOT).
    /// Without this, the store self-accounts and no cap controls its bytes.
    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }
}

impl<Key, Value> SimpleRetainedMapStore<Key, Value>
where
    Key: Clone + PartialEq,
    Value: Clone + ByteSized,
{
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, MapState<Key, Value>>, MemoryError> {
        self.state
            .lock()
            .map_err(|_| MemoryError::InvalidState("retained map lock poisoned".to_string()))
    }

    /// Committed retained bytes (sum of live entry charges).
    pub fn committed_bytes(&self) -> Result<u64, MemoryError> {
        Ok(self.lock()?.committed_bytes)
    }
}

impl<Key, Value> RetainedMapStore<Key, Value> for SimpleRetainedMapStore<Key, Value>
where
    Key: Clone + PartialEq,
    Value: Clone + ByteSized,
{
    fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError> {
        let reserved_bytes = value.byte_size();
        let mut state = self.lock()?;

        if let Some(index) = state.entries.iter().position(|entry| entry.key == key) {
            // OVERWRITE. For a charged store this is a single atomic recharge
            // (release old + charge new under one broker lock), so the overwrite
            // never transiently double-counts against caps/group admission nor
            // strands bytes if a rebuild interleaves. Fail-closed: on denial the
            // entry keeps its old value and a valid ticket (the original, or the
            // broker-restored one), and committed_bytes is unchanged.
            match &self.charger {
                Some(sink) => {
                    let old_ticket = match state.entries[index].ticket.take() {
                        Some(t) => t,
                        None => {
                            // Unreachable: a charged store always stores `Some(ticket)`
                            // on insert — even a zero-byte value mints a live ledger
                            // ticket via `sink.charge(.., 0)`. A ticket-less charged
                            // entry would mean a prior charge was lost, so fail closed
                            // LOUDLY rather than silently re-charging and diverging the
                            // store from the ledger (the old silent fallback could mask
                            // a charge-invariant regression).
                            return Err(MemoryError::InvalidState(
                                "charged retained-map entry has no ticket on overwrite: \
                                 charge invariant broken"
                                    .to_string(),
                            ));
                        }
                    };
                    match sink.recharge(
                        &old_ticket,
                        self.binding.class.clone(),
                        self.binding.owner.clone(),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedMetadata,
                        reserved_bytes,
                    ) {
                        Ok(new_ticket) => {
                            let old_charge = state.entries[index].charged_bytes;
                            state.committed_bytes = state
                                .committed_bytes
                                .saturating_sub(old_charge)
                                .saturating_add(reserved_bytes);
                            state.entries[index].value = value;
                            state.entries[index].charged_bytes = reserved_bytes;
                            state.entries[index].ticket = Some(new_ticket);
                            Ok(())
                        }
                        Err((err, restored)) => {
                            // Restore the entry's ticket so no charge is lost: the
                            // broker-restored fresh ticket if old was released, else
                            // the original old ticket (untouched). Value/bytes stay.
                            state.entries[index].ticket = restored.or(Some(old_ticket));
                            Err(err)
                        }
                    }
                }
                None => {
                    // Uncharged (non-budgeted) overwrite: swap value + self counter.
                    let old_charge = state.entries[index].charged_bytes;
                    state.committed_bytes = state
                        .committed_bytes
                        .saturating_sub(old_charge)
                        .saturating_add(reserved_bytes);
                    state.entries[index].value = value;
                    state.entries[index].charged_bytes = reserved_bytes;
                    Ok(())
                }
            }
        } else {
            // INSERT a new key. Charge the ledger BEFORE pushing, so a cap denial
            // fails closed (no entry created, ledger untouched).
            let new_ticket = match &self.charger {
                Some(sink) => Some(sink.charge(
                    self.binding.class.clone(),
                    self.binding.owner.clone(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedMetadata,
                    reserved_bytes,
                )?),
                None => None,
            };
            state.entries.push(MapEntry {
                key,
                value,
                charged_bytes: reserved_bytes,
                ticket: new_ticket,
            });
            state.committed_bytes = state.committed_bytes.saturating_add(reserved_bytes);
            Ok(())
        }
    }

    fn get(&self, key: &Key) -> Option<Value> {
        let state = self.lock().ok()?;
        state.entries.iter().find(|entry| &entry.key == key).map(|entry| entry.value.clone())
    }

    fn remove(&self, key: &Key) -> Result<Option<Value>, MemoryError> {
        let mut state = self.lock()?;
        let Some(index) = state.entries.iter().position(|entry| &entry.key == key) else {
            return Ok(None);
        };
        if let (Some(sink), Some(ticket)) = (&self.charger, state.entries[index].ticket.as_ref()) {
            sink.refund(ticket)?;
        }
        let removed = state.entries.remove(index);
        state.committed_bytes = state.committed_bytes.saturating_sub(removed.charged_bytes);
        Ok(Some(removed.value))
    }

    fn sweep(&self, hint: SweepHint) -> Result<SweepStats, MemoryError> {
        let mut state = self.lock()?;
        let mut reclaimed_items = 0usize;
        let mut reclaimed_bytes = 0u64;
        let mut refunds: Vec<ReservationTicket> = Vec::new();
        // Indices to evict, collected newest-first (LIFO). Descending order, so
        // removing them in sequence keeps the lower indices valid.
        let mut evict_indices: Vec<usize> = Vec::new();
        for idx in (0..state.entries.len()).rev() {
            if reclaimed_bytes >= hint.target_bytes {
                break;
            }
            let entry = &state.entries[idx];
            // A zero-byte entry frees no memory toward a BYTE target; evicting it is
            // pure data loss that never advances `reclaimed_bytes`. Skip it so a
            // positive target can never wipe an entire store of zero-byte entries
            // (over-eviction). It stays resident; only byte-bearing entries are swept.
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
        for idx in &evict_indices {
            let removed = state.entries.remove(*idx);
            state.committed_bytes = state.committed_bytes.saturating_sub(removed.charged_bytes);
        }
        Ok(SweepStats { reclaimed_items, reclaimed_bytes })
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

    fn keys(&self) -> Vec<Key>
    where
        Key: Clone + Ord,
    {
        let state = match self.lock() {
            Ok(state) => state,
            // Fail-closed: a poisoned lock yields no keys; accounting/state is
            // unobservable, so we never fabricate a partial key set.
            Err(_) => return Vec::new(),
        };
        let mut keys: Vec<Key> = state.entries.iter().map(|entry| entry.key.clone()).collect();
        // Deterministic, key-sorted order independent of insertion order.
        keys.sort();
        keys
    }

    fn entries(&self) -> Vec<(Key, Value)>
    where
        Key: Clone + Ord,
        Value: Clone,
    {
        let state = match self.lock() {
            Ok(state) => state,
            Err(_) => return Vec::new(),
        };
        let mut entries: Vec<(Key, Value)> =
            state.entries.iter().map(|entry| (entry.key.clone(), entry.value.clone())).collect();
        // Same deterministic key order as `keys`.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};

    fn store() -> SimpleRetainedMapStore<String, Vec<u8>> {
        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-map".to_string(), None);
        SimpleRetainedMapStore::new(binding)
    }

    #[test]
    fn insert_commits_envelope_bytes() {
        let store = store();
        store.insert_checked("k1".to_string(), vec![1u8, 2, 3]).expect("insert");
        // reserve -> mutate -> commit: committed bytes == envelope byte size.
        assert_eq!(store.committed_bytes().expect("committed"), 3);
        assert_eq!(store.stats(), StoreStats { item_count: 1, estimated_bytes: 3 });
        assert_eq!(store.get(&"k1".to_string()), Some(vec![1u8, 2, 3]));
    }

    #[test]
    fn put_get_remove_get_none_round_trip_corrects_bytes() {
        // scenario F: Simple* store put -> get -> remove -> get(None) round-trip
        // with byte-accounting correction at each step.
        let store = store();
        // get on an empty store is None and accounting is zero.
        assert_eq!(store.get(&"k".to_string()), None);
        assert_eq!(store.committed_bytes().expect("empty"), 0);

        store.insert_checked("k".to_string(), vec![1u8, 2, 3, 4]).expect("put");
        // get returns the stored value; bytes reflect exactly the value size.
        assert_eq!(store.get(&"k".to_string()), Some(vec![1u8, 2, 3, 4]));
        assert_eq!(store.committed_bytes().expect("after put"), 4);

        let removed = store.remove(&"k".to_string()).expect("remove");
        assert_eq!(removed, Some(vec![1u8, 2, 3, 4]), "remove returns the stored value");
        // After remove: get(None) and bytes corrected back to zero.
        assert_eq!(store.get(&"k".to_string()), None, "removed key must not be retrievable");
        assert_eq!(store.committed_bytes().expect("after remove"), 0);
        assert_eq!(store.stats(), StoreStats { item_count: 0, estimated_bytes: 0 });
    }

    // --- Audit U: the map Key is `PartialEq`-only (not necessarily bit-identical
    // for equal keys). A second put of a key that is `==` an existing key must
    // overwrite in place and must NOT double-count bytes. ---

    #[derive(Clone, Debug)]
    struct CaseInsensitiveKey {
        // The canonical (lowercased) form drives equality; `raw` can differ for
        // two equal keys, exercising the PartialEq-only contract.
        raw: String,
    }

    impl PartialEq for CaseInsensitiveKey {
        fn eq(&self, other: &Self) -> bool {
            self.raw.eq_ignore_ascii_case(&other.raw)
        }
    }

    #[test]
    fn partial_eq_only_duplicate_key_overwrites_and_does_not_double_count() {
        // scenario F (audit U): two keys equal under PartialEq but not bit-identical
        // ("Key" vs "KEY") must collapse to one live entry with one live charge.
        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            "owner-partialeq".to_string(),
            None,
        );
        let store: SimpleRetainedMapStore<CaseInsensitiveKey, Vec<u8>> =
            SimpleRetainedMapStore::new(binding);

        store
            .insert_checked(CaseInsensitiveKey { raw: "Key".to_string() }, vec![0u8; 10])
            .expect("first put");
        assert_eq!(store.committed_bytes().expect("c1"), 10);
        assert_eq!(store.stats().item_count, 1);

        // Re-put under an equal-but-not-identical key form: overwrite in place.
        store
            .insert_checked(CaseInsensitiveKey { raw: "KEY".to_string() }, vec![0u8; 3])
            .expect("overwrite put");
        assert_eq!(
            store.committed_bytes().expect("c2"),
            3,
            "PartialEq-equal duplicate must overwrite, never double-count bytes"
        );
        assert_eq!(store.stats().item_count, 1, "equal keys must collapse to one entry");
        // The new value is observable under either equal key form.
        assert_eq!(
            store.get(&CaseInsensitiveKey { raw: "key".to_string() }),
            Some(vec![0u8; 3]),
            "get under any equal key form returns the overwritten value"
        );
    }

    #[test]
    fn reinsert_same_key_recharges_to_new_size_not_doubled() {
        let store = store();
        store.insert_checked("k".to_string(), vec![0u8; 10]).expect("insert");
        assert_eq!(store.committed_bytes().expect("c1"), 10);
        // Replace with a smaller value: committed bytes track the live charge only,
        // never the sum of both inserts.
        store.insert_checked("k".to_string(), vec![0u8; 4]).expect("reinsert");
        assert_eq!(
            store.committed_bytes().expect("c2"),
            4,
            "replacement must not double-charge the key"
        );
        assert_eq!(store.stats().item_count, 1);
    }

    #[test]
    fn remove_releases_committed_bytes() {
        let store = store();
        store.insert_checked("k".to_string(), vec![9u8; 7]).expect("insert");
        let removed = store.remove(&"k".to_string()).expect("remove");
        assert_eq!(removed, Some(vec![9u8; 7]));
        // NEGATIVE-PAIR: removing the only entry must drop committed bytes to 0.
        assert_eq!(store.committed_bytes().expect("released"), 0);
        assert_eq!(store.stats().item_count, 0);
        // Removing again returns None and keeps accounting at 0.
        assert_eq!(store.remove(&"k".to_string()).expect("remove missing"), None);
        assert_eq!(store.committed_bytes().expect("still released"), 0);
    }

    #[test]
    fn sweep_reclaims_at_least_target_bytes_and_releases_them() {
        let store = store();
        store.insert_checked("a".to_string(), vec![0u8; 5]).expect("a");
        store.insert_checked("b".to_string(), vec![0u8; 5]).expect("b");
        store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
        assert_eq!(store.committed_bytes().expect("before"), 15);

        let stats = store.sweep(SweepHint { target_bytes: 6 }).expect("sweep");
        // Must reclaim until >= target: two entries (10 bytes) cover a 6-byte target.
        assert_eq!(stats.reclaimed_items, 2);
        assert_eq!(stats.reclaimed_bytes, 10);
        assert_eq!(store.committed_bytes().expect("after sweep"), 5);
        assert_eq!(store.stats().item_count, 1);
    }

    #[test]
    fn byte_targeted_sweep_does_not_over_evict_zero_byte_entries() {
        // Regression: a byte-targeted sweep must skip zero-byte entries (they free
        // no memory toward the target) rather than wiping them. Here only the one
        // byte-bearing entry should be swept; the two zero-byte entries survive.
        let store = store();
        store.insert_checked("a".to_string(), vec![]).expect("a"); // 0 bytes
        store.insert_checked("b".to_string(), vec![]).expect("b"); // 0 bytes
        store.insert_checked("c".to_string(), vec![0u8; 8]).expect("c"); // 8 bytes
        assert_eq!(store.committed_bytes().expect("before"), 8);

        let stats = store.sweep(SweepHint { target_bytes: 6 }).expect("sweep");
        assert_eq!(stats.reclaimed_items, 1, "only the byte-bearing entry is swept");
        assert_eq!(stats.reclaimed_bytes, 8);
        assert_eq!(store.stats().item_count, 2, "both zero-byte entries survive the byte sweep");
        assert!(
            store.get(&"a".to_string()).is_some() && store.get(&"b".to_string()).is_some(),
            "zero-byte entries are retained, not over-evicted"
        );
    }

    #[test]
    fn byte_targeted_sweep_of_all_zero_byte_entries_reclaims_nothing() {
        // The pathological case: a store of only zero-byte entries can never reach a
        // POSITIVE byte target, so the sweep must reclaim nothing — NOT wipe the
        // whole store trying (and failing) to hit the target.
        let store = store();
        for k in ["a", "b", "c"] {
            store.insert_checked(k.to_string(), vec![]).expect("zero-byte insert");
        }
        let stats = store.sweep(SweepHint { target_bytes: 6 }).expect("sweep");
        assert_eq!(stats.reclaimed_items, 0, "zero-byte entries free no bytes; none swept");
        assert_eq!(
            store.stats().item_count,
            3,
            "a positive byte target must not wipe a zero-byte store"
        );
    }

    #[test]
    fn get_missing_key_is_none() {
        let store = store();
        assert_eq!(store.get(&"absent".to_string()), None);
    }

    #[test]
    fn keys_lists_inserted_keys_in_deterministic_sorted_order() {
        let store = store();
        // Insert OUT of sorted order to prove enumeration sorts, not echoes
        // insertion order.
        store.insert_checked("c".to_string(), vec![0u8; 1]).expect("c");
        store.insert_checked("a".to_string(), vec![0u8; 1]).expect("a");
        store.insert_checked("b".to_string(), vec![0u8; 1]).expect("b");
        assert_eq!(
            store.keys(),
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            "keys must be returned in deterministic key-sorted order"
        );
    }

    #[test]
    fn entries_lists_pairs_in_same_sorted_order_as_keys() {
        let store = store();
        store.insert_checked("z".to_string(), vec![9u8]).expect("z");
        store.insert_checked("m".to_string(), vec![7u8, 7]).expect("m");
        let entries = store.entries();
        assert_eq!(
            entries,
            vec![("m".to_string(), vec![7u8, 7]), ("z".to_string(), vec![9u8])],
            "entries must be key-sorted and carry the stored value"
        );
        // The entry key order matches keys().
        let entry_keys: Vec<String> = entries.into_iter().map(|(k, _)| k).collect();
        assert_eq!(entry_keys, store.keys());
    }

    #[test]
    fn keys_after_remove_drops_the_removed_key() {
        let store = store();
        store.insert_checked("a".to_string(), vec![0u8]).expect("a");
        store.insert_checked("b".to_string(), vec![0u8]).expect("b");
        assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string()]);
        store.remove(&"a".to_string()).expect("remove a");
        // NEGATIVE-PAIR: a removed key must no longer enumerate.
        assert_eq!(store.keys(), vec!["b".to_string()]);
    }

    #[test]
    fn keys_on_empty_store_is_empty() {
        let store = store();
        assert!(store.keys().is_empty(), "empty store must enumerate no keys");
        assert!(store.entries().is_empty(), "empty store must enumerate no entries");
    }

    // --- Typed value: an arbitrary `Clone + ByteSized` value round-trips and the
    // broker accounting reflects the value's own byte size. ---

    #[derive(Clone, Debug, PartialEq)]
    struct TypedRecord {
        tag: u32,
        payload: Vec<u8>,
    }

    impl crate::shared::measure::ByteSized for TypedRecord {
        fn byte_size(&self) -> u64 {
            // 4 bytes for the tag + the payload length.
            4 + self.payload.len() as u64
        }
    }

    // --- F#3 completion: a CHARGED retained map routes its bytes through the broker
    // ledger (the accounting SSOT); the explicit uncharged mode does not. ---

    #[cfg(feature = "core-live-accounting")]
    #[test]
    fn uncharged_retained_map_bytes_are_not_reflected_in_a_broker_ledger_snapshot() {
        // scenario F#3 (explicit non-budgeted mode): a SimpleRetainedMapStore built
        // WITHOUT `charged_against` self-accounts; its committed bytes live only in
        // the store's own counter and are invisible to any broker ledger snapshot.
        // This is the intentional non-budgeted path, kept distinct from the charged
        // path below.
        use quanta_memory_platform_core::{
            EffectiveMemoryBudgetReport, ExecutionBudget, LiveProcessMemoryBroker,
            MemoryBudgetManifest, MemoryPreset, ResidencyPolicy,
        };

        let report = EffectiveMemoryBudgetReport::from_manifest(
            MemoryBudgetManifest::new(
                MemoryPreset::Production,
                ExecutionBudget::default(),
                ResidencyPolicy::new(1_000_000),
            ),
            vec!["test".to_string()],
        );
        let broker = LiveProcessMemoryBroker::from_report(report).expect("valid broker");

        // No `charged_against`: explicit non-budgeted mode.
        let store = store();
        store.insert_checked("k".to_string(), vec![0u8; 4096]).expect("insert");

        // The store sees its own bytes...
        assert_eq!(store.committed_bytes().expect("store bytes"), 4096);
        // ...but the broker ledger (the SSOT) sees none of them: an uncharged store
        // never wires its bytes into the ledger.
        assert_eq!(
            broker.ledger_snapshot().expect("snap").total_owner_bytes(),
            0,
            "an uncharged retained map must not route bytes through the broker ledger"
        );
    }

    #[test]
    fn typed_value_round_trips_and_accounts_its_own_byte_size() {
        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-typed".to_string(), None);
        let store: SimpleRetainedMapStore<String, TypedRecord> =
            SimpleRetainedMapStore::new(binding);
        let record = TypedRecord { tag: 7, payload: vec![1u8, 2, 3, 4, 5] };
        store.insert_checked("rec".to_string(), record.clone()).expect("insert typed");
        // get returns the exact typed value (no codec needed).
        assert_eq!(store.get(&"rec".to_string()), Some(record));
        // Broker accounting charges the value's own byte_size (4 + 5).
        assert_eq!(store.committed_bytes().expect("committed"), 9);
        assert_eq!(store.stats(), StoreStats { item_count: 1, estimated_bytes: 9 });
    }

    // --- Opaque handle: `Arc<dyn Any + Send + Sync>` wrapped in MeasuredValue
    // (caller-supplied size) round-trips and is accounted by the broker. ---

    #[test]
    fn measured_any_handle_round_trips_and_accounts_caller_size() {
        use crate::shared::measure::MeasuredValue;
        use std::any::Any;
        use std::sync::Arc;

        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-handle".to_string(), None);
        let store: SimpleRetainedMapStore<String, MeasuredValue<Arc<dyn Any + Send + Sync>>> =
            SimpleRetainedMapStore::new(binding);

        let handle: Arc<dyn Any + Send + Sync> = Arc::new(vec![10u32, 20, 30]);
        // Caller computes the size accounting explicitly (4096 bytes here).
        store
            .insert_checked("h".to_string(), MeasuredValue::new(handle.clone(), 4096))
            .expect("insert handle");

        // get returns the same Arc; downcast proves identity of the carried value.
        let got = store.get(&"h".to_string()).expect("handle present");
        assert_eq!(got.size(), 4096, "wrapper carries the caller size");
        // Same Arc allocation (resume-state / whole-program-authority identity):
        // the retrieved erased Arc points at the original allocation.
        assert!(Arc::ptr_eq(&handle, got.value()), "retrieved handle must be the same Arc");
        let downcast = got.value().clone().downcast::<Vec<u32>>().expect("downcast to original");
        assert_eq!(*downcast, vec![10u32, 20, 30]);

        // Broker accounting reflects the caller-supplied size, not Arc::size_of.
        assert_eq!(store.committed_bytes().expect("committed"), 4096);
        assert_eq!(store.stats(), StoreStats { item_count: 1, estimated_bytes: 4096 });

        // NEGATIVE-PAIR: removing the handle releases the caller-supplied charge.
        let removed =
            store.remove(&"h".to_string()).expect("remove handle").expect("handle present");
        assert_eq!(removed.size(), 4096);
        assert_eq!(store.committed_bytes().expect("released"), 0);
        assert!(store.get(&"h".to_string()).is_none(), "removed handle must not be retrievable");
    }

    // ---- #3: a charged retained map routes real bytes through the broker ledger ----
    #[cfg(feature = "core-live-accounting")]
    mod charged {
        use super::*;
        use crate::shared::accounting::SharedAccountingSink;
        use quanta_memory_platform_core::{
            EffectiveMemoryBudgetReport, ExecutionBudget, LiveProcessMemoryBroker,
            MemoryBudgetManifest, MemoryClassBudget, MemoryPreset, ReclaimPriority,
            ResidencyPolicy,
        };
        use std::sync::Arc;

        fn live_broker(class_hard: u64) -> LiveProcessMemoryBroker {
            let residency = ResidencyPolicy::new(10_000_000).with_class_budget(
                MemoryClass::MemoResidency,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: class_hard,
                    hard_cap_bytes: class_hard,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            );
            let report = EffectiveMemoryBudgetReport::from_manifest(
                MemoryBudgetManifest::new(
                    MemoryPreset::Production,
                    ExecutionBudget::default(),
                    residency,
                ),
                vec!["test".to_string()],
            );
            LiveProcessMemoryBroker::from_report(report).expect("valid broker")
        }

        fn charged(broker: &LiveProcessMemoryBroker) -> SimpleRetainedMapStore<String, Vec<u8>> {
            let sink: SharedAccountingSink = Arc::new(broker.clone());
            let binding =
                StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-map".to_string(), None);
            SimpleRetainedMapStore::new(binding).charged_against(sink)
        }

        #[test]
        fn charged_insert_routes_bytes_through_the_ledger() {
            // (a) charged mode: the ledger (SSOT) sees the map's real bytes, an
            // overwrite re-charges to the new size without double-counting, and
            // remove refunds the ledger.
            let broker = live_broker(1_000_000);
            let store = charged(&broker);

            store.insert_checked("k".to_string(), vec![0u8; 100]).expect("insert within cap");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                100,
                "the ledger must see the charged map's bytes"
            );

            // Overwrite in place with a SMALLER value: the ledger tracks the live
            // charge only (40), never the sum of both inserts.
            store.insert_checked("k".to_string(), vec![0u8; 40]).expect("overwrite");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                40,
                "an overwrite must re-charge to the new size, not double-count"
            );

            // remove refunds the live charge back to the ledger.
            store.remove(&"k".to_string()).expect("remove");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                0,
                "remove must refund the ledger"
            );
        }

        #[test]
        fn charged_insert_over_cap_is_denied_and_fails_closed() {
            // (b) over-cap insert is denied and fails closed: capture ledger + store
            // state before the denied op and assert both are byte-for-byte unchanged.
            let broker = live_broker(50); // MemoResidency hard cap 50 bytes
            let store = charged(&broker);

            // Seed a surviving entry so the denied op cannot quietly drop it.
            store.insert_checked("keep".to_string(), vec![0u8; 40]).expect("seed within cap");
            let ledger_before = broker.ledger_snapshot().expect("snap").total_owner_bytes();
            let committed_before = store.committed_bytes().expect("committed before");
            let stats_before = store.stats();
            assert_eq!(ledger_before, 40);

            // 40 (existing) + 20 = 60 > 50 hard cap: the new insert must be denied.
            let denied = store.insert_checked("big".to_string(), vec![0u8; 20]);
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "an over-cap insert must be a typed capacity error, got {denied:?}"
            );
            // Fail-closed: ledger, store counter, and stats all unchanged; the
            // surviving entry is intact.
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                ledger_before,
                "a denied insert must leave the ledger byte-for-byte unchanged"
            );
            assert_eq!(store.committed_bytes().expect("committed after"), committed_before);
            assert_eq!(store.stats(), stats_before);
            assert_eq!(store.get(&"keep".to_string()), Some(vec![0u8; 40]), "survivor intact");
        }

        #[test]
        fn charged_overwrite_denial_leaves_existing_entry_unchanged() {
            // (b, overwrite variant) a denied overwrite of an existing key must NOT
            // disturb the existing entry's value or its ledger charge — fail-closed
            // before any mutation or refund of the old charge.
            let broker = live_broker(50);
            let store = charged(&broker);
            store.insert_checked("k".to_string(), vec![1u8; 30]).expect("seed");
            let ledger_before = broker.ledger_snapshot().expect("snap").total_owner_bytes();
            assert_eq!(ledger_before, 30);

            // Re-insert the same key with a value that, charged BEFORE the old refund,
            // would push 30 + 60 = 90 > 50 over the cap: must be denied.
            let denied = store.insert_checked("k".to_string(), vec![2u8; 60]);
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "over-cap overwrite must be denied, got {denied:?}"
            );
            // The old entry, its value, and its ledger charge survive untouched.
            assert_eq!(store.get(&"k".to_string()), Some(vec![1u8; 30]), "old value survives");
            assert_eq!(store.committed_bytes().expect("committed"), 30);
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                ledger_before,
                "a denied overwrite must not refund the old charge"
            );
        }

        #[test]
        fn entry_remains_refundable_after_a_denied_overwrite() {
            // After a denied overwrite the entry must still hold a LIVE ledger
            // ticket (the original, or the broker-restored one) — never a stranded
            // non-live ticket. Proven end-to-end: a follow-up remove drains the
            // ledger to zero (a non-live ticket would be rejected and strand bytes).
            let broker = live_broker(50);
            let store = charged(&broker);
            store.insert_checked("k".to_string(), vec![1u8; 30]).expect("seed");

            let denied = store.insert_checked("k".to_string(), vec![2u8; 60]);
            assert!(matches!(denied, Err(MemoryError::Capacity(_))), "over-cap overwrite denied");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 30);

            // The surviving entry's ticket is refundable: removing it drains the ledger.
            let removed = store.remove(&"k".to_string()).expect("remove must succeed");
            assert_eq!(removed, Some(vec![1u8; 30]), "remove returns the surviving value");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                0,
                "the post-denial ticket was live and refundable (no strand)"
            );
        }

        #[test]
        fn charged_overwrite_during_rebuild_is_fail_closed_and_leaves_entry_intact() {
            // A charged overwrite while the broker is rebuilding must fail closed
            // (the recharge's release-old hits the rebuild gate), leaving the old
            // value and its ledger charge byte-for-byte intact — and the entry's
            // ticket still live so the value survives and is refundable after.
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("k".to_string(), vec![1u8; 40]).expect("seed");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 40);

            broker.begin_rebuild().expect("begin rebuild");
            let denied = store.insert_checked("k".to_string(), vec![2u8; 20]);
            assert!(
                matches!(denied, Err(MemoryError::InvalidState(_))),
                "overwrite during rebuild must be rejected, got {denied:?}"
            );
            assert_eq!(store.get(&"k".to_string()), Some(vec![1u8; 40]), "old value intact");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 40);

            // After rebuild the entry is still live: it overwrites and removes cleanly.
            broker.finish_rebuild().expect("finish");
            store.insert_checked("k".to_string(), vec![2u8; 20]).expect("overwrite resumes");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 20);
            store.remove(&"k".to_string()).expect("remove");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
        }

        #[test]
        fn charged_sweep_refunds_reclaimed_bytes_to_the_ledger() {
            // sweep must refund every reclaimed entry's bytes to the ledger.
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("a".to_string(), vec![0u8; 5]).expect("a");
            store.insert_checked("b".to_string(), vec![0u8; 5]).expect("b");
            store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 15);

            let stats = store.sweep(SweepHint { target_bytes: 6 }).expect("sweep");
            assert_eq!(stats.reclaimed_items, 2, "two 5-byte entries cover a 6-byte target");
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                5,
                "sweep must refund the reclaimed bytes to the ledger"
            );
        }

        #[test]
        fn charged_remove_during_rebuild_is_fail_closed() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("k".to_string(), vec![0u8; 7]).expect("seed");
            broker.begin_rebuild().expect("begin rebuild");

            let denied = store.remove(&"k".to_string());
            assert!(
                matches!(denied, Err(MemoryError::InvalidState(_))),
                "remove during rebuild must fail closed, got {denied:?}"
            );
            assert_eq!(
                store.get(&"k".to_string()),
                Some(vec![0u8; 7]),
                "entry must survive denial"
            );
            assert_eq!(store.committed_bytes().expect("committed"), 7);
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 7);
        }

        #[test]
        fn charged_sweep_during_rebuild_is_fail_closed() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("a".to_string(), vec![0u8; 5]).expect("a");
            store.insert_checked("b".to_string(), vec![0u8; 5]).expect("b");
            store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
            broker.begin_rebuild().expect("begin rebuild");

            let denied = store.sweep(SweepHint { target_bytes: 6 });
            assert!(
                matches!(denied, Err(MemoryError::InvalidState(_))),
                "sweep during rebuild must fail closed, got {denied:?}"
            );
            assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string(), "c".to_string()]);
            assert_eq!(store.committed_bytes().expect("committed"), 15);
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 15);
        }

        #[test]
        fn uncharged_store_is_explicit_non_budgeted_mode() {
            // (c) no sink: the store self-accounts and never touches a ledger.
            let binding =
                StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-map".to_string(), None);
            let store: SimpleRetainedMapStore<String, Vec<u8>> =
                SimpleRetainedMapStore::new(binding);
            store.insert_checked("k".to_string(), vec![0u8; 7]).expect("insert");
            assert_eq!(store.committed_bytes().expect("committed"), 7);
        }
    }
}
