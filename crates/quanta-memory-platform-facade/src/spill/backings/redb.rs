//! Durable spill backing over `redb` (opt-in `spill-redb` feature).
//!
//! `RedbSpillBacking` is a durable [`SpillTarget`] for the spill slice: it stores
//! spilled handles (opaque `Vec<u8>`) in a `redb` table and restores them FIFO.
//! It is reached ONLY through the `SpillTarget` abstraction — the raw `redb`
//! database handle and `redb`'s error/types never appear in the facade's public
//! surface (boundary rule: no backing-specific nouns leak out of the slice).

use std::path::Path;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use quanta_memory_platform_core::{MemoryError, MemoryResult};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};

use crate::spill::service::ReconciledDurableSpillTarget;
use crate::spill::{
    DurableCommitFailure, DurableRecordId, DurableRecordResolution, DurableRestoreCandidate,
    DurableRestoreClaim, DurableRestorePhase, DurableRestoreRecovery, DurableRestoreTarget,
    DurableSpillTarget, DurableWriteFailure, SpillFailure, SpillInventory, SpillReceipt,
    SpillTarget,
};

/// The single FIFO queue table: monotonic sequence -> opaque handle bytes.
const SPILL_QUEUE: TableDefinition<'static, u64, &[u8]> = TableDefinition::new("qmp_spill_queue");
/// Payloads removed from the ordinary queue but not yet acknowledged by the
/// resident destination. This closes the delete-to-publication crash window.
const SPILL_RECOVERY: TableDefinition<'static, u64, &[u8]> =
    TableDefinition::new("qmp_spill_recovery");
/// Exact per-record disposition (FUP-001). The sequence allocator and the
/// record lifecycle are separate authorities: a reserved-but-never-committed
/// id is a hole (`Absent`), never evidence of a consumed record. Every payload
/// or phase change updates this row in the SAME write transaction, so the
/// disposition can never disagree with the persisted bytes. `max(sequence)`
/// estimation is forbidden as a disposition oracle.
const SPILL_RECORD_STATE: TableDefinition<'static, u64, u8> =
    TableDefinition::new("qmp_spill_record_state");
/// Persistent allocator state. Queue emptiness must never reset sequence
/// identity, otherwise a stale restore token can delete a later record (ABA).
const SPILL_META: TableDefinition<'static, &str, u64> = TableDefinition::new("qmp_spill_meta");
const NEXT_SEQUENCE: &str = "next_sequence";
const ACTIVE_RESTORE_SEQUENCE: &str = "active_restore_sequence";
/// Generation of the current active restore claim (FUP-002). Present exactly
/// when [`ACTIVE_RESTORE_SEQUENCE`] is present on files written by claim-aware
/// code; a legacy active claim without a generation matches no live claim.
const ACTIVE_RESTORE_GENERATION: &str = "active_restore_generation";
/// Monotonic counter minting restore-claim generations. Every `prepare` and
/// every recovery issues the next generation in the same transaction that
/// publishes it, so each generation has exactly one owner.
const RESTORE_CLAIM_GENERATION: &str = "restore_claim_generation";
/// Per-record finalize proof (FUP-002): sequence -> the generation whose
/// finalize consumed it. A single "last finalized" slot would go stale as soon
/// as another record finalizes, misclassifying a legitimate same-claimant
/// retry as superseded; the proof must be per record. Written in the same
/// transaction as the `Finalized` disposition, so the two always agree. One
/// `u64` row per consumed record, proportional to the disposition rows that
/// already persist per record; never garbage-collected without a replay
/// horizon (same rule as FUP-001 tombstones).
const SPILL_FINALIZED_BY: TableDefinition<'static, u64, u64> =
    TableDefinition::new("qmp_spill_finalized_by");
/// Persisted 128-bit identity of this backend file (FUP-002), stored as two
/// halves. Generated once, stable across renames and reopens, never derived
/// from pathnames or memory addresses. A claim presented to a file that did
/// not issue it is foreign and rejected.
const BACKEND_ID_LO: &str = "backend_id_lo";
const BACKEND_ID_HI: &str = "backend_id_hi";
/// Legacy watermark (schema 1): greatest sequence believed consumed. It cannot
/// distinguish a reserved hole from a consumed record, so schema 2 files never
/// consult it for disposition; the value is preserved untouched for forensic
/// readability and ignored by all resolution paths. Retained so legacy-format
/// fixtures and forensic readers can name the key.
#[allow(dead_code)]
const FINALIZED_THROUGH: &str = "finalized_through";
/// Schema version of the durable spill format. Absent means schema 1
/// (watermark-only history, possibly ambiguous); 2 means every record carries
/// an exact [`RecordDisposition`] row.
const SCHEMA_VERSION: &str = "schema_version";
const SCHEMA_VERSION_V2: u64 = 2;

/// Exact persisted disposition of one durable record id (schema 2 authority).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum RecordDisposition {
    /// The id was allocated but no payload ever committed. Resolves `Absent`.
    Reserved = 0,
    /// Payload lives in the ordinary FIFO queue. Resolves `Present`.
    Queued = 1,
    /// Payload lives in the recovery slot under an active claim. Resolves
    /// `Present` on the write path (the bytes still exist).
    Recovery = 2,
    /// The record existed and was durably consumed. Resolves `Finalized`; the
    /// id can never be recreated.
    Finalized = 3,
}

impl RecordDisposition {
    fn decode(raw: u8) -> MemoryResult<Self> {
        match raw {
            0 => Ok(Self::Reserved),
            1 => Ok(Self::Queued),
            2 => Ok(Self::Recovery),
            3 => Ok(Self::Finalized),
            other => Err(MemoryError::InvalidState(format!(
                "durable record disposition {other} is not a known schema-2 state"
            ))),
        }
    }
}

/// Fail-closed error for an id whose history predates exact dispositions: a
/// legacy watermark-only file cannot tell a reserved hole from a consumed
/// record, so neither `Finalized` nor `Absent` may be guessed.
fn unresolved_record_error(record_id: u64) -> MemoryError {
    MemoryError::InvalidState(format!(
        "unresolved durable record {record_id}: legacy watermark-only history cannot \
         distinguish a reserved hole from a consumed record; refusing to guess"
    ))
}

/// A durable spill backing over `redb`. Implements [`SpillTarget<Vec<u8>>`]; the
/// `redb` database is fully encapsulated.
#[derive(Debug)]
pub struct RedbSpillBacking {
    database: Database,
    /// Cached copy of the persisted [`BACKEND_ID_LO`]/[`BACKEND_ID_HI`]
    /// identity, loaded once at open. Claim validation compares the presented
    /// backend against this value without extra reads.
    backend_id: u128,
    #[cfg(test)]
    faults: RedbFaults,
}

/// Best-effort 128-bit backend identity minted once per database file.
///
/// Entropy mixes per-call random hash keys with the process id, wall-clock
/// time, and a process counter, then persists the result in the file itself —
/// so identity is stable across renames/reopens and never derived from a
/// pathname or a memory address. Single-owner correctness WITHIN one backend
/// does not depend on cross-file uniqueness (the per-backend persisted claim
/// generation is the authority there); the identity only has to make
/// accidental cross-file claim acceptance implausible while deliberate
/// cross-file presentation is always rejected by comparison.
fn generate_backend_id() -> u128 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let mut first = RandomState::new().build_hasher();
    first.write_u32(pid);
    first.write(&nanos.to_le_bytes());
    first.write_u64(counter);
    first.write_u8(0);
    let mut second = RandomState::new().build_hasher();
    second.write_u32(pid);
    second.write(&nanos.to_le_bytes());
    second.write_u64(counter);
    second.write_u8(1);
    u128::from(second.finish()) << 64 | u128::from(first.finish())
}

#[cfg(test)]
#[derive(Debug, Default)]
struct RedbFaults {
    fail_write_before_transaction: AtomicBool,
    lose_write_commit_ack: AtomicBool,
    lose_restore_commit_ack: AtomicBool,
    lose_finalize_commit_ack: AtomicBool,
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
        let backend_id = {
            let queue = txn.open_table(SPILL_QUEUE).map_err(|e| {
                MemoryError::build(format!("redb spill queue initialization failed: {e}"))
            })?;
            let recovery = txn.open_table(SPILL_RECOVERY).map_err(|e| {
                MemoryError::build(format!("redb spill recovery initialization failed: {e}"))
            })?;
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::build(format!("redb spill metadata initialization failed: {e}"))
            })?;
            // Ensure every table exists so later read transactions never meet
            // a missing table: the disposition table (schema 2) and the
            // finalize-proof table (single-owner claims) are created here and
            // populated by migration or by later writes.
            let mut states = txn.open_table(SPILL_RECORD_STATE).map_err(|e| {
                MemoryError::build(format!("redb record disposition initialization failed: {e}"))
            })?;
            let _finalized_by = txn.open_table(SPILL_FINALIZED_BY).map_err(|e| {
                MemoryError::build(format!("redb finalize proof initialization failed: {e}"))
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
            // Backend identity (FUP-002): mint once, persist in the same
            // transaction, then cache. Legacy files without an identity adopt
            // one here; it is stable from this open onward.
            if meta
                .get(BACKEND_ID_LO)
                .map_err(|e| MemoryError::build(format!("redb backend identity read failed: {e}")))?
                .is_none()
            {
                let minted = generate_backend_id();
                meta.insert(BACKEND_ID_LO, minted as u64).map_err(|e| {
                    MemoryError::build(format!("redb backend identity seed failed: {e}"))
                })?;
                meta.insert(BACKEND_ID_HI, (minted >> 64) as u64).map_err(|e| {
                    MemoryError::build(format!("redb backend identity seed failed: {e}"))
                })?;
            }
            // Schema-2 migration: convert only explicit payload evidence
            // (queue/recovery rows) into exact disposition rows, in the same
            // transaction that stamps the schema version. Ids below the next
            // sequence with no payload evidence are genuinely ambiguous
            // (reserved hole vs. consumed record) and are deliberately left
            // without a disposition row, so every later resolution fails closed
            // with an explicit unresolved error instead of guessing from the
            // legacy watermark. The legacy watermark itself is preserved
            // untouched and never consulted again.
            if meta
                .get(SCHEMA_VERSION)
                .map_err(|e| MemoryError::build(format!("redb schema version read failed: {e}")))?
                .is_none()
            {
                for entry in queue.iter().map_err(|e| {
                    MemoryError::build(format!("redb spill migration scan failed: {e}"))
                })? {
                    let (key, _) = entry.map_err(|e| {
                        MemoryError::build(format!("redb spill migration scan failed: {e}"))
                    })?;
                    states.insert(key.value(), RecordDisposition::Queued as u8).map_err(|e| {
                        MemoryError::build(format!("redb record disposition migration failed: {e}"))
                    })?;
                }
                for entry in recovery.iter().map_err(|e| {
                    MemoryError::build(format!("redb recovery migration scan failed: {e}"))
                })? {
                    let (key, _) = entry.map_err(|e| {
                        MemoryError::build(format!("redb recovery migration scan failed: {e}"))
                    })?;
                    states.insert(key.value(), RecordDisposition::Recovery as u8).map_err(|e| {
                        MemoryError::build(format!("redb record disposition migration failed: {e}"))
                    })?;
                }
                meta.insert(SCHEMA_VERSION, SCHEMA_VERSION_V2).map_err(|e| {
                    MemoryError::build(format!("redb schema version migration failed: {e}"))
                })?;
            }
            let lo = meta
                .get(BACKEND_ID_LO)
                .map_err(|e| MemoryError::build(format!("redb backend identity read failed: {e}")))?
                .map(|value| value.value())
                .ok_or_else(|| {
                    MemoryError::InvalidState(
                        "redb backend identity is missing after seeding".into(),
                    )
                })?;
            let hi = meta
                .get(BACKEND_ID_HI)
                .map_err(|e| MemoryError::build(format!("redb backend identity read failed: {e}")))?
                .map(|value| value.value())
                .ok_or_else(|| {
                    MemoryError::InvalidState(
                        "redb backend identity is missing after seeding".into(),
                    )
                })?;
            u128::from(hi) << 64 | u128::from(lo)
        };
        txn.commit()
            .map_err(|e| MemoryError::build(format!("redb spill metadata commit failed: {e}")))?;
        Ok(Self {
            database,
            backend_id,
            #[cfg(test)]
            faults: RedbFaults::default(),
        })
    }

    #[cfg(test)]
    fn inject_fail_write_before_transaction(&self) {
        self.faults.fail_write_before_transaction.store(true, AtomicOrdering::SeqCst);
    }

    #[cfg(test)]
    fn inject_lost_write_commit_ack(&self) {
        self.faults.lose_write_commit_ack.store(true, AtomicOrdering::SeqCst);
    }

    /// Test-only restore-commit ACK-loss injection, shared with in-crate store
    /// tests so the ambiguous-commit fence path runs against real claims.
    #[cfg(test)]
    pub(crate) fn inject_lost_restore_commit_ack(&self) {
        self.faults.lose_restore_commit_ack.store(true, AtomicOrdering::SeqCst);
    }

    /// Test-only finalize ACK-loss injection, shared with in-crate store tests
    /// so the full fence→reconcile→publish path runs against real claims.
    #[cfg(test)]
    pub(crate) fn inject_lost_finalize_commit_ack(&self) {
        self.faults.lose_finalize_commit_ack.store(true, AtomicOrdering::SeqCst);
    }

    /// Reject a claim minted for a different backend file. Numeric sequences
    /// collide across files, so the backend identity is always compared first.
    fn check_claim_backend(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
        if claim.backend() != self.backend_id {
            return Err(MemoryError::InvalidState(format!(
                "durable restore claim targets backend {:032x} but this backend is \
                 {:032x}; numeric sequences do not transfer across files",
                claim.backend(),
                self.backend_id
            )));
        }
        Ok(())
    }

    /// Non-mutating probe of the persisted active claimant: the sequence plus
    /// the generation (a legacy claim predates generations and reports
    /// `None`, which matches no live claim). Used only to classify ambiguous
    /// commit acknowledgements — never as an authority grant.
    fn active_claim(&self) -> MemoryResult<Option<(u64, Option<u64>)>> {
        let read = self.database.begin_read().map_err(|e| {
            MemoryError::report(format!("redb active claim probe transaction failed: {e}"))
        })?;
        let meta = read
            .open_table(SPILL_META)
            .map_err(|e| MemoryError::report(format!("redb spill metadata table failed: {e}")))?;
        let sequence = meta
            .get(ACTIVE_RESTORE_SEQUENCE)
            .map_err(|e| MemoryError::report(format!("redb active restore read failed: {e}")))?
            .map(|value| value.value());
        let generation = meta
            .get(ACTIVE_RESTORE_GENERATION)
            .map_err(|e| MemoryError::report(format!("redb active claim read failed: {e}")))?
            .map(|value| value.value());
        match (sequence, generation) {
            (Some(sequence), generation) => Ok(Some((sequence, generation))),
            (None, None) => Ok(None),
            (None, Some(_)) => Err(MemoryError::InvalidState(
                "durable restore generation without an active sequence".into(),
            )),
        }
    }

    /// Number of handles currently retained (restorable). For diagnostics/tests.
    pub fn retained_count(&self) -> MemoryResult<u64> {
        let read = self
            .database
            .begin_read()
            .map_err(|e| MemoryError::report(format!("redb spill read txn failed: {e}")))?;
        match read.open_table(SPILL_QUEUE) {
            Ok(table) => {
                let recovery = read.open_table(SPILL_RECOVERY).map_err(|e| {
                    MemoryError::report(format!("redb spill recovery table open failed: {e}"))
                })?;
                let queue_len = table
                    .len()
                    .map_err(|e| MemoryError::report(format!("redb spill len failed: {e}")))?;
                let recovery_len = recovery
                    .len()
                    .map_err(|e| MemoryError::report(format!("redb recovery len failed: {e}")))?;
                queue_len
                    .checked_add(recovery_len)
                    .ok_or_else(|| MemoryError::capacity("redb spill inventory exceeds u64"))
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
            let mut states = txn.open_table(SPILL_RECORD_STATE).map_err(|e| {
                MemoryError::report(format!("redb record disposition table failed: {e}"))
            })?;
            states.insert(key, RecordDisposition::Queued as u8).map_err(|e| {
                MemoryError::report(format!("redb record disposition seed failed: {e}"))
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
            // The allocator and the disposition share one transaction: an
            // allocated id always carries an exact `Reserved` row, so a hole
            // can never be mistaken for a consumed record.
            let mut states = txn.open_table(SPILL_RECORD_STATE).map_err(|e| {
                MemoryError::report(format!("redb record disposition table failed: {e}"))
            })?;
            states.insert(next, RecordDisposition::Reserved as u8).map_err(|e| {
                MemoryError::report(format!("redb record disposition write failed: {e}"))
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
        #[cfg(test)]
        if self.faults.fail_write_before_transaction.swap(false, AtomicOrdering::SeqCst) {
            return Err(not_committed(MemoryError::report(
                "injected redb failure before write transaction",
            )));
        }
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
                    let existing_bytes = table
                        .get(record_id.get())
                        .map_err(|e| {
                            not_committed(MemoryError::report(format!(
                                "redb spill idempotency read failed: {e}"
                            )))
                        })?
                        .map(|existing| existing.value().to_vec())
                        .ok_or_else(|| {
                            not_committed(MemoryError::InvalidState(format!(
                                "durable record {} is queued but its payload is missing",
                                record_id.get()
                            )))
                        })?;
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
                }
                RecordDisposition::Reserved => {
                    table.insert(record_id.get(), bytes).map_err(|e| {
                        not_committed(MemoryError::report(format!("redb spill insert failed: {e}")))
                    })?;
                    states.insert(record_id.get(), RecordDisposition::Queued as u8).map_err(
                        |e| {
                            not_committed(MemoryError::report(format!(
                                "redb record disposition write failed: {e}"
                            )))
                        },
                    )?;
                    table.len().map_err(|e| {
                        not_committed(MemoryError::report(format!(
                            "redb spill post-insert inventory failed: {e}"
                        )))
                    })?
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
            Err(error) => Some(MemoryError::report(format!(
                "redb spill commit acknowledgement failed: {error}"
            ))),
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
            DurableRestoreCandidate {
                sequence: sequence.value(),
                bytes: bytes.value().to_vec(),
                claim: DurableRestoreClaim::issued(self.backend_id, generation, sequence.value()),
            }
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
                .map(|bytes| bytes.value().to_vec())
                .ok_or_else(|| {
                    DurableCommitFailure::retained(MemoryError::InvalidState(
                        "durable restore candidate vanished before commit".into(),
                    ))
                })?;
            recovery.insert(sequence, removed.as_slice()).map_err(|e| {
                DurableCommitFailure::retained(MemoryError::report(format!(
                    "redb recovery payload write failed: {e}"
                )))
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
            Err(error) => Err(DurableCommitFailure::ambiguous(MemoryError::report(format!(
                "redb spill restore commit outcome is ambiguous: {error}"
            )))),
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
            let recovery_present = recovery
                .get(sequence)
                .map_err(|e| MemoryError::report(format!("redb recovery read failed: {e}")))?
                .is_some();
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
                    recovery.remove(sequence).map_err(|e| {
                        MemoryError::report(format!("redb recovery payload clear failed: {e}"))
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
        let commit_error = match txn.commit() {
            Ok(()) => {
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
                // Non-mutating probe only: a claiming recovery here would mint
                // a spurious generation and steal this finalizer's own claim.
                match self.active_claim()? {
                    None => Ok(()),
                    Some(_) => Err(MemoryError::report(format!(
                        "redb restore-finalize commit outcome remains unresolved: {error}"
                    ))),
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
        match txn.commit() {
            Ok(()) => Ok(()),
            Err(error) => match self.active_claim()? {
                None => Ok(()),
                Some((active, Some(active_gen)))
                    if active == sequence && active_gen == generation =>
                {
                    Err(MemoryError::report(format!(
                        "redb restore-abort commit outcome remains unresolved: {error}"
                    )))
                }
                Some((active, active_gen)) => Err(MemoryError::InvalidState(format!(
                    "restore-abort {sequence} observed foreign active claim ({active}, {active_gen:?})"
                ))),
            },
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
            let (bytes, phase) =
                if let Some(payload) = recovery_table.get(sequence).map_err(|e| {
                    MemoryError::report(format!("redb recovery payload read failed: {e}"))
                })? {
                    (payload.value().to_vec(), DurableRestorePhase::Committed)
                } else if let Some(payload) = queue.get(sequence).map_err(|e| {
                    MemoryError::report(format!("redb spill payload read failed: {e}"))
                })? {
                    (payload.value().to_vec(), DurableRestorePhase::Prepared)
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
                candidate: DurableRestoreCandidate {
                    sequence,
                    bytes,
                    claim: DurableRestoreClaim::issued(self.backend_id, generation, sequence),
                },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spill::{
        DurableRestoreController, ReconciledDurableSpillController, RoutedSpillController,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    struct TestDb {
        directory: std::path::PathBuf,
        path: std::path::PathBuf,
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn test_db() -> TestDb {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let process = std::process::id();
        let directory = std::env::temp_dir().join(format!("qmp-redb-backing-test-{process}-{n}"));
        std::fs::create_dir(&directory).expect("create isolated redb test directory");
        let path = directory.join("spill.redb");
        TestDb { directory, path }
    }

    fn restore_backing(backing: &RedbSpillBacking) -> MemoryResult<Option<Vec<u8>>> {
        let Some(candidate) = backing.prepare_restore()? else {
            return Ok(None);
        };
        let bytes = candidate.bytes;
        backing.commit_restore(&candidate.claim).map_err(|failure| match failure {
            DurableCommitFailure::Retained(error) | DurableCommitFailure::Ambiguous(error) => {
                *error
            }
        })?;
        backing.finalize_restore(&candidate.claim)?;
        Ok(Some(bytes))
    }

    fn restore_controller<C: DurableRestoreController>(
        controller: &C,
    ) -> MemoryResult<Option<Vec<u8>>> {
        let Some(candidate) = controller.prepare_restore()? else {
            return Ok(None);
        };
        let bytes = candidate.bytes;
        controller.commit_restore(&candidate.claim).map_err(|failure| match failure {
            DurableCommitFailure::Retained(error) | DurableCommitFailure::Ambiguous(error) => {
                *error
            }
        })?;
        controller.finalize_restore(&candidate.claim)?;
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
    fn live_backing_excludes_second_open_and_drop_releases_persisted_file() {
        let db = test_db();
        let first = RedbSpillBacking::create(&db.path).expect("open first");
        let _receipt = spill_backing(&first, b"persisted").expect("seed");

        let err =
            RedbSpillBacking::create(&db.path).expect_err("second live open must be excluded");
        assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");

        drop(first);
        let reopened = RedbSpillBacking::create(&db.path).expect("open after drop");
        assert_eq!(
            restore_backing(&reopened).expect("restore persisted"),
            Some(b"persisted".to_vec())
        );
    }

    #[test]
    fn durable_spill_target_round_trips_fifo_through_the_trait() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");

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
    }

    #[test]
    fn reopened_backing_resumes_sequence_and_restores_persisted_handles() {
        let db = test_db();
        {
            let backing = RedbSpillBacking::create(&db.path).expect("open 1");
            let _receipt = spill_backing(&backing, b"persisted").expect("accept");
        }
        // Reopen: the persisted handle survives and the sequence does not collide.
        let backing = RedbSpillBacking::create(&db.path).expect("open 2");
        let receipt = spill_backing(&backing, b"after-restart").expect("accept again");
        assert_eq!(receipt, SpillReceipt::Durable { sequence: 1, inventory_after: 2 });
        assert_eq!(restore_backing(&backing).expect("r1"), Some(b"persisted".to_vec()));
        assert_eq!(restore_backing(&backing).expect("r2"), Some(b"after-restart".to_vec()));
    }

    #[test]
    fn drained_queue_never_reuses_sequence_or_accepts_stale_restore_token() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");

        let first = spill_backing(&backing, b"first").expect("accept first");
        assert_eq!(first, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });
        let stale = backing.prepare_restore().expect("prepare").expect("candidate");
        backing.commit_restore(&stale.claim).expect("drain first");
        backing.finalize_restore(&stale.claim).expect("acknowledge first restore");

        let replacement = spill_backing(&backing, b"replacement").expect("accept after full drain");
        assert_eq!(replacement, SpillReceipt::Durable { sequence: 1, inventory_after: 1 });
        assert!(matches!(
            backing.commit_restore(&stale.claim),
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
    }

    #[test]
    fn persisted_restore_claim_blocks_duplicate_prepare_and_recovers_after_reopen() {
        let db = test_db();
        {
            let backing = RedbSpillBacking::create(&db.path).expect("open 1");
            let _receipt = spill_backing(&backing, b"claimed").expect("seed");
            let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
            assert_eq!(candidate.sequence, 0);
            assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
        }

        let backing = RedbSpillBacking::create(&db.path).expect("open 2");
        assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
        assert!(matches!(backing.prepare_restore(), Err(MemoryError::OperationInProgress(_))));
        // After a restart the new owner claims the persisted restore before
        // reconciling; the recovered claim authorizes the abort-and-retain.
        let recovery =
            backing.recover_active_restore().expect("claim persisted restore").expect("recovery");
        assert_eq!(recovery.phase, DurableRestorePhase::Prepared);
        assert!(matches!(
            backing
                .reconcile_restore(&recovery.candidate.claim)
                .expect("retained claim reconciliation"),
            DurableRecordResolution::Present { inventory_after: 1 }
        ));
        assert_eq!(backing.active_restore(), Ok(None));
        assert_eq!(backing.prepare_restore().expect("prepare after recovery").unwrap().sequence, 0);
    }

    #[test]
    fn committed_restore_payload_survives_reopen_until_resident_ack() {
        let db = test_db();
        {
            let backing = RedbSpillBacking::create(&db.path).expect("open 1");
            let _receipt = spill_backing(&backing, b"crash-safe").expect("seed");
            let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
            backing.commit_restore(&candidate.claim).expect("move to recovery");
            assert_eq!(backing.retained_count(), Ok(1));
            assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
        }

        let backing = RedbSpillBacking::create(&db.path).expect("open 2");
        let recovery =
            backing.recover_active_restore().expect("read recovery").expect("persisted recovery");
        assert_eq!(recovery.phase, DurableRestorePhase::Committed);
        assert_eq!(recovery.candidate.sequence, 0);
        assert_eq!(recovery.candidate.bytes, b"crash-safe");
        assert!(matches!(backing.prepare_restore(), Err(MemoryError::OperationInProgress(_))));

        backing
            .finalize_restore(&recovery.candidate.claim)
            .expect("resident publication acknowledgement");
        assert_eq!(backing.active_restore(), Ok(None));
        assert_eq!(backing.retained_count(), Ok(0));
        assert_eq!(
            backing.resolve_record(DurableRecordId::new(0)),
            Ok(DurableRecordResolution::Finalized)
        );
    }

    #[test]
    fn injected_write_crash_points_converge_without_duplicate_records() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");

        let first = backing.reserve_record_id().expect("reserve first");
        backing.inject_fail_write_before_transaction();
        assert!(matches!(
            backing.commit_spill(first, b"first"),
            Err(DurableWriteFailure::NotCommitted(_))
        ));
        assert_eq!(backing.resolve_record(first), Ok(DurableRecordResolution::Absent));

        let second = backing.reserve_record_id().expect("reserve second");
        backing.inject_lost_write_commit_ack();
        let receipt = backing
            .commit_spill(second, b"second")
            .expect("post-commit probe converts lost ack to committed");
        assert_eq!(receipt, SpillReceipt::Durable { sequence: 1, inventory_after: 1 });
        let retry = backing
            .commit_spill(second, b"second")
            .expect("same record and payload are idempotent");
        assert_eq!(retry, receipt);
        assert_eq!(backing.retained_count(), Ok(1));
    }

    #[test]
    fn injected_restore_and_finalize_ack_loss_preserve_recovery_authority() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");
        let _receipt = spill_backing(&backing, b"recoverable").expect("seed");
        let candidate = backing.prepare_restore().expect("prepare").expect("candidate");

        backing.inject_lost_restore_commit_ack();
        assert!(matches!(
            backing.commit_restore(&candidate.claim),
            Err(DurableCommitFailure::Ambiguous(_))
        ));
        // The live claimant reconciles with its OWN claim: recovery bytes plus
        // the still-live claim prove the commit landed. (`recover` would mint
        // a new generation and steal this claimant's authority, so it is not
        // a same-owner probe.)
        assert_eq!(
            backing.reconcile_restore(&candidate.claim),
            Ok(DurableRecordResolution::Finalized)
        );

        backing.inject_lost_finalize_commit_ack();
        backing
            .finalize_restore(&candidate.claim)
            .expect("post-finalize probe converts lost ack to success");
        assert_eq!(backing.active_restore(), Ok(None));
        assert_eq!(backing.retained_count(), Ok(0));
    }

    #[test]
    fn generic_restore_is_non_destructive_and_requires_owned_two_phase_path() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");
        let _receipt = spill_backing(&backing, b"only-copy").expect("accept");

        let error = SpillTarget::restore(&backing)
            .expect_err("generic destructive restore must be unavailable for durable records");
        assert!(matches!(error, MemoryError::Unsupported(_)));
        assert_eq!(backing.retained_count(), Ok(1));
        assert_eq!(
            backing.prepare_restore().expect("prepare").expect("candidate").bytes,
            b"only-copy"
        );
    }

    #[test]
    fn generic_write_is_non_mutating_and_returns_the_original_payload() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");

        let failure = SpillTarget::accept(&backing, b"only-copy".to_vec())
            .expect_err("generic write cannot represent an ambiguous durable commit");
        assert!(matches!(failure.error(), MemoryError::Unsupported(_)));
        assert_eq!(failure.into_parts().1, b"only-copy");
        assert_eq!(backing.retained_count(), Ok(0));
    }

    #[test]
    fn reopened_redb_reconciles_controller_count_from_persisted_inventory() {
        let db = test_db();
        {
            let controller =
                RoutedSpillController::new(RedbSpillBacking::create(&db.path).expect("open 1"));
            let first = spill_controller(&controller, b"first").expect("first");
            let second = spill_controller(&controller, b"second").expect("second");
            assert_eq!(first, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });
            assert_eq!(second, SpillReceipt::Durable { sequence: 1, inventory_after: 2 });
            assert_eq!(controller.spilled_count(), Ok(2));
        }

        let reopened =
            RoutedSpillController::new(RedbSpillBacking::create(&db.path).expect("open 2"));
        assert_eq!(reopened.spilled_count(), Ok(2));
        assert_eq!(restore_controller(&reopened).expect("restore first"), Some(b"first".to_vec()));
        assert_eq!(reopened.spilled_count(), Ok(1));
        assert_eq!(
            restore_controller(&reopened).expect("restore second"),
            Some(b"second".to_vec())
        );
        assert_eq!(reopened.spilled_count(), Ok(0));
    }

    // Concurrency: the FIFO key is derived INSIDE each write txn (redb serializes
    // writers), so concurrent accepts assign distinct keys with no loss and no
    // duplicate. Drains every handle and asserts the exact multiset round-trips.
    #[test]
    fn concurrent_accepts_lose_no_handle_and_assign_distinct_keys() {
        use std::collections::{BTreeMap, BTreeSet};
        use std::sync::{Arc, Barrier};

        const WRITERS: usize = 4;
        const WRITES_PER_WRITER: usize = 2;

        let db = test_db();
        let backing = Arc::new(RedbSpillBacking::create(&db.path).expect("open"));
        let start = Arc::new(Barrier::new(WRITERS + 1));
        let expected = WRITERS.checked_mul(WRITES_PER_WRITER).expect("bounded test record count");
        let expected_u64 = u64::try_from(expected).expect("record count fits u64");

        let mut handles = Vec::new();
        for writer in 0..WRITERS {
            let backing = Arc::clone(&backing);
            let start = Arc::clone(&start);
            handles.push(std::thread::spawn(move || -> Vec<(Vec<u8>, SpillReceipt)> {
                let mut accepted = Vec::with_capacity(WRITES_PER_WRITER);
                start.wait();
                for record in 0..WRITES_PER_WRITER {
                    let payload = format!("writer-{writer}-record-{record}").into_bytes();
                    let receipt = spill_backing(&backing, &payload).expect("concurrent accept");
                    accepted.push((payload, receipt));
                }
                accepted
            }));
        }
        start.wait();

        let mut accepted = Vec::with_capacity(expected);
        for handle in handles {
            accepted.extend(handle.join().expect("writer thread panicked"));
        }
        assert_eq!(accepted.len(), expected, "every writer returned every acceptance receipt");

        let mut accepted_payloads = BTreeMap::new();
        let mut sequences = BTreeSet::new();
        for (payload, receipt) in accepted {
            *accepted_payloads.entry(payload).or_insert(0usize) += 1;
            let SpillReceipt::Durable { sequence, .. } = receipt else {
                panic!("redb backing must report durable acceptance: {receipt:?}");
            };
            assert!(sequences.insert(sequence), "duplicate durable sequence {sequence}");
        }
        let expected_sequences = (0..expected_u64).collect::<BTreeSet<_>>();
        assert_eq!(sequences, expected_sequences, "durable sequences are contiguous from zero");
        assert_eq!(
            backing.retained_count().expect("len"),
            expected_u64,
            "no accepted record was lost"
        );

        let mut restored_payloads = BTreeMap::new();
        while let Some(payload) = restore_backing(&backing).expect("restore") {
            *restored_payloads.entry(payload).or_insert(0usize) += 1;
        }
        assert_eq!(restored_payloads, accepted_payloads, "exact payload multiset round-trips");
        assert_eq!(backing.retained_count().expect("len"), 0, "fully drained");
    }

    // --- FUP-001: exact durable record disposition ---------------------------
    //
    // The sequence allocator and the record lifecycle are separate authorities.
    // A reserved-but-never-committed id is a hole (`Absent`); only an
    // explicitly consumed record is `Finalized`. Watermark estimation is
    // forbidden as a disposition oracle.

    #[test]
    fn sep23_fup001_reserved_hole_is_not_finalized_by_later_record() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");
        let hole = backing.reserve_record_id().expect("reserve hole id");
        assert_eq!(hole.get(), 0);
        let second = backing.reserve_record_id().expect("reserve second id");
        assert_eq!(second.get(), 1);
        assert_eq!(backing.resolve_record(hole), Ok(DurableRecordResolution::Absent));

        let receipt = backing.commit_spill(second, b"second").expect("commit id 1");
        assert_eq!(receipt, SpillReceipt::Durable { sequence: 1, inventory_after: 1 });
        assert_eq!(backing.retained_count().expect("one payload retained"), 1);

        // Restore and consume ONLY id 1; the queue/recovery bytes prove which
        // record actually existed.
        let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
        assert_eq!(candidate.sequence, 1);
        assert_eq!(candidate.bytes, b"second");
        backing.commit_restore(&candidate.claim).expect("commit restore");
        assert!(
            matches!(backing.resolve_record(second), Ok(DurableRecordResolution::Present { .. })),
            "a recovery-phase record still exists and is not consumed"
        );
        backing.finalize_restore(&candidate.claim).expect("finalize");
        assert_eq!(backing.retained_count().expect("drained"), 0);
        assert_eq!(backing.resolve_record(second), Ok(DurableRecordResolution::Finalized));
        // The hole was never durable: it must NOT be misread as consumed.
        assert_eq!(backing.resolve_record(hole), Ok(DurableRecordResolution::Absent));
        drop(backing);

        let reopened = RedbSpillBacking::create(&db.path).expect("reopen");
        assert_eq!(
            reopened.resolve_record(hole),
            Ok(DurableRecordResolution::Absent),
            "hole disposition survives reopen"
        );
        assert_eq!(
            reopened.resolve_record(second),
            Ok(DurableRecordResolution::Finalized),
            "consumed disposition survives reopen"
        );
        assert_eq!(reopened.retained_count().expect("still drained"), 0);
    }

    #[test]
    fn sep23_fup001_reserve_only_reopen_is_not_consumed() {
        let db = test_db();
        let reserved = {
            let backing = RedbSpillBacking::create(&db.path).expect("open 1");
            backing.reserve_record_id().expect("reserve only")
        };
        let backing = RedbSpillBacking::create(&db.path).expect("open 2");
        // An empty queue after a bare reserve is a hole, not a consumed record.
        assert_eq!(backing.retained_count().expect("empty"), 0);
        assert_eq!(backing.resolve_record(reserved), Ok(DurableRecordResolution::Absent));
        // The reservation itself survived: the same id still commits exactly once.
        let receipt = backing.commit_spill(reserved, b"late").expect("late commit");
        assert_eq!(receipt, SpillReceipt::Durable { sequence: reserved.get(), inventory_after: 1 });
        assert!(matches!(
            backing.resolve_record(reserved),
            Ok(DurableRecordResolution::Present { .. })
        ));
        assert_eq!(restore_backing(&backing).expect("restore"), Some(b"late".to_vec()));
        assert_eq!(backing.resolve_record(reserved), Ok(DurableRecordResolution::Finalized));
    }

    #[test]
    fn sep23_fup001_out_of_order_commit_finalized_reinsert_rejected() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");
        let first = backing.reserve_record_id().expect("reserve 0");
        let second = backing.reserve_record_id().expect("reserve 1");
        // Commit in reverse allocation order: disposition follows the payload,
        // not the id order.
        let _receipt = backing.commit_spill(second, b"bbb").expect("commit 1 first");
        let _receipt = backing.commit_spill(first, b"aaa").expect("commit 0 second");
        assert!(matches!(
            backing.resolve_record(first),
            Ok(DurableRecordResolution::Present { .. })
        ));
        assert!(matches!(
            backing.resolve_record(second),
            Ok(DurableRecordResolution::Present { .. })
        ));

        // FIFO restores id 0 first; finalizing it must not affect id 1.
        let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
        assert_eq!(candidate.sequence, 0);
        backing.commit_restore(&candidate.claim).expect("commit restore 0");
        backing.finalize_restore(&candidate.claim).expect("finalize 0");
        assert_eq!(backing.resolve_record(first), Ok(DurableRecordResolution::Finalized));
        assert!(matches!(
            backing.resolve_record(second),
            Ok(DurableRecordResolution::Present { .. })
        ));

        // A truly finalized id can never be recreated, with same or new bytes.
        assert!(matches!(
            backing.commit_spill(first, b"aaa"),
            Err(DurableWriteFailure::NotCommitted(_))
        ));
        assert!(matches!(
            backing.commit_spill(first, b"other"),
            Err(DurableWriteFailure::NotCommitted(_))
        ));
        // Conflicting bytes on a live id are rejected and preserve the payload.
        assert!(matches!(
            backing.commit_spill(second, b"different"),
            Err(DurableWriteFailure::NotCommitted(_))
        ));
        let survivor = backing.prepare_restore().expect("prepare").expect("candidate");
        assert_eq!(survivor.sequence, 1);
        assert_eq!(survivor.bytes, b"bbb", "rejected rewrite must preserve bytes");
        backing.commit_restore(&survivor.claim).expect("commit restore 1");
        backing.finalize_restore(&survivor.claim).expect("finalize 1");
        assert_eq!(backing.retained_count().expect("drained"), 0);
    }

    #[test]
    fn sep23_fup001_same_bytes_retry_keeps_inventory_stable() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");
        let id = backing.reserve_record_id().expect("reserve");
        let first = backing.commit_spill(id, b"payload").expect("commit");
        let retry = backing.commit_spill(id, b"payload").expect("idempotent retry");
        assert_eq!(retry, first, "same id + same bytes is a duplicate insert-free retry");
        assert_eq!(backing.retained_count().expect("one record"), 1);
        assert!(matches!(backing.resolve_record(id), Ok(DurableRecordResolution::Present { .. })));
    }

    #[test]
    fn sep23_fup001_legacy_watermark_file_migrates_payload_and_fails_closed() {
        let db = test_db();
        // Build a schema-1 (watermark-only) file by hand: queued payload 5,
        // recovery payload 3, allocator frontier 6, watermark claiming
        // "everything through 4 is consumed". No disposition rows exist.
        {
            let database = redb::Database::create(&db.path).expect("legacy fixture");
            let txn = database.begin_write().expect("legacy txn");
            {
                let mut queue = txn.open_table(SPILL_QUEUE).expect("legacy queue");
                queue.insert(5u64, b"five".as_slice()).expect("legacy queued row");
                let mut recovery = txn.open_table(SPILL_RECOVERY).expect("legacy recovery");
                recovery.insert(3u64, b"three".as_slice()).expect("legacy recovery row");
                let mut meta = txn.open_table(SPILL_META).expect("legacy meta");
                meta.insert(NEXT_SEQUENCE, 6u64).expect("legacy next");
                meta.insert(FINALIZED_THROUGH, 4u64).expect("legacy watermark");
            }
            txn.commit().expect("legacy commit");
        }
        let backing = RedbSpillBacking::create(&db.path).expect("open migrates");
        // Explicit payload evidence converts exactly; bytes survive migration.
        assert!(
            matches!(
                backing.resolve_record(DurableRecordId::new(5)),
                Ok(DurableRecordResolution::Present { .. })
            ),
            "migrated queued payload must resolve present"
        );
        assert!(
            matches!(
                backing.resolve_record(DurableRecordId::new(3)),
                Ok(DurableRecordResolution::Present { .. })
            ),
            "migrated recovery payload must resolve present"
        );
        assert_eq!(backing.retained_count().expect("migrated inventory"), 2);
        let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
        assert_eq!(candidate.sequence, 5);
        assert_eq!(candidate.bytes, b"five");
        backing.commit_restore(&candidate.claim).expect("commit migrated restore");
        backing.finalize_restore(&candidate.claim).expect("finalize migrated restore");
        assert_eq!(
            backing.resolve_record(DurableRecordId::new(5)),
            Ok(DurableRecordResolution::Finalized)
        );

        // Below-frontier ids with no payload evidence are genuinely ambiguous:
        // the legacy watermark claims 0..=4 were consumed, but a reserved hole
        // is indistinguishable from a consumed record. Fail closed — never
        // guess `Finalized` (wrong retire) or `Absent` (wrong resurrection).
        for ambiguous in [0u64, 1, 2, 4] {
            let probed = backing.resolve_record(DurableRecordId::new(ambiguous));
            assert!(
                matches!(&probed, Err(MemoryError::InvalidState(message))
                    if message.contains("unresolved durable record")),
                "legacy id {ambiguous} must fail closed as unresolved, got {probed:?}"
            );
            assert!(
                matches!(
                    backing.commit_spill(DurableRecordId::new(ambiguous), b"rewrite"),
                    Err(DurableWriteFailure::NotCommitted(_))
                ),
                "legacy id {ambiguous} must not be rewritable"
            );
        }
        // Never-issued ids stay exactly `Absent` (not unresolved).
        for fresh in [6u64, 100] {
            assert_eq!(
                backing.resolve_record(DurableRecordId::new(fresh)),
                Ok(DurableRecordResolution::Absent)
            );
        }
        // The allocator frontier and exact states continue after migration.
        let next = backing.reserve_record_id().expect("post-migration reserve");
        assert_eq!(next.get(), 6);
        let _receipt = backing.commit_spill(next, b"six").expect("post-migration commit");
        assert!(matches!(
            backing.resolve_record(next),
            Ok(DurableRecordResolution::Present { .. })
        ));
    }

    // --- FUP-002: single-owner recovery -----------------------------------
    //
    // Record identity (stable numeric sequence) and recovery attempt authority
    // (backend identity + monotonic generation) are separate. Every permission
    // change validates the presented claim; exactly one claimant publishes.

    #[test]
    fn sep23_fup002_stale_generation_rejected_after_claim_steal() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, b"owned").expect("commit");
        let first = backing.prepare_restore().expect("prepare").expect("candidate");
        assert_eq!(first.claim.generation(), 1);

        // A second recovery steals the live claim with a fresh generation.
        let stolen = backing.recover_active_restore().expect("recover").expect("recovery");
        assert_eq!(stolen.phase, DurableRestorePhase::Prepared);
        assert_eq!(stolen.candidate.sequence, 0);
        assert_eq!(stolen.candidate.bytes, b"owned");
        assert_eq!(stolen.candidate.claim.generation(), 2);
        assert_eq!(stolen.candidate.claim.backend(), first.claim.backend());

        // The stale generation can no longer move the record; the payload stays
        // authoritative and the rejection is `Retained`, not ambiguous.
        assert!(matches!(
            backing.commit_restore(&first.claim),
            Err(DurableCommitFailure::Retained(_))
        ));
        backing.commit_restore(&stolen.candidate.claim).expect("live claimant commits");
        assert!(
            matches!(backing.finalize_restore(&first.claim), Err(MemoryError::InvalidState(_))),
            "stale generation must not finalize"
        );
        backing.finalize_restore(&stolen.candidate.claim).expect("live claimant finalizes");
        assert_eq!(backing.resolve_record(id), Ok(DurableRecordResolution::Finalized));
        // Even after the record is gone, the OTHER claimant's finalize is
        // rejected: idempotent success requires the same generation's proof.
        assert!(matches!(
            backing.finalize_restore(&first.claim),
            Err(MemoryError::InvalidState(_))
        ));
        // ...while the winner's own retry is exactly idempotent.
        backing.finalize_restore(&stolen.candidate.claim).expect("same-claimant finalize retry");
    }

    #[test]
    fn sep23_fup002_finalize_proof_is_per_record_not_last_writer() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");
        for payload in [b"r0".as_slice(), b"r1".as_slice()] {
            let id = backing.reserve_record_id().expect("reserve");
            let _receipt = backing.commit_spill(id, payload).expect("commit");
        }
        let first = backing.prepare_restore().expect("prepare 0").expect("candidate 0");
        backing.commit_restore(&first.claim).expect("commit 0");
        backing.finalize_restore(&first.claim).expect("finalize 0");
        let second = backing.prepare_restore().expect("prepare 1").expect("candidate 1");
        backing.commit_restore(&second.claim).expect("commit 1");
        backing.finalize_restore(&second.claim).expect("finalize 1");

        // A later record's finalize must not invalidate the first claimant's
        // proof: same-generation retries stay exactly idempotent per record.
        backing.finalize_restore(&first.claim).expect("retry first finalize");
        backing.finalize_restore(&second.claim).expect("retry second finalize");
        assert_eq!(
            backing.resolve_record(DurableRecordId::new(0)),
            Ok(DurableRecordResolution::Finalized)
        );
        assert_eq!(
            backing.resolve_record(DurableRecordId::new(1)),
            Ok(DurableRecordResolution::Finalized)
        );
    }

    #[test]
    fn sep23_fup002_foreign_backend_claim_rejected_on_every_permission_change() {
        let db_a = test_db();
        let db_b = test_db();
        let backing_a = RedbSpillBacking::create(&db_a.path).expect("open a");
        let backing_b = RedbSpillBacking::create(&db_b.path).expect("open b");
        // Same numeric sequences in both files: sequence alone proves nothing.
        for backing in [&backing_a, &backing_b] {
            let id = backing.reserve_record_id().expect("reserve");
            assert_eq!(id.get(), 0);
            let _receipt = backing.commit_spill(id, b"payload").expect("commit");
        }
        let claim_a = backing_a.prepare_restore().expect("prepare a").expect("candidate a").claim;
        let claim_b = backing_b.prepare_restore().expect("prepare b").expect("candidate b").claim;
        assert_ne!(claim_a.backend(), claim_b.backend(), "distinct files, distinct identities");

        // Every authority-changing operation rejects the foreign claim before
        // touching the record; both records stay authoritative.
        assert!(matches!(
            backing_b.commit_restore(&claim_a),
            Err(DurableCommitFailure::Retained(_))
        ));
        assert!(matches!(backing_b.finalize_restore(&claim_a), Err(MemoryError::InvalidState(_))));
        assert!(matches!(backing_b.abort_restore(&claim_a), Err(MemoryError::InvalidState(_))));
        assert!(matches!(backing_b.reconcile_restore(&claim_a), Err(MemoryError::InvalidState(_))));
        assert!(matches!(
            backing_a.commit_restore(&claim_b),
            Err(DurableCommitFailure::Retained(_))
        ));
        for (backing, id) in [(&backing_a, 0u64), (&backing_b, 0u64)] {
            assert!(
                matches!(
                    backing.resolve_record(DurableRecordId::new(id)),
                    Ok(DurableRecordResolution::Present { .. })
                ),
                "foreign attempts must not move either record"
            );
        }
        // Each backend still serves its own claimant exactly once.
        backing_a.commit_restore(&claim_a).expect("commit a");
        backing_a.finalize_restore(&claim_a).expect("finalize a");
        backing_b.abort_restore(&claim_b).expect("abort b releases its own claim");
        assert_eq!(backing_b.active_restore(), Ok(None));
    }

    #[test]
    fn sep23_fup002_backend_identity_stable_across_reopen() {
        let db = test_db();
        let before = {
            let backing = RedbSpillBacking::create(&db.path).expect("open 1");
            let id = backing.reserve_record_id().expect("reserve");
            let _receipt = backing.commit_spill(id, b"stable").expect("commit");
            backing.prepare_restore().expect("prepare").expect("candidate").claim.backend()
        };
        let backing = RedbSpillBacking::create(&db.path).expect("open 2");
        let recovery = backing.recover_active_restore().expect("recover").expect("recovery");
        assert_eq!(recovery.candidate.claim.backend(), before, "identity survives reopen");
        assert_eq!(recovery.candidate.bytes, b"stable");
        backing.commit_restore(&recovery.candidate.claim).expect("recovered claim commits");
        backing.finalize_restore(&recovery.candidate.claim).expect("recovered claim finalizes");
    }

    // Self-audit P2-1: a landed finalize is terminal. Retrying it with the
    // same claim succeeds exactly even while an unrelated claim is live, and
    // the retry disturbs neither the live claim nor the remaining record.
    #[test]
    fn sep23_audit_finalize_retry_succeeds_while_unrelated_claim_is_live() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");
        for payload in [b"first".as_slice(), b"second".as_slice()] {
            let id = backing.reserve_record_id().expect("reserve");
            let _receipt = backing.commit_spill(id, payload).expect("commit");
        }
        let first = backing.prepare_restore().expect("prepare 0").expect("candidate 0");
        assert_eq!(first.sequence, 0);
        backing.commit_restore(&first.claim).expect("commit 0");
        backing.finalize_restore(&first.claim).expect("finalize 0 lands");
        // A new restore makes an unrelated claim live.
        let second = backing.prepare_restore().expect("prepare 1").expect("candidate 1");
        assert_eq!(second.sequence, 1);
        // Same-claimant ACK-loss retry of the landed finalize: idempotent Ok,
        // not an error the caller must reconcile away.
        backing.finalize_restore(&first.claim).expect("landed finalize retry succeeds");
        assert_eq!(
            backing.active_restore().expect("active"),
            Some(DurableRecordId::new(1)),
            "the retry leaves the live claim untouched"
        );
        assert_eq!(second.bytes, b"second", "the live candidate is intact");
        backing.abort_restore(&second.claim).expect("abort the live claim");
        backing
            .finalize_restore(&first.claim)
            .expect("landed proof still answers after the live claim aborts");
        assert_eq!(backing.retained_count().expect("count"), 1, "only record 1 remains");
        assert_eq!(
            backing.resolve_record(DurableRecordId::new(0)).expect("resolve"),
            DurableRecordResolution::Finalized,
            "record 0 stays exactly finalized"
        );
    }

    #[test]
    fn sep23_audit_finalize_proof_does_not_mask_reappeared_recovery_payload() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, b"payload").expect("commit");
        let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
        backing.commit_restore(&candidate.claim).expect("commit restore");
        backing.finalize_restore(&candidate.claim).expect("finalize");

        let txn = backing.database.begin_write().expect("write");
        {
            let mut recovery = txn.open_table(SPILL_RECOVERY).expect("recovery");
            recovery.insert(id.get(), b"corrupt".as_slice()).expect("inject inconsistency");
        }
        txn.commit().expect("commit inconsistency");

        assert!(
            matches!(backing.finalize_restore(&candidate.claim), Err(MemoryError::InvalidState(_))),
            "a finalize proof must not hide a recovery payload that contradicts terminal state"
        );
    }

    // Exhausted keyspace: with u64::MAX already present, allocating a new stable
    // record ID must fail before any payload can be accepted.
    #[test]
    fn accept_at_exhausted_keyspace_fails_closed_without_overwriting() {
        let db = test_db();
        let backing = RedbSpillBacking::create(&db.path).expect("open");
        backing.seed_key_for_test(u64::MAX, b"last").expect("seed max key");

        let err = backing
            .reserve_record_id()
            .expect_err("reservation at an exhausted keyspace must be rejected");
        assert!(matches!(err, MemoryError::Capacity(_)), "got {err:?}");

        // The retained handle at u64::MAX was NOT overwritten or lost.
        assert_eq!(backing.retained_count().expect("len"), 1, "the seeded handle survives");
        assert_eq!(restore_backing(&backing).expect("restore"), Some(b"last".to_vec()));
    }
}
