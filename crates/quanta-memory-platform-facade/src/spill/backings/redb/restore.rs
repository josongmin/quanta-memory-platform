use super::*;

impl DurableRestoreTarget for RedbSpillBacking {
    fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
        let txn = self.database.begin_write().map_err(|e| {
            MemoryError::report(format!("redb restore-claim transaction failed: {e}"))
        })?;
        let candidate = {
            let table = txn
                .open_table(SPILL_QUEUE)
                .map_err(|e| MemoryError::report(format!("redb spill table open failed: {e}")))?;
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::report(format!("redb spill metadata table failed: {e}"))
            })?;
            if let Some(active) = meta
                .get(ACTIVE_RESTORE_SEQUENCE)
                .map_err(|e| MemoryError::report(format!("redb active restore read failed: {e}")))?
            {
                return Err(MemoryError::OperationInProgress(format!(
                    "durable restore {} requires reconciliation",
                    active.value()
                )));
            }
            let first = table
                .first()
                .map_err(|e| MemoryError::report(format!("redb spill first failed: {e}")))?;
            let Some((sequence, bytes)) = first else {
                return Ok(None);
            };
            let (read_bytes, lease) = self.copy_restore_bytes(bytes.value())?;
            // Mint the next claim generation in the SAME transaction that
            // publishes the active claimant: exactly one caller owns each
            // generation, and concurrent preparers serialize here.
            let counter = meta
                .get(RESTORE_CLAIM_GENERATION)
                .map_err(|e| MemoryError::report(format!("redb claim counter read failed: {e}")))?
                .map(|value| value.value())
                .unwrap_or(0);
            let generation = counter.checked_add(1).ok_or_else(|| {
                MemoryError::capacity("durable restore claim generation exhausted (u64::MAX)")
            })?;
            meta.insert(RESTORE_CLAIM_GENERATION, generation).map_err(|e| {
                MemoryError::report(format!("redb claim counter write failed: {e}"))
            })?;
            meta.insert(ACTIVE_RESTORE_SEQUENCE, sequence.value()).map_err(|e| {
                MemoryError::report(format!("redb restore claim write failed: {e}"))
            })?;
            meta.insert(ACTIVE_RESTORE_GENERATION, generation).map_err(|e| {
                MemoryError::report(format!("redb restore claim write failed: {e}"))
            })?;
            DurableRestoreCandidate::from_redb(
                sequence.value(),
                read_bytes,
                DurableRestoreClaim::issued(self.backend_id, generation, sequence.value()),
                lease,
            )
        };
        match txn.commit() {
            Ok(()) => Ok(Some(candidate)),
            Err(error) => match self.active_claim()? {
                // The claim landed: only THIS generation proves it.
                Some((sequence, Some(generation)))
                    if sequence == candidate.sequence
                        && generation == candidate.claim.generation() =>
                {
                    Ok(Some(candidate))
                }
                _ => Err(MemoryError::report(format!(
                    "redb restore claim commit outcome is unknown: {error}"
                ))),
            },
        }
    }

    fn commit_restore(&self, claim: &DurableRestoreClaim) -> Result<(), DurableCommitFailure> {
        self.check_claim_backend(claim).map_err(DurableCommitFailure::retained)?;
        let sequence = claim.sequence();
        let generation = claim.generation();
        let txn = self.database.begin_write().map_err(|e| {
            DurableCommitFailure::retained(redb_storage_error("redb spill write txn failed", e))
        })?;
        {
            let mut table = txn.open_table(SPILL_QUEUE).map_err(|e| {
                DurableCommitFailure::retained(MemoryError::report(format!(
                    "redb spill table open failed: {e}"
                )))
            })?;
            let mut recovery = txn.open_table(SPILL_RECOVERY).map_err(|e| {
                DurableCommitFailure::retained(MemoryError::report(format!(
                    "redb spill recovery table open failed: {e}"
                )))
            })?;
            let meta = txn.open_table(SPILL_META).map_err(|e| {
                DurableCommitFailure::retained(MemoryError::report(format!(
                    "redb spill metadata table failed: {e}"
                )))
            })?;
            let mut states = txn.open_table(SPILL_RECORD_STATE).map_err(|e| {
                DurableCommitFailure::retained(MemoryError::report(format!(
                    "redb record disposition table failed: {e}"
                )))
            })?;
            let active = meta
                .get(ACTIVE_RESTORE_SEQUENCE)
                .map_err(|e| {
                    DurableCommitFailure::retained(MemoryError::report(format!(
                        "redb active restore read failed: {e}"
                    )))
                })?
                .map(|value| value.value());
            let active_generation = meta
                .get(ACTIVE_RESTORE_GENERATION)
                .map_err(|e| {
                    DurableCommitFailure::retained(MemoryError::report(format!(
                        "redb active claim read failed: {e}"
                    )))
                })?
                .map(|value| value.value());
            // Only the live claimant may commit: a stale generation (superseded
            // by a newer recovery) or a foreign sequence is rejected before any
            // payload moves, and the record stays authoritative.
            if active != Some(sequence) || active_generation != Some(generation) {
                return Err(DurableCommitFailure::retained(MemoryError::InvalidState(format!(
                    "durable restore candidate {sequence} generation {generation} is not the \
                     active claim (sequence {active:?}, generation {active_generation:?})"
                ))));
            }
            #[cfg(feature = "durability-test-hooks")]
            restore_commit_volume_hook::fire(sequence, generation);
            if recovery
                .get(sequence)
                .map_err(|e| {
                    DurableCommitFailure::retained(MemoryError::report(format!(
                        "redb recovery idempotency read failed: {e}"
                    )))
                })?
                .is_some()
            {
                // Idempotent retry of a landed commit: the disposition must
                // already be `Recovery`, or payload and disposition disagree.
                let disposition = states
                    .get(sequence)
                    .map_err(|e| {
                        DurableCommitFailure::retained(MemoryError::report(format!(
                            "redb record disposition read failed: {e}"
                        )))
                    })?
                    .map(|value| {
                        RecordDisposition::decode(value.value())
                            .map_err(DurableCommitFailure::retained)
                    })
                    .transpose()?;
                if disposition != Some(RecordDisposition::Recovery) {
                    return Err(DurableCommitFailure::retained(MemoryError::InvalidState(
                        format!(
                            "durable restore {sequence} has recovery bytes but disposition {disposition:?}"
                        ),
                    )));
                }
                return Ok(());
            }
            let first_sequence = table
                .first()
                .map_err(|e| {
                    DurableCommitFailure::retained(MemoryError::report(format!(
                        "redb spill first failed: {e}"
                    )))
                })?
                .map(|(key, _)| key.value());
            if first_sequence != Some(sequence) {
                return Err(DurableCommitFailure::retained(MemoryError::InvalidState(
                    format!(
                        "durable restore candidate {sequence} is stale; current first is {first_sequence:?}"
                    ),
                )));
            }
            let removed = table
                .remove(sequence)
                .map_err(|e| {
                    DurableCommitFailure::retained(MemoryError::report(format!(
                        "redb spill exact remove failed: {e}"
                    )))
                })?
                .ok_or_else(|| {
                    DurableCommitFailure::retained(MemoryError::InvalidState(
                        "durable restore candidate vanished before commit".into(),
                    ))
                })?;
            let (removed, _move_lease) =
                self.copy_restore_bytes(removed.value()).map_err(DurableCommitFailure::retained)?;
            recovery.insert(sequence, removed.as_slice()).map_err(|e| {
                DurableCommitFailure::retained(redb_storage_error(
                    "redb recovery payload write failed",
                    e,
                ))
            })?;
            // The phase change and the disposition share this transaction: the
            // record is `Recovery` (still present), never prematurely
            // `Finalized`. The legacy watermark is no longer advanced.
            states.insert(sequence, RecordDisposition::Recovery as u8).map_err(|e| {
                DurableCommitFailure::retained(MemoryError::report(format!(
                    "redb record disposition write failed: {e}"
                )))
            })?;
        }
        match txn.commit() {
            Ok(()) => {
                #[cfg(test)]
                if self.faults.lose_restore_commit_ack.swap(false, AtomicOrdering::SeqCst) {
                    return Err(DurableCommitFailure::ambiguous(MemoryError::report(
                        "injected lost redb restore commit acknowledgement",
                    )));
                }
                Ok(())
            }
            Err(error) => Err(DurableCommitFailure::ambiguous(redb_storage_error(
                "redb spill restore commit outcome is ambiguous",
                error,
            ))),
        }
    }

    fn finalize_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
        self.check_claim_backend(claim)?;
        let sequence = claim.sequence();
        let generation = claim.generation();
        let txn = self.database.begin_write().map_err(|e| {
            MemoryError::report(format!("redb restore-finalize transaction failed: {e}"))
        })?;
        {
            let mut recovery = txn.open_table(SPILL_RECOVERY).map_err(|e| {
                MemoryError::report(format!("redb spill recovery table open failed: {e}"))
            })?;
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::report(format!("redb spill metadata table failed: {e}"))
            })?;
            let mut states = txn.open_table(SPILL_RECORD_STATE).map_err(|e| {
                MemoryError::report(format!("redb record disposition table failed: {e}"))
            })?;
            let mut finalized_by = txn.open_table(SPILL_FINALIZED_BY).map_err(|e| {
                MemoryError::report(format!("redb finalize proof table failed: {e}"))
            })?;
            let active = meta
                .get(ACTIVE_RESTORE_SEQUENCE)
                .map_err(|e| MemoryError::report(format!("redb active restore read failed: {e}")))?
                .map(|value| value.value());
            let active_generation = meta
                .get(ACTIVE_RESTORE_GENERATION)
                .map_err(|e| MemoryError::report(format!("redb active claim read failed: {e}")))?
                .map(|value| value.value());
            let proof = finalized_by
                .get(sequence)
                .map_err(|e| MemoryError::report(format!("redb finalize proof read failed: {e}")))?
                .map(|value| value.value());
            let recovery_len = recovery
                .get(sequence)
                .map_err(|e| MemoryError::report(format!("redb recovery read failed: {e}")))?
                .map(|payload| u64::try_from(payload.value().len()))
                .transpose()
                .map_err(|_| MemoryError::capacity("redb recovery payload length exceeds u64"))?;
            let recovery_present = recovery_len.is_some();
            // Terminal proof first: this generation's finalize already landed
            // (proof and payload removal share one transaction), so the retry
            // succeeds exactly — regardless of which unrelated claim is live
            // now. A landed finalize can never be un-consumed, and the proof
            // row is immutable, so no live state can contradict this.
            if proof == Some(generation) && !recovery_present {
                return Ok(());
            }
            // Single-owner consume: exactly one claimant's finalize can land.
            // A bare sequence with no live claim and no finalize proof is
            // rejected — the old unconditional `(None, false) => Ok(())` let a
            // second claimant "succeed" after the first consumed the record.
            match (active, active_generation, recovery_present) {
                (Some(active), Some(active_gen), true)
                    if active == sequence && active_gen == generation =>
                {
                    let retained_bytes = meta
                        .get(RETAINED_PAYLOAD_BYTES)
                        .map_err(|e| {
                            MemoryError::report(format!("redb payload usage read failed: {e}"))
                        })?
                        .map(|value| value.value())
                        .ok_or_else(|| {
                            MemoryError::InvalidState(
                                "redb retained payload usage authority is missing".into(),
                            )
                        })?;
                    let after_bytes = retained_bytes
                        .checked_sub(recovery_len.expect("matched recovery payload"))
                        .ok_or_else(|| {
                            MemoryError::InvalidState(
                                "redb retained payload usage is below finalized record".into(),
                            )
                        })?;
                    recovery.remove(sequence).map_err(|e| {
                        MemoryError::report(format!("redb recovery payload clear failed: {e}"))
                    })?;
                    meta.insert(RETAINED_PAYLOAD_BYTES, after_bytes).map_err(|e| {
                        MemoryError::report(format!("redb payload usage update failed: {e}"))
                    })?;
                    meta.remove(ACTIVE_RESTORE_SEQUENCE).map_err(|e| {
                        MemoryError::report(format!("redb active restore clear failed: {e}"))
                    })?;
                    meta.remove(ACTIVE_RESTORE_GENERATION).map_err(|e| {
                        MemoryError::report(format!("redb active claim clear failed: {e}"))
                    })?;
                    // Consume completion, disposition, and finalize proof share
                    // this transaction: only now is the record exactly
                    // `Finalized` by exactly this generation.
                    states.insert(sequence, RecordDisposition::Finalized as u8).map_err(|e| {
                        MemoryError::report(format!("redb record disposition write failed: {e}"))
                    })?;
                    finalized_by.insert(sequence, generation).map_err(|e| {
                        MemoryError::report(format!("redb finalize proof write failed: {e}"))
                    })?;
                }
                (None, _, false) => {
                    return Err(MemoryError::InvalidState(format!(
                        "cannot finalize restore {sequence}: no active claim and no finalize \
                         proof for generation {generation}"
                    )))
                }
                _ => {
                    return Err(MemoryError::InvalidState(format!(
                        "cannot finalize restore {sequence} generation {generation}; active \
                         claim is ({active:?}, {active_generation:?}) and \
                         recovery_present={recovery_present}"
                    )))
                }
            }
        }
        #[cfg(feature = "durability-test-hooks")]
        finalize_kill_hooks::fire(
            finalize_kill_hooks::FinalizeKillPoint::BeforeCommit,
            sequence,
            generation,
        );
        let commit_error = match txn.commit() {
            Ok(()) => {
                #[cfg(feature = "durability-test-hooks")]
                finalize_kill_hooks::fire(
                    finalize_kill_hooks::FinalizeKillPoint::AfterCommit,
                    sequence,
                    generation,
                );
                #[cfg(test)]
                if self.faults.lose_finalize_commit_ack.swap(false, AtomicOrdering::SeqCst) {
                    Some("injected lost redb restore-finalize acknowledgement".to_string())
                } else {
                    None
                }
                #[cfg(not(test))]
                None
            }
            Err(error) => Some(error.to_string()),
        };
        match commit_error {
            None => Ok(()),
            Some(error) => {
                #[cfg(test)]
                if self.faults.claim_after_lost_ack.swap(false, AtomicOrdering::SeqCst) {
                    let _next = self.prepare_restore()?;
                }
                if self.finalize_landed(sequence, generation)? {
                    Ok(())
                } else {
                    Err(MemoryError::report(format!(
                        "redb restore-finalize commit outcome remains unresolved: {error}"
                    )))
                }
            }
        }
    }

    fn abort_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
        self.check_claim_backend(claim)?;
        let sequence = claim.sequence();
        let generation = claim.generation();
        let txn = self.database.begin_write().map_err(|e| {
            MemoryError::report(format!("redb restore-abort transaction failed: {e}"))
        })?;
        {
            let recovery = txn.open_table(SPILL_RECOVERY).map_err(|e| {
                MemoryError::report(format!("redb spill recovery table open failed: {e}"))
            })?;
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::report(format!("redb spill metadata table failed: {e}"))
            })?;
            let finalized_by = txn.open_table(SPILL_FINALIZED_BY).map_err(|e| {
                MemoryError::report(format!("redb finalize proof table failed: {e}"))
            })?;
            let active = meta
                .get(ACTIVE_RESTORE_SEQUENCE)
                .map_err(|e| MemoryError::report(format!("redb active restore read failed: {e}")))?
                .map(|value| value.value());
            let active_generation = meta
                .get(ACTIVE_RESTORE_GENERATION)
                .map_err(|e| MemoryError::report(format!("redb active claim read failed: {e}")))?
                .map(|value| value.value());
            match (active, active_generation) {
                (Some(active), Some(active_gen))
                    if active == sequence && active_gen == generation =>
                {
                    if recovery
                        .get(sequence)
                        .map_err(|e| {
                            MemoryError::report(format!("redb recovery read failed: {e}"))
                        })?
                        .is_some()
                    {
                        return Err(MemoryError::InvalidState(format!(
                            "restore {sequence} is already committed to durable recovery and cannot be aborted"
                        )));
                    }
                    meta.remove(ACTIVE_RESTORE_SEQUENCE).map_err(|e| {
                        MemoryError::report(format!("redb active restore clear failed: {e}"))
                    })?;
                    meta.remove(ACTIVE_RESTORE_GENERATION).map_err(|e| {
                        MemoryError::report(format!("redb active claim clear failed: {e}"))
                    })?;
                }
                (None, _) => return Ok(()),
                _ => {
                    // Another claimant owns the live claim. If THIS generation
                    // already finalized the record, the desired end state (this
                    // claim holds nothing) holds exactly — success. Otherwise
                    // this claim never owned the live state: reject without
                    // touching the other claimant's authority.
                    let proof = finalized_by
                        .get(sequence)
                        .map_err(|e| {
                            MemoryError::report(format!("redb finalize proof read failed: {e}"))
                        })?
                        .map(|value| value.value());
                    if proof == Some(generation) {
                        return Ok(());
                    }
                    return Err(MemoryError::InvalidState(format!(
                        "cannot abort restore {sequence} generation {generation}; active claim \
                         is ({active:?}, {active_generation:?})"
                    )));
                }
            }
        }
        let commit_error = match txn.commit() {
            Ok(()) => {
                #[cfg(test)]
                if self.faults.lose_abort_commit_ack.swap(false, AtomicOrdering::SeqCst) {
                    Some("injected lost redb restore-abort acknowledgement".to_string())
                } else {
                    None
                }
                #[cfg(not(test))]
                None
            }
            Err(error) => Some(error.to_string()),
        };
        match commit_error {
            None => Ok(()),
            Some(error) => {
                #[cfg(test)]
                if self.faults.claim_after_lost_ack.swap(false, AtomicOrdering::SeqCst) {
                    let _next = self.prepare_restore()?;
                }
                match self.active_claim()? {
                    None => Ok(()),
                    Some((active, Some(active_gen)))
                        if active == sequence && active_gen == generation =>
                    {
                        Err(MemoryError::report(format!(
                            "redb restore-abort commit outcome remains unresolved: {error}"
                        )))
                    }
                    // The write began with this exact live claim. A foreign claim
                    // after the ambiguous commit means this one no longer holds
                    // the global authority, regardless of whether the next claim
                    // appeared before this read.
                    Some(_) => Ok(()),
                }
            }
        }
    }

    fn reconcile_restore(
        &self,
        claim: &DurableRestoreClaim,
    ) -> MemoryResult<DurableRecordResolution> {
        self.check_claim_backend(claim)?;
        let sequence = claim.sequence();
        let generation = claim.generation();
        let read = self.database.begin_read().map_err(|e| {
            MemoryError::report(format!("redb restore reconciliation read failed: {e}"))
        })?;
        let table = read
            .open_table(SPILL_QUEUE)
            .map_err(|e| MemoryError::report(format!("redb spill table open failed: {e}")))?;
        let recovery = read.open_table(SPILL_RECOVERY).map_err(|e| {
            MemoryError::report(format!("redb spill recovery table open failed: {e}"))
        })?;
        let meta = read
            .open_table(SPILL_META)
            .map_err(|e| MemoryError::report(format!("redb spill metadata table failed: {e}")))?;
        let present = table
            .get(sequence)
            .map_err(|e| MemoryError::report(format!("redb restore probe failed: {e}")))?
            .is_some();
        let recovery_present = recovery
            .get(sequence)
            .map_err(|e| MemoryError::report(format!("redb recovery probe failed: {e}")))?
            .is_some();
        let inventory_after = table
            .len()
            .map_err(|e| MemoryError::report(format!("redb spill inventory read failed: {e}")))?;
        let active = meta
            .get(ACTIVE_RESTORE_SEQUENCE)
            .map_err(|e| MemoryError::report(format!("redb active restore read failed: {e}")))?
            .map(|value| value.value());
        let active_generation = meta
            .get(ACTIVE_RESTORE_GENERATION)
            .map_err(|e| MemoryError::report(format!("redb active claim read failed: {e}")))?
            .map(|value| value.value());
        let next = meta
            .get(NEXT_SEQUENCE)
            .map_err(|e| MemoryError::report(format!("redb spill metadata read failed: {e}")))?
            .map(|value| value.value());
        let states = read.open_table(SPILL_RECORD_STATE).map_err(|e| {
            MemoryError::report(format!("redb record disposition table open failed: {e}"))
        })?;
        let disposition = states
            .get(sequence)
            .map_err(|e| MemoryError::report(format!("redb record disposition read failed: {e}")))?
            .map(|value| RecordDisposition::decode(value.value()))
            .transpose()?;
        let finalized_by = read.open_table(SPILL_FINALIZED_BY).map_err(|e| {
            MemoryError::report(format!("redb finalize proof table open failed: {e}"))
        })?;
        let proof = finalized_by
            .get(sequence)
            .map_err(|e| MemoryError::report(format!("redb finalize proof read failed: {e}")))?
            .map(|value| value.value());
        drop(finalized_by);
        drop(states);
        drop(meta);
        drop(recovery);
        drop(table);
        drop(read);

        let mine_active = active == Some(sequence) && active_generation == Some(generation);
        let other_active = active.is_some() && !mine_active;

        if present {
            if other_active {
                return Err(MemoryError::InvalidState(format!(
                    "restore {sequence} is present while foreign claim ({active:?}, \
                     {active_generation:?}) is active"
                )));
            }
            if disposition != Some(RecordDisposition::Queued) {
                return Err(MemoryError::InvalidState(format!(
                    "durable restore {sequence} has queued bytes but disposition {disposition:?}"
                )));
            }
            if mine_active {
                self.abort_restore(claim)?;
            }
            return Ok(DurableRecordResolution::Present { inventory_after });
        }
        if recovery_present {
            if disposition != Some(RecordDisposition::Recovery) {
                return Err(MemoryError::InvalidState(format!(
                    "durable restore {sequence} has recovery bytes but disposition {disposition:?}"
                )));
            }
            // Recovery bytes exist: a landed finalize removes them in the same
            // transaction that writes the proof, so any proof row here
            // contradicts the bytes. Whoever holds the live claim owns them.
            if mine_active {
                return Ok(DurableRecordResolution::Finalized);
            }
            if proof.is_some() {
                return Err(MemoryError::InvalidState(format!(
                    "durable restore {sequence} has recovery bytes but finalize proof \
                     {proof:?} contradicts an unconsumed record"
                )));
            }
            if active.is_none() {
                return Err(MemoryError::InvalidState(format!(
                    "restore {sequence} has recovery bytes without an active claim"
                )));
            }
            // A newer claimant stole the live claim: my fenced copy must be
            // retired, never published.
            return Ok(DurableRecordResolution::Superseded);
        }
        // No payload anywhere: disposition plus per-record finalize proof are
        // the sole authority — the legacy watermark is never consulted.
        match disposition {
            Some(RecordDisposition::Finalized) => match proof {
                Some(proof) if proof == generation => Ok(DurableRecordResolution::Finalized),
                Some(_) => Ok(DurableRecordResolution::Superseded),
                None => Err(MemoryError::InvalidState(format!(
                    "durable restore {sequence} is finalized but its finalize proof is missing"
                ))),
            },
            Some(RecordDisposition::Reserved) => Err(MemoryError::InvalidState(format!(
                "restore {sequence} is neither retained nor durably finalized"
            ))),
            Some(RecordDisposition::Queued) | Some(RecordDisposition::Recovery) => {
                Err(MemoryError::InvalidState(format!(
                    "durable restore {sequence} is queued for restore but its payload is missing"
                )))
            }
            None if next.is_some_and(|next| sequence >= next) => Err(MemoryError::InvalidState(
                format!("restore {sequence} is neither retained nor durably finalized"),
            )),
            None => Err(unresolved_record_error(sequence)),
        }
    }

    fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
        let read = self.database.begin_read().map_err(|e| {
            MemoryError::report(format!("redb active restore transaction failed: {e}"))
        })?;
        let meta = read
            .open_table(SPILL_META)
            .map_err(|e| MemoryError::report(format!("redb spill metadata table failed: {e}")))?;
        meta.get(ACTIVE_RESTORE_SEQUENCE)
            .map_err(|e| MemoryError::report(format!("redb active restore read failed: {e}")))
            .map(|active| active.map(|value| DurableRecordId::new(value.value())))
    }

    fn recover_active_restore(&self) -> MemoryResult<Option<DurableRestoreRecovery>> {
        // Claiming recovery (FUP-002): reading the persisted candidate and
        // minting a fresh generation share ONE write transaction, so each
        // concurrent recoverer owns a distinct generation and all older
        // holders of the sequence lose authority. The phase is preserved, not
        // collapsed: a Committed payload stays Committed.
        let txn = self.database.begin_write().map_err(|e| {
            MemoryError::report(format!("redb active restore recovery transaction failed: {e}"))
        })?;
        let recovery = {
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::report(format!("redb spill metadata table failed: {e}"))
            })?;
            let Some(sequence) = meta
                .get(ACTIVE_RESTORE_SEQUENCE)
                .map_err(|e| MemoryError::report(format!("redb active restore read failed: {e}")))?
                .map(|value| value.value())
            else {
                return Ok(None);
            };
            let recovery_table = txn.open_table(SPILL_RECOVERY).map_err(|e| {
                MemoryError::report(format!("redb spill recovery table open failed: {e}"))
            })?;
            let queue = txn
                .open_table(SPILL_QUEUE)
                .map_err(|e| MemoryError::report(format!("redb spill table open failed: {e}")))?;
            let (bytes, lease, phase) =
                if let Some(payload) = recovery_table.get(sequence).map_err(|e| {
                    MemoryError::report(format!("redb recovery payload read failed: {e}"))
                })? {
                    let (bytes, lease) = self.copy_restore_bytes(payload.value())?;
                    (bytes, lease, DurableRestorePhase::Committed)
                } else if let Some(payload) = queue.get(sequence).map_err(|e| {
                    MemoryError::report(format!("redb spill payload read failed: {e}"))
                })? {
                    let (bytes, lease) = self.copy_restore_bytes(payload.value())?;
                    (bytes, lease, DurableRestorePhase::Prepared)
                } else {
                    return Err(MemoryError::InvalidState(format!(
                        "active restore {sequence} has neither queued nor recovery payload"
                    )));
                };
            let counter = meta
                .get(RESTORE_CLAIM_GENERATION)
                .map_err(|e| MemoryError::report(format!("redb claim counter read failed: {e}")))?
                .map(|value| value.value())
                .unwrap_or(0);
            let generation = counter.checked_add(1).ok_or_else(|| {
                MemoryError::capacity("durable restore claim generation exhausted (u64::MAX)")
            })?;
            meta.insert(RESTORE_CLAIM_GENERATION, generation).map_err(|e| {
                MemoryError::report(format!("redb claim counter write failed: {e}"))
            })?;
            meta.insert(ACTIVE_RESTORE_GENERATION, generation)
                .map_err(|e| MemoryError::report(format!("redb active claim write failed: {e}")))?;
            DurableRestoreRecovery {
                candidate: DurableRestoreCandidate::from_redb(
                    sequence,
                    bytes,
                    DurableRestoreClaim::issued(self.backend_id, generation, sequence),
                    lease,
                ),
                phase,
            }
        };
        match txn.commit() {
            Ok(()) => Ok(Some(recovery)),
            Err(error) => match self.active_claim()? {
                // This generation landed: only it proves the recovery.
                Some((sequence, Some(generation)))
                    if sequence == recovery.candidate.sequence
                        && generation == recovery.candidate.claim.generation() =>
                {
                    Ok(Some(recovery))
                }
                _ => Err(MemoryError::report(format!(
                    "redb restore recovery outcome is unknown: {error}"
                ))),
            },
        }
    }
}
