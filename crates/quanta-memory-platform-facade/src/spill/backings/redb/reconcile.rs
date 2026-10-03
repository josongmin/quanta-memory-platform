use super::*;

impl ReconciledDurableSpillTarget for RedbSpillBacking {
    fn max_record_bytes(&self) -> Option<u64> {
        Some(self.limits.max_record_bytes)
    }

    fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
        let txn = self
            .database
            .begin_write()
            .map_err(|e| redb_storage_error("redb record-id transaction failed", e))?;
        let record_id = {
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::report(format!("redb spill metadata table failed: {e}"))
            })?;
            let next = meta
                .get(NEXT_SEQUENCE)
                .map_err(|e| MemoryError::report(format!("redb spill metadata read failed: {e}")))?
                .map(|value| value.value())
                .ok_or_else(|| {
                    MemoryError::InvalidState(
                        "redb spill next-sequence authority is missing".into(),
                    )
                })?;
            let following = next.checked_add(1).ok_or_else(|| {
                MemoryError::capacity("redb spill record-id space exhausted (u64::MAX)")
            })?;
            meta.insert(NEXT_SEQUENCE, following)
                .map_err(|e| redb_storage_error("redb spill metadata update failed", e))?;
            // The allocator and the disposition share one transaction: an
            // allocated id always carries an exact `Reserved` row, so a hole
            // can never be mistaken for a consumed record.
            let mut states = txn.open_table(SPILL_RECORD_STATE).map_err(|e| {
                MemoryError::report(format!("redb record disposition table failed: {e}"))
            })?;
            states
                .insert(next, RecordDisposition::Reserved as u8)
                .map_err(|e| redb_storage_error("redb record disposition write failed", e))?;
            DurableRecordId::new(next)
        };
        // A lost acknowledgement may burn an ID, but no payload was moved and
        // IDs are never reused. A hole is safe; guessing an ID is not.
        txn.commit()
            .map_err(|e| redb_storage_error("redb record-id commit outcome is unknown", e))?;
        Ok(record_id)
    }

    fn commit_spill(
        &self,
        record_id: DurableRecordId,
        bytes: &[u8],
    ) -> Result<SpillReceipt, DurableWriteFailure> {
        let not_committed = |error| DurableWriteFailure::NotCommitted(Box::new(error));
        let record_bytes = self.limits.check_record(bytes.len()).map_err(not_committed)?;
        #[cfg(test)]
        if self.faults.fail_write_before_transaction.swap(false, AtomicOrdering::SeqCst) {
            return Err(not_committed(MemoryError::report(
                "injected redb failure before write transaction",
            )));
        }
        let txn = self.database.begin_write().map_err(|e| {
            not_committed(redb_storage_error("redb spill write transaction failed", e))
        })?;
        let inventory_after = {
            let mut table = txn.open_table(SPILL_QUEUE).map_err(|e| {
                not_committed(MemoryError::report(format!("redb spill table open failed: {e}")))
            })?;
            let recovery = txn.open_table(SPILL_RECOVERY).map_err(|e| {
                not_committed(MemoryError::report(format!("redb recovery table open failed: {e}")))
            })?;
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                not_committed(MemoryError::report(format!("redb spill metadata table failed: {e}")))
            })?;
            let mut states = txn.open_table(SPILL_RECORD_STATE).map_err(|e| {
                not_committed(MemoryError::report(format!(
                    "redb record disposition table failed: {e}"
                )))
            })?;
            let next = meta
                .get(NEXT_SEQUENCE)
                .map_err(|e| {
                    not_committed(MemoryError::report(format!(
                        "redb spill metadata read failed: {e}"
                    )))
                })?
                .map(|value| value.value())
                .ok_or_else(|| {
                    not_committed(MemoryError::InvalidState(
                        "redb spill next-sequence authority is missing".into(),
                    ))
                })?;
            if record_id.get() >= next {
                return Err(not_committed(MemoryError::InvalidState(format!(
                    "durable record {} was not reserved; next sequence is {next}",
                    record_id.get()
                ))));
            }
            // Exact disposition authority: no watermark estimation. An id below
            // the allocator frontier without a disposition row is legacy
            // history (hole vs. consumed is unknowable) and fails closed.
            let disposition = states
                .get(record_id.get())
                .map_err(|e| {
                    not_committed(MemoryError::report(format!(
                        "redb record disposition read failed: {e}"
                    )))
                })?
                .map(|value| RecordDisposition::decode(value.value()).map_err(not_committed))
                .transpose()?
                .ok_or_else(|| not_committed(unresolved_record_error(record_id.get())))?;
            match disposition {
                RecordDisposition::Finalized => {
                    return Err(not_committed(MemoryError::InvalidState(format!(
                        "durable record {} was already finalized and cannot be recreated",
                        record_id.get()
                    ))));
                }
                RecordDisposition::Recovery => {
                    return Err(not_committed(MemoryError::InvalidState(format!(
                        "durable record {} is committed to the recovery slot and cannot be rewritten",
                        record_id.get()
                    ))));
                }
                RecordDisposition::Queued => {
                    let existing = table
                        .get(record_id.get())
                        .map_err(|e| {
                            not_committed(MemoryError::report(format!(
                                "redb spill idempotency read failed: {e}"
                            )))
                        })?
                        .ok_or_else(|| {
                            not_committed(MemoryError::InvalidState(format!(
                                "durable record {} is queued but its payload is missing",
                                record_id.get()
                            )))
                        })?;
                    // Compare the borrowed Redb value: an idempotency probe
                    // must not allocate a second unbudgeted payload copy.
                    if existing.value() != bytes {
                        return Err(not_committed(MemoryError::InvalidState(format!(
                            "durable record {} already contains different bytes",
                            record_id.get()
                        ))));
                    }
                    let queued = table.len().map_err(|e| {
                        not_committed(MemoryError::report(format!(
                            "redb spill inventory read failed: {e}"
                        )))
                    })?;
                    let recovering = recovery.len().map_err(|e| {
                        not_committed(MemoryError::report(format!(
                            "redb recovery inventory read failed: {e}"
                        )))
                    })?;
                    queued.checked_add(recovering).ok_or_else(|| {
                        not_committed(MemoryError::capacity("redb spill inventory exceeds u64"))
                    })?
                }
                RecordDisposition::Reserved => {
                    let retained_records = table
                        .len()
                        .and_then(|queued| recovery.len().map(|recovering| (queued, recovering)))
                        .map_err(|e| {
                            not_committed(MemoryError::report(format!(
                                "redb retained count failed: {e}"
                            )))
                        })?;
                    let after_count = retained_records
                        .0
                        .checked_add(retained_records.1)
                        .and_then(|count| count.checked_add(1))
                        .ok_or_else(|| {
                            not_committed(MemoryError::capacity(
                                "redb retained record count exceeds u64",
                            ))
                        })?;
                    if after_count > self.limits.max_retained_records {
                        return Err(not_committed(MemoryError::capacity(format!(
                            "redb spill would retain {after_count} records, above configured {}-record limit",
                            self.limits.max_retained_records
                        ))));
                    }
                    let retained_bytes = meta
                        .get(RETAINED_PAYLOAD_BYTES)
                        .map_err(|e| {
                            not_committed(MemoryError::report(format!(
                                "redb payload usage read failed: {e}"
                            )))
                        })?
                        .map(|value| value.value())
                        .ok_or_else(|| {
                            not_committed(MemoryError::InvalidState(
                                "redb retained payload usage authority is missing".into(),
                            ))
                        })?;
                    let after_bytes =
                        retained_bytes.checked_add(record_bytes).ok_or_else(|| {
                            not_committed(MemoryError::capacity(
                                "redb retained payload bytes exceed u64",
                            ))
                        })?;
                    if after_bytes > self.limits.max_retained_payload_bytes {
                        return Err(not_committed(MemoryError::capacity(format!(
                            "redb spill would retain {after_bytes} payload bytes, above configured {}-byte limit",
                            self.limits.max_retained_payload_bytes
                        ))));
                    }
                    table.insert(record_id.get(), bytes).map_err(|e| {
                        not_committed(redb_storage_error("redb spill insert failed", e))
                    })?;
                    meta.insert(RETAINED_PAYLOAD_BYTES, after_bytes).map_err(|e| {
                        not_committed(redb_storage_error("redb payload usage update failed", e))
                    })?;
                    states.insert(record_id.get(), RecordDisposition::Queued as u8).map_err(
                        |e| {
                            not_committed(redb_storage_error(
                                "redb record disposition write failed",
                                e,
                            ))
                        },
                    )?;
                    after_count
                }
            }
        };

        let commit_error = match txn.commit() {
            Ok(()) => {
                #[cfg(test)]
                if self.faults.lose_write_commit_ack.swap(false, AtomicOrdering::SeqCst) {
                    Some(MemoryError::report("injected lost redb spill commit acknowledgement"))
                } else {
                    None
                }
                #[cfg(not(test))]
                None
            }
            Err(error) => {
                Some(redb_storage_error("redb spill commit acknowledgement failed", error))
            }
        };
        match commit_error {
            None => Ok(SpillReceipt::Durable { sequence: record_id.get(), inventory_after }),
            Some(commit_error) => match self.resolve_record(record_id) {
                Ok(DurableRecordResolution::Present { inventory_after }) => {
                    Ok(SpillReceipt::Durable { sequence: record_id.get(), inventory_after })
                }
                Ok(DurableRecordResolution::Absent) => {
                    Err(DurableWriteFailure::NotCommitted(Box::new(commit_error)))
                }
                Ok(DurableRecordResolution::Finalized) => {
                    let inventory_after = self.retained_count().map_err(|probe_error| {
                        DurableWriteFailure::RecoveryRequired {
                            record_id,
                            commit_error: Box::new(commit_error),
                            probe_error: Box::new(probe_error),
                        }
                    })?;
                    Ok(SpillReceipt::Durable { sequence: record_id.get(), inventory_after })
                }
                // Defensive only: the write probe never reports a superseded
                // restore claim. Fence rather than guess if it ever does.
                Ok(DurableRecordResolution::Superseded) => {
                    Err(DurableWriteFailure::RecoveryRequired {
                        record_id,
                        commit_error: Box::new(commit_error),
                        probe_error: Box::new(MemoryError::InvalidState(
                            "durable write probe unexpectedly reported a superseded claim".into(),
                        )),
                    })
                }
                Err(probe_error) => Err(DurableWriteFailure::RecoveryRequired {
                    record_id,
                    commit_error: Box::new(commit_error),
                    probe_error: Box::new(probe_error),
                }),
            },
        }
    }

    fn resolve_record(&self, record_id: DurableRecordId) -> MemoryResult<DurableRecordResolution> {
        let read = self.database.begin_read().map_err(|e| {
            MemoryError::report(format!("redb spill probe transaction failed: {e}"))
        })?;
        match read.open_table(SPILL_QUEUE) {
            Ok(table) => {
                let recovery = read.open_table(SPILL_RECOVERY).map_err(|error| {
                    MemoryError::report(format!(
                        "redb spill recovery probe table open failed: {error}"
                    ))
                })?;
                let states = read.open_table(SPILL_RECORD_STATE).map_err(|error| {
                    MemoryError::report(format!(
                        "redb record disposition probe table open failed: {error}"
                    ))
                })?;
                let queue_present = table
                    .get(record_id.get())
                    .map_err(|e| MemoryError::report(format!("redb spill probe failed: {e}")))?
                    .is_some();
                let recovery_present = recovery
                    .get(record_id.get())
                    .map_err(|e| MemoryError::report(format!("redb recovery probe failed: {e}")))?
                    .is_some();
                if queue_present || recovery_present {
                    // Payload evidence wins for presence, but the disposition
                    // row must agree — payload and disposition share every write
                    // transaction, so disagreement is an internal invariant
                    // break, never a guessable state.
                    let disposition = states
                        .get(record_id.get())
                        .map_err(|e| {
                            MemoryError::report(format!(
                                "redb record disposition probe failed: {e}"
                            ))
                        })?
                        .map(|value| RecordDisposition::decode(value.value()))
                        .transpose()?;
                    let expected = if queue_present {
                        RecordDisposition::Queued
                    } else {
                        RecordDisposition::Recovery
                    };
                    if disposition != Some(expected) {
                        return Err(MemoryError::InvalidState(format!(
                            "durable record {} has payload evidence but disposition {disposition:?}",
                            record_id.get()
                        )));
                    }
                    let queue_len = table.len().map_err(|e| {
                        MemoryError::report(format!("redb spill probe inventory failed: {e}"))
                    })?;
                    let recovery_len = recovery.len().map_err(|e| {
                        MemoryError::report(format!("redb recovery probe inventory failed: {e}"))
                    })?;
                    Ok(DurableRecordResolution::Present {
                        inventory_after: queue_len.checked_add(recovery_len).ok_or_else(|| {
                            MemoryError::capacity("redb spill inventory exceeds u64")
                        })?,
                    })
                } else {
                    // No payload anywhere: the disposition row is the sole
                    // authority. The legacy watermark is never consulted.
                    match states
                        .get(record_id.get())
                        .map_err(|e| {
                            MemoryError::report(format!(
                                "redb record disposition probe failed: {e}"
                            ))
                        })?
                        .map(|value| RecordDisposition::decode(value.value()))
                        .transpose()?
                    {
                        Some(RecordDisposition::Finalized) => Ok(DurableRecordResolution::Finalized),
                        Some(RecordDisposition::Reserved) => Ok(DurableRecordResolution::Absent),
                        Some(RecordDisposition::Queued) | Some(RecordDisposition::Recovery) => {
                            Err(MemoryError::InvalidState(format!(
                                "durable record {} is queued for restore but its payload is missing",
                                record_id.get()
                            )))
                        }
                        None => {
                            let meta = read.open_table(SPILL_META).map_err(|error| {
                                MemoryError::report(format!(
                                    "redb spill probe metadata open failed: {error}"
                                ))
                            })?;
                            let next = meta
                                .get(NEXT_SEQUENCE)
                                .map_err(|error| {
                                    MemoryError::report(format!(
                                        "redb spill probe metadata read failed: {error}"
                                    ))
                                })?
                                .map(|value| value.value());
                            match next {
                                // Never issued: genuinely absent, not a legacy gap.
                                Some(next) if record_id.get() >= next => {
                                    Ok(DurableRecordResolution::Absent)
                                }
                                // Below the allocator frontier with no exact
                                // history: fail closed, never guess.
                                _ => Err(unresolved_record_error(record_id.get())),
                            }
                        }
                    }
                }
            }
            Err(redb::TableError::TableDoesNotExist(_)) => Ok(DurableRecordResolution::Absent),
            Err(error) => {
                Err(MemoryError::report(format!("redb spill probe table open failed: {error}")))
            }
        }
    }
}
