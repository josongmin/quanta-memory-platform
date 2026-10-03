use super::*;

#[cfg(feature = "spill")]
impl SimpleRevocableStore<String, Vec<u8>, Budgeted> {
    /// Spill using the versioned `String`/`Vec<u8>` codec. Its complete output
    /// length is checked against the backend cap and broker-charged as live
    /// scratch before allocation. The source and output charges coexist until
    /// the durable receipt resolves; denial leaves the source authoritative.
    /// Backend transactional buffers and allocator overhead have separate
    /// limits and are not part of this logical scratch reservation.
    pub fn spill_one_durable_bounded<C>(&self, controller: &C) -> MemoryResult<Option<SpillReceipt>>
    where
        C: ReconciledDurableSpillController,
    {
        let result = self.spill_one_durable_inner(controller, |key, value| {
            let cap = bounded_record_cap(controller)?;
            let len = durable_codec::encoded_len(key, value, cap)?;
            let bytes = u64::try_from(len)
                .map_err(|_| MemoryError::capacity("durable codec output length exceeds u64"))?;
            let charge = self
                .accounting
                .charge(
                    self.binding.class.clone(),
                    self.binding.owner.clone(),
                    MemoryContextKind::Runtime,
                    ChargeKind::LiveScratch,
                    bytes,
                )?
                .ok_or_else(|| {
                    MemoryError::InvalidState("budgeted durable codec has no scratch charge".into())
                })?;
            let encoded = durable_codec::encode(key, value, len)?;
            Ok((encoded, Some(charge)))
        });
        self.accounting.emit_result(MemoryOperation::SpillDurable, None, &result);
        result
    }

    /// Restore with resident-byte admission before the audited codec allocates
    /// the key or value. A malformed record, capacity denial, or allocation
    /// failure preserves the durable source and releases provisional charge.
    pub fn restore_one_durable_bounded<C>(
        &self,
        controller: &C,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController + ReconciledDurableSpillController,
    {
        let result = bounded_record_cap(controller).and_then(|cap| {
            self.restore_one_durable_inner(
                controller,
                |bytes| durable_codec::decode(durable_codec::inspect(bytes)?),
                |bytes| {
                    durable_codec::inspect_bounded(bytes, cap)
                        .map(|layout| Some(layout.charged_bytes))
                },
            )
        });
        self.accounting.emit_result(MemoryOperation::RestoreDurable, None, &result);
        result
    }

    /// Resume a persisted restore claim with the same pre-allocation admission
    /// as `restore_one_durable_bounded`.
    pub fn recover_active_durable_restore_bounded<C>(
        &self,
        controller: &C,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController + ReconciledDurableSpillController,
    {
        let result = bounded_record_cap(controller).and_then(|cap| {
            self.recover_active_durable_restore_inner(
                controller,
                |bytes| durable_codec::decode(durable_codec::inspect(bytes)?),
                |bytes| {
                    durable_codec::inspect_bounded(bytes, cap)
                        .map(|layout| Some(layout.charged_bytes))
                },
            )
        });
        self.accounting.emit_result(MemoryOperation::RestoreDurable, None, &result);
        result
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

    pub(super) fn spill_one_into_inner<C>(&self, controller: &C) -> MemoryResult<bool>
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
    ///
    /// This explicit escape hatch accepts an arbitrary encoder. Its private
    /// allocations are NOT preflighted or broker-charged. Use
    /// `spill_one_durable_bounded` on a `String`/`Vec<u8>` store when the codec
    /// output must be reserved before allocation.
    #[deprecated(
        since = "0.2.0",
        note = "use spill_one_durable_unbounded to acknowledge caller-owned codec allocations, or spill_one_durable_bounded for String/Vec<u8>"
    )]
    pub fn spill_one_durable<C, Encode>(
        &self,
        controller: &C,
        encode: Encode,
    ) -> MemoryResult<Option<SpillReceipt>>
    where
        C: ReconciledDurableSpillController,
        Encode: FnOnce(&Key, &Value) -> MemoryResult<Vec<u8>>,
    {
        self.spill_one_durable_unbounded(controller, encode)
    }

    /// Arbitrary caller codec path. Unlike the bounded typed codec, private
    /// encoder allocations are outside the broker's pre-allocation guarantee.
    pub fn spill_one_durable_unbounded<C, Encode>(
        &self,
        controller: &C,
        encode: Encode,
    ) -> MemoryResult<Option<SpillReceipt>>
    where
        C: ReconciledDurableSpillController,
        Encode: FnOnce(&Key, &Value) -> MemoryResult<Vec<u8>>,
    {
        let result = self.spill_one_durable_inner(controller, |key, value| {
            encode(key, value).map(|bytes| (bytes, None))
        });
        self.accounting.emit_result(MemoryOperation::SpillDurable, None, &result);
        result
    }

    pub(super) fn spill_one_durable_inner<C, Encode>(
        &self,
        controller: &C,
        encode: Encode,
    ) -> MemoryResult<Option<SpillReceipt>>
    where
        C: ReconciledDurableSpillController,
        Encode: FnOnce(&Key, &Value) -> MemoryResult<(Vec<u8>, Option<ManagedCharge>)>,
    {
        let _mutation = self.lifecycle.begin_mutation()?;
        let (bytes, _codec_charge) = {
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
            Err(panic) => {
                // The target may have persisted the record before panicking.
                // Treat this exactly like an unknown commit outcome: retain
                // the charged source, but make it unreadable until an exact
                // record probe selects durable or resident authority.
                let mut state = self.lock_state_for_commit();
                let entry = state.entries.pop_front().expect("front entry was checked");
                let replaced = state.pending_spills.insert(record_id, entry);
                debug_assert!(
                    replaced.is_none(),
                    "mutation permit prevents duplicate spill fencing"
                );
                drop(state);
                std::panic::resume_unwind(panic)
            }
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
    /// This explicit escape hatch can allocate before its result is charged;
    /// use `restore_one_durable_bounded` for the audited `String`/`Vec<u8>` codec.
    #[deprecated(
        since = "0.2.0",
        note = "use restore_one_durable_unbounded to acknowledge caller-owned codec allocations, or restore_one_durable_bounded for String/Vec<u8>"
    )]
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
        self.restore_one_durable_unbounded(controller, decode)
    }

    /// Arbitrary caller decoder path; it does not preflight private codec
    /// allocations against a broker budget.
    pub fn restore_one_durable_unbounded<C, Decode>(
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
        let result = self.restore_one_durable_inner(controller, decode, |_| Ok(None));
        self.accounting.emit_result(MemoryOperation::RestoreDurable, None, &result);
        result
    }

    pub(super) fn restore_one_durable_inner<C, Decode, Preflight>(
        &self,
        controller: &C,
        decode: Decode,
        preflight: Preflight,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController,
        Decode: FnOnce(&[u8]) -> MemoryResult<(Key, Value)>,
        Preflight: FnOnce(&[u8]) -> MemoryResult<Option<u64>>,
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
        self.restore_durable_candidate(
            controller,
            candidate,
            DurableRestorePhase::Prepared,
            decode,
            preflight,
        )
    }

    /// Resume the single persisted restore claim after its volatile owner is
    /// gone: for example, after process restart or after a prepare callback
    /// panicked before returning the claim token. This is deliberately separate
    /// from ordinary FIFO restore. Do not call while another live caller still
    /// owns the token; the redb file lock and persisted claim fence ordinary
    /// restore, but cannot prove that an external caller has relinquished it.
    /// Explicitly unbounded recovery decoder. Its private allocations are
    /// outside the broker's pre-allocation admission contract.
    #[deprecated(
        since = "0.2.0",
        note = "use recover_active_durable_restore_unbounded to acknowledge caller-owned codec allocations, or recover_active_durable_restore_bounded for String/Vec<u8>"
    )]
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
        self.recover_active_durable_restore_unbounded(controller, decode)
    }

    /// Arbitrary caller decoder path for a persisted restore claim.
    pub fn recover_active_durable_restore_unbounded<C, Decode>(
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
        let result = self.recover_active_durable_restore_inner(controller, decode, |_| Ok(None));
        self.accounting.emit_result(MemoryOperation::RestoreDurable, None, &result);
        result
    }

    pub(super) fn recover_active_durable_restore_inner<C, Decode, Preflight>(
        &self,
        controller: &C,
        decode: Decode,
        preflight: Preflight,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController,
        Decode: FnOnce(&[u8]) -> MemoryResult<(Key, Value)>,
        Preflight: FnOnce(&[u8]) -> MemoryResult<Option<u64>>,
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
        self.restore_durable_candidate(
            controller,
            recovery.candidate,
            recovery.phase,
            decode,
            preflight,
        )
    }

    pub(super) fn restore_durable_candidate<C, Decode, Preflight>(
        &self,
        controller: &C,
        candidate: crate::spill::DurableRestoreCandidate,
        phase: DurableRestorePhase,
        decode: Decode,
        preflight: Preflight,
    ) -> MemoryResult<DurableRestoreOutcome>
    where
        C: DurableRestoreController,
        Decode: FnOnce(&[u8]) -> MemoryResult<(Key, Value)>,
        Preflight: FnOnce(&[u8]) -> MemoryResult<Option<u64>>,
        Key: ByteSized,
        Value: ByteSized,
    {
        let planned =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| preflight(candidate.bytes())));
        let precharged = match planned {
            Ok(Ok(Some(charged_bytes))) => {
                let next_committed = match self.lock_state() {
                    Ok(state) => state.committed_bytes.checked_add(charged_bytes),
                    Err(error) => {
                        return self
                            .finish_failed_restore_prepare(controller, &candidate, phase, error)
                    }
                };
                if next_committed.is_none() {
                    return self.finish_failed_restore_prepare(
                        controller,
                        &candidate,
                        phase,
                        MemoryError::capacity("revocable restore byte accounting overflow"),
                    );
                }
                match self.accounting.charge(
                    self.binding.class.clone(),
                    self.binding.owner.clone(),
                    MemoryContextKind::Runtime,
                    ChargeKind::LiveScratch,
                    charged_bytes,
                ) {
                    Ok(charge) => Some((charged_bytes, charge)),
                    Err(error) => {
                        return self
                            .finish_failed_restore_prepare(controller, &candidate, phase, error)
                    }
                }
            }
            Ok(Ok(None)) => None,
            Ok(Err(error)) => {
                return self.finish_failed_restore_prepare(controller, &candidate, phase, error)
            }
            Err(panic) => {
                if phase == DurableRestorePhase::Prepared {
                    self.abort_prepared_restore_on_panic(controller, &candidate, phase);
                }
                std::panic::resume_unwind(panic)
            }
        };
        let decoded =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(candidate.bytes())));
        let (key, value) = match decoded {
            Ok(Ok(decoded)) => decoded,
            Ok(Err(error)) => {
                return self.finish_failed_restore_prepare(controller, &candidate, phase, error)
            }
            Err(panic) => {
                if phase == DurableRestorePhase::Prepared {
                    self.abort_prepared_restore_on_panic(controller, &candidate, phase);
                }
                std::panic::resume_unwind(panic)
            }
        };
        let measured = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            revocable_entry_cost(&key, &value)
        }));
        let charged_bytes = match measured {
            Ok(Ok(charged_bytes)) => charged_bytes,
            Ok(Err(error)) => {
                retire_client_value(key);
                retire_client_value(value);
                return self.finish_failed_restore_prepare(controller, &candidate, phase, error);
            }
            Err(panic) => {
                // Caller measurement is part of resident preparation. No
                // charge or publication exists yet. Retire decoded values
                // without letting a second client destructor panic abort the
                // process, then apply the same claim disposition as a decode
                // panic: abort Prepared, preserve Committed for recovery.
                retire_client_value(key);
                retire_client_value(value);
                if phase == DurableRestorePhase::Prepared {
                    self.abort_prepared_restore_on_panic(controller, &candidate, phase);
                }
                std::panic::resume_unwind(panic)
            }
        };
        if let Some((planned_bytes, _)) = &precharged {
            if *planned_bytes != charged_bytes {
                retire_client_value(key);
                retire_client_value(value);
                return self.finish_failed_restore_prepare(
                    controller,
                    &candidate,
                    phase,
                    MemoryError::InvalidState(format!(
                        "durable codec planned {planned_bytes} resident bytes but decoded {charged_bytes}"
                    )),
                );
            }
        }
        let next_committed = match self.lock_state() {
            Ok(state) => state.committed_bytes.checked_add(charged_bytes),
            Err(error) => {
                retire_client_value(key);
                retire_client_value(value);
                return self.finish_failed_restore_prepare(controller, &candidate, phase, error);
            }
        };
        let next_committed = match next_committed {
            Some(next_committed) => next_committed,
            None => {
                retire_client_value(key);
                retire_client_value(value);
                let error = MemoryError::capacity("revocable restore byte accounting overflow");
                return self.finish_failed_restore_prepare(controller, &candidate, phase, error);
            }
        };
        let charge = if let Some((_, charge)) = precharged {
            charge
        } else {
            match self.accounting.charge(
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
                    return self
                        .finish_failed_restore_prepare(controller, &candidate, phase, error);
                }
            }
        };

        if phase == DurableRestorePhase::Prepared {
            let committed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                controller.commit_restore(&candidate.claim)
            }));
            match committed {
                Ok(Ok(())) => {}
                Ok(Err(DurableCommitFailure::Retained(error))) => {
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
                Ok(Err(DurableCommitFailure::Ambiguous(commit_error))) => {
                    self.fence_durable_restore(
                        DurableRecordId::new(candidate.sequence),
                        RevocableEntry {
                            key,
                            value,
                            charged_bytes,
                            charge,
                            restore_claim: Some(Arc::new(candidate.claim)),
                        },
                        next_committed,
                    );
                    return Ok(DurableRestoreOutcome::RecoveryRequired {
                        sequence: candidate.sequence,
                        commit_error,
                    });
                }
                Err(panic) => {
                    // A target can commit the delete and then panic. Keep the
                    // provisional resident copy charged but unreadable until
                    // the exact claim is reconciled; a pre-commit panic will
                    // instead retire this copy and retain the durable record.
                    self.fence_durable_restore(
                        DurableRecordId::new(candidate.sequence),
                        RevocableEntry {
                            key,
                            value,
                            charged_bytes,
                            charge,
                            restore_claim: Some(Arc::new(candidate.claim)),
                        },
                        next_committed,
                    );
                    std::panic::resume_unwind(panic)
                }
            }
        }

        // Publish into the store state before deleting the durable recovery
        // payload. The mutation permit keeps readers out until finalize returns,
        // so there is no visible double owner and no delete-before-publication
        // crash window.
        {
            let mut state = self.lock_state_for_commit();
            state.entries.push_back(RevocableEntry {
                key,
                value,
                charged_bytes,
                charge,
                restore_claim: None,
            });
            state.committed_bytes = next_committed;
        }
        let finalized = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            controller.finalize_restore(&candidate.claim)
        }));
        match finalized {
            Ok(Ok(())) => Ok(DurableRestoreOutcome::Restored { sequence: candidate.sequence }),
            Ok(Err(finalize_error)) => {
                self.fence_published_restore(candidate.sequence, Arc::new(candidate.claim));
                Ok(DurableRestoreOutcome::RecoveryRequired {
                    sequence: candidate.sequence,
                    commit_error: Box::new(finalize_error),
                })
            }
            Err(panic) => {
                self.fence_published_restore(candidate.sequence, Arc::new(candidate.claim));
                std::panic::resume_unwind(panic)
            }
        }
    }

    pub(super) fn abort_prepared_restore_on_panic<C>(
        &self,
        controller: &C,
        candidate: &crate::spill::DurableRestoreCandidate,
        phase: DurableRestorePhase,
    ) where
        C: DurableRestoreController,
    {
        if phase == DurableRestorePhase::Prepared {
            // The caller's panic remains primary. Abort failure cannot be
            // returned while unwinding; the backing's recovery path owns any
            // claim that remains active.
            #[expect(
                clippy::let_underscore_must_use,
                reason = "panic unwind must preserve the original payload; abort failure cannot replace it"
            )]
            let _ = controller.abort_restore(&candidate.claim);
        }
    }

    pub(super) fn finish_failed_restore_prepare<C>(
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

    pub(super) fn fence_durable_restore(
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

    pub(super) fn fence_published_restore(&self, sequence: u64, claim: Arc<DurableRestoreClaim>) {
        let mut state = self.lock_state_for_commit();
        let mut entry =
            state.entries.pop_back().expect("restore entry was published before finalization");
        entry.restore_claim = Some(claim);
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
            Arc::clone(entry.restore_claim.as_ref().ok_or_else(|| {
                MemoryError::InvalidState(format!(
                    "durable record {} has a fenced restore without its claimant",
                    record_id.get()
                ))
            })?)
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
                        self.fence_published_restore(record_id.get(), Arc::clone(&claim));
                        return Err(error);
                    }
                    Err(panic) => {
                        self.fence_published_restore(record_id.get(), Arc::clone(&claim));
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
