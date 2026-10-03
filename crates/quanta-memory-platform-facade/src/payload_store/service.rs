use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, RandomState};
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
    invoke_client_read_while_locked, retire_client_value, BeginClose, DropTransition, InFlight,
    StoreLifecycle,
};
use crate::shared::measure::{semantic_cost_v1, ByteSized, CanonicalKey, RetainedCostV1};

/// Randomized digest seed mirroring the retained map's hardening: fixed-key
/// hashing would let attacker-chosen canonical keys be ground into one index
/// bucket, degrading every dedup/pin/delete point operation to O(bucket)
/// caller-trait work. A per-store [`RandomState`] makes the bucket assignment
/// non-deterministic across processes.
fn canonical_digest(seed: &RandomState, key: &[u8]) -> u64 {
    seed.hash_one(key)
}

fn reference_digest<PayloadRef: Hash>(seed: &RandomState, payload_ref: &PayloadRef) -> u64 {
    seed.hash_one(payload_ref)
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
    digest_seed: RandomState,
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
            digest_seed: RandomState::new(),
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
        let digest = reference_digest(&self.digest_seed, payload_ref);
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
        let digest = reference_digest(&self.digest_seed, payload_ref);
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
            let digest = reference_digest(&self.digest_seed, payload_ref);
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
        // Caller-trait code (CanonicalKey/ByteSized/Clone/Eq) runs while the
        // owned envelope is guarded, so a trait panic can never unwind through
        // a caller destructor (compound-hostile containment).
        let mut value = InFlight::new(value);
        let context_kind = self.context_kind(value.get());
        let canonical_key = value.get().bytes.canonical_key();
        let digest = canonical_digest(&self.digest_seed, &canonical_key);

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
        let mut payload_ref = InFlight::new(PayloadRef::from(payload_ref_text));
        let payload_ref_digest = reference_digest(&self.digest_seed, payload_ref.get());
        // `From<String>` does not promise an injective conversion. The
        // mutation permit keeps this check stable until publication; perform
        // caller equality before charging or entering the publication lock.
        let reference_exists = {
            let state = self.lock()?;
            invoke_client_read_while_locked(state, |state| {
                find_reference_slot(state, payload_ref_digest, payload_ref.get()).is_some()
            })
        };
        if reference_exists {
            return Err(MemoryError::InvalidState(
                "generated payload reference collides with a live entry".into(),
            ));
        }
        let index_bytes = semantic_cost_v1::DIGEST_BYTES
            .checked_mul(2)
            .ok_or_else(|| MemoryError::capacity("payload index cost exceeds u64"))?;
        let charged_bytes = RetainedCostV1 {
            payload_bytes: value.get().bytes.byte_size(),
            key_identity_bytes: payload_ref.get().byte_size(),
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
        let mut stored_payload_ref = InFlight::new(payload_ref.get().clone());

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
            payload_ref: stored_payload_ref.take(),
            canonical_digest: digest,
            charged_bytes,
            pin_count: 0,
            charge,
            bytes: value.take().bytes,
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
        Ok(PublishedPayload::new(payload_ref.take()))
    }

    fn delete_if_unpinned(&self, payload_ref: &PayloadRef) -> Result<(), StoreDeleteError> {
        let _permit = self.lifecycle.begin_mutation()?;
        let reference_digest = reference_digest(&self.digest_seed, payload_ref);
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
mod tests;
