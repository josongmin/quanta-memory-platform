use std::collections::hash_map::RandomState;
use std::hash::Hash;
use std::sync::Mutex;

use hashlink::linked_hash_map::Entry;
use hashlink::LinkedHashMap;

use quanta_memory_platform_core::{
    ChargeKind, MemoryContextKind, MemoryError, ReservationTicket, StoreBindingHandle, StoreStats,
    StoreWriteError, SweepHint, SweepStats,
};

use crate::retained_map::{RetainedMapInsertOutcomeV1, RetainedMapStore};
use crate::shared::accounting::SharedAccountingSink;
use crate::shared::measure::ByteSized;

#[derive(Debug)]
struct MapEntry<Value> {
    value: Value,
    charged_bytes: u64,
    /// Ledger receipt when the store is charged against a broker; `None` in
    /// non-budgeted mode. Refunded exactly once when the entry leaves the map
    /// (remove / overwrite / sweep).
    ticket: Option<ReservationTicket>,
}

#[derive(Debug)]
struct MapState<Key, Value> {
    /// Single canonical payload/index/order owner. Randomized hashing prevents
    /// attacker-selected keys from deterministically degrading point operations;
    /// the linked order preserves newest-first sweep semantics.
    entries: LinkedHashMap<Key, MapEntry<Value>, RandomState>,
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
///
/// Charged construction is builder-only so a ledger sink can never be attached
/// after uncharged entries already exist:
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{SharedAccountingSink, SimpleRetainedMapStore};
/// use quanta_memory_platform_core::StoreBindingHandle;
///
/// fn attach_after_construction(
///     store: SimpleRetainedMapStore<String, Vec<u8>>,
///     sink: SharedAccountingSink,
/// ) {
///     let _ = store.charged_against(sink);
/// }
/// ```
#[derive(Debug)]
pub struct SimpleRetainedMapStore<Key, Value> {
    binding: StoreBindingHandle,
    charger: Option<SharedAccountingSink>,
    state: Mutex<MapState<Key, Value>>,
}

impl<Key, Value> SimpleRetainedMapStore<Key, Value> {
    pub fn new(binding: StoreBindingHandle) -> Self {
        Self::with_charger(binding, None)
    }

    pub(crate) fn new_charged(binding: StoreBindingHandle, sink: SharedAccountingSink) -> Self {
        Self::with_charger(binding, Some(sink))
    }

    fn with_charger(binding: StoreBindingHandle, charger: Option<SharedAccountingSink>) -> Self {
        Self {
            binding,
            charger,
            state: Mutex::new(MapState {
                entries: LinkedHashMap::with_hasher(RandomState::new()),
                committed_bytes: 0,
            }),
        }
    }
}

impl<Key, Value> SimpleRetainedMapStore<Key, Value>
where
    Key: Clone + Eq + Hash,
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

    fn charge_new_entry_v1(
        &self,
        reserved_bytes: u64,
    ) -> Result<Option<ReservationTicket>, StoreWriteError> {
        self.charger
            .as_ref()
            .map(|sink| {
                sink.charge(
                    self.binding.class.clone(),
                    self.binding.owner.clone(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedMetadata,
                    reserved_bytes,
                )
            })
            .transpose()
    }
}

impl<Key, Value> RetainedMapStore<Key, Value> for SimpleRetainedMapStore<Key, Value>
where
    Key: Clone + Eq + Hash,
    Value: Clone + ByteSized,
{
    fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError> {
        let reserved_bytes = value.byte_size();
        let mut state = self.lock()?;
        let MapState { entries, committed_bytes } = &mut *state;

        match entries.entry(key) {
            Entry::Occupied(mut occupied) => {
                // Mutate an occupied value in place. Unlike an order-refreshing
                // insert, this preserves the original insertion position and thus
                // the store's established newest-first sweep contract.
                match &self.charger {
                    Some(sink) => {
                        let entry = occupied.get_mut();
                        let old_ticket = match entry.ticket.take() {
                            Some(ticket) => ticket,
                            None => {
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
                                *committed_bytes = committed_bytes
                                    .saturating_sub(entry.charged_bytes)
                                    .saturating_add(reserved_bytes);
                                entry.value = value;
                                entry.charged_bytes = reserved_bytes;
                                entry.ticket = Some(new_ticket);
                                Ok(())
                            }
                            Err((err, restored)) => {
                                // Recharge restores the old ledger charge before
                                // returning failure. Publish the restored ticket but
                                // leave payload, bytes, and insertion position intact.
                                entry.ticket = restored.or(Some(old_ticket));
                                Err(err)
                            }
                        }
                    }
                    None => {
                        let entry = occupied.get_mut();
                        *committed_bytes = committed_bytes
                            .saturating_sub(entry.charged_bytes)
                            .saturating_add(reserved_bytes);
                        entry.value = value;
                        entry.charged_bytes = reserved_bytes;
                        Ok(())
                    }
                }
            }
            Entry::Vacant(vacant) => {
                // Charge before publication so a denied admission leaves payload,
                // order, and local accounting unchanged.
                let new_ticket = self.charge_new_entry_v1(reserved_bytes)?;
                vacant.insert(MapEntry {
                    value,
                    charged_bytes: reserved_bytes,
                    ticket: new_ticket,
                });
                *committed_bytes = committed_bytes.saturating_add(reserved_bytes);
                Ok(())
            }
        }
    }

    fn insert_if_absent_checked(
        &self,
        key: Key,
        value: Value,
    ) -> Result<RetainedMapInsertOutcomeV1, StoreWriteError> {
        let mut state = self.lock()?;
        let MapState { entries, committed_bytes } = &mut *state;
        match entries.entry(key) {
            Entry::Occupied(_) => Ok(RetainedMapInsertOutcomeV1::AlreadyPresent),
            Entry::Vacant(vacant) => {
                let reserved_bytes = value.byte_size();
                let new_ticket = self.charge_new_entry_v1(reserved_bytes)?;
                vacant.insert(MapEntry {
                    value,
                    charged_bytes: reserved_bytes,
                    ticket: new_ticket,
                });
                *committed_bytes = committed_bytes.saturating_add(reserved_bytes);
                Ok(RetainedMapInsertOutcomeV1::Inserted)
            }
        }
    }

    fn get_checked(&self, key: &Key) -> Result<Option<Value>, MemoryError> {
        let state = self.lock()?;
        Ok(state.entries.get(key).map(|entry| entry.value.clone()))
    }

    fn remove(&self, key: &Key) -> Result<Option<Value>, MemoryError> {
        let mut state = self.lock()?;
        let Some(entry) = state.entries.get(key) else {
            return Ok(None);
        };
        if let Some(sink) = &self.charger {
            let ticket = entry.ticket.as_ref().ok_or_else(|| {
                MemoryError::InvalidState(
                    "charged retained-map entry has no ticket on remove: charge invariant broken"
                        .to_string(),
                )
            })?;
            sink.refund(ticket)?;
        }
        let removed = state.entries.remove(key).ok_or_else(|| {
            MemoryError::InvalidState("retained-map key disappeared during remove".to_string())
        })?;
        state.committed_bytes = state.committed_bytes.saturating_sub(removed.charged_bytes);
        Ok(Some(removed.value))
    }

    fn sweep(&self, hint: SweepHint) -> Result<SweepStats, MemoryError> {
        let mut state = self.lock()?;
        let mut reclaimed_items = 0usize;
        let mut reclaimed_bytes = 0u64;
        let mut refunds: Vec<ReservationTicket> = Vec::new();
        // Keys to evict, collected newest-first from the canonical linked order.
        // Only selected keys are cloned; each publication remove is average O(1).
        let mut evict_keys: Vec<Key> = Vec::new();
        for (key, entry) in state.entries.iter().rev() {
            if reclaimed_bytes >= hint.target_bytes {
                break;
            }
            // A zero-byte entry frees no memory toward a BYTE target; evicting it is
            // pure data loss that never advances `reclaimed_bytes`. Skip it so a
            // positive target can never wipe an entire store of zero-byte entries
            // (over-eviction). It stays resident; only byte-bearing entries are swept.
            if entry.charged_bytes == 0 {
                continue;
            }
            reclaimed_items += 1;
            reclaimed_bytes = reclaimed_bytes.saturating_add(entry.charged_bytes);
            if self.charger.is_some() {
                let ticket = entry.ticket.as_ref().ok_or_else(|| {
                    MemoryError::InvalidState(
                        "charged retained-map entry has no ticket on sweep: charge invariant broken"
                            .to_string(),
                    )
                })?;
                refunds.push(ticket.clone());
            }
            evict_keys.push(key.clone());
        }
        if let Some(sink) = &self.charger {
            sink.refund_many(&refunds)?;
        }
        for key in &evict_keys {
            let removed = state.entries.remove(key).ok_or_else(|| {
                MemoryError::InvalidState(
                    "retained-map sweep key disappeared while its state lock was held".to_string(),
                )
            })?;
            state.committed_bytes = state.committed_bytes.saturating_sub(removed.charged_bytes);
        }
        Ok(SweepStats { reclaimed_items, reclaimed_bytes })
    }

    fn stats_checked(&self) -> Result<StoreStats, MemoryError> {
        let state = self.lock()?;
        Ok(StoreStats { item_count: state.entries.len(), estimated_bytes: state.committed_bytes })
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
        let mut keys: Vec<Key> = state.entries.keys().cloned().collect();
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
            state.entries.iter().map(|(key, entry)| (key.clone(), entry.value.clone())).collect();
        // Same deterministic key order as `keys`.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
    use std::hash::{Hash, Hasher};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    fn store() -> SimpleRetainedMapStore<String, Vec<u8>> {
        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-map".to_string(), None);
        SimpleRetainedMapStore::new(binding)
    }

    #[test]
    fn retained_map_owner_has_one_keyed_container_and_no_linear_point_path() {
        let source = include_str!("service.rs");
        for forbidden in [
            concat!("Vec<", "MapEntry"),
            concat!("Index", "Map"),
            concat!(".iter()", ".position("),
            concat!(".iter()", ".find("),
            concat!("shift_", "remove"),
            concat!("pub fn charged", "_against"),
        ] {
            assert!(
                !source.contains(forbidden),
                "retained-map owner must not regain forbidden linear/dual-authority path: {forbidden}"
            );
        }
        assert!(source.contains("LinkedHashMap<Key, MapEntry<Value>, RandomState>"));
        assert!(source.contains("pub(crate) fn new_charged"));
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
    fn concurrent_insert_if_absent_has_one_creator_and_never_overwrites_v1() {
        const WRITERS: usize = 12;
        let store = Arc::new(store());
        let barrier = Arc::new(Barrier::new(WRITERS));
        let mut joins = Vec::new();
        for writer in 0..WRITERS {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            joins.push(std::thread::spawn(move || {
                barrier.wait();
                let writer_byte = u8::try_from(writer).expect("writer index fits u8");
                store
                    .insert_if_absent_checked("canonical".to_string(), vec![writer_byte])
                    .expect("atomic insert-if-absent")
            }));
        }
        let outcomes: Vec<_> =
            joins.into_iter().map(|join| join.join().expect("writer joins")).collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == RetainedMapInsertOutcomeV1::Inserted)
                .count(),
            1,
            "exactly one caller owns creation"
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == RetainedMapInsertOutcomeV1::AlreadyPresent)
                .count(),
            WRITERS - 1,
            "all losing callers observe an existing row"
        );
        assert_eq!(store.stats().item_count, 1);
        assert_eq!(store.committed_bytes().expect("committed bytes"), 1);
    }

    #[test]
    fn concurrent_checked_read_observes_only_complete_old_or_new_payloads_v1() {
        const READERS: usize = 4;
        const ITERATIONS: usize = 2_000;
        const PAYLOAD_BYTES: usize = 4_096;

        let store = Arc::new(store());
        let key = "concurrent-overwrite".to_string();
        let old = vec![0x11; PAYLOAD_BYTES];
        let new = vec![0xee; PAYLOAD_BYTES];
        store.insert_checked(key.clone(), old.clone()).expect("seed old payload");

        let barrier = Arc::new(Barrier::new(READERS + 1));
        let old_observations = Arc::new(AtomicUsize::new(0));
        let new_observations = Arc::new(AtomicUsize::new(0));
        let invalid_observations = Arc::new(AtomicUsize::new(0));
        let mut readers = Vec::new();
        for _ in 0..READERS {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let key = key.clone();
            let old = old.clone();
            let new = new.clone();
            let old_observations = Arc::clone(&old_observations);
            let new_observations = Arc::clone(&new_observations);
            let invalid_observations = Arc::clone(&invalid_observations);
            readers.push(std::thread::spawn(move || {
                for _ in 0..ITERATIONS {
                    // Reusable two-phase barrier: every read races exactly one
                    // overwrite epoch instead of merely starting near the writer.
                    barrier.wait();
                    match store.get_checked(&key) {
                        Ok(Some(observed)) if observed == old => {
                            old_observations.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(Some(observed)) if observed == new => {
                            new_observations.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(Some(_)) | Ok(None) | Err(_) => {
                            invalid_observations.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    barrier.wait();
                }
            }));
        }

        let mut write_failures = 0usize;
        for iteration in 0..ITERATIONS {
            barrier.wait();
            let payload = if iteration % 2 == 0 { new.clone() } else { old.clone() };
            if store.insert_checked(key.clone(), payload).is_err() {
                write_failures += 1;
            }
            barrier.wait();
        }
        for reader in readers {
            reader.join().expect("reader joins without invalid observation");
        }

        assert_eq!(
            old_observations.load(Ordering::Relaxed) + new_observations.load(Ordering::Relaxed),
            READERS * ITERATIONS,
            "every checked read must classify as one complete published payload"
        );
        assert_eq!(write_failures, 0, "every synchronized overwrite must publish successfully");
        assert_eq!(
            invalid_observations.load(Ordering::Relaxed),
            0,
            "no checked read may observe absence, error, torn bytes, or a foreign generation"
        );
        assert!(
            old_observations.load(Ordering::Relaxed) > 0
                && new_observations.load(Ordering::Relaxed) > 0,
            "the synchronized overwrite epochs must expose both complete generations"
        );
    }

    #[derive(Debug)]
    struct ClonePanicsWhenArmed {
        bytes: Vec<u8>,
        armed: Arc<AtomicBool>,
    }

    impl Clone for ClonePanicsWhenArmed {
        fn clone(&self) -> Self {
            assert!(
                !self.armed.load(Ordering::SeqCst),
                "injected clone panic while retained-map lock is held"
            );
            Self { bytes: self.bytes.clone(), armed: Arc::clone(&self.armed) }
        }
    }

    impl ByteSized for ClonePanicsWhenArmed {
        fn byte_size(&self) -> u64 {
            u64::try_from(self.bytes.len()).expect("test payload length fits u64")
        }
    }

    #[test]
    fn checked_reads_report_typed_invalid_state_after_owner_mutex_poison_v1() {
        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-poison".to_string(), None);
        let store: SimpleRetainedMapStore<String, ClonePanicsWhenArmed> =
            SimpleRetainedMapStore::new(binding);
        let armed = Arc::new(AtomicBool::new(false));
        let key = "poison".to_string();
        store
            .insert_checked(
                key.clone(),
                ClonePanicsWhenArmed { bytes: vec![1, 2, 3], armed: Arc::clone(&armed) },
            )
            .expect("seed panic-capable payload");

        armed.store(true, Ordering::SeqCst);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.get_checked(&key);
        }));
        assert!(panic.is_err(), "clone panic must poison the canonical owner mutex");

        assert!(
            matches!(store.get_checked(&key), Err(MemoryError::InvalidState(message)) if message == "retained map lock poisoned"),
            "checked point read must preserve the typed poison failure"
        );
        assert!(
            matches!(store.stats_checked(), Err(MemoryError::InvalidState(message)) if message == "retained map lock poisoned"),
            "checked stats must preserve the typed poison failure"
        );
        assert_eq!(store.get(&key), None, "advisory get must fail closed after poison");
        assert_eq!(
            store.stats(),
            StoreStats::default(),
            "advisory stats must not fabricate retained usage after poison"
        );
        assert!(store.keys().is_empty(), "advisory keys must expose no partial poisoned state");
        assert!(
            store.entries().is_empty(),
            "advisory entries must expose no partial poisoned state"
        );
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

    // --- Audit U: equal keys need not be bit-identical. A second put of a key
    // that is equal under the lawful Eq + Hash contract must overwrite in place
    // and must NOT double-count bytes. ---

    #[derive(Clone, Debug)]
    struct CaseInsensitiveKey {
        // The canonical (lowercased) form drives equality; `raw` can differ for
        // two equal keys, exercising the Eq + Hash identity law.
        raw: String,
    }

    impl PartialEq for CaseInsensitiveKey {
        fn eq(&self, other: &Self) -> bool {
            self.raw.eq_ignore_ascii_case(&other.raw)
        }
    }

    impl Eq for CaseInsensitiveKey {}

    impl Hash for CaseInsensitiveKey {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.raw.len().hash(state);
            for byte in self.raw.bytes() {
                byte.to_ascii_lowercase().hash(state);
            }
        }
    }

    #[test]
    fn equal_hash_key_duplicate_overwrites_and_does_not_double_count() {
        // scenario F (audit U): two keys equal under Eq but not bit-identical
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
            "Eq-equal duplicate must overwrite, never double-count bytes"
        );
        assert_eq!(store.stats().item_count, 1, "equal keys must collapse to one entry");
        // The new value is observable under either equal key form.
        assert_eq!(
            store.get(&CaseInsensitiveKey { raw: "key".to_string() }),
            Some(vec![0u8; 3]),
            "get under any equal key form returns the overwritten value"
        );
    }

    #[derive(Clone, Debug)]
    struct LookupProbeKey {
        id: u64,
        equality_probes: Arc<AtomicUsize>,
        hash_probes: Arc<AtomicUsize>,
    }

    impl PartialEq for LookupProbeKey {
        fn eq(&self, other: &Self) -> bool {
            self.equality_probes.fetch_add(1, Ordering::Relaxed);
            self.id == other.id
        }
    }

    impl Eq for LookupProbeKey {}

    impl Hash for LookupProbeKey {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.hash_probes.fetch_add(1, Ordering::Relaxed);
            self.id.hash(state);
        }
    }

    fn exercise_endpoint_point_operations_without_linear_probe_growth(endpoint_id: u64) {
        const LIVE_ENTRIES: u64 = 4_096;
        const MAX_EQUALITY_PROBES_PER_POINT_OPERATION: usize = 64;

        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            format!("owner-keyed-lookup-{endpoint_id}"),
            None,
        );
        let equality_probes = Arc::new(AtomicUsize::new(0));
        let hash_probes = Arc::new(AtomicUsize::new(0));
        let store: SimpleRetainedMapStore<LookupProbeKey, Vec<u8>> =
            SimpleRetainedMapStore::new(binding);

        for id in 0..LIVE_ENTRIES {
            store
                .insert_checked(
                    LookupProbeKey {
                        id,
                        equality_probes: Arc::clone(&equality_probes),
                        hash_probes: Arc::clone(&hash_probes),
                    },
                    vec![u8::try_from(id % 256).expect("modulo value fits u8")],
                )
                .expect("seed keyed entry");
        }

        equality_probes.store(0, Ordering::Relaxed);
        hash_probes.store(0, Ordering::Relaxed);
        let query = LookupProbeKey {
            id: endpoint_id,
            equality_probes: Arc::clone(&equality_probes),
            hash_probes: Arc::clone(&hash_probes),
        };
        let expected_byte = u8::try_from(endpoint_id % 256).expect("modulo value fits u8");
        assert_eq!(
            store.get_checked(&query).expect("checked endpoint lookup"),
            Some(vec![expected_byte])
        );
        assert!(
            equality_probes.load(Ordering::Relaxed) <= MAX_EQUALITY_PROBES_PER_POINT_OPERATION,
            "endpoint lookup must stay within the documented N-independent equality-probe bound"
        );
        assert_eq!(hash_probes.load(Ordering::Relaxed), 1, "lookup hashes the exact key once");

        equality_probes.store(0, Ordering::Relaxed);
        hash_probes.store(0, Ordering::Relaxed);
        store.insert_checked(query.clone(), vec![7u8; 3]).expect("overwrite exact key");
        assert!(
            equality_probes.load(Ordering::Relaxed) <= MAX_EQUALITY_PROBES_PER_POINT_OPERATION,
            "endpoint overwrite must stay within the documented N-independent equality-probe bound"
        );
        assert_eq!(hash_probes.load(Ordering::Relaxed), 1, "overwrite hashes the exact key once");

        equality_probes.store(0, Ordering::Relaxed);
        hash_probes.store(0, Ordering::Relaxed);
        assert_eq!(store.remove(&query).expect("remove exact key"), Some(vec![7u8; 3]));
        assert!(
            equality_probes.load(Ordering::Relaxed) <= MAX_EQUALITY_PROBES_PER_POINT_OPERATION,
            "endpoint remove must stay within the documented N-independent equality-probe bound"
        );
        assert_eq!(
            hash_probes.load(Ordering::Relaxed),
            2,
            "fail-closed remove validates then removes by exact key"
        );
    }

    #[test]
    fn first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1() {
        exercise_endpoint_point_operations_without_linear_probe_growth(0);
        exercise_endpoint_point_operations_without_linear_probe_growth(4_095);
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
    fn overwrite_preserves_original_insertion_position_for_newest_first_sweep() {
        let store = store();
        store.insert_checked("a".to_string(), vec![1]).expect("a");
        store.insert_checked("b".to_string(), vec![2]).expect("b");
        store.insert_checked("c".to_string(), vec![3]).expect("c");

        // Updating b must not turn it into the newest entry.
        store.insert_checked("b".to_string(), vec![9, 9]).expect("overwrite b");
        let swept = store.sweep(SweepHint { target_bytes: 1 }).expect("sweep newest");

        assert_eq!(swept, SweepStats { reclaimed_items: 1, reclaimed_bytes: 1 });
        assert_eq!(store.get(&"b".to_string()), Some(vec![9, 9]), "overwritten b stays resident");
        assert_eq!(store.get(&"c".to_string()), None, "original newest c is evicted first");
        assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn arbitrary_remove_preserves_remaining_newest_first_sweep_order() {
        let store = store();
        for key in ["a", "b", "c", "d"] {
            store.insert_checked(key.to_string(), vec![1]).expect("seed");
        }

        assert_eq!(store.remove(&"b".to_string()).expect("remove b"), Some(vec![1]));
        store.sweep(SweepHint { target_bytes: 1 }).expect("sweep d");
        assert_eq!(store.get(&"d".to_string()), None, "d remains newest after removing b");
        assert!(store.get(&"c".to_string()).is_some(), "c must survive the first sweep");

        store.sweep(SweepHint { target_bytes: 1 }).expect("sweep c");
        assert_eq!(store.get(&"c".to_string()), None, "c is next newest");
        assert_eq!(store.keys(), vec!["a".to_string()]);
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
            SimpleRetainedMapStore::new_charged(binding, sink)
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
        fn atomic_insert_if_absent_never_recharges_or_overwrites_an_existing_entry_v1() {
            let broker = live_broker(50);
            let store = charged(&broker);
            store.insert_checked("canonical".to_string(), vec![1u8; 30]).expect("seed");

            // The losing candidate is larger than the remaining budget. An
            // occupied decision must happen before charging, so it still returns
            // AlreadyPresent and leaves both authorities byte-for-byte intact.
            assert_eq!(
                store
                    .insert_if_absent_checked("canonical".to_string(), vec![2u8; 60])
                    .expect("occupied publication is not an admission attempt"),
                RetainedMapInsertOutcomeV1::AlreadyPresent
            );
            assert_eq!(
                store.get_checked(&"canonical".to_string()).expect("checked read"),
                Some(vec![1u8; 30]),
                "a losing publisher must not replace the canonical payload"
            );
            assert_eq!(store.stats_checked().expect("checked stats").estimated_bytes, 30);
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                30,
                "a losing publisher must not charge or refund the ledger"
            );
        }

        #[test]
        fn denied_atomic_insert_if_absent_publishes_neither_payload_nor_charge_v1() {
            let broker = live_broker(50);
            let store = charged(&broker);
            store.insert_checked("keep".to_string(), vec![0u8; 40]).expect("seed");
            let ledger_before = broker.ledger_snapshot().expect("snap").total_owner_bytes();
            let stats_before = store.stats_checked().expect("checked stats before");

            let denied = store.insert_if_absent_checked("denied".to_string(), vec![0u8; 20]);
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "a denied atomic publication must preserve the typed error: {denied:?}"
            );
            assert_eq!(store.stats_checked().expect("checked stats after"), stats_before);
            assert_eq!(
                store.get_checked(&"denied".to_string()).expect("checked absent read"),
                None,
                "charge denial must not publish a payload"
            );
            assert_eq!(
                broker.ledger_snapshot().expect("snap").total_owner_bytes(),
                ledger_before,
                "charge denial must not mutate the accounting authority"
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
        fn charged_full_eviction_leaves_no_entry_key_ticket_or_accounting_residue_v1() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("a".to_string(), vec![0u8; 2]).expect("a");
            store.insert_checked("b".to_string(), vec![0u8; 3]).expect("b");
            store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
            assert_eq!(broker.ledger_snapshot().expect("before").total_owner_bytes(), 10);

            let swept = store.sweep(SweepHint { target_bytes: 10 }).expect("full sweep");
            assert_eq!(swept, SweepStats { reclaimed_items: 3, reclaimed_bytes: 10 });
            assert_eq!(store.stats_checked().expect("checked empty stats"), StoreStats::default());
            assert!(store.keys().is_empty(), "full eviction leaves no reachable key");
            assert!(store.entries().is_empty(), "full eviction leaves no reachable entry");
            assert_eq!(store.committed_bytes().expect("empty committed bytes"), 0);
            assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
            assert!(
                store.state.lock().expect("state").entries.is_empty(),
                "the canonical owner contains no entry and therefore no live ticket"
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
        fn charged_remove_rejects_a_ticketless_entry_without_mutating_state() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            store.insert_checked("k".to_string(), vec![0u8; 7]).expect("seed");
            store.state.lock().expect("state").entries.get_mut("k").expect("entry").ticket = None;

            let denied = store.remove(&"k".to_string());
            assert!(matches!(denied, Err(MemoryError::InvalidState(_))), "got {denied:?}");
            assert_eq!(store.get(&"k".to_string()), Some(vec![0u8; 7]));
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

            // A failed refund_many must also leave internal insertion order intact.
            // Once the accounting gate reopens, the original newest entry is still
            // the first one reclaimed.
            broker.finish_rebuild().expect("finish rebuild");
            let swept = store.sweep(SweepHint { target_bytes: 5 }).expect("sweep after retry");
            assert_eq!(swept, SweepStats { reclaimed_items: 1, reclaimed_bytes: 5 });
            assert_eq!(store.get(&"c".to_string()), None, "c remains newest after denial");
            assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string()]);
            assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 10);
        }

        #[test]
        fn charged_sweep_rejects_a_ticketless_candidate_without_partial_refund_or_eviction() {
            let broker = live_broker(1_000_000);
            let store = charged(&broker);
            for key in ["a", "b", "c"] {
                store.insert_checked(key.to_string(), vec![0u8; 5]).expect("seed");
            }
            store.state.lock().expect("state").entries.get_mut("b").expect("entry").ticket = None;

            // c is visited first and has a ticket, then b exposes corruption.
            // No refund_many call or keyed removal may happen before all selected
            // entries have proven their ticket invariant.
            let denied = store.sweep(SweepHint { target_bytes: 10 });
            assert!(matches!(denied, Err(MemoryError::InvalidState(_))), "got {denied:?}");
            let insertion_order =
                store.state.lock().expect("state").entries.keys().cloned().collect::<Vec<_>>();
            assert_eq!(
                insertion_order,
                vec!["a".to_string(), "b".to_string(), "c".to_string()],
                "ticket validation failure must preserve canonical linked order"
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
