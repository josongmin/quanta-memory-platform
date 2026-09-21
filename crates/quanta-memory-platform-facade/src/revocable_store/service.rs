use std::marker::PhantomData;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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

#[derive(Debug)]
struct RevocableEntry<Key, Value> {
    key: Key,
    value: Value,
    charged_bytes: u64,
    /// Non-clone authority that always travels with its retained payload.
    charge: Option<ManagedCharge>,
}

/// Detached revocable entry that still owns its original accounting lease.
/// Moving this value to another in-process container keeps resident bytes
/// charged. Dropping it is an explicit discard and releases the lease.
#[derive(Debug)]
pub struct ReclaimedEntry<Key, Value> {
    binding: StoreBindingHandle,
    source_capability: Arc<()>,
    key: Key,
    value: Value,
    charged_bytes: u64,
    charge: Option<ManagedCharge>,
}

impl<Key, Value> ReclaimedEntry<Key, Value> {
    #[must_use]
    pub fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    #[must_use]
    pub fn key(&self) -> &Key {
        &self.key
    }

    #[must_use]
    pub fn value(&self) -> &Value {
        &self.value
    }

    #[must_use]
    pub fn charged_bytes(&self) -> u64 {
        self.charged_bytes
    }

    /// Explicitly discard the detached entry. A failed ledger release returns
    /// the complete entry and live charge for source recovery or retry.
    pub fn discard(mut self) -> Result<(Key, Value), ReclaimFinalizeError<Key, Value>> {
        if let Some(charge) = self.charge.as_mut() {
            if let Err(error) = charge.release() {
                return Err(ReclaimFinalizeError { error, entry: Box::new(self) });
            }
        }
        self.charge = None;
        Ok((self.key, self.value))
    }
}

#[derive(Debug)]
pub struct ReclaimFinalizeError<Key, Value> {
    error: MemoryError,
    entry: Box<ReclaimedEntry<Key, Value>>,
}

impl<Key, Value> ReclaimFinalizeError<Key, Value> {
    #[must_use]
    pub fn error(&self) -> &MemoryError {
        &self.error
    }

    #[must_use]
    pub fn into_parts(self) -> (MemoryError, ReclaimedEntry<Key, Value>) {
        (self.error, *self.entry)
    }
}

#[derive(Debug)]
pub struct RestoreReclaimedError<Key, Value> {
    error: MemoryError,
    entry: Box<ReclaimedEntry<Key, Value>>,
}

impl<Key, Value> RestoreReclaimedError<Key, Value> {
    #[must_use]
    pub fn new(error: MemoryError, entry: ReclaimedEntry<Key, Value>) -> Self {
        Self { error, entry: Box::new(entry) }
    }

    #[must_use]
    pub fn error(&self) -> &MemoryError {
        &self.error
    }

    #[must_use]
    pub fn into_parts(self) -> (MemoryError, ReclaimedEntry<Key, Value>) {
        (self.error, *self.entry)
    }
}

fn retire_entries<Key, Value>(entries: Vec<RevocableEntry<Key, Value>>) {
    for entry in entries {
        let RevocableEntry { key, value, charge, .. } = entry;
        drop(charge);
        retire_client_value(key);
        retire_client_value(value);
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
    source_capability: Arc<()>,
    accounting: StoreAccounting,
    lifecycle: StoreLifecycle,
    state: Mutex<RevocableState<Key, Value>>,
    _mode: PhantomData<fn() -> Mode>,
}

impl<Key, Value, Mode> SimpleRevocableStore<Key, Value, Mode> {
    fn with_accounting(binding: StoreBindingHandle, accounting: StoreAccounting) -> Self {
        Self {
            binding,
            source_capability: Arc::new(()),
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

    fn prepare_reclaim_one(&self) -> MemoryResult<Option<ReclaimedEntry<Key, Value>>> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let (entry, next_committed) = {
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
        self.lock_state_for_commit().committed_bytes = next_committed;
        Ok(Some(ReclaimedEntry {
            binding: self.binding.clone(),
            source_capability: Arc::clone(&self.source_capability),
            key: entry.key,
            value: entry.value,
            charged_bytes: entry.charged_bytes,
            charge: entry.charge,
        }))
    }

    fn restore_reclaimed(
        &self,
        entry: ReclaimedEntry<Key, Value>,
    ) -> Result<(), RestoreReclaimedError<Key, Value>> {
        let _mutation = match self.lifecycle.begin_mutation() {
            Ok(mutation) => mutation,
            Err(error) => return Err(RestoreReclaimedError { error, entry: Box::new(entry) }),
        };
        if entry.binding != self.binding
            || !Arc::ptr_eq(&entry.source_capability, &self.source_capability)
        {
            return Err(RestoreReclaimedError {
                error: MemoryError::InvalidState(
                    "reclaimed entry belongs to a different store binding".to_string(),
                ),
                entry: Box::new(entry),
            });
        }
        match (self.accounting.broker().is_some(), entry.charge.is_some()) {
            (true, false) => {
                return Err(RestoreReclaimedError {
                    error: MemoryError::InvalidState(
                        "budgeted reclaimed entry has no managed charge".to_string(),
                    ),
                    entry: Box::new(entry),
                });
            }
            (false, true) => {
                return Err(RestoreReclaimedError {
                    error: MemoryError::InvalidState(
                        "unbudgeted reclaimed entry owns a managed charge".to_string(),
                    ),
                    entry: Box::new(entry),
                });
            }
            _ => {}
        }
        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(error) => return Err(RestoreReclaimedError { error, entry: Box::new(entry) }),
        };
        let Some(next_committed) = state.committed_bytes.checked_add(entry.charged_bytes) else {
            return Err(RestoreReclaimedError {
                error: MemoryError::capacity("revocable restore byte accounting overflow"),
                entry: Box::new(entry),
            });
        };
        state.entries.insert(
            0,
            RevocableEntry {
                key: entry.key,
                value: entry.value,
                charged_bytes: entry.charged_bytes,
                charge: entry.charge,
            },
        );
        state.committed_bytes = next_committed;
        Ok(())
    }

    fn discard_one(&self) -> MemoryResult<Option<(Key, Value)>> {
        let Some(entry) = self.prepare_reclaim_one()? else {
            return Ok(None);
        };
        match entry.discard() {
            Ok(parts) => Ok(Some(parts)),
            Err(failure) => {
                let (release_error, entry) = failure.into_parts();
                match self.restore_reclaimed(entry) {
                    Ok(()) => Err(release_error),
                    Err(restore_failure) => {
                        let (restore_error, entry) = restore_failure.into_parts();
                        drop(entry);
                        Err(MemoryError::InvalidState(format!(
                            "destructive reclaim release failed ({release_error}); source restore also failed ({restore_error})"
                        )))
                    }
                }
            }
        }
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
    fn discard_one_is_fifo_and_releases_its_bytes() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".to_string(), vec![0; 4]).expect("first");
        store.insert_checked("second".to_string(), vec![0; 6]).expect("second");
        let expected = expected_entry_cost("first", 4) + expected_entry_cost("second", 6);
        assert_eq!(store.stats().expect("stats").estimated_bytes, expected);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);

        let (key, value) = store.discard_one().expect("discard").expect("entry");
        assert_eq!((key.as_str(), value.len()), ("first", 4));
        assert_eq!(store.committed_bytes().expect("bytes"), expected_entry_cost("second", 6));
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            expected_entry_cost("second", 6)
        );
    }

    #[test]
    fn prepared_reclaim_preserves_charge_until_explicit_discard() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".to_string(), vec![0; 4]).expect("insert");
        let expected = expected_entry_cost("first", 4);

        let prepared = store.prepare_reclaim_one().expect("prepare").expect("entry");
        assert_eq!(prepared.charged_bytes(), expected);
        assert_eq!(store.stats().expect("source stats"), StoreStats::default());
        assert_eq!(broker.ledger_snapshot().expect("charge remains").total_owner_bytes(), expected);

        let (key, value) = prepared.discard().expect("explicit discard");
        assert_eq!((key.as_str(), value.len()), ("first", 4));
        assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
    }

    #[test]
    fn failed_restore_to_same_binding_but_wrong_instance_returns_recovery_authority() {
        let broker = live_broker(1_000_000);
        let source = store_with_broker(&broker);
        let wrong = store_with_broker(&broker);
        source.insert_checked("first".to_string(), vec![0; 4]).expect("insert");
        let prepared = source.prepare_reclaim_one().expect("prepare").expect("entry");

        let failure = wrong.restore_reclaimed(prepared).expect_err("wrong source must reject");
        let (error, prepared) = failure.into_parts();
        assert!(matches!(error, MemoryError::InvalidState(_)));
        source.restore_reclaimed(prepared).expect("restore exact source");
        assert_eq!(source.stats().expect("restored").item_count, 1);
    }

    #[cfg(feature = "spill")]
    #[test]
    fn reclaim_to_in_memory_tier_keeps_bytes_charged() {
        use crate::spill::{SpillController, SpillOwnership};
        use crate::{RetainingSpillTarget, RoutedSpillController};

        let broker = live_broker(1_000_000);
        let source = store_with_broker(&broker);
        source.insert_checked("first".to_string(), vec![0; 4]).expect("insert");
        let expected = expected_entry_cost("first", 4);
        let prepared = source.prepare_reclaim_one().expect("prepare").expect("entry");
        let tier = RoutedSpillController::new(RetainingSpillTarget::available());

        let receipt = tier.spill(prepared).expect("in-process transfer");
        assert_eq!(receipt.ownership(), SpillOwnership::InProcessRetained);
        assert_eq!(broker.ledger_snapshot().expect("charged tier").total_owner_bytes(), expected);
        let restored = tier.restore().expect("restore tier").expect("entry");
        source.restore_reclaimed(restored).expect("restore source");
        assert_eq!(source.stats().expect("source").estimated_bytes, expected);
        assert_eq!(broker.ledger_snapshot().expect("one charge").total_owner_bytes(), expected);
    }

    #[cfg(feature = "spill")]
    #[test]
    fn failed_destination_write_preserves_source_entry_and_charge() {
        use crate::spill::SpillController;
        use crate::{RetainingSpillTarget, RoutedSpillController};

        let broker = live_broker(1_000_000);
        let source = store_with_broker(&broker);
        source.insert_checked("first".to_string(), vec![0; 4]).expect("insert");
        let expected = expected_entry_cost("first", 4);
        let prepared = source.prepare_reclaim_one().expect("prepare").expect("entry");
        let offline = RoutedSpillController::new(RetainingSpillTarget::unavailable());

        let failure = offline.spill(prepared).expect_err("offline destination");
        let (_, prepared) = failure.into_parts();
        assert_eq!(
            broker.ledger_snapshot().expect("charge preserved").total_owner_bytes(),
            expected
        );
        source.restore_reclaimed(prepared).expect("source recovery");
        assert_eq!(source.stats().expect("restored").estimated_bytes, expected);
    }

    #[cfg(feature = "spill-redb")]
    #[test]
    fn durable_spill_releases_source_only_after_commit() {
        use crate::spill::backings::redb::RedbSpillBacking;
        use crate::spill::SpillOwnership;
        use crate::RoutedSpillController;
        use std::sync::atomic::{AtomicU64, Ordering};

        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "qmp-reclaimed-durable-transfer-{}-{}.redb",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let broker = live_broker(1_000_000);
        let source = store_with_broker(&broker);
        source.insert_checked("first".to_string(), vec![1, 2, 3, 4]).expect("insert");
        let prepared = source.prepare_reclaim_one().expect("prepare").expect("entry");
        let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("open"));

        let receipt = controller
            .spill_reclaimed_durable(prepared, |key, value| {
                let mut encoded = key.as_bytes().to_vec();
                encoded.push(0);
                encoded.extend_from_slice(value);
                Ok(encoded)
            })
            .expect("durable commit");
        assert_eq!(receipt.ownership(), SpillOwnership::DurableCommitted);
        assert_eq!(broker.ledger_snapshot().expect("source released").total_owner_bytes(), 0);
        assert_eq!(controller.spilled_count::<Vec<u8>>().expect("inventory"), 1);
        drop(controller);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn discard_one_on_empty_returns_none() {
        let store = store();
        assert!(store.discard_one().expect("empty").is_none());
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
        assert_eq!(store.discard_one().expect("last remains").expect("entry").0, "b");
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
        let (key, value) = store.discard_one().expect("revoke").expect("entry");
        assert_eq!((key.as_str(), value.len()), ("only", 9));
        assert_eq!(store.committed_bytes().expect("bytes"), 0);
        assert!(store.discard_one().expect("second revoke").is_none());
    }

    #[test]
    fn empty_reclaim_leaves_state_byte_for_byte_unchanged() {
        let store = store();
        let before = store.stats().expect("before");
        assert!(store.discard_one().expect("empty").is_none());
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
        store.discard_one().expect("one").expect("entry");
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
        assert!(matches!(store.discard_one(), Err(MemoryError::Closed(_))));
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
        assert_eq!(store.discard_one().expect("fifo").expect("entry").0, "a");
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
        assert_eq!(store.discard_one().expect("fifo").expect("entry").0, "a");
    }

    #[test]
    fn reclaim_one_during_rebuild_is_fail_closed() {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("a".into(), vec![0; 4]).expect("a");
        store.insert_checked("b".into(), vec![0; 6]).expect("b");
        broker.begin_rebuild().expect("begin rebuild");
        assert!(matches!(store.discard_one(), Err(MemoryError::InvalidState(_))));
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
        assert_eq!(store.discard_one().expect("next").expect("entry").0, "middle");
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
