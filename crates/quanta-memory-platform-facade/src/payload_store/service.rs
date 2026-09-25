use std::collections::HashMap;
use std::hash::{Hash, Hasher};
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
use crate::shared::lifecycle::{
    invoke_client_read_while_locked, retire_client_value, BeginClose, DropTransition,
    StoreLifecycle,
};
use crate::shared::measure::{semantic_cost_v1, ByteSized, CanonicalKey, RetainedCostV1};

fn canonical_digest(key: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

fn reference_digest<PayloadRef: Hash>(payload_ref: &PayloadRef) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    payload_ref.hash(&mut hasher);
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

fn retire_entry<PayloadRef, Bytes>(entry: PayloadEntry<PayloadRef, Bytes>) {
    let PayloadEntry { payload_ref, bytes, charge, .. } = entry;
    drop(charge);
    retire_client_value(payload_ref);
    retire_client_value(bytes);
}

fn retire_entries<PayloadRef, Bytes>(entries: Vec<Option<PayloadEntry<PayloadRef, Bytes>>>) {
    for entry in entries.into_iter().flatten() {
        retire_entry(entry);
    }
}

#[derive(Debug)]
struct PayloadState<PayloadRef, Bytes> {
    /// Stable slots avoid shifting every index on delete. A slot is reusable
    /// only after both internal indices have removed it; no slot identity is
    /// exposed outside the store.
    entries: Vec<Option<PayloadEntry<PayloadRef, Bytes>>>,
    free_slots: Vec<usize>,
    canonical_index: HashMap<u64, Vec<usize>>,
    reference_index: HashMap<u64, Vec<usize>>,
    live_entries: usize,
    next_ref: u64,
    charged_total: u64,
}

fn find_reference_slot<PayloadRef: Eq, Bytes>(
    state: &PayloadState<PayloadRef, Bytes>,
    digest: u64,
    payload_ref: &PayloadRef,
) -> Option<usize> {
    state.reference_index.get(&digest).and_then(|slots| {
        slots.iter().copied().find(|slot| {
            state.entries[*slot].as_ref().is_some_and(|entry| &entry.payload_ref == payload_ref)
        })
    })
}

/// Canonical payload store branded by its construction-time accounting mode.
///
/// Materialized stores expose neither raw construction nor late accounting
/// attachment:
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{Budgeted, SimplePayloadStore};
/// let _ = SimplePayloadStore::<String, Vec<u8>, Budgeted>::charged_against;
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
            state: Mutex::new(PayloadState {
                entries: Vec::new(),
                free_slots: Vec::new(),
                canonical_index: HashMap::new(),
                reference_index: HashMap::new(),
                live_entries: 0,
                next_ref: 0,
                charged_total: 0,
            }),
            _mode: PhantomData,
        }
    }

    fn lock(&self) -> MemoryResult<std::sync::MutexGuard<'_, PayloadState<PayloadRef, Bytes>>> {
        self.lifecycle.try_lock_state(&self.state)
    }

    fn context_kind(&self, value: &RetainedEnvelope<Bytes>) -> MemoryContextKind {
        value
            .context
            .as_ref()
            .or(self.context.as_ref())
            .map_or(MemoryContextKind::Request, |handle| handle.kind)
    }

    pub fn charged_bytes(&self) -> MemoryResult<u64> {
        self.lifecycle.with_read_state(&self.state, |state| state.charged_total)
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
                let Some(entry) = entry.as_mut() else {
                    continue;
                };
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
                state.entries[index].as_mut().expect("close slot remains live").charge =
                    Some(charge);
            }
            close.rollback();
            return Err(error);
        }

        let entries = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.charged_total = 0;
            state.live_entries = 0;
            state.free_slots.clear();
            state.canonical_index.clear();
            state.reference_index.clear();
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
    PayloadRef: Clone + From<String> + Eq + Hash + ByteSized,
    Bytes: Clone + ByteSized + CanonicalKey,
{
    pub fn pin(&self, payload_ref: &PayloadRef) -> MemoryResult<()> {
        let _permit = self.lifecycle.begin_mutation()?;
        let digest = reference_digest(payload_ref);
        let index = invoke_client_read_while_locked(self.lock()?, |state| {
            find_reference_slot(state, digest, payload_ref)
        })
        .ok_or_else(|| MemoryError::InvalidState("pin: payload not found".to_string()))?;
        // The mutation permit excludes writers between lookup and this lock.
        let mut state = self.lock()?;
        let entry = state.entries[index].as_mut().expect("reference index points to live slot");
        entry.pin_count = entry
            .pin_count
            .checked_add(1)
            .ok_or_else(|| MemoryError::capacity("payload pin count overflow"))?;
        Ok(())
    }

    pub fn unpin(&self, payload_ref: &PayloadRef) -> MemoryResult<()> {
        let _permit = self.lifecycle.begin_mutation()?;
        let digest = reference_digest(payload_ref);
        let index = invoke_client_read_while_locked(self.lock()?, |state| {
            find_reference_slot(state, digest, payload_ref)
        })
        .ok_or_else(|| MemoryError::InvalidState("unpin: payload not found".to_string()))?;
        let mut state = self.lock()?;
        let entry = state.entries[index].as_mut().expect("reference index points to live slot");
        entry.pin_count = entry
            .pin_count
            .checked_sub(1)
            .ok_or_else(|| MemoryError::InvalidState("unpin: payload not pinned".to_string()))?;
        Ok(())
    }

    pub fn pin_count(&self, payload_ref: &PayloadRef) -> MemoryResult<u64> {
        self.lifecycle.with_read_state(&self.state, |state| {
            let digest = reference_digest(payload_ref);
            find_reference_slot(state, digest, payload_ref)
                .and_then(|index| state.entries[index].as_ref())
                .map_or(0, |entry| entry.pin_count)
        })
    }
}

impl<PayloadRef, Bytes, Mode> PayloadStore<PayloadRef, Bytes>
    for SimplePayloadStore<PayloadRef, Bytes, Mode>
where
    PayloadRef: Clone + From<String> + Eq + Hash + ByteSized,
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

        let existing = {
            let state = self.lock()?;
            invoke_client_read_while_locked(state, |state| {
                state
                    .canonical_index
                    .get(&digest)
                    .and_then(|slots| {
                        slots.iter().filter_map(|slot| state.entries[*slot].as_ref()).find(
                            |entry| {
                                entry.canonical_digest == digest
                                    && entry.bytes.canonical_key() == canonical_key
                            },
                        )
                    })
                    .map(|entry| entry.payload_ref.clone())
            })
        };
        if let Some(existing) = existing {
            return Ok(PublishedPayload::new(existing));
        }
        let (payload_ref_text, next_ref) = {
            let state = self.lock()?;
            let next_ref = state
                .next_ref
                .checked_add(1)
                .ok_or_else(|| MemoryError::capacity("payload reference counter overflow"))?;
            (format!("payload-{}", state.next_ref), next_ref)
        };
        // Client measurement callbacks run with no store state lock held. A
        // panic or reentry cannot poison the authoritative state or charge the
        // broker before the complete semantic cost is known.
        let payload_ref = PayloadRef::from(payload_ref_text);
        let payload_ref_digest = reference_digest(&payload_ref);
        let index_bytes = semantic_cost_v1::DIGEST_BYTES
            .checked_mul(2)
            .ok_or_else(|| MemoryError::capacity("payload index cost exceeds u64"))?;
        let charged_bytes = RetainedCostV1 {
            payload_bytes: value.bytes.byte_size(),
            key_identity_bytes: payload_ref.byte_size(),
            index_bytes,
            fixed_overhead_bytes: semantic_cost_v1::ENTRY_STATE_BYTES,
            ..RetainedCostV1::default()
        }
        .total()?;
        let next_total = self
            .lock()?
            .charged_total
            .checked_add(charged_bytes)
            .ok_or_else(|| MemoryError::capacity("payload byte counter overflow"))?;

        // PayloadRef::Clone is caller code. Prepare the stored identity before
        // charging or holding the publication lock so its panic cannot poison
        // the container or leave a provisional accounting lease behind.
        let stored_payload_ref = payload_ref.clone();

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
                return Err(MemoryError::poisoned("payload store lock after accounting"));
            }
        };
        let entry = PayloadEntry {
            payload_ref: stored_payload_ref,
            canonical_digest: digest,
            charged_bytes,
            pin_count: 0,
            charge,
            bytes: value.bytes,
        };
        let slot = if let Some(slot) = state.free_slots.pop() {
            debug_assert!(state.entries[slot].is_none(), "free slot must be vacant");
            state.entries[slot] = Some(entry);
            slot
        } else {
            let slot = state.entries.len();
            state.entries.push(Some(entry));
            slot
        };
        state.canonical_index.entry(digest).or_default().push(slot);
        state.reference_index.entry(payload_ref_digest).or_default().push(slot);
        state.live_entries = state
            .live_entries
            .checked_add(1)
            .expect("live entry count cannot exceed allocated slots");
        state.next_ref = next_ref;
        state.charged_total = next_total;
        Ok(PublishedPayload::new(payload_ref))
    }

    fn delete_if_unpinned(&self, payload_ref: &PayloadRef) -> Result<(), StoreDeleteError> {
        let _permit = self.lifecycle.begin_mutation()?;
        let reference_digest = reference_digest(payload_ref);
        let index = invoke_client_read_while_locked(self.lock()?, |state| {
            find_reference_slot(state, reference_digest, payload_ref)
        })
        .ok_or_else(|| MemoryError::InvalidState("delete: payload not found".to_string()))?;
        let (index, canonical_digest, next_total, mut charge) = {
            let mut state = self.lock()?;
            let entry = state.entries[index].as_mut().expect("reference index points to live slot");
            if entry.pin_count > 0 {
                return Err(MemoryError::InvalidState(format!(
                    "delete rejected: payload still pinned ({} dependents)",
                    entry.pin_count
                )));
            }
            let canonical_digest = entry.canonical_digest;
            let charged_bytes = entry.charged_bytes;
            let charge = entry.charge.take();
            let next_total = state.charged_total.checked_sub(charged_bytes).ok_or_else(|| {
                MemoryError::InvalidState("payload byte counter underflow".to_string())
            })?;
            (index, canonical_digest, next_total, charge)
        };

        if let Some(active) = charge.as_mut() {
            if let Err(error) = self.accounting.release_many(std::slice::from_mut(active)) {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.entries[index].as_mut().expect("delete slot remains live").charge = charge;
                return Err(error);
            }
        }

        let removed = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let removed = state.entries[index].take().expect("delete slot remains live");
            remove_index_slot(&mut state.canonical_index, canonical_digest, index);
            remove_index_slot(&mut state.reference_index, reference_digest, index);
            state.free_slots.push(index);
            state.live_entries = state
                .live_entries
                .checked_sub(1)
                .expect("removing a live slot requires a positive live count");
            state.charged_total = next_total;
            removed
        };
        // The terminal delete postimage is already published. Retire generic
        // fields separately so one client destructor cannot abort the process
        // while another field is being dropped during unwind.
        retire_entry(removed);
        Ok(())
    }

    fn stats(&self) -> MemoryResult<StoreStats> {
        self.lifecycle.with_read_state(&self.state, |state| StoreStats {
            item_count: state.live_entries,
            estimated_bytes: state.charged_total,
        })
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
        state.free_slots.clear();
        state.canonical_index.clear();
        state.reference_index.clear();
        state.live_entries = 0;
        state.charged_total = 0;
        retire_entries(entries);
    }
}

fn remove_index_slot(index: &mut HashMap<u64, Vec<usize>>, digest: u64, slot: usize) {
    let bucket = index.get_mut(&digest).expect("live payload has an index bucket");
    let position = bucket
        .iter()
        .position(|candidate| *candidate == slot)
        .expect("live payload index bucket contains its stable slot");
    bucket.swap_remove(position);
    if bucket.is_empty() {
        index.remove(&digest);
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

    fn retained_cost(payload_bytes: u64, sequence: u64) -> u64 {
        // Independent oracle: do not call the production formula or constants.
        payload_bytes + format!("payload-{sequence}").len() as u64 + 64 + 16
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

    #[derive(Clone)]
    struct ZeroMeasuredHugeCanonicalKey;

    impl ByteSized for ZeroMeasuredHugeCanonicalKey {
        fn byte_size(&self) -> u64 {
            0
        }
    }

    impl CanonicalKey for ZeroMeasuredHugeCanonicalKey {
        fn canonical_key(&self) -> Vec<u8> {
            vec![7; 1_000_000]
        }
    }

    #[derive(Clone)]
    struct MaxMeasuredPayload;

    impl ByteSized for MaxMeasuredPayload {
        fn byte_size(&self) -> u64 {
            u64::MAX
        }
    }

    impl CanonicalKey for MaxMeasuredPayload {
        fn canonical_key(&self) -> Vec<u8> {
            vec![9]
        }
    }

    #[derive(Clone)]
    struct PanicMeasuredPayload;

    impl ByteSized for PanicMeasuredPayload {
        fn byte_size(&self) -> u64 {
            panic!("injected ByteSized panic")
        }
    }

    impl CanonicalKey for PanicMeasuredPayload {
        fn canonical_key(&self) -> Vec<u8> {
            vec![10]
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

    impl ByteSized for ReentrantPayloadRef {
        fn byte_size(&self) -> u64 {
            self.0.len() as u64
        }
    }

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

    impl Eq for ReentrantPayloadRef {}

    impl Hash for ReentrantPayloadRef {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.0.hash(state);
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
        assert_eq!(store.charged_bytes().expect("bytes"), retained_cost(4, 0));
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            retained_cost(4, 0)
        );
    }

    #[test]
    fn payload_lookup_probe_count_is_n_independent() {
        #[derive(Clone)]
        struct ProbeBytes {
            bytes: Vec<u8>,
            probes: Arc<AtomicUsize>,
        }

        impl ByteSized for ProbeBytes {
            fn byte_size(&self) -> u64 {
                self.bytes.len() as u64
            }
        }

        impl CanonicalKey for ProbeBytes {
            fn canonical_key(&self) -> Vec<u8> {
                self.probes.fetch_add(1, Ordering::SeqCst);
                self.bytes.clone()
            }
        }

        let broker = live_broker(1_000_000, 600_000);
        let store = SimplePayloadStore::<String, ProbeBytes, Budgeted>::new_budgeted(
            binding(),
            None,
            broker,
        );
        let probes = Arc::new(AtomicUsize::new(0));
        for value in 0u32..2_048 {
            store
                .put_canonical(RetainedEnvelope::new(ProbeBytes {
                    bytes: value.to_be_bytes().to_vec(),
                    probes: Arc::clone(&probes),
                }))
                .expect("unique payload insert");
        }

        probes.store(0, Ordering::SeqCst);
        let duplicate = store
            .put_canonical(RetainedEnvelope::new(ProbeBytes {
                bytes: 1_024u32.to_be_bytes().to_vec(),
                probes: Arc::clone(&probes),
            }))
            .expect("indexed duplicate lookup");
        assert_eq!(duplicate.payload_ref(), "payload-1024");
        assert_eq!(
            probes.load(Ordering::SeqCst),
            2,
            "lookup evaluates only the query and its digest-bucket candidate, independent of N"
        );
        assert_eq!(store.stats().expect("stats").item_count, 2_048);
    }

    #[test]
    fn payload_delete_reuses_vacant_slots_without_stale_index_alias() {
        let (broker, store) = store();
        let mut first_generation = Vec::new();
        for value in 0u32..128 {
            first_generation.push(
                store
                    .put_canonical(RetainedEnvelope::new(value.to_be_bytes().to_vec()))
                    .expect("first generation")
                    .payload_ref()
                    .clone(),
            );
        }
        for payload_ref in &first_generation {
            store.delete_if_unpinned(payload_ref).expect("delete first generation");
        }
        {
            let state = store.state.lock().expect("state");
            assert_eq!(state.entries.len(), 128);
            assert_eq!(state.free_slots.len(), 128);
            assert!(state.canonical_index.is_empty() && state.reference_index.is_empty());
        }

        for value in 128u32..256 {
            store
                .put_canonical(RetainedEnvelope::new(value.to_be_bytes().to_vec()))
                .expect("second generation");
        }
        let state = store.state.lock().expect("state");
        assert_eq!(state.entries.len(), 128, "churn must reuse stable vacant slots");
        assert!(state.free_slots.is_empty());
        assert_eq!(state.live_entries, 128);
        drop(state);
        drop(store);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn transient_canonical_key_is_not_charged_but_retained_identity_is() {
        let broker = live_broker(100, 100);
        let store =
            SimplePayloadStore::<String, ZeroMeasuredHugeCanonicalKey, Budgeted>::new_budgeted(
                binding(),
                None,
                broker.clone(),
            );
        let first = store
            .put_canonical(RetainedEnvelope::new(ZeroMeasuredHugeCanonicalKey))
            .expect("semantic cost fits");
        let second = store
            .put_canonical(RetainedEnvelope::new(ZeroMeasuredHugeCanonicalKey))
            .expect("dedup");
        assert_eq!(first.payload_ref(), second.payload_ref());
        assert_eq!(store.charged_bytes().expect("bytes"), retained_cost(0, 0));
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            retained_cost(0, 0)
        );
    }

    #[test]
    fn cost_component_sum_overflow_denies_without_publishing_or_charging() {
        let broker = live_broker(1_000, 1_000);
        let store = SimplePayloadStore::<String, MaxMeasuredPayload, Budgeted>::new_budgeted(
            binding(),
            None,
            broker.clone(),
        );
        assert!(matches!(
            store.put_canonical(RetainedEnvelope::new(MaxMeasuredPayload)),
            Err(MemoryError::Capacity(_))
        ));
        assert_eq!(store.stats().expect("unchanged stats").item_count, 0);
        assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn composite_entry_charges_one_payload_bucket() {
        let broker = live_broker(1_000_000, 600_000);
        let store = SimplePayloadStore::<String, Vec<u8>, Budgeted>::new_budgeted(
            binding(),
            None,
            broker.clone(),
        );
        store.put_canonical(envelope(b"payload")).expect("put");
        let charged = store.charged_bytes().expect("charged total");
        assert!(charged > 0, "a published payload holds a live charge");
        // One owned lifetime component, one bucket: the composite
        // payload+key/index/entry cost is charged entirely as RetainedPayload.
        let snap = broker.ledger_snapshot().expect("snapshot");
        assert_eq!(snap.total_retained_payload_bytes(), charged);
        assert_eq!(snap.total_retained_metadata_bytes(), 0);
        assert_eq!(snap.total_live_scratch_bytes(), 0);
        assert_eq!(snap.total_owner_bytes(), charged);
        assert_eq!(snap.live_ticket_count, 1);
        let usage = snap.owner_usage.values().next().expect("sole owner");
        assert_eq!(
            (usage.retained_payload_bytes, usage.retained_metadata_bytes, usage.live_scratch_bytes),
            (charged, 0, 0)
        );
        store.close().expect("close");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }

    #[test]
    fn counter_reads_show_open_busy_and_closed_images() {
        let broker = live_broker(1_000_000, 600_000);
        let store = SimplePayloadStore::<String, Vec<u8>, Budgeted>::new_budgeted(
            binding(),
            None,
            broker.clone(),
        );
        store.put_canonical(envelope(b"payload")).expect("put");
        // Open image: the plain counters mirror the authoritative ledger charge.
        let charged = broker.ledger_snapshot().expect("open ledger").total_owner_bytes();
        assert!(charged > 0, "seed payload must hold a live charge");
        assert_eq!(
            store.stats().expect("open stats"),
            StoreStats { item_count: 1, estimated_bytes: charged }
        );
        assert_eq!(store.charged_bytes().expect("open bytes"), charged);

        // Busy image: while the mutation lane is held, counter reads are typed
        // contention — never an empty success masking an in-flight plan.
        let permit = store.lifecycle.begin_mutation().expect("own lane");
        assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
        assert!(matches!(store.charged_bytes(), Err(MemoryError::OperationInProgress(_))));
        drop(permit);

        // Closed image: the terminal empty image stays readable and consistent.
        store.close().expect("close");
        assert_eq!(store.stats().expect("closed stats"), StoreStats::default());
        assert_eq!(store.charged_bytes().expect("closed bytes"), 0);
        assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn measurement_panic_precedes_accounting_and_does_not_poison_state() {
        let broker = live_broker(1_000, 1_000);
        let store = SimplePayloadStore::<String, PanicMeasuredPayload, Budgeted>::new_budgeted(
            binding(),
            None,
            broker.clone(),
        );
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.put_canonical(RetainedEnvelope::new(PanicMeasuredPayload));
        }));
        assert!(caught.is_err());
        assert_eq!(store.stats().expect("state not poisoned"), StoreStats::default());
        assert_eq!(broker.ledger_snapshot().expect("no charge").total_owner_bytes(), 0);
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
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            retained_cost(8, 0)
        );
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
        assert_eq!(
            broker.ledger_snapshot().expect("charged once").total_owner_bytes(),
            retained_cost(4, 0)
        );
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
        assert_eq!(
            store.charged_bytes().expect("bytes"),
            retained_cost(2, 0) + retained_cost(4, 1)
        );
        assert_eq!(store.stats().expect("stats").item_count, 2);
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            retained_cost(2, 0) + retained_cost(4, 1)
        );
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
        let broker = live_broker(1_000, 140);
        let store = store_with(&broker);
        assert!(matches!(store.put_canonical(envelope(&[0; 100])), Err(MemoryError::Capacity(_))));
        assert_eq!(store.stats().expect("stats").item_count, 0);
        let published = store.put_canonical(envelope(&[0; 40])).expect("within cap");
        assert_eq!(published.payload_ref(), &"payload-0".to_string());
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            retained_cost(40, 0)
        );
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
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            retained_cost(7, 0)
        );
        store.unpin(published.payload_ref()).expect("unpin");
        store.unpin(published.payload_ref()).expect("unpin again");
        store.delete_if_unpinned(published.payload_ref()).expect("delete");
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn reference_eq_panic_before_mutation_keeps_public_store_available() {
        use std::sync::atomic::AtomicBool;

        static PANIC_NEXT_EQ: AtomicBool = AtomicBool::new(false);

        #[derive(Clone, Debug)]
        struct Ref(String);
        impl From<String> for Ref {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
        impl ByteSized for Ref {
            fn byte_size(&self) -> u64 {
                self.0.len() as u64
            }
        }
        impl Hash for Ref {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.0.hash(state);
            }
        }
        impl PartialEq for Ref {
            fn eq(&self, other: &Self) -> bool {
                if PANIC_NEXT_EQ.swap(false, Ordering::SeqCst) {
                    panic!("injected reference Eq panic");
                }
                self.0 == other.0
            }
        }
        impl Eq for Ref {}

        for operation in ["pin", "unpin", "pin_count", "delete"] {
            let broker = live_broker(1_000_000, 600_000);
            let store = SimplePayloadStore::<Ref, Vec<u8>, Budgeted>::new_budgeted(
                binding(),
                None,
                broker.clone(),
            );
            let published = store.put_canonical(envelope(b"payload")).expect("put");
            if operation == "unpin" {
                store.pin(published.payload_ref()).expect("seed pin");
            }
            let before = broker.ledger_snapshot().expect("before");
            let pins_before = store.pin_count(published.payload_ref()).expect("pins before");
            PANIC_NEXT_EQ.store(true, Ordering::SeqCst);
            let panicked =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match operation {
                    "pin" => {
                        let _ = store.pin(published.payload_ref());
                    }
                    "unpin" => {
                        let _ = store.unpin(published.payload_ref());
                    }
                    "pin_count" => {
                        let _ = store.pin_count(published.payload_ref());
                    }
                    "delete" => {
                        let _ = store.delete_if_unpinned(published.payload_ref());
                    }
                    _ => unreachable!(),
                }));
            assert!(panicked.is_err(), "{operation}: Eq trap must fire");
            assert_eq!(broker.ledger_snapshot().expect("after"), before);
            assert_eq!(store.stats().expect("public stats after panic").item_count, 1);
            assert_eq!(
                store.pin_count(published.payload_ref()).expect("public pin count"),
                pins_before
            );
            if pins_before > 0 {
                store.unpin(published.payload_ref()).expect("unpin after panic");
            }
            store.delete_if_unpinned(published.payload_ref()).expect("delete after panic");
            assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
        }
    }

    #[test]
    fn reference_clone_panic_before_publication_keeps_public_store_available() {
        use std::sync::atomic::AtomicBool;

        static PANIC_NEXT_CLONE: AtomicBool = AtomicBool::new(false);

        #[derive(Debug, PartialEq, Eq, Hash)]
        struct Ref(String);
        impl From<String> for Ref {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
        impl Clone for Ref {
            fn clone(&self) -> Self {
                if PANIC_NEXT_CLONE.swap(false, Ordering::SeqCst) {
                    panic!("injected reference Clone panic");
                }
                Self(self.0.clone())
            }
        }
        impl ByteSized for Ref {
            fn byte_size(&self) -> u64 {
                self.0.len() as u64
            }
        }

        let broker = live_broker(1_000_000, 600_000);
        let store = SimplePayloadStore::<Ref, Vec<u8>, Budgeted>::new_budgeted(
            binding(),
            None,
            broker.clone(),
        );
        PANIC_NEXT_CLONE.store(true, Ordering::SeqCst);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.put_canonical(envelope(b"payload"));
        }));
        assert!(panicked.is_err(), "Clone trap must fire before publication");
        assert_eq!(broker.ledger_snapshot().expect("after panic").total_owner_bytes(), 0);
        assert_eq!(store.stats().expect("public stats after panic").item_count, 0);

        let published = store.put_canonical(envelope(b"payload")).expect("retry after panic");
        assert_eq!(store.stats().expect("published stats").item_count, 1);
        store.delete_if_unpinned(published.payload_ref()).expect("cleanup");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
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
        assert_eq!(store.charged_bytes().expect("preserved bytes"), retained_cost(80, 0));
        assert_eq!(
            broker.ledger_snapshot().expect("preserved ledger").total_owner_bytes(),
            retained_cost(80, 0)
        );

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
        assert_eq!(store.charged_bytes().expect("bytes"), retained_cost(4, 0));
        assert_eq!(store.stats().expect("stats").item_count, 1);
        store.close().expect("close unbudgeted");
    }

    #[test]
    fn close_refunds_every_live_entry_exactly_once() {
        let (broker, store) = store();
        store.put_canonical(envelope(b"aa")).expect("a");
        store.put_canonical(envelope(b"bbbb")).expect("b");
        assert_eq!(
            broker.ledger_snapshot().expect("before").total_owner_bytes(),
            retained_cost(2, 0) + retained_cost(4, 1)
        );
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
        assert_eq!(store.charged_bytes().expect("preserved bytes"), retained_cost(5, 0));
        assert_eq!(
            broker.ledger_snapshot().expect("preserved ledger").total_owner_bytes(),
            retained_cost(5, 0)
        );
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
        assert_eq!(
            broker.ledger_snapshot().expect("deferred").total_owner_bytes(),
            retained_cost(5, 0)
        );
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
        assert_eq!(
            broker.ledger_snapshot().expect("before").total_owner_bytes(),
            retained_cost(4, 0) + retained_cost(6, 1)
        );

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
        assert!(matches!(store.stats(), Err(MemoryError::Poisoned(_))));
    }

    type DropReentryPayloadStore = SimplePayloadStore<String, DropReentryBody, Budgeted>;

    #[derive(Clone)]
    struct DropReentryBody {
        id: u8,
        armed: Arc<std::sync::atomic::AtomicBool>,
        home: Arc<OnceLock<Weak<DropReentryPayloadStore>>>,
        peer: Arc<OnceLock<Weak<DropReentryPayloadStore>>>,
        hits: Arc<AtomicUsize>,
    }

    impl ByteSized for DropReentryBody {
        fn byte_size(&self) -> u64 {
            1
        }
    }

    impl CanonicalKey for DropReentryBody {
        fn canonical_key(&self) -> Vec<u8> {
            vec![self.id]
        }
    }

    impl Drop for DropReentryBody {
        fn drop(&mut self) {
            if !self.armed.swap(false, Ordering::SeqCst) {
                return;
            }
            let home = self.home.get().expect("home installed").upgrade().expect("home live");
            assert!(matches!(
                home.stats(),
                Err(MemoryError::OperationInProgress(resource)) if resource == "payload store"
            ));
            let peer = self.peer.get().expect("peer installed").upgrade().expect("peer live");
            peer.put_canonical(RetainedEnvelope::new(Self {
                id: 2,
                armed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                home: Arc::clone(&self.home),
                peer: Arc::clone(&self.peer),
                hits: Arc::clone(&self.hits),
            }))
            .expect("other store can publish during retired body drop");
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn delete_drop_reentry_is_bounded_and_peer_publication_retains_exact_charge() {
        const CHILD: &str = "QMP_PAYLOAD_DELETE_DROP_REENTRY_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let mut child =
                std::process::Command::new(std::env::current_exe().expect("test binary"))
                    .arg("delete_drop_reentry_is_bounded_and_peer_publication_retains_exact_charge")
                    .env(CHILD, "1")
                    .spawn()
                    .expect("spawn bounded reentry test");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                if let Some(status) = child.try_wait().expect("poll child") {
                    assert!(status.success(), "reentry child failed: {status}");
                    return;
                }
                if std::time::Instant::now() >= deadline {
                    child.kill().expect("terminate deadlocked child");
                    child.wait().expect("reap child");
                    panic!("reentry child exceeded bounded deadline");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        let broker = live_broker(1_000_000, 600_000);
        let home = Arc::new(DropReentryPayloadStore::new_budgeted(binding(), None, broker.clone()));
        let peer = Arc::new(DropReentryPayloadStore::new_budgeted(
            StoreBindingHandle::new(MemoryClass::ArtifactCache, "drop-peer".into(), None),
            None,
            broker.clone(),
        ));
        let home_slot = Arc::new(OnceLock::new());
        let peer_slot = Arc::new(OnceLock::new());
        home_slot.set(Arc::downgrade(&home)).expect("install home");
        peer_slot.set(Arc::downgrade(&peer)).expect("install peer");
        let hits = Arc::new(AtomicUsize::new(0));
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let published = home
            .put_canonical(RetainedEnvelope::new(DropReentryBody {
                id: 1,
                armed: Arc::clone(&armed),
                home: Arc::clone(&home_slot),
                peer: Arc::clone(&peer_slot),
                hits: Arc::clone(&hits),
            }))
            .expect("seed");
        assert_eq!(
            broker.ledger_snapshot().expect("seed ledger").total_owner_bytes(),
            retained_cost(1, 0)
        );
        armed.store(true, Ordering::SeqCst);
        home.delete_if_unpinned(&published.payload_ref).expect("delete and retire body");

        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(home.stats().expect("home postimage").item_count, 0);
        assert_eq!(peer.stats().expect("peer postimage").item_count, 1);
        assert_eq!(
            broker.ledger_snapshot().expect("peer ledger").total_owner_bytes(),
            retained_cost(1, 0)
        );
        home.close().expect("close home");
        peer.close().expect("close peer");
        assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn delete_contains_two_panicking_client_destructors_after_exact_refund() {
        use std::sync::atomic::AtomicBool;

        static PANIC_REF: AtomicBool = AtomicBool::new(false);
        static PANIC_BODY: AtomicBool = AtomicBool::new(false);
        static PANIC_COUNT: AtomicUsize = AtomicUsize::new(0);

        #[derive(Debug)]
        struct Ref(String);

        impl Clone for Ref {
            fn clone(&self) -> Self {
                Self(self.0.clone())
            }
        }

        impl From<String> for Ref {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl PartialEq for Ref {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl Eq for Ref {}

        impl Hash for Ref {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.0.hash(state);
            }
        }

        impl ByteSized for Ref {
            fn byte_size(&self) -> u64 {
                self.0.len() as u64
            }
        }

        impl Drop for Ref {
            fn drop(&mut self) {
                if PANIC_REF.load(Ordering::SeqCst) {
                    PANIC_COUNT.fetch_add(1, Ordering::SeqCst);
                    panic!("reference drop trap");
                }
            }
        }

        #[derive(Clone, Debug)]
        struct Body(Vec<u8>);

        impl ByteSized for Body {
            fn byte_size(&self) -> u64 {
                self.0.len() as u64
            }
        }

        impl CanonicalKey for Body {
            fn canonical_key(&self) -> Vec<u8> {
                self.0.clone()
            }
        }

        impl Drop for Body {
            fn drop(&mut self) {
                if PANIC_BODY.load(Ordering::SeqCst) {
                    PANIC_COUNT.fetch_add(1, Ordering::SeqCst);
                    panic!("body drop trap");
                }
            }
        }

        const CHILD: &str = "QMP_PAYLOAD_DELETE_DROP_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let broker = live_broker(1_000_000, 600_000);
            let store = SimplePayloadStore::<Ref, Body, Budgeted>::new_budgeted(
                binding(),
                None,
                broker.clone(),
            );
            let published =
                store.put_canonical(RetainedEnvelope::new(Body(vec![1, 2, 3]))).expect("seed");
            PANIC_REF.store(true, Ordering::SeqCst);
            PANIC_BODY.store(true, Ordering::SeqCst);
            store.delete_if_unpinned(&published.payload_ref).expect("delete");
            PANIC_REF.store(false, Ordering::SeqCst);
            PANIC_BODY.store(false, Ordering::SeqCst);
            assert_eq!(PANIC_COUNT.load(Ordering::SeqCst), 2, "both fields retired");
            assert_eq!(store.stats().expect("postimage").item_count, 0);
            assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
            return;
        }

        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("delete_contains_two_panicking_client_destructors_after_exact_refund")
            .arg("--nocapture")
            .env(CHILD, "1")
            .output()
            .expect("spawn child");
        assert!(
            output.status.success(),
            "child {:?}; stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
