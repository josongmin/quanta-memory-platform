//! Opt-in Linux ENOSPC oracle. Run only with an isolated, disposable tmpfs:
//! `QMP_ENOSPC_ISOLATED_TMPFS=1 ... spill_redb_enospc_volume /mount/db.redb`.
//! The ordinary workspace rail does not run this example.

#[cfg(feature = "spill-redb")]
mod scenario {
    use std::fs::File;
    use std::io::{self, Write};
    use std::path::Path;
    #[cfg(all(feature = "revocable_store", feature = "durability-test-hooks"))]
    use std::path::PathBuf;
    #[cfg(all(feature = "revocable_store", feature = "durability-test-hooks"))]
    use std::sync::atomic::{AtomicBool, Ordering};
    #[cfg(all(feature = "revocable_store", feature = "durability-test-hooks"))]
    use std::sync::OnceLock;

    use quanta_memory_platform_core::MemoryError;
    #[cfg(feature = "revocable_store")]
    use quanta_memory_platform_core::{MemoryClass, MemoryPreset, StoreBindingHandle};
    #[cfg(all(feature = "revocable_store", feature = "durability-test-hooks"))]
    use quanta_memory_platform_facade::spill::backings::redb::restore_commit_volume_hook;
    use quanta_memory_platform_facade::spill::service::{
        DurableRestoreTarget, ReconciledDurableSpillTarget,
    };
    #[cfg(all(feature = "revocable_store", feature = "durability-test-hooks"))]
    use quanta_memory_platform_facade::DurableRestoreController;
    use quanta_memory_platform_facade::{
        DurableCommitFailure, DurableRecordResolution, DurableRestorePhase, DurableWriteFailure,
        RedbSpillBacking, RedbSpillLimits,
    };
    #[cfg(feature = "revocable_store")]
    use quanta_memory_platform_facade::{
        DurableRecordId, DurableRestoreOutcome, MemoryPlatformBuilder,
        ReconciledDurableSpillController, RevocableStore, RoutedSpillController,
    };

    const RECORDS: usize = 24;
    const RECORD_BYTES: usize = 512 * 1024;
    const MAX_FILL_BYTES: usize = 16 * 1024 * 1024;
    #[cfg(all(feature = "revocable_store", feature = "durability-test-hooks"))]
    static RESTORE_COMMIT_FILLER: OnceLock<PathBuf> = OnceLock::new();
    #[cfg(all(feature = "revocable_store", feature = "durability-test-hooks"))]
    static RESTORE_COMMIT_HOOK_FIRED: AtomicBool = AtomicBool::new(false);

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

    #[cfg(all(feature = "revocable_store", feature = "durability-test-hooks"))]
    fn fill_at_restore_commit(sequence: u64, generation: u64) {
        assert_eq!(sequence, 0, "the oldest charged record must own the commit claim");
        assert!(generation > 0, "commit must present a live claim generation");
        let filler_path = RESTORE_COMMIT_FILLER.get().expect("commit filler path installed");
        assert!(!filler_path.exists(), "commit filler must be fresh");
        drop(fill_volume(filler_path));
        RESTORE_COMMIT_HOOK_FIRED.store(true, Ordering::SeqCst);
        println!("CHARGED_RESTORE_COMMIT_VOLUME_FULL sequence={sequence} generation={generation}");
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

    #[cfg(feature = "revocable_store")]
    pub fn run_charged(path: &Path) {
        assert_eq!(std::env::var("QMP_ENOSPC_ISOLATED_TMPFS").as_deref(), Ok("1"));
        assert_eq!(std::env::consts::OS, "linux", "this oracle requires Linux ENOSPC");
        assert!(!path.exists(), "use a fresh database path on the disposable tmpfs");
        let filler_path = path.with_extension("fill");
        assert!(!filler_path.exists(), "use a fresh filler path");

        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("build public platform");
        let store = platform
            .revocable_store_builder::<String, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ArtifactCache,
                "enospc-consumer".to_owned(),
                None,
            ))
            .build()
            .expect("build charged store");
        let payload = vec![0xA5; RECORD_BYTES];
        store.insert_checked("source".to_owned(), payload.clone()).expect("charge source");
        let charged =
            platform.broker().ledger_snapshot().expect("initial ledger").total_owner_bytes();
        assert!(charged > 0, "source must be charged before spill");
        let controller = RoutedSpillController::new(RedbSpillBacking::create(path).expect("open"));
        let filler = fill_volume(&filler_path);
        // Leave room for the record-id transaction while keeping less than
        // one payload's worth of volume headroom. The charged store must see
        // the real write failure, not only a pre-write allocator rejection.
        let filler_len = filler.metadata().expect("filler metadata").len();
        const COMMIT_HEADROOM_BYTES: u64 = 128 * 1024;
        assert!(filler_len > COMMIT_HEADROOM_BYTES);
        filler
            .set_len(filler_len - COMMIT_HEADROOM_BYTES)
            .expect("release bounded record-id headroom");

        let error = store
            .spill_one_durable_bounded(&controller)
            .expect_err("full volume must reject the charged spill");
        let pending = store.pending_durable_spills().expect("pending record ids");
        match &error {
            MemoryError::Capacity(_) => assert!(pending.is_empty(), "definite denial cannot fence"),
            MemoryError::OperationInProgress(_) => {
                assert_eq!(pending.len(), 1, "ambiguous denial fences exactly one source")
            }
            other => panic!("full-volume charged spill returned unexpected error: {other:?}"),
        }
        assert_eq!(
            platform.broker().ledger_snapshot().expect("ledger after denial").total_owner_bytes(),
            charged,
            "denial must not release the source charge"
        );
        drop(filler);
        std::fs::remove_file(&filler_path).expect("release filler blocks");
        drop(controller);

        let reopened =
            RoutedSpillController::new(RedbSpillBacking::create(path).expect("cold reopen"));
        if pending.is_empty() {
            assert_eq!(
                reopened.resolve_record(DurableRecordId::new(0)),
                Ok(DurableRecordResolution::Absent),
                "record id must have been reserved: ENOSPC must reach the payload commit"
            );
        }
        if let Some(record_id) = pending.first() {
            assert!(matches!(
                store.reconcile_durable_spill(&reopened, *record_id).expect("exact reconciliation"),
                DurableRecordResolution::Present { .. } | DurableRecordResolution::Absent
            ));
        }
        assert!(store.pending_durable_spills().expect("fence drained").is_empty());
        if store.committed_bytes().expect("source postimage") == 0 {
            assert!(matches!(
                store.restore_one_durable_bounded(&reopened).expect("restore durable source"),
                DurableRestoreOutcome::Restored { .. }
            ));
        } else {
            assert_eq!(reopened.spilled_count(), Ok(0), "resident source has no durable twin");
        }
        assert_eq!(
            store.reclaim_one().expect("read surviving source"),
            Some(("source".to_owned(), payload))
        );
        assert_eq!(reopened.spilled_count(), Ok(0), "durable tier drains exactly once");
        assert_eq!(
            platform.broker().ledger_snapshot().expect("final ledger").total_owner_bytes(),
            0,
            "source charge drains exactly once"
        );
        println!("CHARGED_ENOSPC_SOURCE_RECOVERED stage=payload_commit pending={}", pending.len());
    }

    /// Drive the public, broker-charged bounded restore through a real Linux
    /// volume fault after its persisted claim and resident charge exist, but
    /// before Redb moves the record into recovery. The hook is compiled only
    /// into opt-in durability test tooling; production has no callback.
    #[cfg(all(feature = "revocable_store", feature = "durability-test-hooks"))]
    pub fn run_charged_restore_commit(path: &Path) {
        assert_eq!(std::env::var("QMP_ENOSPC_ISOLATED_TMPFS").as_deref(), Ok("1"));
        assert_eq!(std::env::consts::OS, "linux", "this oracle requires Linux ENOSPC");
        assert!(!path.exists(), "use a fresh database path on the disposable tmpfs");
        let filler_path = path.with_extension("fill");
        assert!(!filler_path.exists(), "use a fresh filler path");

        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("build public platform");
        let store = platform
            .revocable_store_builder::<String, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ArtifactCache,
                "enospc-restore-consumer".to_owned(),
                None,
            ))
            .build()
            .expect("build charged store");
        let victim = vec![0xA5; RECORD_BYTES];
        let unrelated = vec![0x3C; RECORD_BYTES];
        let controller = RoutedSpillController::new(RedbSpillBacking::create(path).expect("open"));
        for (key, value) in [("victim", &victim), ("unrelated", &unrelated)] {
            store.insert_checked(key.to_owned(), value.clone()).expect("charge source");
            assert!(store.spill_one_durable_bounded(&controller).expect("durable spill").is_some());
        }
        assert_eq!(controller.spilled_count(), Ok(2));
        assert_eq!(
            platform.broker().ledger_snapshot().expect("after spill").total_owner_bytes(),
            0,
        );

        RESTORE_COMMIT_FILLER.set(filler_path.clone()).expect("install filler path once");
        restore_commit_volume_hook::install(fill_at_restore_commit);
        let result = store.restore_one_durable_bounded(&controller);
        assert!(RESTORE_COMMIT_HOOK_FIRED.load(Ordering::SeqCst), "commit hook was not reached");
        let pending = store.pending_durable_restores().expect("pending restore ids");
        match &result {
            Err(MemoryError::Capacity(_)) => assert!(pending.is_empty()),
            Err(MemoryError::OperationInProgress(_)) => {
                assert!(pending.is_empty(), "failed abort retains a backing claim, not a resident");
                assert_eq!(controller.active_restore(), Ok(Some(DurableRecordId::new(0))));
            }
            Ok(DurableRestoreOutcome::RecoveryRequired { sequence: 0, commit_error }) => {
                assert!(matches!(**commit_error, MemoryError::Capacity(_)));
                assert_eq!(pending, vec![DurableRecordId::new(0)]);
            }
            other => panic!("charged restore did not surface real commit ENOSPC: {other:?}"),
        }
        let expected_charge = if pending.is_empty() { 0 } else { victim.len() as u64 + 6 + 32 };
        assert_eq!(store.committed_bytes(), Ok(expected_charge));
        assert_eq!(
            platform.broker().ledger_snapshot().expect("fault ledger").total_owner_bytes(),
            expected_charge,
            "only a fenced resident copy may remain charged",
        );
        assert_eq!(store.reclaim_one().expect("fenced copy unreadable"), None);

        std::fs::remove_file(&filler_path).expect("release filler blocks");
        drop(controller);
        let reopened =
            RoutedSpillController::new(RedbSpillBacking::create(path).expect("cold reopen"));
        if let Some(id) = pending.first() {
            assert!(matches!(
                store.reconcile_durable_restore(&reopened, *id).expect("exact reconciliation"),
                DurableRecordResolution::Present { .. } | DurableRecordResolution::Finalized
            ));
        } else if reopened.active_restore().expect("claim after reopen").is_some() {
            assert!(matches!(
                store.recover_active_durable_restore_bounded(&reopened).expect("recover claim"),
                DurableRestoreOutcome::Restored { sequence: 0 }
            ));
        }
        loop {
            match store.restore_one_durable_bounded(&reopened).expect("restore after space freed") {
                DurableRestoreOutcome::Empty => break,
                DurableRestoreOutcome::Restored { .. } => {}
                outcome => panic!("unexpected second uncertain restore: {outcome:?}"),
            }
        }
        let mut restored = Vec::new();
        while let Some(entry) = store.reclaim_one().expect("read recovered entries") {
            restored.push(entry);
        }
        restored.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(
            restored,
            vec![("unrelated".to_owned(), unrelated), ("victim".to_owned(), victim)]
        );
        assert_eq!(reopened.spilled_count(), Ok(0));
        assert_eq!(reopened.active_restore(), Ok(None));
        assert!(store.pending_durable_restores().expect("pending drained").is_empty());
        assert_eq!(
            platform.broker().ledger_snapshot().expect("final ledger").total_owner_bytes(),
            0
        );
        println!("CHARGED_RESTORE_COMMIT_ENOSPC_RECOVERED");
    }
}

#[cfg(feature = "spill-redb")]
fn main() {
    let path = std::env::args().nth(1).expect("database path argument required");
    match std::env::args().nth(2).as_deref() {
        None | Some("spill") => scenario::run(std::path::Path::new(&path)),
        Some("restore") => scenario::run_restore(std::path::Path::new(&path)),
        Some("finalize") => scenario::run_finalize(std::path::Path::new(&path)),
        #[cfg(feature = "revocable_store")]
        Some("charged") => scenario::run_charged(std::path::Path::new(&path)),
        #[cfg(all(feature = "revocable_store", feature = "durability-test-hooks"))]
        Some("charged-restore-commit") => {
            scenario::run_charged_restore_commit(std::path::Path::new(&path))
        }
        Some(mode) => panic!("unknown ENOSPC scenario {mode}"),
    }
}

#[cfg(not(feature = "spill-redb"))]
fn main() {
    panic!("spill-redb feature is required");
}
