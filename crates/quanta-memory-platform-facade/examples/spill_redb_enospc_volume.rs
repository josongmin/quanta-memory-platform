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
        DurableCommitFailure, DurableRecordResolution, DurableRestorePhase, DurableWriteFailure,
        RedbSpillBacking, RedbSpillLimits,
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

    fn fill_volume(filler_path: &Path) -> File {
        let mut filler = File::create(filler_path).expect("create filler on isolated tmpfs");
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
        filler
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

        let filler = fill_volume(&filler_path);

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

    pub fn run_restore(path: &Path) {
        assert_eq!(std::env::var("QMP_ENOSPC_ISOLATED_TMPFS").as_deref(), Ok("1"));
        assert_eq!(std::env::consts::OS, "linux", "this oracle requires Linux ENOSPC");
        assert!(!path.exists(), "use a fresh database path on the disposable tmpfs");
        let filler_path = path.with_extension("fill");
        assert!(!filler_path.exists(), "use a fresh filler path");

        let limits = RedbSpillLimits::default();
        let payload = vec![0xA5; RECORD_BYTES];
        let backing = RedbSpillBacking::create_with_limits(path, limits).expect("open on tmpfs");
        let id = backing.reserve_record_id().expect("reserve restore source");
        let _receipt = backing.commit_spill(id, &payload).expect("commit restore source");
        let filler = fill_volume(&filler_path);

        let fault_stage = match backing.prepare_restore() {
            Err(error) => {
                assert!(matches!(&error, MemoryError::Capacity(_)), "prepare error: {error:?}");
                "prepare"
            }
            Ok(Some(candidate)) => {
                assert_eq!(candidate.bytes(), payload, "prepared source must be whole");
                match backing.commit_restore(&candidate.claim) {
                    Err(DurableCommitFailure::Retained(error))
                    | Err(DurableCommitFailure::Ambiguous(error)) => {
                        assert!(
                            matches!(&*error, MemoryError::Capacity(_)),
                            "restore commit error: {error:?}"
                        );
                        "commit"
                    }
                    Ok(()) => panic!("restore commit did not require space on full tmpfs"),
                }
            }
            Ok(None) => panic!("committed restore source disappeared"),
        };
        drop(filler);
        std::fs::remove_file(&filler_path).expect("release filler blocks");
        drop(backing);

        let reopened = RedbSpillBacking::create_with_limits(path, limits).expect("cold reopen");
        let candidate = if let Some(recovery) =
            reopened.recover_active_restore().expect("recover persisted claim")
        {
            if recovery.phase == DurableRestorePhase::Prepared {
                reopened.commit_restore(&recovery.candidate.claim).expect("commit recovered claim");
            }
            recovery.candidate
        } else {
            let candidate =
                reopened.prepare_restore().expect("prepare after ENOSPC").expect("record");
            reopened.commit_restore(&candidate.claim).expect("commit after ENOSPC");
            candidate
        };
        assert_eq!(candidate.bytes(), payload, "restored record must be whole");
        reopened.finalize_restore(&candidate.claim).expect("finalize after ENOSPC");
        assert!(reopened.prepare_restore().expect("drained read").is_none());
        assert_eq!(reopened.retained_count(), Ok(0));
        println!("RESTORE_ENOSPC_RECOVERED fault_stage={fault_stage}");
    }

    pub fn run_finalize(path: &Path) {
        assert_eq!(std::env::var("QMP_ENOSPC_ISOLATED_TMPFS").as_deref(), Ok("1"));
        assert_eq!(std::env::consts::OS, "linux", "this oracle requires Linux ENOSPC");
        assert!(!path.exists(), "use a fresh database path on the disposable tmpfs");
        let filler_path = path.with_extension("fill");
        assert!(!filler_path.exists(), "use a fresh filler path");

        let limits = RedbSpillLimits::default();
        let payload = vec![0xA5; RECORD_BYTES];
        let backing = RedbSpillBacking::create_with_limits(path, limits).expect("open on tmpfs");
        let id = backing.reserve_record_id().expect("reserve source");
        let _receipt = backing.commit_spill(id, &payload).expect("commit source");
        let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
        assert_eq!(candidate.bytes(), payload);
        backing.commit_restore(&candidate.claim).expect("commit restore before ENOSPC");
        let filler = fill_volume(&filler_path);

        let finalize_result = backing.finalize_restore(&candidate.claim);
        if let Err(error) = &finalize_result {
            assert!(matches!(error, MemoryError::Capacity(_)), "finalize error: {error:?}");
        }
        assert_eq!(candidate.bytes(), payload, "resident copy stays whole");
        drop(filler);
        std::fs::remove_file(&filler_path).expect("release filler blocks");
        drop(backing);

        let reopened = RedbSpillBacking::create_with_limits(path, limits).expect("cold reopen");
        match finalize_result {
            Ok(()) => {
                assert!(reopened
                    .recover_active_restore()
                    .expect("probe finalized claim")
                    .is_none());
            }
            Err(_) => {
                let recovery = reopened
                    .recover_active_restore()
                    .expect("recover after ENOSPC")
                    .expect("persisted claim remains");
                assert_eq!(recovery.phase, DurableRestorePhase::Committed);
                assert_eq!(
                    recovery.candidate.bytes(),
                    payload,
                    "durable recovery copy stays whole"
                );
                reopened
                    .finalize_restore(&recovery.candidate.claim)
                    .expect("finalize recovered claim");
            }
        }
        assert_eq!(reopened.resolve_record(id), Ok(DurableRecordResolution::Finalized));
        assert!(reopened.prepare_restore().expect("drained read").is_none());
        assert_eq!(reopened.retained_count(), Ok(0));
        println!("FINALIZE_FULL_VOLUME_COLD_REOPEN_VERIFIED");
    }
}

#[cfg(feature = "spill-redb")]
fn main() {
    let path = std::env::args().nth(1).expect("database path argument required");
    match std::env::args().nth(2).as_deref() {
        None | Some("spill") => scenario::run(std::path::Path::new(&path)),
        Some("restore") => scenario::run_restore(std::path::Path::new(&path)),
        Some("finalize") => scenario::run_finalize(std::path::Path::new(&path)),
        Some(mode) => panic!("unknown ENOSPC scenario {mode}"),
    }
}

#[cfg(not(feature = "spill-redb"))]
fn main() {
    panic!("spill-redb feature is required");
}
