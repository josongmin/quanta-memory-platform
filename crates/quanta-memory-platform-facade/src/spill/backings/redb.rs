//! Durable spill backing over `redb` (opt-in `spill-redb` feature).
//!
//! This existing opt-in implementation is outside the current product plan:
//! it is not production-qualified or supported as a current product capability.
//! A consuming host must separately qualify its filesystem and volume/quota
//! policy, required durability level, and any caller-owned codec scratch.
//!
//! `RedbSpillBacking` is a durable [`SpillTarget`] for the spill slice: it stores
//! spilled handles (opaque `Vec<u8>`) in a `redb` table and restores them FIFO.
//! It is reached ONLY through the `SpillTarget` abstraction — the raw `redb`
//! database handle and `redb`'s error/types never appear in the facade's public
//! surface (boundary rule: no backing-specific nouns leak out of the slice).

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicU64;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;

use quanta_memory_platform_core::{MemoryError, MemoryResult};
use redb::backends::FileBackend;
use redb::{Database, ReadableTable, ReadableTableMetadata, StorageBackend, TableDefinition};

use crate::spill::service::ReconciledDurableSpillTarget;
use crate::spill::RestoreReadLease;
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
/// Sum of the payload lengths in the queue and recovery slot. Updated in the
/// same transaction as a new payload or finalization; legacy files seed it by
/// scanning the two payload tables once at open.
const RETAINED_PAYLOAD_BYTES: &str = "retained_payload_bytes";
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
/// Persisted 128-bit logical-lineage identity of this backend (FUP-002), stored
/// as two halves. Generated once, stable across renames and reopens, never
/// derived from pathnames or memory addresses. Consistent byte copies preserve
/// the lineage; they are backups/replacements, not independent writable
/// backends. The identity fences accidental cross-lineage claim presentation;
/// it is not cryptographic authentication against hostile file modification.
const BACKEND_ID_LO: &str = "backend_id_lo";
const BACKEND_ID_HI: &str = "backend_id_hi";
/// Legacy watermark (schema 1): greatest sequence believed consumed. It cannot
/// distinguish a reserved hole from a consumed record, so schema 2 files never
/// consult it for disposition; the value is preserved untouched for forensic
/// readability and ignored by all resolution paths. Retained so legacy-format
/// fixtures and forensic readers can name the key.
#[allow(dead_code, reason = "schema key remains reserved for durable format compatibility")]
const FINALIZED_THROUGH: &str = "finalized_through";
/// Schema version of the durable spill format. Absent means schema 1
/// (watermark-only history, possibly ambiguous); 2 means every record carries
/// an exact [`RecordDisposition`] row.
const SCHEMA_VERSION: &str = "schema_version";
const SCHEMA_VERSION_V2: u64 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PersistedSchema {
    LegacyV1,
    V2,
}

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

fn redb_storage_error(context: &str, error: impl Into<redb::Error>) -> MemoryError {
    let error = error.into();
    match &error {
        redb::Error::Io(io_error) if io_error.kind() == io::ErrorKind::StorageFull => {
            MemoryError::capacity(format!("{context}: {error}"))
        }
        redb::Error::PreviousIo => MemoryError::OperationInProgress(format!(
            "{context}: {error}; close and reopen the Redb backing before retrying"
        )),
        _ => MemoryError::report(format!("{context}: {error}")),
    }
}

/// Logical payload, retained-record, Redb file-length and cache limits. The
/// file-length cap does not reserve volume space or bound filesystem metadata,
/// snapshots, other files, or private allocations inside caller codecs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RedbSpillLimits {
    /// Maximum encoded bytes in one retained record. Does not bound the
    /// caller's encoder or decoder's private scratch allocations.
    pub max_record_bytes: u64,
    /// Maximum number of queued plus recovery records.
    pub max_retained_records: u64,
    /// Maximum sum of queued plus recovery payload lengths. Redb page,
    /// tombstone, copy-on-write and filesystem overhead are NOT included.
    pub max_retained_payload_bytes: u64,
    /// Maximum length of this Redb database file, including Redb's pages and
    /// copy-on-write overhead. A full volume may fail earlier with ENOSPC.
    pub max_database_file_bytes: u64,
    /// Redb's own configured cache size, in bytes.
    pub cache_bytes: usize,
    /// Sum of backing-owned restore candidate and transactional copy buffers.
    /// Caller clones and codec scratch are outside this budget.
    pub max_inflight_restore_bytes: u64,
    /// Maximum backing-owned restore buffers, including 0-byte candidates and
    /// transactional copies.
    pub max_inflight_restore_buffers: u64,
}

impl Default for RedbSpillLimits {
    fn default() -> Self {
        Self {
            max_record_bytes: 16 * 1024 * 1024,
            max_retained_records: 100_000,
            max_retained_payload_bytes: 1024 * 1024 * 1024,
            max_database_file_bytes: 4 * 1024 * 1024 * 1024,
            cache_bytes: 32 * 1024 * 1024,
            max_inflight_restore_bytes: 64 * 1024 * 1024,
            max_inflight_restore_buffers: 64,
        }
    }
}

impl RedbSpillLimits {
    fn validate(self) -> MemoryResult<Self> {
        if self.max_retained_records == 0
            || self.max_database_file_bytes == 0
            || self.max_inflight_restore_buffers < 2
            || self.max_record_bytes > self.max_retained_payload_bytes
            || self
                .max_record_bytes
                .checked_mul(2)
                .is_none_or(|minimum| self.max_inflight_restore_bytes < minimum)
        {
            return Err(MemoryError::config(
                "redb spill limits require positive record and database-file limits, at least two in-flight buffers, max_record_bytes <= max_retained_payload_bytes, and an in-flight byte budget for a candidate plus transactional copy",
            ));
        }
        Ok(self)
    }

    fn check_record(self, len: usize) -> MemoryResult<u64> {
        let bytes = u64::try_from(len)
            .map_err(|_| MemoryError::capacity("redb spill record length exceeds u64"))?;
        if bytes > self.max_record_bytes {
            return Err(MemoryError::capacity(format!(
                "redb spill record requires {bytes} bytes, above configured {}-byte record limit",
                self.max_record_bytes
            )));
        }
        Ok(bytes)
    }
}

/// Delegate locking and durable I/O to Redb's native file backend while
/// rejecting every operation that could extend this database past its cap.
/// This is a per-file bound, not an allocation guarantee for the volume.
#[derive(Debug)]
struct CappedFileBackend {
    inner: FileBackend,
    max_bytes: u64,
}

impl CappedFileBackend {
    fn new(file: File, max_bytes: u64) -> MemoryResult<Self> {
        let inner = FileBackend::new(file)
            .map_err(|error| MemoryError::build(format!("redb file lock failed: {error}")))?;
        let existing = inner.len().map_err(|error| {
            MemoryError::build(format!("redb file length read failed: {error}"))
        })?;
        if existing > max_bytes {
            return Err(MemoryError::capacity(format!(
                "existing redb database file has {existing} bytes, above configured {max_bytes}-byte limit"
            )));
        }
        Ok(Self { inner, max_bytes })
    }

    fn check_end(&self, end: u64) -> io::Result<()> {
        if end > self.max_bytes {
            Err(io::Error::new(
                io::ErrorKind::StorageFull,
                format!("redb database file would exceed configured {}-byte limit", self.max_bytes),
            ))
        } else {
            Ok(())
        }
    }
}

impl StorageBackend for CappedFileBackend {
    fn len(&self) -> io::Result<u64> {
        let len = self.inner.len()?;
        self.check_end(len)?;
        Ok(len)
    }

    fn read(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let len_u64 =
            u64::try_from(len).map_err(|_| io::Error::other("redb read length exceeds u64"))?;
        let end = offset
            .checked_add(len_u64)
            .ok_or_else(|| io::Error::other("redb read end exceeds u64"))?;
        self.check_end(end)?;
        self.inner.read(offset, len)
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        self.check_end(len)?;
        self.inner.set_len(len)
    }

    fn sync_data(&self, eventual: bool) -> io::Result<()> {
        self.inner.sync_data(eventual)
    }

    fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        let len = u64::try_from(data.len())
            .map_err(|_| io::Error::other("redb write length exceeds u64"))?;
        let end = offset
            .checked_add(len)
            .ok_or_else(|| io::Error::other("redb write end exceeds u64"))?;
        self.check_end(end)?;
        self.inner.write(offset, data)
    }
}

/// An opt-in durable spill backing over `redb`. Implements
/// [`SpillTarget<Vec<u8>>`]; the `redb` database is fully encapsulated.
/// This backing is not production-qualified or supported as a current product
/// capability; see the module-level qualification boundaries.
#[derive(Debug)]
pub struct RedbSpillBacking {
    database: Database,
    limits: RedbSpillLimits,
    restore_budget_used: Arc<AtomicU64>,
    restore_buffers_used: Arc<AtomicU64>,
    /// Cached copy of the persisted [`BACKEND_ID_LO`]/[`BACKEND_ID_HI`]
    /// identity, loaded once at open. Claim validation compares the presented
    /// backend against this value without extra reads.
    backend_id: u128,
    #[cfg(test)]
    faults: RedbFaults,
}

/// Test-only finalize-kill L1 hooks. This module exists only
/// under the non-default `durability-test-hooks` feature, which is crash-rail
/// instrumentation — never a production callback, environment kill switch, or
/// always-present global. Default builds compile `finalize_restore` with zero
/// added code: both call sites are `#[cfg]`-gated statements that vanish.
///
/// Only the dedicated `spill_redb_finalize_kill_child` installs a hook, and it
/// installs exactly one. Any other process linked with this feature (notably
/// the parent test, which reopens and drains in-process) observes a cheap
/// lock-and-`None`-check no-op when nothing is installed.
#[cfg(feature = "durability-test-hooks")]
#[doc(hidden)]
pub mod finalize_kill_hooks {
    use std::sync::Mutex;

    /// The two externally visible sides of the finalize linearization point.
    /// No hook exists before individual table mutations: those writes share
    /// one uncommitted transaction and one externally visible state.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum FinalizeKillPoint {
        /// Staged in the write transaction, `txn.commit()` not yet called.
        BeforeCommit,
        /// `txn.commit()` returned `Ok(())`; no ACK handling, probe, or caller
        /// return has run yet.
        AfterCommit,
    }

    impl FinalizeKillPoint {
        /// Handshake token the child emits over the control pipe.
        pub fn token(self) -> &'static str {
            match self {
                Self::BeforeCommit => "FinalizeBeforeCommit",
                Self::AfterCommit => "FinalizeAfterCommit",
            }
        }
    }

    /// Installed hook: the armed point plus `fn(point, sequence, generation)`.
    /// Function pointers (not closures) keep registration `Sync` without
    /// allocation or lifetime plumbing.
    pub type FinalizeKillHook = fn(FinalizeKillPoint, u64, u64);

    static INSTALLED: Mutex<Option<(FinalizeKillPoint, FinalizeKillHook)>> = Mutex::new(None);

    /// Arm exactly one hook. A second install is a test-tooling bug and panics
    /// rather than silently replacing the armed point.
    pub fn install(point: FinalizeKillPoint, hook: FinalizeKillHook) {
        let mut installed = INSTALLED.lock().expect("finalize kill hook mutex");
        assert!(installed.is_none(), "a finalize kill hook is already installed");
        *installed = Some((point, hook));
    }

    /// Fire the armed hook when its point is reached. No-op when nothing is
    /// installed or a different point is armed. The hook runs after the mutex
    /// is released so a panicking hook cannot poison registration.
    pub(crate) fn fire(point: FinalizeKillPoint, sequence: u64, generation: u64) {
        let armed = INSTALLED.lock().expect("finalize kill hook mutex");
        let hook = match *armed {
            Some((armed_point, hook)) if armed_point == point => Some(hook),
            _ => None,
        };
        drop(armed);
        if let Some(hook) = hook {
            hook(point, sequence, generation);
        }
    }
}

/// One-shot, opt-in fault boundary for the Linux charged restore ENOSPC rail.
/// The callback fills an isolated test volume only after the active claim has
/// been checked and before its queue-to-recovery transaction writes. It is
/// absent from production feature graphs.
#[cfg(feature = "durability-test-hooks")]
#[doc(hidden)]
pub mod restore_commit_volume_hook {
    use std::sync::Mutex;

    type Hook = fn(u64, u64);
    static INSTALLED: Mutex<Option<Hook>> = Mutex::new(None);

    pub fn install(hook: Hook) {
        let mut installed = INSTALLED.lock().expect("restore volume hook mutex");
        assert!(installed.is_none(), "a restore volume hook is already installed");
        *installed = Some(hook);
    }

    pub(crate) fn fire(sequence: u64, generation: u64) {
        let hook = INSTALLED.lock().expect("restore volume hook mutex").take();
        if let Some(hook) = hook {
            hook(sequence, generation);
        }
    }
}

/// Best-effort 128-bit backend identity minted once per logical database
/// lineage.
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
    lose_abort_commit_ack: AtomicBool,
    claim_after_lost_ack: AtomicBool,
}

mod backing;

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

mod reconcile;
mod restore;

#[cfg(test)]
mod tests;
