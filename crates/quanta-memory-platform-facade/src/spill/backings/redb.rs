//! Durable spill backing over `redb` (opt-in `spill-redb` feature).
//!
//! `RedbSpillBacking` is a durable [`SpillTarget`] for the spill slice: it stores
//! spilled handles (opaque `Vec<u8>`) in a `redb` table and restores them FIFO.
//! It is reached ONLY through the `SpillTarget` abstraction — the raw `redb`
//! database handle and `redb`'s error/types never appear in the facade's public
//! surface (boundary rule: no backing-specific nouns leak out of the slice).

use std::path::Path;

use quanta_memory_platform_core::{MemoryError, MemoryResult};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};

use crate::spill::service::ReconciledDurableSpillTarget;
use crate::spill::{
    DurableCommitFailure, DurableRecordId, DurableRecordResolution, DurableRestoreCandidate,
    DurableRestoreTarget, DurableSpillTarget, DurableWriteFailure, SpillFailure, SpillInventory,
    SpillReceipt, SpillTarget,
};

/// The single FIFO queue table: monotonic sequence -> opaque handle bytes.
const SPILL_QUEUE: TableDefinition<'static, u64, &[u8]> = TableDefinition::new("qmp_spill_queue");
/// Persistent allocator state. Queue emptiness must never reset sequence
/// identity, otherwise a stale restore token can delete a later record (ABA).
const SPILL_META: TableDefinition<'static, &str, u64> = TableDefinition::new("qmp_spill_meta");
const NEXT_SEQUENCE: &str = "next_sequence";
const ACTIVE_RESTORE_SEQUENCE: &str = "active_restore_sequence";
const FINALIZED_THROUGH: &str = "finalized_through";

/// A durable spill backing over `redb`. Implements [`SpillTarget<Vec<u8>>`]; the
/// `redb` database is fully encapsulated.
#[derive(Debug)]
pub struct RedbSpillBacking {
    database: Database,
}

impl RedbSpillBacking {
    /// Open (creating if absent) a redb-backed durable spill tier at `path`. A
    /// backend failure is surfaced as a neutral [`MemoryError`], never a leaked
    /// `redb` error type.
    pub fn create(path: impl AsRef<Path>) -> MemoryResult<Self> {
        let database = Database::create(path)
            .map_err(|e| MemoryError::build(format!("redb spill backing open failed: {e}")))?;
        let txn = database.begin_write().map_err(|e| {
            MemoryError::build(format!("redb spill metadata transaction failed: {e}"))
        })?;
        {
            let queue = txn.open_table(SPILL_QUEUE).map_err(|e| {
                MemoryError::build(format!("redb spill queue initialization failed: {e}"))
            })?;
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::build(format!("redb spill metadata initialization failed: {e}"))
            })?;
            if meta
                .get(NEXT_SEQUENCE)
                .map_err(|e| MemoryError::build(format!("redb spill metadata read failed: {e}")))?
                .is_none()
            {
                let next = match queue
                    .last()
                    .map_err(|e| {
                        MemoryError::build(format!("redb spill migration scan failed: {e}"))
                    })?
                    .map(|(key, _)| key.value())
                {
                    Some(last) => last.checked_add(1).ok_or_else(|| {
                        MemoryError::capacity("redb spill queue cannot migrate a u64::MAX sequence")
                    })?,
                    None => 0,
                };
                meta.insert(NEXT_SEQUENCE, next).map_err(|e| {
                    MemoryError::build(format!("redb spill metadata seed failed: {e}"))
                })?;
            }
            if meta
                .get(FINALIZED_THROUGH)
                .map_err(|e| {
                    MemoryError::build(format!("redb finalized watermark read failed: {e}"))
                })?
                .is_none()
            {
                let next = meta
                    .get(NEXT_SEQUENCE)
                    .map_err(|e| {
                        MemoryError::build(format!("redb spill metadata read failed: {e}"))
                    })?
                    .map(|value| value.value())
                    .ok_or_else(|| {
                        MemoryError::InvalidState(
                            "redb spill next-sequence authority is missing after initialization"
                                .into(),
                        )
                    })?;
                let first_retained = queue
                    .first()
                    .map_err(|e| {
                        MemoryError::build(format!("redb spill migration scan failed: {e}"))
                    })?
                    .map(|(key, _)| key.value());
                let finalized = first_retained
                    .and_then(|first| first.checked_sub(1))
                    .or_else(|| (first_retained.is_none() && next > 0).then_some(next - 1));
                if let Some(finalized) = finalized {
                    meta.insert(FINALIZED_THROUGH, finalized).map_err(|e| {
                        MemoryError::build(format!(
                            "redb finalized watermark migration failed: {e}"
                        ))
                    })?;
                }
            }
        }
        txn.commit()
            .map_err(|e| MemoryError::build(format!("redb spill metadata commit failed: {e}")))?;
        Ok(Self { database })
    }

    /// Number of handles currently retained (restorable). For diagnostics/tests.
    pub fn retained_count(&self) -> MemoryResult<u64> {
        let read = self
            .database
            .begin_read()
            .map_err(|e| MemoryError::report(format!("redb spill read txn failed: {e}")))?;
        match read.open_table(SPILL_QUEUE) {
            Ok(table) => {
                table.len().map_err(|e| MemoryError::report(format!("redb spill len failed: {e}")))
            }
            Err(redb::TableError::TableDoesNotExist(_)) => Ok(0),
            Err(e) => Err(MemoryError::report(format!("redb spill table open failed: {e}"))),
        }
    }

    /// Test-only seam: insert a handle under an explicit key, so a test can drive
    /// the (otherwise astronomically unreachable) exhausted-keyspace branch.
    #[cfg(test)]
    fn seed_key_for_test(&self, key: u64, value: &[u8]) -> MemoryResult<()> {
        let txn = self
            .database
            .begin_write()
            .map_err(|e| MemoryError::report(format!("redb spill write txn failed: {e}")))?;
        {
            let mut table = txn
                .open_table(SPILL_QUEUE)
                .map_err(|e| MemoryError::report(format!("redb spill table open failed: {e}")))?;
            table
                .insert(key, value)
                .map_err(|e| MemoryError::report(format!("redb spill insert failed: {e}")))?;
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::report(format!("redb spill metadata table failed: {e}"))
            })?;
            meta.insert(NEXT_SEQUENCE, key).map_err(|e| {
                MemoryError::report(format!("redb spill metadata seed failed: {e}"))
            })?;
        }
        txn.commit().map_err(|e| MemoryError::report(format!("redb spill commit failed: {e}")))?;
        Ok(())
    }
}

impl SpillTarget<Vec<u8>> for RedbSpillBacking {
    fn is_available(&self) -> bool {
        // A durable on-disk tier is always available within the process; per-op
        // backend errors surface as typed `Err` from accept/restore.
        true
    }

    fn accept(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
        Err(SpillFailure::new(
            MemoryError::Unsupported(
                "durable redb writes require reserve_record_id/commit_spill so an ambiguous commit cannot duplicate source authority"
                    .into(),
            ),
            handle,
        ))
    }

    fn restore(&self) -> MemoryResult<Option<Vec<u8>>> {
        Err(MemoryError::Unsupported(
            "durable redb restore requires prepare_restore/commit_restore so ambiguous delete retains the recovery payload"
                .into(),
        ))
    }
}

impl SpillInventory for RedbSpillBacking {
    fn retained_count(&self) -> MemoryResult<u64> {
        RedbSpillBacking::retained_count(self)
    }
}

impl DurableSpillTarget<Vec<u8>> for RedbSpillBacking {}
impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for RedbSpillBacking {}

impl ReconciledDurableSpillTarget for RedbSpillBacking {
    fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
        let txn = self
            .database
            .begin_write()
            .map_err(|e| MemoryError::report(format!("redb record-id transaction failed: {e}")))?;
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
            meta.insert(NEXT_SEQUENCE, following).map_err(|e| {
                MemoryError::report(format!("redb spill metadata update failed: {e}"))
            })?;
            DurableRecordId::new(next)
        };
        // A lost acknowledgement may burn an ID, but no payload was moved and
        // IDs are never reused. A hole is safe; guessing an ID is not.
        txn.commit().map_err(|e| {
            MemoryError::report(format!("redb record-id commit outcome is unknown: {e}"))
        })?;
        Ok(record_id)
    }

    fn commit_spill(
        &self,
        record_id: DurableRecordId,
        bytes: &[u8],
    ) -> Result<SpillReceipt, DurableWriteFailure> {
        let not_committed = |error| DurableWriteFailure::NotCommitted(Box::new(error));
        let txn = self.database.begin_write().map_err(|e| {
            not_committed(MemoryError::report(format!("redb spill write transaction failed: {e}")))
        })?;
        let inventory_after = {
            let mut table = txn.open_table(SPILL_QUEUE).map_err(|e| {
                not_committed(MemoryError::report(format!("redb spill table open failed: {e}")))
            })?;
            let meta = txn.open_table(SPILL_META).map_err(|e| {
                not_committed(MemoryError::report(format!("redb spill metadata table failed: {e}")))
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
            let finalized_through = meta
                .get(FINALIZED_THROUGH)
                .map_err(|e| {
                    not_committed(MemoryError::report(format!(
                        "redb finalized watermark read failed: {e}"
                    )))
                })?
                .map(|value| value.value());
            if finalized_through.is_some_and(|finalized| record_id.get() <= finalized) {
                return Err(not_committed(MemoryError::InvalidState(format!(
                    "durable record {} was already finalized and cannot be recreated",
                    record_id.get()
                ))));
            }
            let existing_bytes = table
                .get(record_id.get())
                .map_err(|e| {
                    not_committed(MemoryError::report(format!(
                        "redb spill idempotency read failed: {e}"
                    )))
                })?
                .map(|existing| existing.value().to_vec());
            if let Some(existing_bytes) = existing_bytes {
                if existing_bytes != bytes {
                    return Err(not_committed(MemoryError::InvalidState(format!(
                        "durable record {} already contains different bytes",
                        record_id.get()
                    ))));
                }
                table.len().map_err(|e| {
                    not_committed(MemoryError::report(format!(
                        "redb spill inventory read failed: {e}"
                    )))
                })?
            } else {
                table.insert(record_id.get(), bytes).map_err(|e| {
                    not_committed(MemoryError::report(format!("redb spill insert failed: {e}")))
                })?;
                table.len().map_err(|e| {
                    not_committed(MemoryError::report(format!(
                        "redb spill post-insert inventory failed: {e}"
                    )))
                })?
            }
        };

        match txn.commit() {
            Ok(()) => Ok(SpillReceipt::Durable { sequence: record_id.get(), inventory_after }),
            Err(error) => {
                let commit_error = MemoryError::report(format!(
                    "redb spill commit acknowledgement failed: {error}"
                ));
                match self.resolve_record(record_id) {
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
                    Err(probe_error) => Err(DurableWriteFailure::RecoveryRequired {
                        record_id,
                        commit_error: Box::new(commit_error),
                        probe_error: Box::new(probe_error),
                    }),
                }
            }
        }
    }

    fn resolve_record(&self, record_id: DurableRecordId) -> MemoryResult<DurableRecordResolution> {
        let read = self.database.begin_read().map_err(|e| {
            MemoryError::report(format!("redb spill probe transaction failed: {e}"))
        })?;
        match read.open_table(SPILL_QUEUE) {
            Ok(table) => {
                if table
                    .get(record_id.get())
                    .map_err(|e| MemoryError::report(format!("redb spill probe failed: {e}")))?
                    .is_some()
                {
                    Ok(DurableRecordResolution::Present {
                        inventory_after: table.len().map_err(|e| {
                            MemoryError::report(format!("redb spill probe inventory failed: {e}"))
                        })?,
                    })
                } else {
                    let meta = read.open_table(SPILL_META).map_err(|error| {
                        MemoryError::report(format!(
                            "redb spill probe metadata open failed: {error}"
                        ))
                    })?;
                    let finalized = meta
                        .get(FINALIZED_THROUGH)
                        .map_err(|error| {
                            MemoryError::report(format!(
                                "redb finalized watermark probe failed: {error}"
                            ))
                        })?
                        .map(|value| value.value());
                    if finalized.is_some_and(|value| record_id.get() <= value) {
                        Ok(DurableRecordResolution::Finalized)
                    } else {
                        Ok(DurableRecordResolution::Absent)
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
            let candidate = table
                .first()
                .map_err(|e| MemoryError::report(format!("redb spill first failed: {e}")))?
                .map(|(sequence, bytes)| DurableRestoreCandidate {
                    sequence: sequence.value(),
                    bytes: bytes.value().to_vec(),
                });
            if let Some(candidate) = &candidate {
                meta.insert(ACTIVE_RESTORE_SEQUENCE, candidate.sequence).map_err(|e| {
                    MemoryError::report(format!("redb restore claim write failed: {e}"))
                })?;
            }
            candidate
        };
        if candidate.is_none() {
            return Ok(None);
        }
        match txn.commit() {
            Ok(()) => Ok(candidate),
            Err(error) => {
                let candidate = candidate.expect("non-empty candidate was checked");
                match self.active_restore()? {
                    Some(active) if active.get() == candidate.sequence => Ok(Some(candidate)),
                    _ => Err(MemoryError::report(format!(
                        "redb restore claim commit outcome is unknown: {error}"
                    ))),
                }
            }
        }
    }

    fn commit_restore(&self, sequence: u64) -> Result<(), DurableCommitFailure> {
        let txn = self.database.begin_write().map_err(|e| {
            DurableCommitFailure::retained(MemoryError::report(format!(
                "redb spill write txn failed: {e}"
            )))
        })?;
        {
            let mut table = txn.open_table(SPILL_QUEUE).map_err(|e| {
                DurableCommitFailure::retained(MemoryError::report(format!(
                    "redb spill table open failed: {e}"
                )))
            })?;
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                DurableCommitFailure::retained(MemoryError::report(format!(
                    "redb spill metadata table failed: {e}"
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
            if active != Some(sequence) {
                return Err(DurableCommitFailure::retained(MemoryError::InvalidState(format!(
                    "durable restore candidate {sequence} is not the active claim {active:?}"
                ))));
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
            let removed = table.pop_first().map_err(|e| {
                DurableCommitFailure::retained(MemoryError::report(format!(
                    "redb spill pop failed: {e}"
                )))
            })?;
            if removed.is_none() {
                return Err(DurableCommitFailure::retained(MemoryError::InvalidState(
                    "durable restore candidate vanished before commit".into(),
                )));
            }
            meta.remove(ACTIVE_RESTORE_SEQUENCE).map_err(|e| {
                DurableCommitFailure::retained(MemoryError::report(format!(
                    "redb active restore clear failed: {e}"
                )))
            })?;
            let finalized = meta
                .get(FINALIZED_THROUGH)
                .map_err(|e| {
                    DurableCommitFailure::retained(MemoryError::report(format!(
                        "redb finalized watermark read failed: {e}"
                    )))
                })?
                .map(|value| value.value());
            if finalized.is_none_or(|value| sequence > value) {
                meta.insert(FINALIZED_THROUGH, sequence).map_err(|e| {
                    DurableCommitFailure::retained(MemoryError::report(format!(
                        "redb finalized watermark write failed: {e}"
                    )))
                })?;
            }
        }
        txn.commit().map_err(|e| {
            DurableCommitFailure::ambiguous(MemoryError::report(format!(
                "redb spill restore commit outcome is ambiguous: {e}"
            )))
        })
    }

    fn abort_restore(&self, sequence: u64) -> MemoryResult<()> {
        let txn = self.database.begin_write().map_err(|e| {
            MemoryError::report(format!("redb restore-abort transaction failed: {e}"))
        })?;
        {
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::report(format!("redb spill metadata table failed: {e}"))
            })?;
            let active = meta
                .get(ACTIVE_RESTORE_SEQUENCE)
                .map_err(|e| MemoryError::report(format!("redb active restore read failed: {e}")))?
                .map(|value| value.value());
            match active {
                Some(active) if active == sequence => {
                    meta.remove(ACTIVE_RESTORE_SEQUENCE).map_err(|e| {
                        MemoryError::report(format!("redb active restore clear failed: {e}"))
                    })?;
                }
                None => return Ok(()),
                Some(active) => {
                    return Err(MemoryError::InvalidState(format!(
                        "cannot abort restore {sequence}; active claim is {active}"
                    )))
                }
            }
        }
        match txn.commit() {
            Ok(()) => Ok(()),
            Err(error) => match self.active_restore()? {
                None => Ok(()),
                Some(active) if active.get() == sequence => Err(MemoryError::report(format!(
                    "redb restore-abort commit outcome remains unresolved: {error}"
                ))),
                Some(active) => Err(MemoryError::InvalidState(format!(
                    "restore-abort {sequence} observed foreign active claim {}",
                    active.get()
                ))),
            },
        }
    }

    fn reconcile_restore(&self, sequence: u64) -> MemoryResult<DurableRecordResolution> {
        let read = self.database.begin_read().map_err(|e| {
            MemoryError::report(format!("redb restore reconciliation read failed: {e}"))
        })?;
        let table = read
            .open_table(SPILL_QUEUE)
            .map_err(|e| MemoryError::report(format!("redb spill table open failed: {e}")))?;
        let meta = read
            .open_table(SPILL_META)
            .map_err(|e| MemoryError::report(format!("redb spill metadata table failed: {e}")))?;
        let present = table
            .get(sequence)
            .map_err(|e| MemoryError::report(format!("redb restore probe failed: {e}")))?
            .is_some();
        let inventory_after = table
            .len()
            .map_err(|e| MemoryError::report(format!("redb spill inventory read failed: {e}")))?;
        let active = meta
            .get(ACTIVE_RESTORE_SEQUENCE)
            .map_err(|e| MemoryError::report(format!("redb active restore read failed: {e}")))?
            .map(|value| value.value());
        let finalized = meta
            .get(FINALIZED_THROUGH)
            .map_err(|e| MemoryError::report(format!("redb finalized watermark read failed: {e}")))?
            .map(|value| value.value());
        drop(meta);
        drop(table);
        drop(read);

        if present {
            if active.is_some_and(|value| value != sequence) {
                return Err(MemoryError::InvalidState(format!(
                    "restore {sequence} is present while foreign claim {active:?} is active"
                )));
            }
            if active == Some(sequence) {
                self.abort_restore(sequence)?;
            }
            return Ok(DurableRecordResolution::Present { inventory_after });
        }
        if finalized.is_some_and(|value| sequence <= value) {
            return Ok(DurableRecordResolution::Finalized);
        }
        Err(MemoryError::InvalidState(format!(
            "restore {sequence} is neither retained nor durably finalized"
        )))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spill::{
        DurableRestoreController, ReconciledDurableSpillController, RoutedSpillController,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_db_path() -> std::path::PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("qmp-redb-backing-test-{n}.redb"))
    }

    fn restore_backing(backing: &RedbSpillBacking) -> MemoryResult<Option<Vec<u8>>> {
        let Some(candidate) = backing.prepare_restore()? else {
            return Ok(None);
        };
        let bytes = candidate.bytes;
        backing.commit_restore(candidate.sequence).map_err(|failure| match failure {
            DurableCommitFailure::Retained(error) | DurableCommitFailure::Ambiguous(error) => {
                *error
            }
        })?;
        Ok(Some(bytes))
    }

    fn restore_controller<C: DurableRestoreController>(
        controller: &C,
    ) -> MemoryResult<Option<Vec<u8>>> {
        let Some(candidate) = controller.prepare_restore()? else {
            return Ok(None);
        };
        let bytes = candidate.bytes;
        controller.commit_restore(candidate.sequence).map_err(|failure| match failure {
            DurableCommitFailure::Retained(error) | DurableCommitFailure::Ambiguous(error) => {
                *error
            }
        })?;
        Ok(Some(bytes))
    }

    fn spill_backing(backing: &RedbSpillBacking, bytes: &[u8]) -> MemoryResult<SpillReceipt> {
        let record_id = backing.reserve_record_id()?;
        backing.commit_spill(record_id, bytes).map_err(|failure| match failure {
            DurableWriteFailure::NotCommitted(error) => *error,
            DurableWriteFailure::RecoveryRequired { commit_error, probe_error, .. } => {
                MemoryError::OperationInProgress(format!(
                    "durable write requires reconciliation: {commit_error}; probe: {probe_error}"
                ))
            }
        })
    }

    fn spill_controller<C: ReconciledDurableSpillController>(
        controller: &C,
        bytes: &[u8],
    ) -> MemoryResult<SpillReceipt> {
        let record_id = controller.reserve_record_id()?;
        controller.commit_spill(record_id, bytes).map_err(|failure| match failure {
            DurableWriteFailure::NotCommitted(error) => *error,
            DurableWriteFailure::RecoveryRequired { commit_error, probe_error, .. } => {
                MemoryError::OperationInProgress(format!(
                    "durable write requires reconciliation: {commit_error}; probe: {probe_error}"
                ))
            }
        })
    }

    #[test]
    fn create_on_an_invalid_path_is_a_neutral_memory_error() {
        let bad = std::path::Path::new("/nonexistent-qmp-dir/redb/db.redb");
        let err = RedbSpillBacking::create(bad).expect_err("open under a missing dir must fail");
        assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");
    }

    #[test]
    fn durable_spill_target_round_trips_fifo_through_the_trait() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        let backing = RedbSpillBacking::create(&path).expect("open");

        // Reached only through the SpillTarget abstraction — no redb types in sight.
        let first = spill_backing(&backing, b"first").expect("accept 1");
        let second = spill_backing(&backing, b"second").expect("accept 2");
        assert_eq!(first, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });
        assert_eq!(second, SpillReceipt::Durable { sequence: 1, inventory_after: 2 });
        assert_eq!(backing.retained_count().expect("len"), 2);

        assert_eq!(restore_backing(&backing).expect("r1"), Some(b"first".to_vec()));
        assert_eq!(restore_backing(&backing).expect("r2"), Some(b"second".to_vec()));
        assert_eq!(restore_backing(&backing).expect("drained"), None);
        assert_eq!(backing.retained_count().expect("len"), 0);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reopened_backing_resumes_sequence_and_restores_persisted_handles() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        {
            let backing = RedbSpillBacking::create(&path).expect("open 1");
            let _receipt = spill_backing(&backing, b"persisted").expect("accept");
        }
        // Reopen: the persisted handle survives and the sequence does not collide.
        let backing = RedbSpillBacking::create(&path).expect("open 2");
        let receipt = spill_backing(&backing, b"after-restart").expect("accept again");
        assert_eq!(receipt, SpillReceipt::Durable { sequence: 1, inventory_after: 2 });
        assert_eq!(restore_backing(&backing).expect("r1"), Some(b"persisted".to_vec()));
        assert_eq!(restore_backing(&backing).expect("r2"), Some(b"after-restart".to_vec()));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn drained_queue_never_reuses_sequence_or_accepts_stale_restore_token() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        let backing = RedbSpillBacking::create(&path).expect("open");

        let first = spill_backing(&backing, b"first").expect("accept first");
        assert_eq!(first, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });
        let stale = backing.prepare_restore().expect("prepare").expect("candidate");
        backing.commit_restore(stale.sequence).expect("drain first");

        let replacement = spill_backing(&backing, b"replacement").expect("accept after full drain");
        assert_eq!(replacement, SpillReceipt::Durable { sequence: 1, inventory_after: 1 });
        assert!(matches!(
            backing.commit_restore(stale.sequence),
            Err(DurableCommitFailure::Retained(_))
        ));
        let current = backing.prepare_restore().expect("prepare current").expect("current");
        assert_eq!(current.sequence, 1);
        assert_eq!(current.bytes, b"replacement");
        assert_eq!(backing.retained_count(), Ok(1));

        let stale_write = backing
            .commit_spill(DurableRecordId::new(0), b"first")
            .expect_err("a finalized record ID must never be resurrected");
        assert!(matches!(stale_write, DurableWriteFailure::NotCommitted(_)));
        assert_eq!(backing.retained_count(), Ok(1));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn persisted_restore_claim_blocks_duplicate_prepare_and_recovers_after_reopen() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        {
            let backing = RedbSpillBacking::create(&path).expect("open 1");
            let _receipt = spill_backing(&backing, b"claimed").expect("seed");
            let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
            assert_eq!(candidate.sequence, 0);
            assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
        }

        let backing = RedbSpillBacking::create(&path).expect("open 2");
        assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
        assert!(matches!(backing.prepare_restore(), Err(MemoryError::OperationInProgress(_))));
        assert!(matches!(
            backing.reconcile_restore(0).expect("retained claim reconciliation"),
            DurableRecordResolution::Present { inventory_after: 1 }
        ));
        assert_eq!(backing.active_restore(), Ok(None));
        assert_eq!(backing.prepare_restore().expect("prepare after recovery").unwrap().sequence, 0);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn generic_restore_is_non_destructive_and_requires_owned_two_phase_path() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        let backing = RedbSpillBacking::create(&path).expect("open");
        let _receipt = spill_backing(&backing, b"only-copy").expect("accept");

        let error = SpillTarget::restore(&backing)
            .expect_err("generic destructive restore must be unavailable for durable records");
        assert!(matches!(error, MemoryError::Unsupported(_)));
        assert_eq!(backing.retained_count(), Ok(1));
        assert_eq!(
            backing.prepare_restore().expect("prepare").expect("candidate").bytes,
            b"only-copy"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn generic_write_is_non_mutating_and_returns_the_original_payload() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        let backing = RedbSpillBacking::create(&path).expect("open");

        let failure = SpillTarget::accept(&backing, b"only-copy".to_vec())
            .expect_err("generic write cannot represent an ambiguous durable commit");
        assert!(matches!(failure.error(), MemoryError::Unsupported(_)));
        assert_eq!(failure.into_parts().1, b"only-copy");
        assert_eq!(backing.retained_count(), Ok(0));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reopened_redb_reconciles_controller_count_from_persisted_inventory() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        {
            let controller =
                RoutedSpillController::new(RedbSpillBacking::create(&path).expect("open 1"));
            let first = spill_controller(&controller, b"first").expect("first");
            let second = spill_controller(&controller, b"second").expect("second");
            assert_eq!(first, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });
            assert_eq!(second, SpillReceipt::Durable { sequence: 1, inventory_after: 2 });
            assert_eq!(controller.spilled_count(), Ok(2));
        }

        let reopened = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("open 2"));
        assert_eq!(reopened.spilled_count(), Ok(2));
        assert_eq!(restore_controller(&reopened).expect("restore first"), Some(b"first".to_vec()));
        assert_eq!(reopened.spilled_count(), Ok(1));
        assert_eq!(
            restore_controller(&reopened).expect("restore second"),
            Some(b"second".to_vec())
        );
        assert_eq!(reopened.spilled_count(), Ok(0));
        drop(reopened);
        let _ = std::fs::remove_file(&path);
    }

    // Concurrency: the FIFO key is derived INSIDE each write txn (redb serializes
    // writers), so concurrent accepts assign distinct keys with no loss and no
    // duplicate. Drains every handle and asserts the exact multiset round-trips.
    #[test]
    fn concurrent_accepts_lose_no_handle_and_assign_distinct_keys() {
        use std::collections::BTreeSet;
        use std::sync::Arc;

        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        let backing = Arc::new(RedbSpillBacking::create(&path).expect("open"));

        let threads = 8u32;
        let per_thread = 25u32;
        let mut handles = Vec::new();
        for t in 0..threads {
            let backing = Arc::clone(&backing);
            handles.push(std::thread::spawn(move || {
                for i in 0..per_thread {
                    let payload = format!("t{t}-{i}").into_bytes();
                    let _receipt = spill_backing(&backing, &payload).expect("concurrent accept");
                }
            }));
        }
        for h in handles {
            h.join().expect("thread panicked");
        }

        let expected = (threads * per_thread) as u64;
        assert_eq!(backing.retained_count().expect("len"), expected, "no accept was lost");

        // Drain everything; every handle is distinct (no key collision lost a write).
        let mut drained = BTreeSet::new();
        while let Some(h) = restore_backing(&backing).expect("restore") {
            assert!(drained.insert(h), "a handle was restored twice (key collision)");
        }
        assert_eq!(drained.len() as u64, expected, "every accepted handle restored exactly once");
        assert_eq!(backing.retained_count().expect("len"), 0, "fully drained");
        let _ = std::fs::remove_file(&path);
    }

    // Exhausted keyspace: with u64::MAX already present, allocating a new stable
    // record ID must fail before any payload can be accepted.
    #[test]
    fn accept_at_exhausted_keyspace_fails_closed_without_overwriting() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        let backing = RedbSpillBacking::create(&path).expect("open");
        backing.seed_key_for_test(u64::MAX, b"last").expect("seed max key");

        let err = backing
            .reserve_record_id()
            .expect_err("reservation at an exhausted keyspace must be rejected");
        assert!(matches!(err, MemoryError::Capacity(_)), "got {err:?}");

        // The retained handle at u64::MAX was NOT overwritten or lost.
        assert_eq!(backing.retained_count().expect("len"), 1, "the seeded handle survives");
        assert_eq!(restore_backing(&backing).expect("restore"), Some(b"last".to_vec()));
        let _ = std::fs::remove_file(&path);
    }
}
