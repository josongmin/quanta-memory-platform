#[cfg(feature = "spill")]
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, ManagedCharge, MemoryContextKind, MemoryError,
    MemoryOperation, MemoryResult, StoreBindingHandle, StoreStats, StoreWriteError, SweepStats,
};

#[cfg(feature = "spill")]
use crate::revocable_store::durable_codec;
use crate::revocable_store::RevocableStore;
#[cfg(feature = "unbudgeted-stores")]
use crate::shared::accounting::Unbudgeted;
use crate::shared::accounting::{Budgeted, StoreAccounting};
use crate::shared::lifecycle::{retire_client_value, BeginClose, InFlight, StoreLifecycle};
use crate::shared::measure::ByteSized;

#[cfg(feature = "spill")]
mod spill;
#[cfg(feature = "spill")]
use crate::spill::{
    DurableCommitFailure, DurableRecordId, DurableRecordResolution, DurableRestoreClaim,
    DurableRestoreController, DurableRestoreOutcome, DurableRestorePhase, DurableWriteFailure,
    InProcessSpillController, ReconciledDurableSpillController, SpillFailure, SpillReceipt,
};

#[derive(Debug)]
struct RevocableEntry<Key, Value> {
    key: Key,
    value: Value,
    charged_bytes: u64,
    /// Non-clone authority that always travels with its retained payload.
    charge: Option<ManagedCharge>,
    /// Restore claim retained only while a durable restore is fenced. `None`
    /// for ordinary entries and successfully finalized restores. The pending
    /// entry shares the immutable claim allocation with the in-flight
    /// reconciliation call; the private `Arc` pins the same token across
    /// lock-free controller calls, and no caller can clone this custody.
    #[cfg(feature = "spill")]
    restore_claim: Option<Arc<DurableRestoreClaim>>,
}

/// In-process transfer authority. The value, its semantic cost, and the
/// issuing broker lease move together; dropping an un-restored handle retires
/// them together. This is not a durable serialization format.
#[cfg(feature = "spill")]
#[derive(Debug)]
pub struct RevocableSpillHandle<Key, Value> {
    /// Shared store identity. Moving many entries into an in-process tier must
    /// not deep-clone caller-controlled owner/runtime strings per handle.
    binding: Arc<StoreBindingHandle>,
    entry: Option<RevocableEntry<Key, Value>>,
}

#[cfg(feature = "spill")]
impl<Key, Value> RevocableSpillHandle<Key, Value> {
    #[must_use]
    pub fn key(&self) -> &Key {
        &self.entry.as_ref().expect("live spill handle").key
    }

    #[must_use]
    pub fn value(&self) -> &Value {
        &self.entry.as_ref().expect("live spill handle").value
    }

    #[must_use]
    pub fn charged_bytes(&self) -> u64 {
        self.entry.as_ref().expect("live spill handle").charged_bytes
    }

    fn into_entry(mut self) -> RevocableEntry<Key, Value> {
        self.entry.take().expect("live spill handle")
    }
}

#[cfg(feature = "spill")]
impl<Key, Value> Drop for RevocableSpillHandle<Key, Value> {
    fn drop(&mut self) {
        if let Some(entry) = self.entry.take() {
            retire_entry(entry);
        }
    }
}

fn retire_entry<Key, Value>(entry: RevocableEntry<Key, Value>) {
    let RevocableEntry { key, value, charge, .. } = entry;
    drop(charge);
    retire_client_value(key);
    retire_client_value(value);
}

fn retire_entries<Key, Value>(entries: VecDeque<RevocableEntry<Key, Value>>) {
    for entry in entries {
        retire_entry(entry);
    }
}

fn revocable_entry_cost<Key: ByteSized, Value: ByteSized>(
    key: &Key,
    value: &Value,
) -> MemoryResult<u64> {
    super::cost::entry_cost(key.byte_size(), value.byte_size())
}

#[cfg(feature = "spill")]
fn bounded_record_cap<C: ReconciledDurableSpillController>(controller: &C) -> MemoryResult<u64> {
    controller.max_record_bytes().ok_or_else(|| {
        MemoryError::Unsupported(
            "durable controller does not declare an enforced record cap".into(),
        )
    })
}

#[derive(Debug)]
struct RevocableState<Key, Value> {
    entries: VecDeque<RevocableEntry<Key, Value>>,
    /// Source entries fenced while a durable commit acknowledgement is unknown.
    /// Their managed charges remain live until record reconciliation chooses the
    /// durable or resident authority.
    #[cfg(feature = "spill")]
    pending_spills: BTreeMap<DurableRecordId, RevocableEntry<Key, Value>>,
    /// Provisional resident entries fenced while a durable delete
    /// acknowledgement is unknown. They are charged but not readable until
    /// reconciliation proves that the durable record is absent.
    #[cfg(feature = "spill")]
    pending_restores: BTreeMap<DurableRecordId, RevocableEntry<Key, Value>>,
    committed_bytes: u64,
}

/// Portable FIFO revocable store with construction-time accounting branding.
///
/// `Mode` is part of the product type. Raw construction and late accounting
/// attachment are not public:
///
/// ```compile_fail
/// use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
/// use quanta_memory_platform_facade::{Budgeted, SimpleRevocableStore};
///
/// let binding = StoreBindingHandle::new(MemoryClass::ExecutionScratchRevocable, "owner".into(), None);
/// let _ = SimpleRevocableStore::<String, Vec<u8>, Budgeted>::new(binding);
/// ```
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{Budgeted, SimpleRevocableStore};
///
/// let _ = SimpleRevocableStore::<String, Vec<u8>, Budgeted>::charged_against;
/// ```
#[derive(Debug)]
pub struct SimpleRevocableStore<Key, Value, Mode> {
    binding: Arc<StoreBindingHandle>,
    accounting: StoreAccounting,
    lifecycle: StoreLifecycle,
    state: Mutex<RevocableState<Key, Value>>,
    _mode: PhantomData<fn() -> Mode>,
}

impl<Key, Value, Mode> SimpleRevocableStore<Key, Value, Mode> {
    fn with_accounting(binding: StoreBindingHandle, accounting: StoreAccounting) -> Self {
        Self {
            binding: Arc::new(binding),
            accounting,
            lifecycle: StoreLifecycle::new("revocable store"),
            state: Mutex::new(RevocableState {
                entries: VecDeque::new(),
                #[cfg(feature = "spill")]
                pending_spills: BTreeMap::new(),
                #[cfg(feature = "spill")]
                pending_restores: BTreeMap::new(),
                committed_bytes: 0,
            }),
            _mode: PhantomData,
        }
    }

    fn lock_state(&self) -> MemoryResult<MutexGuard<'_, RevocableState<Key, Value>>> {
        self.lifecycle.try_lock_state(&self.state)
    }

    /// Once rows/leases have been detached, a transient stale reader must not
    /// turn the broker's committed refund into an early-return with an
    /// unpublished local counter. The mutation permit prevents new callbacks;
    /// wait for any reader that crossed the phase check just before the permit.
    fn lock_state_for_commit(&self) -> MutexGuard<'_, RevocableState<Key, Value>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn require_charge_shape(
        &self,
        entry: &RevocableEntry<Key, Value>,
        operation: &str,
    ) -> MemoryResult<()> {
        match (self.accounting.broker().is_some(), entry.charge.is_some()) {
            (true, false) => Err(MemoryError::InvalidState(format!(
                "budgeted revocable entry has no managed charge during {operation}"
            ))),
            (false, true) => Err(MemoryError::InvalidState(format!(
                "unbudgeted revocable entry owns a managed charge during {operation}"
            ))),
            _ => Ok(()),
        }
    }

    /// Checked read that cannot observe a two-phase mutation plan while entries
    /// and their leases are temporarily outside the container.
    fn lock_read(&self) -> MemoryResult<MutexGuard<'_, RevocableState<Key, Value>>> {
        self.lifecycle.ensure_readable()?;
        let state = self.lock_state()?;
        self.lifecycle.ensure_readable()?;
        Ok(state)
    }

    pub fn committed_bytes(&self) -> MemoryResult<u64> {
        Ok(self.lock_read()?.committed_bytes)
    }

    /// Reclaim oldest byte-bearing entries until `target_bytes` is satisfied.
    /// Accounting executes outside the state mutex. Failure restores every
    /// payload, FIFO position, byte counter, and live charge.
    pub fn reclaim_until(&self, target_bytes: u64) -> MemoryResult<SweepStats> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let (mut plan, next_committed, reclaimed_bytes) = {
            let mut state = self.lock_state()?;
            let mut selected = Vec::new();
            let mut reclaimed_bytes = 0u64;
            for (index, entry) in state.entries.iter().enumerate() {
                if reclaimed_bytes >= target_bytes {
                    break;
                }
                if entry.charged_bytes == 0 {
                    continue;
                }
                self.require_charge_shape(entry, "reclaim")?;
                reclaimed_bytes =
                    reclaimed_bytes.checked_add(entry.charged_bytes).ok_or_else(|| {
                        MemoryError::capacity("revocable reclaim byte accounting overflow")
                    })?;
                selected.push(index);
            }
            let next_committed =
                state.committed_bytes.checked_sub(reclaimed_bytes).ok_or_else(|| {
                    MemoryError::InvalidState(
                        "revocable committed bytes are smaller than reclaimed bytes".to_string(),
                    )
                })?;
            let plan = extract_entries(&mut state.entries, &selected);
            (plan, next_committed, reclaimed_bytes)
        };

        if plan.is_empty() {
            return Ok(SweepStats { reclaimed_items: 0, reclaimed_bytes: 0 });
        }

        let reclaimed_items = plan.len();
        let (charge_slots, mut charges) = take_charges(&mut plan);
        if let Err(error) = self.accounting.release_many(&mut charges) {
            restore_charges(&mut plan, &charge_slots, charges);
            restore_entries(&mut self.lock_state_for_commit().entries, plan);
            return Err(error);
        }

        self.lock_state_for_commit().committed_bytes = next_committed;
        retire_entries(plan.into_iter().map(|(_, entry)| entry).collect());
        Ok(SweepStats { reclaimed_items, reclaimed_bytes })
    }

    /// Atomically release every entry and enter the terminal closed state.
    pub fn close(&self) -> MemoryResult<()> {
        let result = self.close_inner();
        self.accounting.emit_result(MemoryOperation::Close, None, &result);
        result
    }

    fn close_inner(&self) -> MemoryResult<()> {
        let close = match self.lifecycle.begin_close()? {
            BeginClose::AlreadyClosed => return Ok(()),
            BeginClose::Acquired(close) => close,
        };

        let mut plan = {
            let mut state = self.lock_state()?;
            #[cfg(feature = "spill")]
            if !state.pending_spills.is_empty() || !state.pending_restores.is_empty() {
                return Err(MemoryError::OperationInProgress(
                    "cannot close revocable store with unresolved durable transfers".into(),
                ));
            }
            for entry in &state.entries {
                self.require_charge_shape(entry, "close")?;
            }
            std::mem::take(&mut state.entries).into_iter().enumerate().collect::<Vec<_>>()
        };
        let (charge_slots, mut charges) = take_charges(&mut plan);
        if let Err(error) = self.accounting.release_many(&mut charges) {
            restore_charges(&mut plan, &charge_slots, charges);
            restore_entries(&mut self.lock_state_for_commit().entries, plan);
            return Err(error);
        }

        self.lock_state_for_commit().committed_bytes = 0;
        close.commit();
        retire_entries(plan.into_iter().map(|(_, entry)| entry).collect());
        Ok(())
    }

    #[cfg(test)]
    fn broker(&self) -> Option<&LiveProcessMemoryBroker> {
        self.accounting.broker()
    }
}

impl<Key, Value> SimpleRevocableStore<Key, Value, Budgeted> {
    pub(crate) fn new_budgeted(
        binding: StoreBindingHandle,
        broker: LiveProcessMemoryBroker,
    ) -> Self {
        Self::with_accounting(binding, StoreAccounting::Budgeted(broker))
    }
}

#[cfg(feature = "unbudgeted-stores")]
impl<Key, Value> SimpleRevocableStore<Key, Value, Unbudgeted> {
    pub(crate) fn new_unbudgeted(binding: StoreBindingHandle) -> Self {
        Self::with_accounting(binding, StoreAccounting::Unbudgeted)
    }
}

impl<Key, Value, Mode> RevocableStore<Key, Value> for SimpleRevocableStore<Key, Value, Mode>
where
    Key: ByteSized,
    Value: ByteSized,
{
    fn binding(&self) -> &StoreBindingHandle {
        self.binding.as_ref()
    }

    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let mut key = InFlight::new(key);
        let mut value = InFlight::new(value);
        let charged_bytes = revocable_entry_cost(key.get(), value.get())?;
        let next_committed =
            self.lock_state()?.committed_bytes.checked_add(charged_bytes).ok_or_else(|| {
                MemoryError::capacity("revocable committed byte accounting overflow")
            })?;

        let charge = self.accounting.charge(
            self.binding.class.clone(),
            self.binding.owner.clone(),
            MemoryContextKind::Runtime,
            ChargeKind::LiveScratch,
            charged_bytes,
        )?;

        let mut state = self.lock_state()?;
        state.entries.push_back(RevocableEntry {
            key: key.take(),
            value: value.take(),
            charged_bytes,
            charge,
            #[cfg(feature = "spill")]
            restore_claim: None,
        });
        state.committed_bytes = next_committed;
        Ok(())
    }

    fn reclaim_one(&self) -> MemoryResult<Option<(Key, Value)>> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let (mut entry, next_committed) = {
            let mut state = self.lock_state()?;
            let Some(entry) = state.entries.front() else {
                return Ok(None);
            };
            self.require_charge_shape(entry, "single reclaim")?;
            let next_committed =
                state.committed_bytes.checked_sub(entry.charged_bytes).ok_or_else(|| {
                    MemoryError::InvalidState(
                        "revocable committed bytes are smaller than reclaimed entry".to_string(),
                    )
                })?;
            (state.entries.pop_front().expect("front entry was checked"), next_committed)
        };

        let mut charges = entry.charge.take().into_iter().collect::<Vec<_>>();
        if let Err(error) = self.accounting.release_many(&mut charges) {
            entry.charge = charges.pop();
            self.lock_state_for_commit().entries.push_front(entry);
            return Err(error);
        }

        self.lock_state_for_commit().committed_bytes = next_committed;
        Ok(Some((entry.key, entry.value)))
    }

    fn stats(&self) -> MemoryResult<StoreStats> {
        let state = self.lock_read()?;
        let item_count = state.entries.len();
        #[cfg(feature = "spill")]
        let item_count = item_count + state.pending_spills.len() + state.pending_restores.len();
        Ok(StoreStats { item_count, estimated_bytes: state.committed_bytes })
    }

    fn close(&self) -> MemoryResult<()> {
        SimpleRevocableStore::close(self)
    }
}

impl<Key, Value, Mode> Drop for SimpleRevocableStore<Key, Value, Mode> {
    fn drop(&mut self) {
        let _ = self.lifecycle.transition_to_closed_for_drop();
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        let entries = std::mem::take(&mut state.entries);
        #[cfg(feature = "spill")]
        let pending = std::mem::take(&mut state.pending_spills).into_values().collect();
        #[cfg(feature = "spill")]
        let pending_restores = std::mem::take(&mut state.pending_restores).into_values().collect();
        state.committed_bytes = 0;
        retire_entries(entries);
        #[cfg(feature = "spill")]
        retire_entries(pending);
        #[cfg(feature = "spill")]
        retire_entries(pending_restores);
    }
}

fn extract_entries<Key, Value>(
    entries: &mut VecDeque<RevocableEntry<Key, Value>>,
    indices: &[usize],
) -> Vec<(usize, RevocableEntry<Key, Value>)> {
    if indices.is_empty() {
        return Vec::new();
    }
    let mut plan = Vec::with_capacity(indices.len());
    let mut selected = indices.iter().copied().peekable();
    let original = std::mem::take(entries);
    for (index, entry) in original.into_iter().enumerate() {
        if selected.peek() == Some(&index) {
            selected.next();
            plan.push((index, entry));
        } else {
            entries.push_back(entry);
        }
    }
    assert!(selected.next().is_none(), "selected revocable index must remain present");
    plan
}

fn restore_entries<Key, Value>(
    entries: &mut VecDeque<RevocableEntry<Key, Value>>,
    plan: Vec<(usize, RevocableEntry<Key, Value>)>,
) {
    let mut retained = std::mem::take(entries);
    let total = retained.len() + plan.len();
    let mut selected = plan.into_iter().peekable();
    for index in 0..total {
        if selected.peek().is_some_and(|(selected_index, _)| *selected_index == index) {
            entries.push_back(selected.next().expect("selected entry").1);
        } else {
            entries.push_back(retained.pop_front().expect("retained revocable entry"));
        }
    }
    assert!(
        selected.next().is_none() && retained.is_empty(),
        "revocable rollback must restore every entry"
    );
}

fn take_charges<Key, Value>(
    plan: &mut [(usize, RevocableEntry<Key, Value>)],
) -> (Vec<usize>, Vec<ManagedCharge>) {
    let mut slots = Vec::new();
    let mut charges = Vec::new();
    for (slot, (_, entry)) in plan.iter_mut().enumerate() {
        if let Some(charge) = entry.charge.take() {
            slots.push(slot);
            charges.push(charge);
        }
    }
    (slots, charges)
}

fn restore_charges<Key, Value>(
    plan: &mut [(usize, RevocableEntry<Key, Value>)],
    slots: &[usize],
    charges: Vec<ManagedCharge>,
) {
    for (slot, charge) in slots.iter().copied().zip(charges) {
        plan[slot].1.charge = Some(charge);
    }
}

#[cfg(test)]
mod tests;
