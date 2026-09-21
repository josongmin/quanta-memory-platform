use std::marker::PhantomData;
use std::sync::{Mutex, MutexGuard, PoisonError};

use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, ManagedCharge, MemoryContextKind, MemoryError,
    MemoryResult, StoreBindingHandle, StoreStats, StoreWriteError, SweepStats,
};

use crate::revocable_store::RevocableStore;
#[cfg(feature = "unbudgeted-stores")]
use crate::shared::accounting::Unbudgeted;
use crate::shared::accounting::{Budgeted, StoreAccounting};
use crate::shared::lifecycle::{retire_client_value, BeginClose, StoreLifecycle};
use crate::shared::measure::{semantic_cost_v1, ByteSized, RetainedCostV1};
#[cfg(feature = "spill")]
use crate::spill::{InProcessSpillController, SpillFailure};

#[derive(Debug)]
struct RevocableEntry<Key, Value> {
    key: Key,
    value: Value,
    charged_bytes: u64,
    /// Non-clone authority that always travels with its retained payload.
    charge: Option<ManagedCharge>,
}

/// In-process transfer authority. The value, its semantic cost, and the
/// issuing broker lease move together; dropping an un-restored handle retires
/// them together. This is not a durable serialization format.
#[cfg(feature = "spill")]
#[derive(Debug)]
pub struct RevocableSpillHandle<Key, Value> {
    binding: StoreBindingHandle,
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

fn retire_entries<Key, Value>(entries: Vec<RevocableEntry<Key, Value>>) {
    for entry in entries {
        retire_entry(entry);
    }
}

#[derive(Debug)]
struct RevocableState<Key, Value> {
    entries: Vec<RevocableEntry<Key, Value>>,
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
/// fn late_attach(store: SimpleRevocableStore<String, Vec<u8>, Budgeted>) {
///     let _ = store.charged_against;
/// }
/// ```
#[derive(Debug)]
pub struct SimpleRevocableStore<Key, Value, Mode> {
    binding: StoreBindingHandle,
    accounting: StoreAccounting,
    lifecycle: StoreLifecycle,
    state: Mutex<RevocableState<Key, Value>>,
    _mode: PhantomData<fn() -> Mode>,
}

impl<Key, Value, Mode> SimpleRevocableStore<Key, Value, Mode> {
    fn with_accounting(binding: StoreBindingHandle, accounting: StoreAccounting) -> Self {
        Self {
            binding,
            accounting,
            lifecycle: StoreLifecycle::new("revocable store"),
            state: Mutex::new(RevocableState { entries: Vec::new(), committed_bytes: 0 }),
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
        let close = match self.lifecycle.begin_close()? {
            BeginClose::AlreadyClosed => return Ok(()),
            BeginClose::Acquired(close) => close,
        };

        let mut plan = {
            let mut state = self.lock_state()?;
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

#[cfg(feature = "spill")]
impl<Key, Value> SimpleRevocableStore<Key, Value, Budgeted> {
    /// Transfer the oldest entry to an in-process tier without refunding its
    /// process-resident charge. A failed destination returns the exact handle,
    /// which is restored at its original FIFO position before the error escapes.
    pub fn spill_one_into<C>(&self, controller: &C) -> MemoryResult<bool>
    where
        C: InProcessSpillController<RevocableSpillHandle<Key, Value>>,
    {
        let _mutation = self.lifecycle.begin_mutation()?;
        let (entry, next_committed) = {
            let mut state = self.lock_state()?;
            let Some(entry) = state.entries.first() else {
                return Ok(false);
            };
            self.require_charge_shape(entry, "spill transfer")?;
            let next_committed =
                state.committed_bytes.checked_sub(entry.charged_bytes).ok_or_else(|| {
                    MemoryError::InvalidState(
                        "revocable committed bytes are smaller than transferred entry".into(),
                    )
                })?;
            (state.entries.remove(0), next_committed)
        };

        let handle = RevocableSpillHandle { binding: self.binding.clone(), entry: Some(entry) };
        match controller.spill(handle) {
            Ok(_) => {
                self.lock_state_for_commit().committed_bytes = next_committed;
                Ok(true)
            }
            Err(failure) => {
                let (error, handle) = failure.into_parts();
                self.lock_state_for_commit().entries.insert(0, handle.into_entry());
                Err(error)
            }
        }
    }

    /// Reattach an in-process tier handle to its original broker/binding. If
    /// admission fails, the caller receives the still-charged handle back and
    /// can retry or explicitly retire it; no hidden release occurs on `Err`.
    pub fn accept_restored(
        &self,
        handle: RevocableSpillHandle<Key, Value>,
    ) -> Result<(), SpillFailure<RevocableSpillHandle<Key, Value>>> {
        let _mutation = match self.lifecycle.begin_mutation() {
            Ok(permit) => permit,
            Err(error) => return Err(SpillFailure::new(error, handle)),
        };
        if handle.binding != self.binding {
            return Err(SpillFailure::new(
                MemoryError::InvalidState("restored handle has a foreign store binding".into()),
                handle,
            ));
        }
        let (charged_bytes, valid_charge) = {
            let entry = handle.entry.as_ref().expect("live spill handle");
            let charge = entry.charge.as_ref().expect("budgeted spill handle owns a charge");
            let broker = self.accounting.broker().expect("budgeted store owns a broker");
            (
                entry.charged_bytes,
                charge.belongs_to(broker)
                    && charge.bytes() == Some(entry.charged_bytes)
                    && charge.class() == Some(&self.binding.class)
                    && charge.owner() == Some(&self.binding.owner)
                    && charge.context() == Some(MemoryContextKind::Runtime)
                    && charge.kind() == Some(ChargeKind::LiveScratch),
            )
        };
        if !valid_charge {
            return Err(SpillFailure::new(
                MemoryError::InvalidState(
                    "restored handle carries foreign charge authority".into(),
                ),
                handle,
            ));
        }
        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(error) => return Err(SpillFailure::new(error, handle)),
        };
        let next_committed = match state.committed_bytes.checked_add(charged_bytes) {
            Some(total) => total,
            None => {
                return Err(SpillFailure::new(
                    MemoryError::capacity("restored revocable byte total exceeds u64"),
                    handle,
                ));
            }
        };
        state.entries.push(handle.into_entry());
        state.committed_bytes = next_committed;
        Ok(())
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
        &self.binding
    }

    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let charged_bytes = RetainedCostV1 {
            payload_bytes: value.byte_size(),
            key_identity_bytes: key.byte_size(),
            index_bytes: semantic_cost_v1::FIFO_INDEX_BYTES,
            fixed_overhead_bytes: semantic_cost_v1::ENTRY_STATE_BYTES,
            ..RetainedCostV1::default()
        }
        .total()?;
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
        state.entries.push(RevocableEntry { key, value, charged_bytes, charge });
        state.committed_bytes = next_committed;
        Ok(())
    }

    fn reclaim_one(&self) -> MemoryResult<Option<(Key, Value)>> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let (mut entry, next_committed) = {
            let mut state = self.lock_state()?;
            let Some(entry) = state.entries.first() else {
                return Ok(None);
            };
            self.require_charge_shape(entry, "single reclaim")?;
            let next_committed =
                state.committed_bytes.checked_sub(entry.charged_bytes).ok_or_else(|| {
                    MemoryError::InvalidState(
                        "revocable committed bytes are smaller than reclaimed entry".to_string(),
                    )
                })?;
            (state.entries.remove(0), next_committed)
        };

        let mut charges = entry.charge.take().into_iter().collect::<Vec<_>>();
        if let Err(error) = self.accounting.release_many(&mut charges) {
            entry.charge = charges.pop();
            self.lock_state_for_commit().entries.insert(0, entry);
            return Err(error);
        }

        self.lock_state_for_commit().committed_bytes = next_committed;
        Ok(Some((entry.key, entry.value)))
    }

    fn stats(&self) -> MemoryResult<StoreStats> {
        let state = self.lock_read()?;
        Ok(StoreStats { item_count: state.entries.len(), estimated_bytes: state.committed_bytes })
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
        state.committed_bytes = 0;
        retire_entries(entries);
    }
}

fn extract_entries<Key, Value>(
    entries: &mut Vec<RevocableEntry<Key, Value>>,
    indices: &[usize],
) -> Vec<(usize, RevocableEntry<Key, Value>)> {
    let mut plan =
        indices.iter().rev().map(|index| (*index, entries.remove(*index))).collect::<Vec<_>>();
    plan.reverse();
    plan
}

fn restore_entries<Key, Value>(
    entries: &mut Vec<RevocableEntry<Key, Value>>,
    plan: Vec<(usize, RevocableEntry<Key, Value>)>,
) {
    for (index, entry) in plan {
        entries.insert(index, entry);
    }
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
mod tests {
    use super::*;
    #[cfg(feature = "spill")]
    use crate::spill::{
        InMemorySpillTarget, InProcessSpillTarget, NoopSpillController, RoutedSpillController,
        SpillController, SpillReceipt, SpillTarget,
    };
    use quanta_memory_platform_core::{
        ExecutionBudget, MemoryBudgetManifest, MemoryClass, MemoryClassBudget, MemoryPreset,
        ProcessMemoryBrokerBuilder, ReclaimPriority, ResidencyPolicy,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    type Store = SimpleRevocableStore<String, Vec<u8>, Budgeted>;

    fn expected_entry_cost(key: &str, payload_bytes: u64) -> u64 {
        u64::try_from(key.len()).expect("test key fits") + payload_bytes + 16 + 16
    }

    fn binding() -> StoreBindingHandle {
        StoreBindingHandle::new(
            MemoryClass::ExecutionScratchRevocable,
            "owner-rev".to_string(),
            None,
        )
    }

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
        ProcessMemoryBrokerBuilder::new()
            .with_manifest(MemoryBudgetManifest::new(
                MemoryPreset::Production,
                ExecutionBudget::default(),
                residency,
            ))
            .build()
            .expect("valid broker")
    }

    fn store_with_broker(broker: &LiveProcessMemoryBroker) -> Store {
        SimpleRevocableStore::new_budgeted(binding(), broker.clone())
    }

    fn store() -> Store {
        store_with_broker(&live_broker(1_000_000))
    }

    struct PanickingValue {
        bytes: Vec<u8>,
        drops: Arc<AtomicUsize>,
    }

    impl ByteSized for PanickingValue {
        fn byte_size(&self) -> u64 {
            self.bytes.len() as u64
        }
    }

    impl Drop for PanickingValue {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
            panic!("client revocable destructor panic");
        }
    }

    #[test]
    fn insert_tracks_reclaimable_bytes() {
        let store = store();
        store.insert_checked("a".into(), vec![0; 4]).expect("a");
        store.insert_checked("b".into(), vec![0; 6]).expect("b");
        let expected = expected_entry_cost("a", 4) + expected_entry_cost("b", 6);
        assert_eq!(store.committed_bytes().expect("bytes"), expected);
        assert_eq!(
            store.stats().expect("stats"),
            StoreStats { item_count: 2, estimated_bytes: expected }
        );
    }

    #[test]
    fn reclaim_one_is_fifo_and_releases_its_bytes() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".to_string(), vec![0; 4]).expect("first");
        store.insert_checked("second".to_string(), vec![0; 6]).expect("second");
        let expected = expected_entry_cost("first", 4) + expected_entry_cost("second", 6);
        assert_eq!(store.stats().expect("stats").estimated_bytes, expected);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);

        let (key, value) = store.reclaim_one().expect("reclaim").expect("entry");
        assert_eq!((key.as_str(), value.len()), ("first", 4));
        assert_eq!(store.committed_bytes().expect("bytes"), expected_entry_cost("second", 6));
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            expected_entry_cost("second", 6)
        );
    }

    #[cfg(feature = "spill")]
    #[test]
    fn reclaim_to_in_memory_tier_keeps_bytes_charged() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".into(), vec![1; 4]).expect("first");
        store.insert_checked("second".into(), vec![2; 6]).expect("second");
        let total = expected_entry_cost("first", 4) + expected_entry_cost("second", 6);
        let controller = RoutedSpillController::new(InMemorySpillTarget::available());

        assert!(store.spill_one_into(&controller).expect("transfer"));
        assert_eq!(store.stats().expect("source stats").item_count, 1);
        assert_eq!(
            store.committed_bytes().expect("source bytes"),
            expected_entry_cost("second", 6)
        );
        assert_eq!(controller.spilled_count(), Ok(1));
        assert_eq!(
            broker.ledger_snapshot().expect("retained tier charge").total_owner_bytes(),
            total
        );

        let handle = controller.restore().expect("restore").expect("retained handle");
        assert_eq!(handle.key(), "first");
        assert_eq!(handle.value(), &vec![1; 4]);
        assert_eq!(handle.charged_bytes(), expected_entry_cost("first", 4));
        assert_eq!(controller.spilled_count(), Ok(0));
        store.accept_restored(handle).expect("reattach original authority");
        assert_eq!(store.stats().expect("restored stats").item_count, 2);
        assert_eq!(broker.ledger_snapshot().expect("restored ledger").total_owner_bytes(), total);
        assert_eq!(store.reclaim_one().expect("remaining FIFO").expect("entry").0, "second");
        assert_eq!(store.reclaim_one().expect("restored entry").expect("entry").0, "first");
        assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
    }

    #[cfg(feature = "spill")]
    #[test]
    fn failed_destination_write_preserves_source_entry_and_charge() {
        struct RejectingTarget;
        impl<Handle> SpillTarget<Handle> for RejectingTarget {
            fn is_available(&self) -> bool {
                true
            }
            fn accept(&self, handle: Handle) -> Result<SpillReceipt, SpillFailure<Handle>> {
                Err(SpillFailure::new(MemoryError::capacity("injected write failure"), handle))
            }
        }
        impl crate::spill::SpillInventory for RejectingTarget {
            fn retained_count(&self) -> MemoryResult<u64> {
                Ok(0)
            }
        }
        impl<Handle> InProcessSpillTarget<Handle> for RejectingTarget {}
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".into(), vec![1; 4]).expect("first");
        store.insert_checked("second".into(), vec![2; 6]).expect("second");
        let total = store.committed_bytes().expect("before");
        let controller = RoutedSpillController::new(RejectingTarget);

        assert!(matches!(store.spill_one_into(&controller), Err(MemoryError::Capacity(_))));
        assert_eq!(controller.spilled_count(), Ok(0));
        assert_eq!(store.committed_bytes().expect("unchanged"), total);
        assert_eq!(store.stats().expect("source stats").item_count, 2);
        assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), total);
        assert_eq!(store.reclaim_one().expect("FIFO").expect("first").0, "first");
    }

    #[cfg(feature = "spill")]
    #[test]
    fn noop_spill_does_not_discard_source() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".into(), vec![1; 4]).expect("insert");
        let total = store.committed_bytes().expect("before");
        assert!(matches!(
            store.spill_one_into(&NoopSpillController),
            Err(MemoryError::Capacity(_))
        ));
        assert_eq!(store.stats().expect("after").item_count, 1);
        assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), total);
        assert_eq!(store.reclaim_one().expect("source").expect("entry").0, "first");
    }

    #[cfg(feature = "spill")]
    #[test]
    fn foreign_store_cannot_adopt_spill_charge() {
        let broker = live_broker(1_000_000);
        let source = store_with_broker(&broker);
        let foreign = store_with_broker(&live_broker(1_000_000));
        source.insert_checked("first".into(), vec![1; 4]).expect("insert");
        let total = source.committed_bytes().expect("before");
        let controller = RoutedSpillController::new(InMemorySpillTarget::available());
        assert!(source.spill_one_into(&controller).expect("spill"));

        let handle = controller.restore().expect("restore").expect("entry");
        let (error, handle) =
            foreign.accept_restored(handle).expect_err("foreign broker must reject").into_parts();
        assert!(matches!(error, MemoryError::InvalidState(_)));
        assert_eq!(broker.ledger_snapshot().expect("original charge").total_owner_bytes(), total);
        source.accept_restored(handle).expect("original owner can restore");
        assert_eq!(source.stats().expect("source").item_count, 1);
        assert_eq!(broker.ledger_snapshot().expect("charge").total_owner_bytes(), total);
    }

    #[cfg(feature = "spill")]
    #[test]
    fn source_close_does_not_refund_live_in_process_tier() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".into(), vec![1; 4]).expect("insert");
        let total = store.committed_bytes().expect("before");
        let controller = RoutedSpillController::new(InMemorySpillTarget::available());

        assert!(store.spill_one_into(&controller).expect("spill"));
        store.close().expect("close source");
        assert_eq!(broker.ledger_snapshot().expect("tier authority").total_owner_bytes(), total);
        drop(controller);
        assert_eq!(broker.ledger_snapshot().expect("tier retired").total_owner_bytes(), 0);
    }

    #[test]
    fn reclaim_one_on_empty_returns_none() {
        let store = store();
        assert!(store.reclaim_one().expect("empty").is_none());
        assert_eq!(store.committed_bytes().expect("bytes"), 0);
    }

    #[test]
    fn reclaim_until_frees_at_least_target_and_reports_reclaimed_bytes() {
        let store = store();
        for key in ["a", "b", "c"] {
            store.insert_checked(key.into(), vec![0; 4]).expect("insert");
        }
        let outcome = store.reclaim_until(expected_entry_cost("a", 4) + 1).expect("reclaim");
        assert_eq!(
            outcome,
            SweepStats { reclaimed_items: 2, reclaimed_bytes: 2 * expected_entry_cost("a", 4) }
        );
        assert_eq!(store.committed_bytes().expect("bytes"), expected_entry_cost("c", 4));
    }

    #[test]
    fn reclaim_until_on_empty_store_reports_zero() {
        assert_eq!(
            store().reclaim_until(100).expect("empty"),
            SweepStats { reclaimed_items: 0, reclaimed_bytes: 0 }
        );
    }

    #[test]
    fn reclaim_until_charges_and_evicts_zero_payload_keys() {
        let store = store();
        store.insert_checked("zero".into(), vec![]).expect("zero");
        store.insert_checked("a".into(), vec![0; 4]).expect("a");
        store.insert_checked("b".into(), vec![0; 4]).expect("b");
        let outcome = store.reclaim_until(expected_entry_cost("zero", 0) + 1).expect("reclaim");
        assert_eq!(
            outcome,
            SweepStats {
                reclaimed_items: 2,
                reclaimed_bytes: expected_entry_cost("zero", 0) + expected_entry_cost("a", 4)
            }
        );
        assert_eq!(store.stats().expect("stats").item_count, 1);
        assert_eq!(store.reclaim_one().expect("last remains").expect("entry").0, "b");
    }

    #[test]
    fn reclaim_until_of_zero_payload_entries_reclaims_their_key_and_index_cost() {
        let store = store();
        for key in ["a", "b", "c"] {
            store.insert_checked(key.into(), vec![]).expect("insert");
        }
        assert_eq!(
            store.reclaim_until(5).expect("reclaim"),
            SweepStats { reclaimed_items: 1, reclaimed_bytes: expected_entry_cost("a", 0) }
        );
        assert_eq!(store.stats().expect("stats").item_count, 2);
    }

    #[test]
    fn revoke_makes_entry_unretrievable_and_decrements_accounting() {
        let store = store();
        store.insert_checked("only".into(), vec![0; 9]).expect("insert");
        let (key, value) = store.reclaim_one().expect("revoke").expect("entry");
        assert_eq!((key.as_str(), value.len()), ("only", 9));
        assert_eq!(store.committed_bytes().expect("bytes"), 0);
        assert!(store.reclaim_one().expect("second revoke").is_none());
    }

    #[test]
    fn empty_reclaim_leaves_state_byte_for_byte_unchanged() {
        let store = store();
        let before = store.stats().expect("before");
        assert!(store.reclaim_one().expect("empty").is_none());
        assert_eq!(store.stats().expect("after"), before);
    }

    #[test]
    fn insert_over_cap_is_denied_and_fails_closed() {
        let broker = live_broker(5);
        let store = store_with_broker(&broker);
        let denied = store.insert_checked("big".into(), vec![0; 9]);
        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        assert_eq!(store.stats().expect("stats"), StoreStats::default());
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn large_key_with_zero_value_is_charged_and_denied_without_state_change() {
        let broker = live_broker(100);
        let store = store_with_broker(&broker);
        let denied = store.insert_checked("k".repeat(101), Vec::new());
        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        assert_eq!(store.stats().expect("unchanged state"), StoreStats::default());
        assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn cost_component_sum_overflow_is_fail_closed_before_charge() {
        struct OversizedKey;
        impl ByteSized for OversizedKey {
            fn byte_size(&self) -> u64 {
                u64::MAX
            }
        }
        let broker = live_broker(1_000);
        let store = SimpleRevocableStore::<OversizedKey, Vec<u8>, Budgeted>::new_budgeted(
            binding(),
            broker.clone(),
        );
        assert!(matches!(
            store.insert_checked(OversizedKey, vec![1]),
            Err(MemoryError::Capacity(_))
        ));
        assert_eq!(store.stats().expect("unchanged state"), StoreStats::default());
        assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn insert_charges_and_reclaim_refunds_the_ledger() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("a".into(), vec![0; 4]).expect("a");
        store.insert_checked("b".into(), vec![0; 6]).expect("b");
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            expected_entry_cost("a", 4) + expected_entry_cost("b", 6)
        );
        store.reclaim_one().expect("one").expect("entry");
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            expected_entry_cost("b", 6)
        );
        store.reclaim_until(100).expect("rest");
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn close_refunds_every_live_entry_exactly_once_and_stays_closed() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("a".into(), vec![0; 4]).expect("a");
        store.insert_checked("b".into(), vec![0; 6]).expect("b");

        store.close().expect("close");
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
        store.close().expect("idempotent close");
        assert!(matches!(
            store.insert_checked("late".into(), vec![0; 1]),
            Err(MemoryError::Closed(_))
        ));
        assert!(matches!(store.reclaim_one(), Err(MemoryError::Closed(_))));
        assert_eq!(store.stats().expect("terminal stats"), StoreStats::default());
    }

    #[test]
    fn close_release_failure_preserves_entries_fifo_and_ledger_authority() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("a".into(), vec![0; 4]).expect("a");
        store.insert_checked("b".into(), vec![0; 6]).expect("b");
        broker.begin_rebuild().expect("begin rebuild");

        assert!(matches!(store.close(), Err(MemoryError::InvalidState(_))));
        assert_eq!(store.stats().expect("restored stats").item_count, 2);
        let expected = expected_entry_cost("a", 4) + expected_entry_cost("b", 6);
        assert_eq!(store.committed_bytes().expect("restored bytes"), expected);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);

        broker.finish_rebuild().expect("finish rebuild");
        assert_eq!(store.reclaim_one().expect("fifo").expect("entry").0, "a");
        store.close().expect("retry close");
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn reclaim_batch_failure_preserves_every_entry_and_charge() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        for (key, bytes) in [("a", 4), ("b", 4), ("c", 4)] {
            store.insert_checked(key.into(), vec![0; bytes]).expect("insert");
        }
        broker.begin_rebuild().expect("begin rebuild");
        assert!(matches!(store.reclaim_until(5), Err(MemoryError::InvalidState(_))));
        assert_eq!(store.stats().expect("stats").item_count, 3);
        let expected = 3 * expected_entry_cost("a", 4);
        assert_eq!(store.committed_bytes().expect("bytes"), expected);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);
        broker.finish_rebuild().expect("finish rebuild");
        assert_eq!(store.reclaim_one().expect("fifo").expect("entry").0, "a");
    }

    #[test]
    fn reclaim_one_during_rebuild_is_fail_closed() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("a".into(), vec![0; 4]).expect("a");
        store.insert_checked("b".into(), vec![0; 6]).expect("b");
        broker.begin_rebuild().expect("begin rebuild");
        assert!(matches!(store.reclaim_one(), Err(MemoryError::InvalidState(_))));
        assert_eq!(store.stats().expect("stats").item_count, 2);
        let expected = expected_entry_cost("a", 4) + expected_entry_cost("b", 6);
        assert_eq!(store.committed_bytes().expect("bytes"), expected);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);
        broker.finish_rebuild().expect("finish rebuild");
    }

    #[test]
    fn reclaim_until_during_rebuild_is_fail_closed() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        for key in ["a", "b", "c"] {
            store.insert_checked(key.into(), vec![0; 4]).expect("insert");
        }
        broker.begin_rebuild().expect("begin rebuild");
        assert!(matches!(store.reclaim_until(5), Err(MemoryError::InvalidState(_))));
        assert_eq!(store.stats().expect("stats").item_count, 3);
        let expected = 3 * expected_entry_cost("a", 4);
        assert_eq!(store.committed_bytes().expect("bytes"), expected);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);
        broker.finish_rebuild().expect("finish rebuild");
    }

    #[test]
    fn drop_without_explicit_close_releases_capacity() {
        let broker = live_broker(1_000_000);
        {
            let store = store_with_broker(&broker);
            store.insert_checked("live".into(), vec![0; 9]).expect("insert");
            assert_eq!(
                broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
                expected_entry_cost("live", 9)
            );
        }
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn drop_during_rebuild_defers_exact_release_until_finish() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("live".into(), vec![0; 9]).expect("insert");
        broker.begin_rebuild().expect("begin rebuild");
        drop(store);
        assert_eq!(
            broker.ledger_snapshot().expect("conservative ledger").total_owner_bytes(),
            expected_entry_cost("live", 9)
        );
        broker.finish_rebuild().expect("finish and drain");
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn drop_during_outer_unwind_contains_two_client_panics_and_refunds_all_charges() {
        let broker = live_broker(1_000_000);
        let drops = Arc::new(AtomicUsize::new(0));
        let store = SimpleRevocableStore::<String, PanickingValue, Budgeted>::new_budgeted(
            binding(),
            broker.clone(),
        );
        for (key, bytes) in [("first", 4), ("second", 6)] {
            store
                .insert_checked(
                    key.into(),
                    PanickingValue { bytes: vec![0; bytes], drops: Arc::clone(&drops) },
                )
                .expect("insert before unwind");
        }
        assert_eq!(
            broker.ledger_snapshot().expect("before").total_owner_bytes(),
            expected_entry_cost("first", 4) + expected_entry_cost("second", 6)
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
    fn concurrent_insert_and_close_have_one_linearized_outcome() {
        let broker = live_broker(1_000_000);
        let store = Arc::new(store_with_broker(&broker));
        let barrier = Arc::new(Barrier::new(2));
        let worker_store = Arc::clone(&store);
        let worker_barrier = Arc::clone(&barrier);
        let worker = thread::spawn(move || {
            worker_barrier.wait();
            worker_store.insert_checked("racing".into(), vec![0; 7])
        });

        barrier.wait();
        loop {
            match store.close() {
                Ok(()) => break,
                Err(MemoryError::OperationInProgress(_)) => thread::yield_now(),
                Err(error) => panic!("unexpected close error: {error}"),
            }
        }
        let insert = worker.join().expect("worker");
        assert!(
            insert.is_ok()
                || matches!(
                    insert,
                    Err(MemoryError::Closed(_) | MemoryError::OperationInProgress(_))
                ),
            "a contending mutation may be rejected as Busy before it acquires the lifecycle lane"
        );
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
        assert_eq!(store.stats().expect("terminal stats"), StoreStats::default());
    }

    #[test]
    fn counter_overflow_is_rejected_before_broker_charge_or_mutation() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.state.lock().expect("state").committed_bytes = u64::MAX;

        let denied = store.insert_checked("overflow".into(), vec![0; 1]);
        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        assert_eq!(store.state.lock().expect("state").entries.len(), 0);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn poisoned_stats_is_error_not_empty_and_drop_still_releases() {
        let broker = live_broker(1_000_000);
        let store = Arc::new(store_with_broker(&broker));
        store.insert_checked("live".into(), vec![0; 5]).expect("insert");
        let poison = Arc::clone(&store);
        assert!(thread::spawn(move || {
            let _guard = poison.state.lock().expect("state");
            panic!("intentional poison");
        })
        .join()
        .is_err());

        assert!(matches!(store.stats(), Err(MemoryError::InvalidState(_))));
        drop(store);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    #[test]
    fn reentrant_read_and_close_under_state_lock_return_typed_busy() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("live".into(), vec![0; 4]).expect("insert");
        let held = store.state.lock().expect("simulated client callback holds state lock");
        assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
        assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
        drop(held);
        assert_eq!(
            store.stats().expect("open postimage").estimated_bytes,
            expected_entry_cost("live", 4)
        );
        store.close().expect("retry after callback");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }

    #[test]
    fn constructed_budgeted_store_exposes_its_single_broker_authority() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        assert!(store.broker().is_some());
    }

    #[test]
    fn reclaim_eviction_order_is_deterministic_fifo_at_capacity_boundary() {
        let store = store();
        store.insert_checked("oldest".into(), vec![0; 4]).expect("oldest");
        store.insert_checked("middle".into(), vec![0; 4]).expect("middle");
        store.insert_checked("newest".into(), vec![0; 4]).expect("newest");
        assert_eq!(
            store.reclaim_until(1).expect("reclaim"),
            SweepStats { reclaimed_items: 1, reclaimed_bytes: expected_entry_cost("oldest", 4) }
        );
        assert_eq!(store.reclaim_one().expect("next").expect("entry").0, "middle");
    }

    #[cfg(feature = "unbudgeted-stores")]
    #[test]
    fn unbudgeted_store_is_a_distinct_explicit_product_type() {
        let store = SimpleRevocableStore::<String, Vec<u8>, Unbudgeted>::new_unbudgeted(binding());
        store.insert_checked("fixture".into(), vec![0; 3]).expect("insert");
        assert_eq!(
            store.stats().expect("stats").estimated_bytes,
            expected_entry_cost("fixture", 3)
        );
        store.close().expect("close");
    }
}
