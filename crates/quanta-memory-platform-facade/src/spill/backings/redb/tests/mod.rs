mod cases_01;
mod cases_02;
mod cases_03;

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

#[derive(Debug)]
struct PartialWriteBackend {
    inner: CappedFileBackend,
    armed: Arc<AtomicBool>,
    observed: Arc<AtomicBool>,
    attempted_bytes: Arc<AtomicU64>,
    written_bytes: Arc<AtomicU64>,
}

impl StorageBackend for PartialWriteBackend {
    fn len(&self) -> io::Result<u64> {
        self.inner.len()
    }

    fn read(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        self.inner.read(offset, len)
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        self.inner.set_len(len)
    }

    fn sync_data(&self, eventual: bool) -> io::Result<()> {
        self.inner.sync_data(eventual)
    }

    fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        if data.len() > 1 && self.armed.load(AtomicOrdering::SeqCst) {
            let prefix_len = data.len() / 2;
            let before = self.inner.read(offset, prefix_len)?;
            if before != data[..prefix_len]
                && self
                    .armed
                    .compare_exchange(true, false, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst)
                    .is_ok()
            {
                self.inner.write(offset, &data[..prefix_len])?;
                let after = self.inner.read(offset, prefix_len)?;
                if after != data[..prefix_len] || after == before {
                    return Err(io::Error::other(
                        "partial-write injector could not verify changed prefix",
                    ));
                }
                self.attempted_bytes
                    .store(u64::try_from(data.len()).unwrap_or(u64::MAX), AtomicOrdering::SeqCst);
                self.written_bytes
                    .store(u64::try_from(prefix_len).unwrap_or(u64::MAX), AtomicOrdering::SeqCst);
                self.observed.store(true, AtomicOrdering::SeqCst);
                return Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected partial Redb file write",
                ));
            }
        }
        self.inner.write(offset, data)
    }
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

fn assert_no_v2_initialization(database: &Database) {
    let read = database.begin_read().expect("read rejected fixture");
    assert!(matches!(read.open_table(SPILL_QUEUE), Err(redb::TableError::TableDoesNotExist(_))));
    assert!(matches!(read.open_table(SPILL_RECOVERY), Err(redb::TableError::TableDoesNotExist(_))));
    assert!(matches!(
        read.open_table(SPILL_RECORD_STATE),
        Err(redb::TableError::TableDoesNotExist(_))
    ));
    assert!(matches!(
        read.open_table(SPILL_FINALIZED_BY),
        Err(redb::TableError::TableDoesNotExist(_))
    ));
    let meta = read.open_table(SPILL_META).expect("fixture metadata");
    for key in [
        NEXT_SEQUENCE,
        RETAINED_PAYLOAD_BYTES,
        ACTIVE_RESTORE_SEQUENCE,
        ACTIVE_RESTORE_GENERATION,
        RESTORE_CLAIM_GENERATION,
    ] {
        assert!(meta.get(key).expect("read unseeded metadata").is_none(), "unexpected {key}");
    }
}

fn open_with_partial_write_fault(
    path: &Path,
    limits: RedbSpillLimits,
    armed: Arc<AtomicBool>,
    observed: Arc<AtomicBool>,
    attempted_bytes: Arc<AtomicU64>,
    written_bytes: Arc<AtomicU64>,
) -> RedbSpillBacking {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .expect("open fault-injection file");
    let inner = CappedFileBackend::new(file, limits.max_database_file_bytes)
        .expect("capped fault-injection backend");
    let mut builder = redb::Builder::new();
    builder.set_cache_size(limits.cache_bytes);
    let database = builder
        .create_with_backend(PartialWriteBackend {
            inner,
            armed,
            observed,
            attempted_bytes,
            written_bytes,
        })
        .expect("open fault-injection database");
    let schema = RedbSpillBacking::inspect_schema(&database).expect("inspect fault fixture");
    let backend_id = RedbSpillBacking::inspect_backend_identity(&database, schema)
        .expect("inspect fault identity");
    RedbSpillBacking::initialize(database, limits, schema, backend_id)
        .expect("initialize fault-injection backing")
}

fn restore_backing(backing: &RedbSpillBacking) -> MemoryResult<Option<Vec<u8>>> {
    let Some(candidate) = backing.prepare_restore()? else {
        return Ok(None);
    };
    let bytes = candidate.bytes().to_vec();
    backing.commit_restore(&candidate.claim).map_err(|failure| match failure {
        DurableCommitFailure::Retained(error) | DurableCommitFailure::Ambiguous(error) => *error,
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
    let bytes = candidate.bytes().to_vec();
    controller.commit_restore(&candidate.claim).map_err(|failure| match failure {
        DurableCommitFailure::Retained(error) | DurableCommitFailure::Ambiguous(error) => *error,
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

// Concurrency: the FIFO key is derived INSIDE each write txn (redb serializes
// writers), so concurrent accepts assign distinct keys with no loss and no
// duplicate. Drains every handle and asserts the exact multiset round-trips.

// --- FUP-001: exact durable record disposition ---------------------------
//
// The sequence allocator and the record lifecycle are separate authorities.
// A reserved-but-never-committed id is a hole (`Absent`); only an
// explicitly consumed record is `Finalized`. Watermark estimation is
// forbidden as a disposition oracle.

fn tiny_limits() -> RedbSpillLimits {
    RedbSpillLimits {
        max_record_bytes: 4,
        max_retained_records: 2,
        max_retained_payload_bytes: 6,
        max_database_file_bytes: 64 * 1024 * 1024,
        cache_bytes: 4 * 1024 * 1024,
        max_inflight_restore_bytes: 8,
        max_inflight_restore_buffers: 2,
    }
}

// --- FUP-002: single-owner recovery -----------------------------------
//
// Record identity (stable numeric sequence) and recovery attempt authority
// (backend identity + monotonic generation) are separate. Every permission
// change validates the presented claim; exactly one claimant publishes.

// Self-audit P2-1: a landed finalize is terminal. Retrying it with the
// same claim succeeds exactly even while an unrelated claim is live, and
// the retry disturbs neither the live claim nor the remaining record.

// Exhausted keyspace: with u64::MAX already present, allocating a new stable
// record ID must fail before any payload can be accepted.
