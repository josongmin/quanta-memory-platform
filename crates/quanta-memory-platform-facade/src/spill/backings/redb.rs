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

use crate::spill::SpillTarget;

/// The single FIFO queue table: monotonic sequence -> opaque handle bytes.
const SPILL_QUEUE: TableDefinition<'static, u64, &[u8]> = TableDefinition::new("qmp_spill_queue");

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

    fn accept(&self, handle: Vec<u8>) -> MemoryResult<()> {
        let txn = self
            .database
            .begin_write()
            .map_err(|e| MemoryError::report(format!("redb spill write txn failed: {e}")))?;
        {
            let mut table = txn
                .open_table(SPILL_QUEUE)
                .map_err(|e| MemoryError::report(format!("redb spill table open failed: {e}")))?;
            // Derive the next FIFO key INSIDE the write transaction (redb serializes
            // writers), so sequence assignment + insert are atomic: concurrent
            // accepts stay strictly FIFO and a failed/rolled-back txn burns no key
            // (the table itself is the single source of truth, also restart-safe).
            let last_key = table
                .last()
                .map_err(|e| MemoryError::report(format!("redb spill scan failed: {e}")))?
                .map(|(k, _)| k.value());
            // `checked_add` is the single guard: an exhausted keyspace (last key ==
            // u64::MAX) would otherwise reuse the key and OVERWRITE a retained
            // handle. Reject it fail-closed (astronomically unreachable) rather than
            // losing a handle.
            let next = match last_key {
                Some(k) => k.checked_add(1).ok_or_else(|| {
                    MemoryError::capacity("redb spill queue key space exhausted (u64::MAX)")
                })?,
                None => 0,
            };
            table
                .insert(next, handle.as_slice())
                .map_err(|e| MemoryError::report(format!("redb spill insert failed: {e}")))?;
        }
        txn.commit().map_err(|e| MemoryError::report(format!("redb spill commit failed: {e}")))?;
        Ok(())
    }

    fn restore(&self) -> MemoryResult<Option<Vec<u8>>> {
        let txn = self
            .database
            .begin_write()
            .map_err(|e| MemoryError::report(format!("redb spill write txn failed: {e}")))?;
        let popped = {
            let mut table = txn
                .open_table(SPILL_QUEUE)
                .map_err(|e| MemoryError::report(format!("redb spill table open failed: {e}")))?;
            let entry = table
                .pop_first()
                .map_err(|e| MemoryError::report(format!("redb spill pop failed: {e}")))?;
            entry.map(|(_seq, value)| value.value().to_vec())
        };
        txn.commit().map_err(|e| MemoryError::report(format!("redb spill commit failed: {e}")))?;
        Ok(popped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_db_path() -> std::path::PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("qmp-redb-backing-test-{n}.redb"))
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
        SpillTarget::accept(&backing, b"first".to_vec()).expect("accept 1");
        SpillTarget::accept(&backing, b"second".to_vec()).expect("accept 2");
        assert_eq!(backing.retained_count().expect("len"), 2);

        assert_eq!(SpillTarget::restore(&backing).expect("r1"), Some(b"first".to_vec()));
        assert_eq!(SpillTarget::restore(&backing).expect("r2"), Some(b"second".to_vec()));
        assert_eq!(SpillTarget::restore(&backing).expect("drained"), None);
        assert_eq!(backing.retained_count().expect("len"), 0);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reopened_backing_resumes_sequence_and_restores_persisted_handles() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        {
            let backing = RedbSpillBacking::create(&path).expect("open 1");
            SpillTarget::accept(&backing, b"persisted".to_vec()).expect("accept");
        }
        // Reopen: the persisted handle survives and the sequence does not collide.
        let backing = RedbSpillBacking::create(&path).expect("open 2");
        SpillTarget::accept(&backing, b"after-restart".to_vec()).expect("accept again");
        assert_eq!(SpillTarget::restore(&backing).expect("r1"), Some(b"persisted".to_vec()));
        assert_eq!(SpillTarget::restore(&backing).expect("r2"), Some(b"after-restart".to_vec()));
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
                    SpillTarget::accept(&*backing, payload).expect("concurrent accept");
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
        while let Some(h) = SpillTarget::restore(&*backing).expect("restore") {
            assert!(drained.insert(h), "a handle was restored twice (key collision)");
        }
        assert_eq!(drained.len() as u64, expected, "every accepted handle restored exactly once");
        assert_eq!(backing.retained_count().expect("len"), 0, "fully drained");
        let _ = std::fs::remove_file(&path);
    }

    // Exhausted keyspace: with u64::MAX already present, the next accept would
    // compute key u64::MAX again and OVERWRITE the retained handle. The guard
    // rejects it fail-closed (typed Capacity) and leaves the retained handle intact.
    #[test]
    fn accept_at_exhausted_keyspace_fails_closed_without_overwriting() {
        let path = temp_db_path();
        let _ = std::fs::remove_file(&path);
        let backing = RedbSpillBacking::create(&path).expect("open");
        backing.seed_key_for_test(u64::MAX, b"last").expect("seed max key");

        let err = SpillTarget::accept(&backing, b"overflow".to_vec())
            .expect_err("accept at an exhausted keyspace must be rejected");
        assert!(matches!(err, MemoryError::Capacity(_)), "got {err:?}");

        // The retained handle at u64::MAX was NOT overwritten or lost.
        assert_eq!(backing.retained_count().expect("len"), 1, "the seeded handle survives");
        assert_eq!(SpillTarget::restore(&backing).expect("restore"), Some(b"last".to_vec()));
        let _ = std::fs::remove_file(&path);
    }
}
