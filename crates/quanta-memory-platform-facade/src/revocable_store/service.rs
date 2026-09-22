#[cfg(feature = "spill")]
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, ManagedCharge, MemoryContextKind, MemoryError,
    MemoryOperation, MemoryResult, StoreBindingHandle, StoreStats, StoreWriteError, SweepStats,
};

use crate::revocable_store::RevocableStore;
#[cfg(feature = "unbudgeted-stores")]
use crate::shared::accounting::Unbudgeted;
use crate::shared::accounting::{Budgeted, StoreAccounting};
use crate::shared::lifecycle::{retire_client_value, BeginClose, StoreLifecycle};
use crate::shared::measure::{semantic_cost_v1, ByteSized, RetainedCostV1};
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
    /// Restore claim that produced this copy. `None` for ordinary entries;
    /// `Some` from publication of a durable restore. A fenced provisional copy
    /// presents this exact claimant at reconciliation; on a successful restore
    /// the claim is inert history (the record is already consumed).
    #[cfg(feature = "spill")]
    restore_claim: Option<DurableRestoreClaim>,
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
    RetainedCostV1 {
        payload_bytes: value.byte_size(),
        key_identity_bytes: key.byte_size(),
        index_bytes: semantic_cost_v1::FIFO_INDEX_BYTES,
        fixed_overhead_bytes: semantic_cost_v1::ENTRY_STATE_BYTES,
        ..RetainedCostV1::default()
    }
    .total()
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

#[cfg(feature = "spill")]
impl<Key, Value> SimpleRevocableStore<Key, Value, Budgeted> {
    /// Transfer the oldest entry to an in-process tier without refunding its
    /// process-resident charge. A failed destination returns the exact handle,
    /// which is restored at its original FIFO position before the error escapes.
    pub fn spill_one_into<C>(&self, controller: &C) -> MemoryResult<bool>
    where
        C: InProcessSpillController<RevocableSpillHandle<Key, Value>>,
    {
        let result = self.spill_one_into_inner(controller);
        self.accounting.emit_result(MemoryOperation::SpillInProcess, None, &result);
        result
    }

    fn spill_one_into_inner<C>(&self, controller: &C) -> MemoryResult<bool>
    where
        C: InProcessSpillController<RevocableSpillHandle<Key, Value>>,
    {
        let _mutation = self.lifecycle.begin_mutation()?;
        let (entry, next_committed) = {
            let mut state = self.lock_state()?;
            let Some(entry) = state.entries.front() else {
                return Ok(false);
            };
            self.require_charge_shape(entry, "spill transfer")?;
            let next_committed =
                state.committed_bytes.checked_sub(entry.charged_bytes).ok_or_else(|| {
                    MemoryError::InvalidState(
                        "revocable committed bytes are smaller than transferred entry".into(),
                    )
                })?;
            (state.entries.pop_front().expect("front entry was checked"), next_committed)
        };

        let handle = RevocableSpillHandle { binding: self.binding.clone(), entry: Some(entry) };
        match controller.spill(handle) {
            Ok(_) => {
                self.lock_state_for_commit().committed_bytes = next_committed;
                Ok(true)
            }
            Err(failure) => {
                let (error, handle) = failure.into_parts();
                self.lock_state_for_commit().entries.push_front(handle.into_entry());
                Err(error)
            }
        }
    }

    /// Encode and commit the oldest entry to a durable tier. The source entry
    /// and its live charge remain authoritative until the destination returns a
    /// durable commit receipt. Encoder errors, encoder panics, and ordinary
    /// destination errors restore the original FIFO entry before propagating.
    pub fn spill_one_durable<C, Encode>(
        &self,
        controller: &C,
        encode: Encode,
    ) -> MemoryResult<Option<SpillReceipt>>
    where
        C: ReconciledDurableSpillController,
        Encode: FnOnce(&Key, &Value) -> MemoryResult<Vec<u8>>,
    {
        let result = self.spill_one_durable_inner(controller, encode);
        self.accounting.emit_result(MemoryOperation::SpillDurable, None, &result);
        result
    }

    fn spill_one_durable_inner<C, Encode>(
        &self,
        controller: &C,
        encode: Encode,
    ) -> MemoryResult<Option<SpillReceipt>>
    where
        C: ReconciledDurableSpillController,
        Encode: FnOnce(&Key, &Value) -> MemoryResult<Vec<u8>>,
    {
        let _mutation = self.lifecycle.begin_mutation()?;
        let bytes = {
            let state = self.lock_state()?;
            if !state.pending_spills.is_empty() {
                return Err(MemoryError::OperationInProgress(
                    "durable spill reconciliation is required before another spill".into(),
                ));
            }
            let Some(entry) = state.entries.front() else {
                return Ok(None);
            };
            self.require_charge_shape(entry, "durable spill transfer")?;
            let encoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                encode(&entry.key, &entry.value)
            }));
            drop(state);
            match encoded {
                Ok(result) => result?,
                Err(panic) => std::panic::resume_unwind(panic),
            }
        };

        let record_id = controller.reserve_record_id()?;
        let committed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            controller.commit_spill(record_id, &bytes)
        }));
        let result = match committed {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        };

        match result {
            Ok(receipt @ SpillReceipt::Durable { sequence, .. }) if sequence == record_id.get() => {
                let entry = {
                    let mut state = self.lock_state_for_commit();
                    let entry = state.entries.pop_front().expect("front entry was checked");
                    state.committed_bytes = state
                        .committed_bytes
                        .checked_sub(entry.charged_bytes)
                        .expect("validated committed bytes cover first entry");
                    entry
                };
                retire_entry(entry);
                Ok(Some(receipt))
            }
            Ok(receipt) => {
                let mut state = self.lock_state_for_commit();
                let entry = state.entries.pop_front().expect("front entry was checked");
                state.pending_spills.insert(record_id, entry);
                Err(MemoryError::OperationInProgress(format!(
                    "durable spill returned mismatched receipt {receipt:?}; record {} requires reconciliation",
                    record_id.get()
                )))
            }
            Err(DurableWriteFailure::NotCommitted(error)) => Err(*error),
            Err(DurableWriteFailure::RecoveryRequired { record_id, commit_error, probe_error }) => {
                let mut state = self.lock_state_for_commit();
                let entry = state.entries.pop_front().expect("front entry was checked");
                state.pending_spills.insert(record_id, entry);
                Err(MemoryError::OperationInProgress(format!(
                    "durable record {} requires reconciliation after commit error ({commit_error}) and probe error ({probe_error})",
                    record_id.get()
                )))
            }
        }
    }

    /// IDs whose source entries are fenced after an unresolved durable commit.
    pub fn pending_durable_spills(&self) -> MemoryResult<Vec<DurableRecordId>> {
        Ok(self.lock_read()?.pending_spills.keys().copied().collect())
    }

    /// Resolve one fenced durable write. A present record becomes authoritative
    /// and releases the source charge exactly once; an absent record restores
    /// the original FIFO source entry without changing its charge.
    pub fn reconcile_durable_spill<C>(
        &self,
        controller: &C,
        record_id: DurableRecordId,
    ) -> MemoryResult<DurableRecordResolution>
    where
        C: ReconciledDurableSpillController,
    {
        let _mutation = self.lifecycle.begin_mutation()?;
        let resolution = controller.resolve_record(record_id)?;
        let mut state = self.lock_state()?;
        let entry_bytes = state
            .pending_spills
            .get(&record_id)
            .ok_or_else(|| {
                MemoryError::InvalidState(format!(
                    "durable record {} has no fenced source authority",
                    record_id.get()
                ))
            })?
            .charged_bytes;
        match resolution {
            DurableRecordResolution::Present { .. } | DurableRecordResolution::Finalized => {
                let next_committed =
                    state.committed_bytes.checked_sub(entry_bytes).ok_or_else(|| {
                        MemoryError::InvalidState(
                            "revocable committed bytes are smaller than reconciled spill".into(),
                        )
                    })?;
                let entry = state
                    .pending_spills
                    .remove(&record_id)
                    .expect("pending spill was checked under the mutation permit");
                state.committed_bytes = next_committed;
                drop(state);
                retire_entry(entry);
            }
            DurableRecordResolution::Absent => {
                let entry = state
                    .pending_spills
                    .remove(&record_id)
                    .expect("pending spill was checked under the mutation permit");
                state.entries.push_front(entry);
            }
            // Defensive only: the write probe never reports a superseded
            // restore claim. Keep the source fenced rather than guess.
            DurableRecordResolution::Superseded => {
                return Err(MemoryError::InvalidState(format!(
                    "durable spill {} unexpectedly resolved a superseded restore claim",
                    record_id.get()
                )))
            }
        }
        Ok(resolution)
    }

    /// Restore the oldest durable record without a transient unowned window.
    /// The destination is first claimed and read non-destructively; the source
    /// charge is established before the exact durable sequence is deleted, then
    /// the entry is published locally. Decode failure or admission denial aborts
    /// the claim and leaves the durable record authoritative and unchanged.
    pub fn restore_one_durable<C, Decode>(
        &self,
        controller: &C,
        decode: Decode,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController,
        Decode: FnOnce(&[u8]) -> MemoryResult<(Key, Value)>,
        Key: ByteSized,
        Value: ByteSized,
    {
        let result = self.restore_one_durable_inner(controller, decode);
        self.accounting.emit_result(MemoryOperation::RestoreDurable, None, &result);
        result
    }

    fn restore_one_durable_inner<C, Decode>(
        &self,
        controller: &C,
        decode: Decode,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController,
        Decode: FnOnce(&[u8]) -> MemoryResult<(Key, Value)>,
        Key: ByteSized,
        Value: ByteSized,
    {
        let _mutation = self.lifecycle.begin_mutation()?;
        if !self.lock_state()?.pending_restores.is_empty() {
            return Err(MemoryError::OperationInProgress(
                "durable restore reconciliation is required before another restore".into(),
            ));
        }
        let Some(candidate) = controller.prepare_restore()? else {
            return Ok(DurableRestoreOutcome::Empty);
        };
        self.restore_durable_candidate(controller, candidate, DurableRestorePhase::Prepared, decode)
    }

    /// Resume the single persisted restore claim after process restart. This is
    /// deliberately separate from ordinary FIFO restore: callers must invoke it
    /// only after the previous volatile owner is gone. The redb file lock and
    /// exclusive persisted claim prevent another ordinary restore from passing.
    pub fn recover_active_durable_restore<C, Decode>(
        &self,
        controller: &C,
        decode: Decode,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController,
        Decode: FnOnce(&[u8]) -> MemoryResult<(Key, Value)>,
        Key: ByteSized,
        Value: ByteSized,
    {
        let result = self.recover_active_durable_restore_inner(controller, decode);
        self.accounting.emit_result(MemoryOperation::RestoreDurable, None, &result);
        result
    }

    fn recover_active_durable_restore_inner<C, Decode>(
        &self,
        controller: &C,
        decode: Decode,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController,
        Decode: FnOnce(&[u8]) -> MemoryResult<(Key, Value)>,
        Key: ByteSized,
        Value: ByteSized,
    {
        let _mutation = self.lifecycle.begin_mutation()?;
        if !self.lock_state()?.pending_restores.is_empty() {
            return Err(MemoryError::OperationInProgress(
                "the live store already owns a fenced durable restore; reconcile it instead".into(),
            ));
        }
        let Some(recovery) = controller.recover_active_restore()? else {
            return Ok(DurableRestoreOutcome::Empty);
        };
        self.restore_durable_candidate(controller, recovery.candidate, recovery.phase, decode)
    }

    fn restore_durable_candidate<C, Decode>(
        &self,
        controller: &C,
        candidate: crate::spill::DurableRestoreCandidate,
        phase: DurableRestorePhase,
        decode: Decode,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController,
        Decode: FnOnce(&[u8]) -> MemoryResult<(Key, Value)>,
        Key: ByteSized,
        Value: ByteSized,
    {
        let decoded =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(&candidate.bytes)));
        let (key, value) = match decoded {
            Ok(Ok(decoded)) => decoded,
            Ok(Err(error)) => {
                return self.finish_failed_restore_prepare(controller, &candidate, phase, error)
            }
            Err(panic) => {
                if phase == DurableRestorePhase::Prepared {
                    let _ = controller.abort_restore(&candidate.claim);
                }
                std::panic::resume_unwind(panic)
            }
        };
        let charged_bytes = match revocable_entry_cost(&key, &value) {
            Ok(charged_bytes) => charged_bytes,
            Err(error) => {
                retire_client_value(key);
                retire_client_value(value);
                return self.finish_failed_restore_prepare(controller, &candidate, phase, error);
            }
        };
        let next_committed = match self.lock_state()?.committed_bytes.checked_add(charged_bytes) {
            Some(next_committed) => next_committed,
            None => {
                retire_client_value(key);
                retire_client_value(value);
                let error = MemoryError::capacity("revocable restore byte accounting overflow");
                return self.finish_failed_restore_prepare(controller, &candidate, phase, error);
            }
        };
        let charge = match self.accounting.charge(
            self.binding.class.clone(),
            self.binding.owner.clone(),
            MemoryContextKind::Runtime,
            ChargeKind::LiveScratch,
            charged_bytes,
        ) {
            Ok(charge) => charge,
            Err(error) => {
                retire_client_value(key);
                retire_client_value(value);
                return self.finish_failed_restore_prepare(controller, &candidate, phase, error);
            }
        };

        if phase == DurableRestorePhase::Prepared {
            match controller.commit_restore(&candidate.claim) {
                Ok(()) => {}
                Err(DurableCommitFailure::Retained(error)) => {
                    drop(charge);
                    retire_client_value(key);
                    retire_client_value(value);
                    return match controller.abort_restore(&candidate.claim) {
                        Ok(()) => Err(*error),
                        Err(abort_error) => Err(MemoryError::OperationInProgress(format!(
                            "durable delete retained record after error ({error}) but restore {} claim abort failed ({abort_error})",
                            candidate.sequence
                        ))),
                    };
                }
                Err(DurableCommitFailure::Ambiguous(commit_error)) => {
                    self.fence_durable_restore(
                        DurableRecordId::new(candidate.sequence),
                        RevocableEntry {
                            key,
                            value,
                            charged_bytes,
                            charge,
                            restore_claim: Some(candidate.claim),
                        },
                        next_committed,
                    );
                    return Ok(DurableRestoreOutcome::RecoveryRequired {
                        sequence: candidate.sequence,
                        commit_error,
                    });
                }
            }
        }

        // Publish into the store state before deleting the durable recovery
        // payload. The mutation permit keeps readers out until finalize returns,
        // so there is no visible double owner and no delete-before-publication
        // crash window.
        {
            let mut state = self.lock_state_for_commit();
            // The published copy carries its claim so a later fence preserves
            // the exact claimant for reconciliation.
            state.entries.push_back(RevocableEntry {
                key,
                value,
                charged_bytes,
                charge,
                restore_claim: Some(candidate.claim),
            });
            state.committed_bytes = next_committed;
        }
        let finalized = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            controller.finalize_restore(&candidate.claim)
        }));
        match finalized {
            Ok(Ok(())) => Ok(DurableRestoreOutcome::Restored { sequence: candidate.sequence }),
            Ok(Err(finalize_error)) => {
                self.fence_published_restore(candidate.sequence);
                Ok(DurableRestoreOutcome::RecoveryRequired {
                    sequence: candidate.sequence,
                    commit_error: Box::new(finalize_error),
                })
            }
            Err(panic) => {
                self.fence_published_restore(candidate.sequence);
                std::panic::resume_unwind(panic)
            }
        }
    }

    fn finish_failed_restore_prepare<C>(
        &self,
        controller: &C,
        candidate: &crate::spill::DurableRestoreCandidate,
        phase: DurableRestorePhase,
        error: MemoryError,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController,
    {
        if phase == DurableRestorePhase::Committed {
            return Err(MemoryError::OperationInProgress(format!(
                "durable restore {} is committed to recovery but resident preparation failed ({error}); retry recovery",
                candidate.sequence
            )));
        }
        match controller.abort_restore(&candidate.claim) {
            Ok(()) => Err(error),
            Err(abort_error) => Err(MemoryError::OperationInProgress(format!(
                "resident preparation failed ({error}) and durable restore {} claim abort failed ({abort_error})",
                candidate.sequence
            ))),
        }
    }

    fn fence_durable_restore(
        &self,
        record_id: DurableRecordId,
        entry: RevocableEntry<Key, Value>,
        next_committed: u64,
    ) {
        let mut state = self.lock_state_for_commit();
        let replaced = state.pending_restores.insert(record_id, entry);
        debug_assert!(replaced.is_none(), "mutation permit prevents duplicate restore fencing");
        state.committed_bytes = next_committed;
    }

    fn fence_published_restore(&self, sequence: u64) {
        let mut state = self.lock_state_for_commit();
        let entry =
            state.entries.pop_back().expect("restore entry was published before finalization");
        let replaced = state.pending_restores.insert(DurableRecordId::new(sequence), entry);
        debug_assert!(replaced.is_none(), "mutation permit prevents duplicate restore fencing");
    }

    /// IDs whose provisional resident copies are fenced after an unresolved
    /// durable delete. Fenced copies remain charged but are not readable.
    pub fn pending_durable_restores(&self) -> MemoryResult<Vec<DurableRecordId>> {
        Ok(self.lock_read()?.pending_restores.keys().copied().collect())
    }

    /// Resolve one fenced durable restore. If the durable record is absent, the
    /// charged resident copy becomes authoritative. If it is still present, the
    /// provisional resident copy and its charge are retired exactly once. If a
    /// different claimant consumed the record, the provisional copy is retired
    /// without ever becoming readable.
    pub fn reconcile_durable_restore<C>(
        &self,
        controller: &C,
        record_id: DurableRecordId,
    ) -> MemoryResult<DurableRecordResolution>
    where
        C: DurableRestoreController,
    {
        let _mutation = self.lifecycle.begin_mutation()?;
        let claim = {
            let state = self.lock_state()?;
            let entry = state.pending_restores.get(&record_id).ok_or_else(|| {
                MemoryError::InvalidState(format!(
                    "durable record {} has no fenced restore authority",
                    record_id.get()
                ))
            })?;
            entry.restore_claim.ok_or_else(|| {
                MemoryError::InvalidState(format!(
                    "durable record {} has a fenced restore without its claimant",
                    record_id.get()
                ))
            })?
        };
        let resolution = controller.reconcile_restore(&claim)?;
        let mut state = self.lock_state_for_commit();
        match resolution {
            DurableRecordResolution::Present { .. } | DurableRecordResolution::Superseded => {
                let charged_bytes = state
                    .pending_restores
                    .get(&record_id)
                    .expect("pending restore was checked under the mutation permit")
                    .charged_bytes;
                let next_committed =
                    state.committed_bytes.checked_sub(charged_bytes).ok_or_else(|| {
                        MemoryError::InvalidState(
                            "revocable committed bytes are smaller than fenced restore".into(),
                        )
                    })?;
                let entry = state
                    .pending_restores
                    .remove(&record_id)
                    .expect("pending restore was checked under the mutation permit");
                state.committed_bytes = next_committed;
                drop(state);
                retire_entry(entry);
            }
            DurableRecordResolution::Finalized => {
                let entry = state
                    .pending_restores
                    .remove(&record_id)
                    .expect("pending restore was checked under the mutation permit");
                state.entries.push_back(entry);
                drop(state);
                let finalized = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    controller.finalize_restore(&claim)
                }));
                match finalized {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        self.fence_published_restore(record_id.get());
                        return Err(error);
                    }
                    Err(panic) => {
                        self.fence_published_restore(record_id.get());
                        std::panic::resume_unwind(panic);
                    }
                }
            }
            DurableRecordResolution::Absent => {
                return Err(MemoryError::InvalidState(format!(
                    "durable restore {} resolved absent without a finalized claim",
                    record_id.get()
                )))
            }
        }
        Ok(resolution)
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
        state.entries.push_back(handle.into_entry());
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
        self.binding.as_ref()
    }

    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let charged_bytes = revocable_entry_cost(&key, &value)?;
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
            key,
            value,
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
    let mut plan = indices
        .iter()
        .rev()
        .map(|index| {
            (*index, entries.remove(*index).expect("selected revocable index must remain present"))
        })
        .collect::<Vec<_>>();
    plan.reverse();
    plan
}

fn restore_entries<Key, Value>(
    entries: &mut VecDeque<RevocableEntry<Key, Value>>,
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
        DurableRestoreCandidate, DurableRestoreTarget, DurableSpillTarget, InMemorySpillTarget,
        InProcessSpillTarget, NoopSpillController, RoutedSpillController, SpillController,
        SpillInventory, SpillReceipt, SpillTarget,
    };
    #[cfg(feature = "spill")]
    use quanta_memory_platform_core::MemoryOutcome;
    use quanta_memory_platform_core::{
        ExecutionBudget, MemoryBudgetManifest, MemoryClass, MemoryClassBudget, MemoryEvent,
        MemoryEventSink, MemoryPreset, ProcessMemoryBrokerBuilder, ReclaimPriority,
        ResidencyPolicy,
    };
    #[cfg(feature = "spill")]
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;
    use std::thread;

    type Store = SimpleRevocableStore<String, Vec<u8>, Budgeted>;

    #[derive(Default)]
    struct RecordingEvents(Mutex<Vec<MemoryEvent>>);

    impl MemoryEventSink for RecordingEvents {
        fn emit(&self, event: MemoryEvent) {
            self.0.lock().expect("event recorder").push(event);
        }
    }

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
        live_broker_with_events(class_hard, Arc::new(RecordingEvents::default()))
    }

    fn live_broker_with_events(
        class_hard: u64,
        events: Arc<dyn MemoryEventSink>,
    ) -> LiveProcessMemoryBroker {
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
            .with_event_sink(events)
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
    fn many_spills_share_large_binding_identity_without_unbilled_clones() {
        const ENTRY_COUNT: usize = 128;
        let broker = live_broker(10_000_000);
        let large_owner = "owner-".to_string() + &"o".repeat(64 * 1024);
        let large_runtime = "runtime-".to_string() + &"r".repeat(64 * 1024);
        let store = SimpleRevocableStore::new_budgeted(
            StoreBindingHandle::new(
                MemoryClass::ExecutionScratchRevocable,
                large_owner,
                Some(large_runtime),
            ),
            broker.clone(),
        );
        let controller = RoutedSpillController::new(InMemorySpillTarget::available());

        let mut expected_total = 0u64;
        for index in 0..ENTRY_COUNT {
            let key = format!("key-{index}");
            expected_total += expected_entry_cost(&key, 1);
            store.insert_checked(key, vec![index as u8]).expect("insert");
        }
        assert_eq!(
            broker.ledger_snapshot().expect("initial ledger").total_owner_bytes(),
            expected_total
        );

        for spilled in 1..=ENTRY_COUNT {
            assert!(store.spill_one_into(&controller).expect("spill"));
            assert_eq!(controller.spilled_count(), Ok(spilled as u64));
            assert_eq!(
                broker.ledger_snapshot().expect("spill ledger").total_owner_bytes(),
                expected_total,
                "in-process transfer must preserve the exact semantic charge"
            );
        }
        assert_eq!(store.committed_bytes(), Ok(0));

        let mut handles = Vec::with_capacity(ENTRY_COUNT);
        while let Some(handle) = controller.restore().expect("restore from tier") {
            assert!(
                Arc::ptr_eq(&handle.binding, &store.binding),
                "every spill handle must share the store identity allocation"
            );
            handles.push(handle);
        }
        assert_eq!(handles.len(), ENTRY_COUNT);
        assert_eq!(Arc::strong_count(&store.binding), ENTRY_COUNT + 1);
        assert_eq!(
            broker.ledger_snapshot().expect("restored handles ledger").total_owner_bytes(),
            expected_total
        );

        for handle in handles {
            store.accept_restored(handle).expect("reattach shared binding handle");
            assert_eq!(
                broker.ledger_snapshot().expect("reattach ledger").total_owner_bytes(),
                expected_total
            );
        }
        assert_eq!(Arc::strong_count(&store.binding), 1);
        drop(controller);
        assert_eq!(
            broker.ledger_snapshot().expect("empty tier drop ledger").total_owner_bytes(),
            expected_total
        );
        store.close().expect("close restored source");
        assert_eq!(broker.ledger_snapshot().expect("closed ledger").total_owner_bytes(), 0);
    }

    #[cfg(feature = "spill")]
    #[test]
    fn spill_and_close_emit_bounded_facade_events() {
        let events = Arc::new(RecordingEvents::default());
        let broker = live_broker_with_events(1_000_000, events.clone());
        let store = store_with_broker(&broker);
        store.insert_checked("first".into(), vec![1; 4]).expect("insert");
        let controller = RoutedSpillController::new(InMemorySpillTarget::available());

        assert!(store.spill_one_into(&controller).expect("spill"));
        store.close().expect("close empty source");

        let recorded = events.0.lock().expect("events").clone();
        assert!(recorded.contains(&MemoryEvent::new(
            MemoryOperation::SpillInProcess,
            MemoryOutcome::Succeeded,
            None,
        )));
        assert!(recorded.contains(&MemoryEvent::new(
            MemoryOperation::Close,
            MemoryOutcome::Succeeded,
            None,
        )));
    }

    #[cfg(feature = "spill")]
    #[test]
    fn failed_destination_write_preserves_source_entry_and_charge() {
        struct RejectingTarget;
        impl<Handle> crate::spill::service::sealed::InProcessSpillTarget<Handle> for RejectingTarget {}
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

    #[cfg(feature = "spill")]
    #[test]
    fn durable_spill_releases_source_only_after_commit() {
        struct ObservingDurableTarget {
            broker: LiveProcessMemoryBroker,
            expected_live: u64,
            retained: Mutex<Vec<Vec<u8>>>,
        }
        impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for ObservingDurableTarget {}
        impl SpillInventory for ObservingDurableTarget {
            fn retained_count(&self) -> MemoryResult<u64> {
                Ok(self.retained.lock().expect("retained").len() as u64)
            }
        }
        impl SpillTarget<Vec<u8>> for ObservingDurableTarget {
            fn is_available(&self) -> bool {
                true
            }
            fn accept(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
                assert_eq!(
                    self.broker.ledger_snapshot().expect("pre-commit ledger").total_owner_bytes(),
                    self.expected_live,
                    "source charge must remain live until durable accept commits"
                );
                let mut retained = self.retained.lock().expect("retained");
                retained.push(handle);
                Ok(SpillReceipt::Durable { sequence: 7, inventory_after: retained.len() as u64 })
            }
        }
        impl DurableSpillTarget<Vec<u8>> for ObservingDurableTarget {}
        impl crate::spill::service::ReconciledDurableSpillTarget for ObservingDurableTarget {
            fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
                Ok(DurableRecordId::new(7))
            }

            fn commit_spill(
                &self,
                record_id: DurableRecordId,
                bytes: &[u8],
            ) -> Result<SpillReceipt, DurableWriteFailure> {
                assert_eq!(record_id.get(), 7);
                assert_eq!(
                    self.broker.ledger_snapshot().expect("pre-commit ledger").total_owner_bytes(),
                    self.expected_live,
                    "source charge must remain live until durable accept commits"
                );
                let mut retained = self.retained.lock().expect("retained");
                retained.push(bytes.to_vec());
                Ok(SpillReceipt::Durable {
                    sequence: record_id.get(),
                    inventory_after: retained.len() as u64,
                })
            }

            fn resolve_record(
                &self,
                _record_id: DurableRecordId,
            ) -> MemoryResult<DurableRecordResolution> {
                Ok(DurableRecordResolution::Present {
                    inventory_after: self.retained.lock().expect("retained").len() as u64,
                })
            }
        }

        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".into(), vec![1; 4]).expect("insert");
        let total = store.committed_bytes().expect("before");
        let controller = RoutedSpillController::new(ObservingDurableTarget {
            broker: broker.clone(),
            expected_live: total,
            retained: Mutex::new(Vec::new()),
        });

        let receipt = store
            .spill_one_durable(&controller, |key, value| {
                let mut bytes = key.as_bytes().to_vec();
                bytes.extend(value);
                Ok(bytes)
            })
            .expect("spill")
            .expect("entry");
        assert_eq!(receipt, SpillReceipt::Durable { sequence: 7, inventory_after: 1 });
        assert_eq!(store.stats().expect("source empty"), StoreStats::default());
        assert_eq!(broker.ledger_snapshot().expect("released after commit").total_owner_bytes(), 0);
        assert_eq!(controller.spilled_count(), Ok(1));
    }

    #[cfg(feature = "spill")]
    #[test]
    fn failed_durable_write_and_encoder_error_restore_source_fifo_and_charge() {
        struct RejectingDurableTarget;
        impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for RejectingDurableTarget {}
        impl SpillInventory for RejectingDurableTarget {
            fn retained_count(&self) -> MemoryResult<u64> {
                Ok(0)
            }
        }
        impl SpillTarget<Vec<u8>> for RejectingDurableTarget {
            fn is_available(&self) -> bool {
                true
            }
            fn accept(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
                Err(SpillFailure::new(MemoryError::capacity("injected durable failure"), handle))
            }
        }
        impl DurableSpillTarget<Vec<u8>> for RejectingDurableTarget {}
        impl crate::spill::service::ReconciledDurableSpillTarget for RejectingDurableTarget {
            fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
                Ok(DurableRecordId::new(1))
            }

            fn commit_spill(
                &self,
                _record_id: DurableRecordId,
                _bytes: &[u8],
            ) -> Result<SpillReceipt, DurableWriteFailure> {
                Err(DurableWriteFailure::NotCommitted(Box::new(MemoryError::capacity(
                    "injected durable failure",
                ))))
            }

            fn resolve_record(
                &self,
                _record_id: DurableRecordId,
            ) -> MemoryResult<DurableRecordResolution> {
                Ok(DurableRecordResolution::Absent)
            }
        }

        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".into(), vec![1; 4]).expect("first");
        store.insert_checked("second".into(), vec![2; 6]).expect("second");
        let total = store.committed_bytes().expect("before");
        let controller = RoutedSpillController::new(RejectingDurableTarget);

        assert!(matches!(
            store.spill_one_durable(&controller, |_, _| Ok(vec![9])),
            Err(MemoryError::Capacity(_))
        ));
        assert_eq!(store.committed_bytes().expect("write failure restored"), total);
        assert_eq!(
            broker.ledger_snapshot().expect("write failure ledger").total_owner_bytes(),
            total
        );

        assert!(matches!(
            store.spill_one_durable(&controller, |_, _| Err(MemoryError::report("encode failed"))),
            Err(MemoryError::Report(_))
        ));
        assert_eq!(store.committed_bytes().expect("encoder failure restored"), total);
        assert_eq!(store.reclaim_one().expect("FIFO").expect("first").0, "first");
    }

    #[cfg(feature = "spill")]
    #[test]
    fn panicking_durable_encoder_restores_source_before_unwind() {
        struct DurableSink;
        impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for DurableSink {}
        impl SpillInventory for DurableSink {
            fn retained_count(&self) -> MemoryResult<u64> {
                Ok(0)
            }
        }
        impl SpillTarget<Vec<u8>> for DurableSink {
            fn is_available(&self) -> bool {
                true
            }
            fn accept(&self, _handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
                Ok(SpillReceipt::Durable { sequence: 0, inventory_after: 1 })
            }
        }
        impl DurableSpillTarget<Vec<u8>> for DurableSink {}
        impl crate::spill::service::ReconciledDurableSpillTarget for DurableSink {
            fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
                Ok(DurableRecordId::new(0))
            }

            fn commit_spill(
                &self,
                record_id: DurableRecordId,
                _bytes: &[u8],
            ) -> Result<SpillReceipt, DurableWriteFailure> {
                Ok(SpillReceipt::Durable { sequence: record_id.get(), inventory_after: 1 })
            }

            fn resolve_record(
                &self,
                _record_id: DurableRecordId,
            ) -> MemoryResult<DurableRecordResolution> {
                Ok(DurableRecordResolution::Present { inventory_after: 1 })
            }
        }

        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".into(), vec![1; 4]).expect("insert");
        let total = store.committed_bytes().expect("before");
        let controller = RoutedSpillController::new(DurableSink);

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.spill_one_durable(&controller, |_, _| -> MemoryResult<Vec<u8>> {
                panic!("encoder panic")
            });
        }));
        assert!(panic.is_err());
        assert_eq!(store.committed_bytes().expect("restored"), total);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), total);
        assert_eq!(store.reclaim_one().expect("source").expect("entry").0, "first");
    }

    #[cfg(feature = "spill-redb")]
    #[test]
    fn durable_redb_round_trip_transfers_charge_exactly_once() {
        use crate::spill::backings::redb::RedbSpillBacking;
        static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qmp-revocable-roundtrip-{}-{}.redb",
            std::process::id(),
            DB_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("first".into(), vec![1, 2, 3, 4]).expect("insert");
        let total = store.committed_bytes().expect("before");
        let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb"));

        let receipt = store
            .spill_one_durable(&controller, |key, value| {
                let mut encoded = key.as_bytes().to_vec();
                encoded.push(0);
                encoded.extend(value);
                Ok(encoded)
            })
            .expect("spill")
            .expect("entry");
        assert_eq!(receipt, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });
        assert_eq!(broker.ledger_snapshot().expect("off-process").total_owner_bytes(), 0);
        assert_eq!(controller.spilled_count(), Ok(1));

        let restore = store
            .restore_one_durable(&controller, |bytes| {
                let separator = bytes
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or_else(|| MemoryError::report("missing durable record separator"))?;
                let key = String::from_utf8(bytes[..separator].to_vec())
                    .map_err(|error| MemoryError::report(format!("invalid key: {error}")))?;
                Ok((key, bytes[separator + 1..].to_vec()))
            })
            .expect("restore");
        assert!(matches!(restore, DurableRestoreOutcome::Restored { sequence: 0 }));
        assert_eq!(controller.spilled_count(), Ok(0));
        assert_eq!(store.committed_bytes().expect("restored bytes"), total);
        assert_eq!(broker.ledger_snapshot().expect("restored charge").total_owner_bytes(), total);
        assert_eq!(store.reclaim_one().expect("entry").expect("restored").0, "first");
        drop(controller);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "spill-redb")]
    #[test]
    fn durable_redb_committed_restore_recovers_after_process_restart() {
        use crate::spill::backings::redb::RedbSpillBacking;
        static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qmp-revocable-crash-recovery-{}-{}.redb",
            std::process::id(),
            DB_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        {
            let controller =
                RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb 1"));
            let record_id = controller.reserve_record_id().expect("reserve");
            let _receipt =
                controller.commit_spill(record_id, b"recovered\0payload").expect("seed durable");
            let candidate = controller.prepare_restore().expect("prepare").expect("candidate");
            controller
                .commit_restore(&candidate.claim)
                .expect("persist recovery payload before simulated crash");
            assert_eq!(controller.active_restore(), Ok(Some(record_id)));
            // No finalize: dropping this controller simulates a process that
            // dies after durable delete but before resident acknowledgement.
        }

        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb 2"));
        let outcome = store
            .recover_active_durable_restore(&controller, |bytes| {
                let separator = bytes
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or_else(|| MemoryError::report("missing separator"))?;
                Ok((
                    String::from_utf8(bytes[..separator].to_vec())
                        .map_err(|error| MemoryError::report(format!("invalid key: {error}")))?,
                    bytes[separator + 1..].to_vec(),
                ))
            })
            .expect("restart recovery");
        assert!(matches!(outcome, DurableRestoreOutcome::Restored { sequence: 0 }));
        assert_eq!(controller.active_restore(), Ok(None));
        assert_eq!(controller.spilled_count(), Ok(0));
        assert_eq!(store.reclaim_one().expect("read recovered").expect("entry").0, "recovered");
        assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);

        drop(controller);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "spill-redb")]
    #[test]
    fn durable_redb_prepared_restore_recovers_after_process_restart() {
        use crate::spill::backings::redb::RedbSpillBacking;
        static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qmp-revocable-prepared-recovery-{}-{}.redb",
            std::process::id(),
            DB_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        {
            let controller =
                RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb 1"));
            let record_id = controller.reserve_record_id().expect("reserve");
            let _receipt =
                controller.commit_spill(record_id, b"prepared\0payload").expect("seed durable");
            let candidate = controller.prepare_restore().expect("prepare").expect("candidate");
            assert_eq!(candidate.sequence, record_id.get());
            // No commit: the durable queue plus persisted claim must survive.
        }

        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb 2"));
        let outcome = store
            .recover_active_durable_restore(&controller, |bytes| {
                let separator = bytes
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or_else(|| MemoryError::report("missing separator"))?;
                Ok((
                    String::from_utf8(bytes[..separator].to_vec())
                        .map_err(|error| MemoryError::report(format!("invalid key: {error}")))?,
                    bytes[separator + 1..].to_vec(),
                ))
            })
            .expect("restart recovery");
        assert!(matches!(outcome, DurableRestoreOutcome::Restored { sequence: 0 }));
        assert_eq!(controller.active_restore(), Ok(None));
        assert_eq!(controller.spilled_count(), Ok(0));
        assert_eq!(store.reclaim_one().expect("read recovered").expect("entry").0, "prepared");
        assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);

        drop(controller);
        let _ = std::fs::remove_file(path);
    }

    // --- FUP-001: fenced source vs. exact disposition ------------------------
    //
    // A double-uncertain durable commit (lost ACK + lost probe) fences the
    // source entry with its charge intact. A LATER record's finalize on the
    // same backend must never retire that fenced source: the reserved-but-
    // never-committed id is a hole (`Absent`), not a consumed record.

    #[cfg(feature = "spill-redb")]
    #[test]
    fn sep23_fup001_uncertain_commit_keeps_source_fenced_and_blocks_wrong_retire() {
        use crate::spill::backings::redb::RedbSpillBacking;
        use crate::spill::service::ReconciledDurableSpillTarget;
        use crate::spill::{
            DurableCommitFailure, DurableRecordId, DurableRecordResolution,
            DurableRestoreCandidate, DurableRestoreClaim, DurableRestoreController,
            DurableRestoreRecovery, DurableRestoreTarget, DurableSpillController,
            DurableWriteFailure, ReconciledDurableSpillController, SpillFailure, SpillTarget,
        };
        use std::sync::atomic::AtomicBool;

        /// Real redb backing with one scripted double-uncertain commit: the
        /// commit acknowledgement AND the follow-up probe both fail, so the
        /// store must fence the source. All resolution runs against the real
        /// persisted state.
        struct UncertainOnce {
            backing: RedbSpillBacking,
            fail_next_commit: AtomicBool,
        }
        impl SpillController<Vec<u8>> for UncertainOnce {
            fn spill(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
                self.backing.accept(handle)
            }
            fn restore(&self) -> MemoryResult<Option<Vec<u8>>> {
                self.backing.restore()
            }
        }
        impl DurableSpillController<Vec<u8>> for UncertainOnce {}
        impl crate::spill::service::sealed::DurableSpillController<Vec<u8>> for UncertainOnce {}
        impl ReconciledDurableSpillController for UncertainOnce {
            fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
                self.backing.reserve_record_id()
            }
            fn commit_spill(
                &self,
                record_id: DurableRecordId,
                bytes: &[u8],
            ) -> Result<SpillReceipt, DurableWriteFailure> {
                if self.fail_next_commit.swap(false, Ordering::SeqCst) {
                    let _ = bytes;
                    return Err(DurableWriteFailure::RecoveryRequired {
                        record_id,
                        commit_error: Box::new(MemoryError::report(
                            "sep23 fup001 scripted lost commit ack",
                        )),
                        probe_error: Box::new(MemoryError::report(
                            "sep23 fup001 scripted lost probe",
                        )),
                    });
                }
                self.backing.commit_spill(record_id, bytes)
            }
            fn resolve_record(
                &self,
                record_id: DurableRecordId,
            ) -> MemoryResult<DurableRecordResolution> {
                self.backing.resolve_record(record_id)
            }
        }
        impl DurableRestoreController for UncertainOnce {
            fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
                self.backing.prepare_restore()
            }
            fn commit_restore(
                &self,
                claim: &DurableRestoreClaim,
            ) -> Result<(), DurableCommitFailure> {
                self.backing.commit_restore(claim)
            }
            fn finalize_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
                self.backing.finalize_restore(claim)
            }
            fn abort_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
                self.backing.abort_restore(claim)
            }
            fn reconcile_restore(
                &self,
                claim: &DurableRestoreClaim,
            ) -> MemoryResult<DurableRecordResolution> {
                self.backing.reconcile_restore(claim)
            }
            fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
                self.backing.active_restore()
            }
            fn recover_active_restore(&self) -> MemoryResult<Option<DurableRestoreRecovery>> {
                self.backing.recover_active_restore()
            }
        }

        fn encode(key: &String, value: &Vec<u8>) -> MemoryResult<Vec<u8>> {
            let mut encoded = key.as_bytes().to_vec();
            encoded.push(0);
            encoded.extend(value);
            Ok(encoded)
        }
        fn decode(bytes: &[u8]) -> MemoryResult<(String, Vec<u8>)> {
            let separator = bytes
                .iter()
                .position(|byte| *byte == 0)
                .ok_or_else(|| MemoryError::report("missing durable record separator"))?;
            let key = String::from_utf8(bytes[..separator].to_vec())
                .map_err(|error| MemoryError::report(format!("invalid key: {error}")))?;
            Ok((key, bytes[separator + 1..].to_vec()))
        }

        static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qmp-fup001-fenced-{}-{}.redb",
            std::process::id(),
            DB_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let controller = UncertainOnce {
            backing: RedbSpillBacking::create(&path).expect("redb"),
            fail_next_commit: AtomicBool::new(true),
        };
        let broker = live_broker(1_000_000);
        let store_a = store_with_broker(&broker);
        let store_b = store_with_broker(&broker);

        store_a.insert_checked("fenced".into(), vec![7; 8]).expect("insert a");
        let total_a = store_a.committed_bytes().expect("a bytes");
        // The first commit goes double-uncertain: the source stays fenced AND
        // charged — nothing is retired on an unknown outcome.
        let spilled = store_a.spill_one_durable(&controller, encode);
        assert!(
            matches!(spilled, Err(MemoryError::OperationInProgress(_))),
            "double-uncertain commit must fence, got {spilled:?}"
        );
        assert_eq!(
            store_a.pending_durable_spills().expect("pending"),
            vec![DurableRecordId::new(0)]
        );
        assert_eq!(store_a.committed_bytes().expect("still charged"), total_a);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), total_a);

        // A second store commits AND consumes a LATER id on the same backend.
        store_b.insert_checked("later".into(), vec![9; 8]).expect("insert b");
        let total_b = store_b.committed_bytes().expect("b bytes");
        let receipt =
            store_b.spill_one_durable(&controller, encode).expect("spill b").expect("receipt b");
        assert_eq!(receipt, SpillReceipt::Durable { sequence: 1, inventory_after: 1 });
        let outcome = store_b.restore_one_durable(&controller, decode).expect("restore b");
        assert!(matches!(outcome, DurableRestoreOutcome::Restored { sequence: 1 }));

        // Reconciling the fenced id-0 source must observe the HOLE (Absent):
        // the later finalize must not retire a source that never became
        // durable. The watermark oracle misread this as `Finalized`.
        let resolution = store_a
            .reconcile_durable_spill(&controller, DurableRecordId::new(0))
            .expect("reconcile");
        assert_eq!(resolution, DurableRecordResolution::Absent);
        assert!(store_a.pending_durable_spills().expect("drained").is_empty());
        assert_eq!(store_a.committed_bytes().expect("charge intact"), total_a);
        assert_eq!(
            broker.ledger_snapshot().expect("ledger intact").total_owner_bytes(),
            total_a + total_b
        );
        assert_eq!(store_a.reclaim_one().expect("read a").expect("entry a").0, "fenced");
        assert_eq!(store_b.reclaim_one().expect("read b").expect("entry b").0, "later");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);

        drop(controller);
        let _ = std::fs::remove_file(path);
    }

    // --- FUP-002: single-owner recovery ----------------------------------
    //
    // Concurrent recoverers of one persisted restore each mint a distinct
    // claim generation; exactly one publishes, and every loser retires its
    // provisional copy without ever exposing it.

    #[cfg(feature = "spill-redb")]
    fn sep23_fup002_decode(bytes: &[u8]) -> MemoryResult<(String, Vec<u8>)> {
        let separator = bytes
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| MemoryError::report("missing durable record separator"))?;
        let key = String::from_utf8(bytes[..separator].to_vec())
            .map_err(|error| MemoryError::report(format!("invalid key: {error}")))?;
        Ok((key, bytes[separator + 1..].to_vec()))
    }

    /// Seed one record and leave it in the Committed recovery phase with a
    /// persisted active claim, simulating a process that died after the
    /// durable delete but before resident acknowledgement.
    #[cfg(feature = "spill-redb")]
    fn sep23_fup002_seed_committed_recovery(path: &std::path::Path) {
        use crate::spill::backings::redb::RedbSpillBacking;
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(path).expect("redb seed"));
        let record_id = controller.reserve_record_id().expect("reserve");
        let _receipt =
            controller.commit_spill(record_id, b"shared\0payload").expect("seed durable");
        let candidate = controller.prepare_restore().expect("prepare").expect("candidate");
        controller.commit_restore(&candidate.claim).expect("persist recovery before crash");
        assert_eq!(controller.active_restore(), Ok(Some(record_id)));
    }

    #[cfg(feature = "spill-redb")]
    #[test]
    fn sep23_fup002_concurrent_recovery_has_exactly_one_owner() {
        use crate::spill::backings::redb::RedbSpillBacking;
        use std::sync::Arc;
        static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qmp-fup002-race-{}-{}.redb",
            std::process::id(),
            DB_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        sep23_fup002_seed_committed_recovery(&path);

        // The previous volatile owner is gone; two new stores race to recover
        // the same persisted restore on a reopened backend.
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb reopen"));
        let broker = live_broker(1_000_000);
        let store_a = store_with_broker(&broker);
        let store_b = store_with_broker(&broker);
        let start = Arc::new(Barrier::new(3));

        let (outcome_a, outcome_b) = thread::scope(|scope| {
            let controller = &controller;
            let start = &start;
            let worker_a = scope.spawn(|| {
                start.wait();
                store_a.recover_active_durable_restore(controller, sep23_fup002_decode)
            });
            let worker_b = scope.spawn(|| {
                start.wait();
                store_b.recover_active_durable_restore(controller, sep23_fup002_decode)
            });
            start.wait();
            (
                worker_a.join().expect("worker a panicked"),
                worker_b.join().expect("worker b panicked"),
            )
        });
        let outcome_a = outcome_a.expect("worker a result");
        let outcome_b = outcome_b.expect("worker b result");

        // Exactly one claimant publishes, whichever generation won the race.
        let restored = [&outcome_a, &outcome_b]
            .iter()
            .filter(|outcome| matches!(outcome, DurableRestoreOutcome::Restored { sequence: 0 }))
            .count();
        assert_eq!(restored, 1, "exactly one Restored: {outcome_a:?} vs {outcome_b:?}");

        // The loser either fenced a provisional copy (contention path) or saw
        // an empty backend (fully serialized after the winner). In both cases
        // it exposes no readable copy and holds no charge after reconciliation.
        for (store, outcome) in [(&store_a, &outcome_a), (&store_b, &outcome_b)] {
            if matches!(outcome, DurableRestoreOutcome::Restored { .. }) {
                continue;
            }
            assert!(
                store.reclaim_one().expect("loser read").is_none(),
                "loser must expose no readable copy"
            );
            match outcome {
                DurableRestoreOutcome::RecoveryRequired { sequence: 0, .. } => {
                    assert_eq!(
                        store.pending_durable_restores().expect("fenced"),
                        vec![DurableRecordId::new(0)]
                    );
                    assert_eq!(
                        store
                            .reconcile_durable_restore(&controller, DurableRecordId::new(0))
                            .expect("loser reconcile"),
                        DurableRecordResolution::Superseded
                    );
                    assert!(store.pending_durable_restores().expect("drained").is_empty());
                    assert_eq!(store.committed_bytes().expect("loser charge"), 0);
                }
                DurableRestoreOutcome::Empty => {
                    assert!(store.pending_durable_restores().expect("none fenced").is_empty());
                    assert_eq!(store.committed_bytes().expect("loser charge"), 0);
                }
                other => panic!("unexpected loser outcome: {other:?}"),
            }
        }

        // Exactly one resident copy exists, durably consumed, with exact charge.
        assert_eq!(controller.spilled_count(), Ok(0));
        assert_eq!(controller.active_restore(), Ok(None));
        let mut reads = 0;
        for store in [&store_a, &store_b] {
            if let Some((key, value)) = store.reclaim_one().expect("winner read") {
                reads += 1;
                assert_eq!(key, "shared");
                assert_eq!(value, b"payload");
            }
        }
        assert_eq!(reads, 1, "exactly one readable resident copy");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);

        drop(controller);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "spill-redb")]
    #[test]
    fn sep23_fup002_stolen_claim_reconcile_retires_provisional_copy() {
        use crate::spill::backings::redb::RedbSpillBacking;
        use std::sync::mpsc;
        static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qmp-fup002-steal-{}-{}.redb",
            std::process::id(),
            DB_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        sep23_fup002_seed_committed_recovery(&path);

        // Deterministic steal orchestration (no sleeps): A's decode blocks
        // after A minted its claim, B steals and wins while A waits, then A
        // resumes into a stale finalize, fences, and reconciles as superseded.
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb reopen"));
        let broker = live_broker(1_000_000);
        let store_a = store_with_broker(&broker);
        let store_b = store_with_broker(&broker);
        let (claimed_tx, claimed_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let outcome_a = thread::scope(|scope| {
            // Move only shared refs plus the worker-owned channel ends into
            // the spawned closure (`Receiver` is `Send` but not `Sync`, so it
            // cannot be captured by reference).
            let worker = scope.spawn({
                let store_a = &store_a;
                let controller = &controller;
                move || {
                    store_a.recover_active_durable_restore(controller, |bytes| {
                        claimed_tx.send(()).expect("signal claim minted");
                        release_rx.recv().expect("wait for the steal to land");
                        sep23_fup002_decode(bytes)
                    })
                }
            });
            claimed_rx.recv().expect("A minted its claim");
            let outcome_b = store_b
                .recover_active_durable_restore(&controller, sep23_fup002_decode)
                .expect("B steals and wins");
            assert!(
                matches!(outcome_b, DurableRestoreOutcome::Restored { sequence: 0 }),
                "B must publish exactly once, got {outcome_b:?}"
            );
            release_tx.send(()).expect("release A");
            worker.join().expect("worker A panicked").expect("A result")
        });

        // A resumes with a stale generation: publish attempt fences instead of
        // succeeding, and the fenced copy is never readable.
        assert!(
            matches!(outcome_a, DurableRestoreOutcome::RecoveryRequired { sequence: 0, .. }),
            "stale claimant must fence, got {outcome_a:?}"
        );
        assert!(store_a.reclaim_one().expect("A read").is_none(), "fenced copy not readable");
        assert_eq!(
            store_a.pending_durable_restores().expect("A fenced"),
            vec![DurableRecordId::new(0)]
        );
        assert_eq!(
            store_a
                .reconcile_durable_restore(&controller, DurableRecordId::new(0))
                .expect("A reconcile"),
            DurableRecordResolution::Superseded
        );
        assert!(store_a.pending_durable_restores().expect("A drained").is_empty());
        assert_eq!(store_a.committed_bytes().expect("A charge retired"), 0);

        // Winner owns the single copy; the backend is drained; no charge leaks.
        assert_eq!(controller.spilled_count(), Ok(0));
        assert_eq!(store_b.reclaim_one().expect("read B").expect("entry B").0, "shared");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);

        drop(controller);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "spill-redb")]
    #[test]
    fn sep23_fup002_decode_failure_and_denied_admission_preserve_durable() {
        use crate::spill::backings::redb::RedbSpillBacking;
        static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qmp-fup002-preserve-{}-{}.redb",
            std::process::id(),
            DB_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb"));
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("kept".into(), vec![1; 8]).expect("insert");
        let total = store.committed_bytes().expect("bytes");
        let _receipt = store
            .spill_one_durable(&controller, |key, value| {
                let mut encoded = key.as_bytes().to_vec();
                encoded.push(0);
                encoded.extend(value);
                Ok(encoded)
            })
            .expect("spill")
            .expect("receipt");

        // Decode failure aborts the Prepared claim; the record stays durable.
        let failed = store.restore_one_durable(&controller, |_| {
            Err(MemoryError::report("sep23 fup002 injected decode failure"))
        });
        assert!(matches!(failed, Err(MemoryError::Report(_))), "got {failed:?}");
        assert_eq!(controller.active_restore(), Ok(None));
        assert_eq!(controller.spilled_count(), Ok(1));

        // Admission denial does the same; nothing is charged or published.
        let tiny = live_broker(5);
        let denied_store = store_with_broker(&tiny);
        let denied = denied_store.restore_one_durable(&controller, sep23_fup002_decode);
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
        assert_eq!(controller.active_restore(), Ok(None));
        assert_eq!(controller.spilled_count(), Ok(1));
        assert_eq!(tiny.ledger_snapshot().expect("tiny ledger").total_owner_bytes(), 0);

        // A fresh claim still restores the preserved record exactly once.
        let outcome =
            store.restore_one_durable(&controller, sep23_fup002_decode).expect("restore preserved");
        assert!(matches!(outcome, DurableRestoreOutcome::Restored { sequence: 0 }));
        assert_eq!(controller.spilled_count(), Ok(0));
        assert_eq!(store.committed_bytes().expect("restored bytes"), total);
        assert_eq!(store.reclaim_one().expect("read").expect("entry").0, "kept");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);

        drop(controller);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "spill-redb")]
    #[test]
    fn sep23_fup002_commit_ack_loss_same_claimant_publishes_on_retry() {
        use crate::spill::backings::redb::RedbSpillBacking;
        static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qmp-fup002-commitack-{}-{}.redb",
            std::process::id(),
            DB_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let backing = RedbSpillBacking::create(&path).expect("redb");
        // Inject before wrapping: the flag lives in the backing the controller
        // owns, and fires on the restore commit below (the spill is unaffected).
        backing.inject_lost_restore_commit_ack();
        let controller = RoutedSpillController::new(backing);
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("acked".into(), vec![2; 8]).expect("insert");
        let total = store.committed_bytes().expect("bytes");
        let _receipt = store
            .spill_one_durable(&controller, |key, value| {
                let mut encoded = key.as_bytes().to_vec();
                encoded.push(0);
                encoded.extend(value);
                Ok(encoded)
            })
            .expect("spill")
            .expect("receipt");

        // Ambiguous commit fences the provisional copy with its claim intact.
        let outcome = store
            .restore_one_durable(&controller, sep23_fup002_decode)
            .expect("restore with lost commit ack");
        assert!(
            matches!(outcome, DurableRestoreOutcome::RecoveryRequired { sequence: 0, .. }),
            "ambiguous commit must fence, got {outcome:?}"
        );
        assert!(store.reclaim_one().expect("fenced read").is_none());
        assert_eq!(
            store.pending_durable_restores().expect("fenced"),
            vec![DurableRecordId::new(0)]
        );

        // Same-claimant reconcile proves the commit landed and publishes.
        assert_eq!(
            store
                .reconcile_durable_restore(&controller, DurableRecordId::new(0))
                .expect("reconcile"),
            DurableRecordResolution::Finalized
        );
        assert!(store.pending_durable_restores().expect("resolved").is_empty());
        assert_eq!(store.committed_bytes().expect("published bytes"), total);
        assert_eq!(store.reclaim_one().expect("read").expect("entry").0, "acked");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
        assert_eq!(controller.spilled_count(), Ok(0));

        drop(controller);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "spill-redb")]
    #[test]
    fn denied_durable_restore_leaves_redb_record_authoritative() {
        use crate::spill::backings::redb::RedbSpillBacking;
        static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qmp-revocable-denied-{}-{}.redb",
            std::process::id(),
            DB_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb"));
        let record_id = controller.reserve_record_id().expect("reserve durable record");
        let receipt = controller.commit_spill(record_id, b"large\0payload").expect("seed durable");
        assert!(matches!(receipt, SpillReceipt::Durable { .. }));
        let tiny_broker = live_broker(5);
        let store = store_with_broker(&tiny_broker);

        assert!(matches!(
            store.restore_one_durable(&controller, |_| { Ok(("large".to_string(), vec![0; 100])) }),
            Err(MemoryError::Capacity(_))
        ));
        assert_eq!(controller.spilled_count(), Ok(1));
        assert_eq!(store.stats().expect("source unchanged"), StoreStats::default());
        assert_eq!(tiny_broker.ledger_snapshot().expect("ledger unchanged").total_owner_bytes(), 0);
        drop(controller);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "spill")]
    #[test]
    fn failed_durable_delete_rolls_back_provisional_source_charge() {
        struct DeleteFailTarget;
        impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for DeleteFailTarget {}
        impl SpillInventory for DeleteFailTarget {
            fn retained_count(&self) -> MemoryResult<u64> {
                Ok(1)
            }
        }
        impl SpillTarget<Vec<u8>> for DeleteFailTarget {
            fn is_available(&self) -> bool {
                true
            }
            fn accept(&self, _handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
                unreachable!("restore test does not spill")
            }
        }
        impl DurableSpillTarget<Vec<u8>> for DeleteFailTarget {}
        impl DurableRestoreTarget for DeleteFailTarget {
            fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
                Ok(Some(DurableRestoreCandidate::for_test(
                    9,
                    b"first\0payload".to_vec(),
                    DurableRestoreClaim::for_test(0xDE1, 1, 9),
                )))
            }
            fn commit_restore(
                &self,
                claim: &DurableRestoreClaim,
            ) -> Result<(), crate::spill::DurableCommitFailure> {
                assert_eq!(claim.sequence(), 9);
                Err(crate::spill::DurableCommitFailure::retained(MemoryError::report(
                    "injected durable delete failure",
                )))
            }

            fn finalize_restore(&self, _claim: &DurableRestoreClaim) -> MemoryResult<()> {
                Ok(())
            }

            fn abort_restore(&self, _claim: &DurableRestoreClaim) -> MemoryResult<()> {
                Ok(())
            }

            fn reconcile_restore(
                &self,
                _claim: &DurableRestoreClaim,
            ) -> MemoryResult<DurableRecordResolution> {
                Ok(DurableRecordResolution::Present { inventory_after: 1 })
            }

            fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
                Ok(Some(DurableRecordId::new(9)))
            }

            fn recover_active_restore(
                &self,
            ) -> MemoryResult<Option<crate::spill::DurableRestoreRecovery>> {
                Ok(None)
            }
        }

        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        let controller = RoutedSpillController::new(DeleteFailTarget);
        assert!(matches!(
            store.restore_one_durable(&controller, |_| {
                Ok(("first".to_string(), b"payload".to_vec()))
            }),
            Err(MemoryError::Report(_))
        ));
        assert_eq!(controller.spilled_count(), Ok(1));
        assert_eq!(store.stats().expect("not published"), StoreStats::default());
        assert_eq!(broker.ledger_snapshot().expect("charge rolled back").total_owner_bytes(), 0);
    }

    #[cfg(feature = "spill")]
    #[test]
    fn ambiguous_durable_delete_fences_source_until_absence_is_reconciled() {
        struct AmbiguousDeleteTarget {
            present: Arc<AtomicBool>,
        }
        impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for AmbiguousDeleteTarget {}
        impl SpillInventory for AmbiguousDeleteTarget {
            fn retained_count(&self) -> MemoryResult<u64> {
                Ok(u64::from(self.present.load(Ordering::SeqCst)))
            }
        }
        impl SpillTarget<Vec<u8>> for AmbiguousDeleteTarget {
            fn is_available(&self) -> bool {
                true
            }
            fn accept(&self, _handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
                unreachable!("restore test does not spill")
            }
        }
        impl DurableSpillTarget<Vec<u8>> for AmbiguousDeleteTarget {}
        impl crate::spill::service::ReconciledDurableSpillTarget for AmbiguousDeleteTarget {
            fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
                unreachable!("restore test does not spill")
            }

            fn commit_spill(
                &self,
                _record_id: DurableRecordId,
                _bytes: &[u8],
            ) -> Result<SpillReceipt, DurableWriteFailure> {
                unreachable!("restore test does not spill")
            }

            fn resolve_record(
                &self,
                _record_id: DurableRecordId,
            ) -> MemoryResult<DurableRecordResolution> {
                if self.present.load(Ordering::SeqCst) {
                    Ok(DurableRecordResolution::Present { inventory_after: 1 })
                } else {
                    Ok(DurableRecordResolution::Absent)
                }
            }
        }
        impl DurableRestoreTarget for AmbiguousDeleteTarget {
            fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
                Ok(Some(DurableRestoreCandidate::for_test(
                    11,
                    b"record".to_vec(),
                    DurableRestoreClaim::for_test(0xDE1, 1, 11),
                )))
            }
            fn commit_restore(
                &self,
                _claim: &DurableRestoreClaim,
            ) -> Result<(), crate::spill::DurableCommitFailure> {
                Err(crate::spill::DurableCommitFailure::ambiguous(MemoryError::report(
                    "commit acknowledgement lost",
                )))
            }

            fn finalize_restore(&self, _claim: &DurableRestoreClaim) -> MemoryResult<()> {
                Ok(())
            }

            fn abort_restore(&self, _claim: &DurableRestoreClaim) -> MemoryResult<()> {
                Ok(())
            }

            fn reconcile_restore(
                &self,
                _claim: &DurableRestoreClaim,
            ) -> MemoryResult<DurableRecordResolution> {
                if self.present.load(Ordering::SeqCst) {
                    Ok(DurableRecordResolution::Present { inventory_after: 1 })
                } else {
                    Ok(DurableRecordResolution::Finalized)
                }
            }

            fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
                Ok(Some(DurableRecordId::new(11)))
            }

            fn recover_active_restore(
                &self,
            ) -> MemoryResult<Option<crate::spill::DurableRestoreRecovery>> {
                Ok(None)
            }
        }

        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        let present = Arc::new(AtomicBool::new(true));
        let controller =
            RoutedSpillController::new(AmbiguousDeleteTarget { present: Arc::clone(&present) });
        let outcome = store
            .restore_one_durable(&controller, |_| Ok(("restored".to_string(), b"payload".to_vec())))
            .expect("ambiguity is a typed recovery outcome");
        assert!(matches!(outcome, DurableRestoreOutcome::RecoveryRequired { sequence: 11, .. }));
        assert_eq!(
            store.pending_durable_restores().expect("fenced IDs"),
            vec![DurableRecordId::new(11)]
        );
        assert!(store.reclaim_one().expect("fenced source is not readable").is_none());
        assert!(broker.ledger_snapshot().expect("source charged").total_owner_bytes() > 0);
        assert_eq!(controller.spilled_count(), Ok(1), "target state is intentionally uncertain");

        present.store(false, Ordering::SeqCst);
        assert_eq!(
            store
                .reconcile_durable_restore(&controller, DurableRecordId::new(11))
                .expect("absent durable record publishes resident source"),
            DurableRecordResolution::Finalized
        );
        assert!(store.pending_durable_restores().expect("resolved IDs").is_empty());
        assert_eq!(store.reclaim_one().expect("published source").expect("entry").0, "restored");
        assert_eq!(broker.ledger_snapshot().expect("released source").total_owner_bytes(), 0);
    }

    #[cfg(feature = "spill")]
    #[test]
    fn panicking_finalize_fences_published_source_until_reconciliation() {
        struct PanickingFinalizeTarget {
            finalize_calls: AtomicUsize,
        }

        impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for PanickingFinalizeTarget {}
        impl SpillInventory for PanickingFinalizeTarget {
            fn retained_count(&self) -> MemoryResult<u64> {
                Ok(1)
            }
        }
        impl SpillTarget<Vec<u8>> for PanickingFinalizeTarget {
            fn is_available(&self) -> bool {
                true
            }

            fn accept(&self, _handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
                unreachable!("restore test does not spill")
            }
        }
        impl DurableSpillTarget<Vec<u8>> for PanickingFinalizeTarget {}
        impl DurableRestoreTarget for PanickingFinalizeTarget {
            fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
                Ok(Some(DurableRestoreCandidate::for_test(
                    13,
                    b"restored\0payload".to_vec(),
                    DurableRestoreClaim::for_test(0xDE1, 1, 13),
                )))
            }

            fn commit_restore(
                &self,
                claim: &DurableRestoreClaim,
            ) -> Result<(), crate::spill::DurableCommitFailure> {
                assert_eq!(claim.sequence(), 13);
                Ok(())
            }

            fn finalize_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
                assert_eq!(claim.sequence(), 13);
                if self.finalize_calls.fetch_add(1, Ordering::SeqCst) < 2 {
                    panic!("injected finalize panic");
                }
                Ok(())
            }

            fn abort_restore(&self, _claim: &DurableRestoreClaim) -> MemoryResult<()> {
                Ok(())
            }

            fn reconcile_restore(
                &self,
                claim: &DurableRestoreClaim,
            ) -> MemoryResult<DurableRecordResolution> {
                assert_eq!(claim.sequence(), 13);
                Ok(DurableRecordResolution::Finalized)
            }

            fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
                Ok(Some(DurableRecordId::new(13)))
            }

            fn recover_active_restore(
                &self,
            ) -> MemoryResult<Option<crate::spill::DurableRestoreRecovery>> {
                Ok(Some(crate::spill::DurableRestoreRecovery {
                    candidate: DurableRestoreCandidate::for_test(
                        13,
                        b"restored\0payload".to_vec(),
                        DurableRestoreClaim::for_test(0xDE1, 2, 13),
                    ),
                    phase: crate::spill::DurableRestorePhase::Committed,
                }))
            }
        }

        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        let controller = RoutedSpillController::new(PanickingFinalizeTarget {
            finalize_calls: AtomicUsize::new(0),
        });

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store
                .restore_one_durable(&controller, |bytes| {
                    let separator = bytes
                        .iter()
                        .position(|byte| *byte == 0)
                        .ok_or_else(|| MemoryError::report("missing separator"))?;
                    Ok((
                        String::from_utf8(bytes[..separator].to_vec()).map_err(|error| {
                            MemoryError::report(format!("invalid key: {error}"))
                        })?,
                        bytes[separator + 1..].to_vec(),
                    ))
                })
                .expect("finalize panic occurs after resident preparation");
        }));
        assert!(panic.is_err());
        assert_eq!(
            store.pending_durable_restores().expect("fenced restore"),
            vec![DurableRecordId::new(13)]
        );
        assert!(store.reclaim_one().expect("fenced source is not readable").is_none());
        assert!(broker.ledger_snapshot().expect("source remains charged").total_owner_bytes() > 0);

        let reconcile_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store
                .reconcile_durable_restore(&controller, DurableRecordId::new(13))
                .expect("second finalize panic occurs after provisional publication");
        }));
        assert!(reconcile_panic.is_err());
        assert_eq!(
            store.pending_durable_restores().expect("refenced restore"),
            vec![DurableRecordId::new(13)]
        );
        assert!(store.reclaim_one().expect("refenced source is not readable").is_none());

        assert_eq!(
            store
                .reconcile_durable_restore(&controller, DurableRecordId::new(13))
                .expect("retry publishes and acknowledges the resident copy"),
            DurableRecordResolution::Finalized
        );
        assert!(store.pending_durable_restores().expect("resolved IDs").is_empty());
        assert_eq!(store.reclaim_one().expect("published source").expect("entry").0, "restored");
        assert_eq!(broker.ledger_snapshot().expect("released source").total_owner_bytes(), 0);
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

        assert!(matches!(store.stats(), Err(MemoryError::Poisoned(_))));
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
