//! Opt-in Linux ENOSPC oracle. Run only with an isolated, disposable tmpfs:
//! `QMP_ENOSPC_ISOLATED_TMPFS=1 ... spill_redb_enospc_volume /mount/db.redb`.
//! The ordinary workspace rail does not run this example.

#[cfg(feature = "spill-redb")]
mod scenario {
    use std::fs::File;
    use std::io::{self, Write};
    use std::path::Path;

    use quanta_memory_platform_core::MemoryError;
    use quanta_memory_platform_facade::spill::service::{
        DurableRestoreTarget, ReconciledDurableSpillTarget,
    };
    use quanta_memory_platform_facade::{
        DurableRecordResolution, DurableWriteFailure, RedbSpillBacking, RedbSpillLimits,
    };

    const RECORDS: usize = 24;
    const RECORD_BYTES: usize = 512 * 1024;
    const MAX_FILL_BYTES: usize = 16 * 1024 * 1024;

    fn restore_exact(backing: &RedbSpillBacking, expected: &[u8]) {
        let candidate = backing.prepare_restore().expect("prepare after reopen").expect("record");
        assert_eq!(candidate.bytes(), expected, "record must be whole, not torn");
        backing.commit_restore(&candidate.claim).expect("stage exact record");
        backing.finalize_restore(&candidate.claim).expect("finalize exact record");
    }

    pub fn run(path: &Path) {
        assert_eq!(std::env::var("QMP_ENOSPC_ISOLATED_TMPFS").as_deref(), Ok("1"));
        assert_eq!(std::env::consts::OS, "linux", "this oracle requires Linux ENOSPC");
        assert!(!path.exists(), "use a fresh database path on the disposable tmpfs");
        let filler_path = path.with_extension("fill");
        assert!(!filler_path.exists(), "use a fresh filler path");

        let limits = RedbSpillLimits::default();
        let backing = RedbSpillBacking::create_with_limits(path, limits).expect("open on tmpfs");
        let anchor = backing.reserve_record_id().expect("reserve anchor");
        let _receipt = backing.commit_spill(anchor, b"anchor").expect("commit anchor");
        // Reserve before filling the volume so the fault is exercised by the
        // payload write, not by the record-id allocator transaction.
        let ids = (0..RECORDS)
            .map(|_| backing.reserve_record_id().expect("reserve candidate"))
            .collect::<Vec<_>>();

        let mut filler = File::create(&filler_path).expect("create filler on isolated tmpfs");
        let chunk = [0x3Du8; 64 * 1024];
        let mut filled = 0usize;
        loop {
            assert!(filled < MAX_FILL_BYTES, "isolated volume did not fill within safety cap");
            match filler.write(&chunk) {
                Ok(0) => panic!("filler returned zero bytes without ENOSPC"),
                Ok(n) => filled += n,
                Err(error) if error.kind() == io::ErrorKind::StorageFull => break,
                Err(error) => panic!("filler failed without ENOSPC: {error}"),
            }
        }
        println!("VOLUME_ENOSPC filler_bytes={filled}");

        let payload = vec![0xA5; RECORD_BYTES];
        let mut committed = 0usize;
        let mut denied = None;
        for id in ids {
            match backing.commit_spill(id, &payload) {
                Ok(_) => committed += 1,
                Err(DurableWriteFailure::NotCommitted(error))
                    if matches!(*error, MemoryError::Capacity(_)) =>
                {
                    denied = Some(id);
                    break;
                }
                Err(DurableWriteFailure::RecoveryRequired { record_id, commit_error, .. })
                    if matches!(*commit_error, MemoryError::Capacity(_)) =>
                {
                    assert_eq!(record_id, id);
                    denied = Some(id);
                    break;
                }
                Err(other) => panic!("real ENOSPC was not classified as capacity: {other:?}"),
            }
        }
        let denied = denied.expect("real ENOSPC must deny a payload write");
        drop(filler);
        std::fs::remove_file(&filler_path).expect("release filler blocks");
        drop(backing);

        let reopened = RedbSpillBacking::create_with_limits(path, limits).expect("cold reopen");
        let denied_present = match reopened.resolve_record(denied).expect("exact denied postimage")
        {
            DurableRecordResolution::Present { .. } => true,
            DurableRecordResolution::Absent => false,
            other => panic!("denied record has impossible postimage {other:?}"),
        };
        restore_exact(&reopened, b"anchor");
        for _ in 0..committed + usize::from(denied_present) {
            restore_exact(&reopened, &payload);
        }
        assert!(reopened.prepare_restore().expect("final read").is_none());
        assert_eq!(reopened.retained_count(), Ok(0));
        println!("ENOSPC_RECOVERED committed={committed} denied_present={denied_present}");
    }
}

#[cfg(feature = "spill-redb")]
fn main() {
    let path = std::env::args().nth(1).expect("database path argument required");
    scenario::run(std::path::Path::new(&path));
}

#[cfg(not(feature = "spill-redb"))]
fn main() {
    panic!("spill-redb feature is required");
}
