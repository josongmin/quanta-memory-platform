use super::*;

#[cfg(feature = "spill-redb")]
#[test]
fn sep23_fup002_concurrent_recovery_has_exactly_one_owner() {
    use crate::spill::backings::redb::RedbSpillBacking;
    use std::sync::Arc;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-fup002-race-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    sep23_fup002_seed_committed_recovery(&path);

    // The previous volatile owner is gone; two new stores race to recover
    // the same persisted restore on a reopened backend.
    let controller =
        RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb reopen"));
    let broker = live_broker(1_000_000);
    let store_a = store_with_broker(&broker);
    let store_b = store_with_broker(&broker);
    let start = Arc::new(Barrier::new(3));

    let (outcome_a, outcome_b) = thread::scope(|scope| {
        let controller = &controller;
        let start = &start;
        let worker_a = scope.spawn(|| {
            start.wait();
            store_a.recover_active_durable_restore_unbounded(controller, sep23_fup002_decode)
        });
        let worker_b = scope.spawn(|| {
            start.wait();
            store_b.recover_active_durable_restore_unbounded(controller, sep23_fup002_decode)
        });
        start.wait();
        (worker_a.join().expect("worker a panicked"), worker_b.join().expect("worker b panicked"))
    });
    let outcome_a = outcome_a.expect("worker a result");
    let outcome_b = outcome_b.expect("worker b result");

    // Exactly one claimant publishes, whichever generation won the race.
    let restored = [&outcome_a, &outcome_b]
        .iter()
        .filter(|outcome| matches!(outcome, DurableRestoreOutcome::Restored { sequence: 0 }))
        .count();
    assert_eq!(restored, 1, "exactly one Restored: {outcome_a:?} vs {outcome_b:?}");

    // The loser either fenced a provisional copy (contention path) or saw
    // an empty backend (fully serialized after the winner). In both cases
    // it exposes no readable copy and holds no charge after reconciliation.
    for (store, outcome) in [(&store_a, &outcome_a), (&store_b, &outcome_b)] {
        if matches!(outcome, DurableRestoreOutcome::Restored { .. }) {
            continue;
        }
        assert!(
            store.reclaim_one().expect("loser read").is_none(),
            "loser must expose no readable copy"
        );
        match outcome {
            DurableRestoreOutcome::RecoveryRequired { sequence: 0, .. } => {
                assert_eq!(
                    store.pending_durable_restores().expect("fenced"),
                    vec![DurableRecordId::new(0)]
                );
                assert_eq!(
                    store
                        .reconcile_durable_restore(&controller, DurableRecordId::new(0))
                        .expect("loser reconcile"),
                    DurableRecordResolution::Superseded
                );
                assert!(store.pending_durable_restores().expect("drained").is_empty());
                assert_eq!(store.committed_bytes().expect("loser charge"), 0);
            }
            DurableRestoreOutcome::Empty => {
                assert!(store.pending_durable_restores().expect("none fenced").is_empty());
                assert_eq!(store.committed_bytes().expect("loser charge"), 0);
            }
            other => panic!("unexpected loser outcome: {other:?}"),
        }
    }

    // Exactly one resident copy exists, durably consumed, with exact charge.
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(controller.active_restore(), Ok(None));
    let mut reads = 0;
    for store in [&store_a, &store_b] {
        if let Some((key, value)) = store.reclaim_one().expect("winner read") {
            reads += 1;
            assert_eq!(key, "shared");
            assert_eq!(value, b"payload");
        }
    }
    assert_eq!(reads, 1, "exactly one readable resident copy");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);

    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn sep23_audit_panicking_decode_aborts_prepared_claim_and_retries_clean() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-audit-decode-panic-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb"));
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("first".into(), vec![1, 2, 3, 4]).expect("insert");
    let total = store.committed_bytes().expect("before");
    let receipt = store
        .spill_one_durable_unbounded(&controller, |key, value| {
            let mut encoded = key.as_bytes().to_vec();
            encoded.push(0);
            encoded.extend(value);
            Ok(encoded)
        })
        .expect("spill")
        .expect("entry");
    assert_eq!(receipt, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.restore_one_durable_unbounded(
            &controller,
            |_: &[u8]| -> MemoryResult<(String, Vec<u8>)> { panic!("restore decode panic") },
        );
    }));
    assert!(panic.is_err(), "decode panic unwinds through restore");
    // The prepared claim was aborted: nothing active, nothing fenced,
    // nothing charged — the store is exactly as before the attempt.
    assert_eq!(controller.active_restore(), Ok(None));
    assert!(store.pending_durable_restores().expect("none fenced").is_empty());
    assert_eq!(store.committed_bytes().expect("bytes"), 0);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    assert_eq!(controller.spilled_count(), Ok(1), "durable record retained");

    // A plain retry with honest decode restores exactly once.
    let restore =
        store.restore_one_durable_unbounded(&controller, sep23_fup002_decode).expect("retry");
    assert!(matches!(restore, DurableRestoreOutcome::Restored { sequence: 0 }));
    assert_eq!(store.committed_bytes().expect("restored bytes"), total);
    assert_eq!(store.reclaim_one().expect("entry").expect("restored").0, "first");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    assert_eq!(controller.spilled_count(), Ok(0));
    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn panicking_restore_measurement_aborts_prepared_claim_and_retries_clean() {
    use crate::spill::backings::redb::RedbSpillBacking;

    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-restore-measure-panic-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb"));
    let record_id = controller.reserve_record_id().expect("reserve record");
    let _receipt = controller.commit_spill(record_id, b"shared\0payload").expect("commit record");
    let broker = live_broker(1_000_000);
    let store = SimpleRevocableStore::<String, PanicOnceMeasuredValue, Budgeted>::new_budgeted(
        binding(),
        broker.clone(),
    );
    let panic_once = Arc::new(AtomicBool::new(true));
    let decode = |bytes: &[u8]| {
        let (key, value) = sep23_fup002_decode(bytes)?;
        Ok((key, PanicOnceMeasuredValue { bytes: value, panic_once: Arc::clone(&panic_once) }))
    };

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.restore_one_durable_unbounded(&controller, decode);
    }));
    assert!(panic.is_err(), "caller measurement panic propagates");
    assert_eq!(controller.active_restore(), Ok(None), "prepared claim must be aborted");
    assert_eq!(controller.spilled_count(), Ok(1), "durable copy remains");
    assert!(store.pending_durable_restores().expect("fences").is_empty());
    assert_eq!(store.committed_bytes().expect("resident bytes"), 0);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);

    let restored = store
        .restore_one_durable_unbounded(&controller, |bytes| {
            let (key, value) = sep23_fup002_decode(bytes)?;
            Ok((key, PanicOnceMeasuredValue { bytes: value, panic_once: Arc::clone(&panic_once) }))
        })
        .expect("plain retry after prepared abort");
    assert!(matches!(restored, DurableRestoreOutcome::Restored { sequence: 0 }));
    let (key, value) = store.reclaim_one().expect("reclaim").expect("restored entry");
    assert_eq!(key, "shared");
    assert_eq!(value.bytes, b"payload");
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn panicking_recovery_measurement_preserves_committed_claim_for_retry() {
    use crate::spill::backings::redb::RedbSpillBacking;

    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-recovery-measure-panic-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    sep23_fup002_seed_committed_recovery(&path);
    let controller =
        RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb reopen"));
    let broker = live_broker(1_000_000);
    let store = SimpleRevocableStore::<String, PanicOnceMeasuredValue, Budgeted>::new_budgeted(
        binding(),
        broker.clone(),
    );
    let panic_once = Arc::new(AtomicBool::new(true));

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.recover_active_durable_restore_unbounded(&controller, |bytes| {
            let (key, value) = sep23_fup002_decode(bytes)?;
            Ok((key, PanicOnceMeasuredValue { bytes: value, panic_once: Arc::clone(&panic_once) }))
        });
    }));
    assert!(panic.is_err(), "caller measurement panic propagates");
    assert_eq!(controller.active_restore(), Ok(Some(DurableRecordId::new(0))));
    assert!(store.pending_durable_restores().expect("fences").is_empty());
    assert_eq!(store.committed_bytes().expect("resident bytes"), 0);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);

    let restored = store
        .recover_active_durable_restore_unbounded(&controller, |bytes| {
            let (key, value) = sep23_fup002_decode(bytes)?;
            Ok((key, PanicOnceMeasuredValue { bytes: value, panic_once: Arc::clone(&panic_once) }))
        })
        .expect("recovery retry");
    assert!(matches!(restored, DurableRestoreOutcome::Restored { sequence: 0 }));
    let (key, value) = store.reclaim_one().expect("reclaim").expect("restored entry");
    assert_eq!(key, "shared");
    assert_eq!(value.bytes, b"payload");
    assert_eq!(controller.active_restore(), Ok(None));
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn panicking_restore_commit_fences_resident_until_redb_reconciliation() {
    use crate::spill::backings::redb::RedbSpillBacking;

    struct PanicCommitRestore<'a> {
        inner: &'a RoutedSpillController<RedbSpillBacking>,
        commit_before_panic: bool,
    }

    impl SpillController<Vec<u8>> for PanicCommitRestore<'_> {
        fn spill(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            self.inner.spill(handle)
        }
        fn restore(&self) -> MemoryResult<Option<Vec<u8>>> {
            self.inner.restore()
        }
    }
    impl crate::spill::DurableSpillController<Vec<u8>> for PanicCommitRestore<'_> {}
    impl crate::spill::service::sealed::DurableSpillController<Vec<u8>> for PanicCommitRestore<'_> {}
    impl DurableRestoreController for PanicCommitRestore<'_> {
        fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
            self.inner.prepare_restore()
        }
        fn commit_restore(&self, claim: &DurableRestoreClaim) -> Result<(), DurableCommitFailure> {
            if self.commit_before_panic {
                self.inner.commit_restore(claim)?;
            }
            panic!("injected restore commit panic");
        }
        fn finalize_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
            self.inner.finalize_restore(claim)
        }
        fn abort_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
            self.inner.abort_restore(claim)
        }
        fn reconcile_restore(
            &self,
            claim: &DurableRestoreClaim,
        ) -> MemoryResult<DurableRecordResolution> {
            self.inner.reconcile_restore(claim)
        }
        fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
            self.inner.active_restore()
        }
        fn recover_active_restore(
            &self,
        ) -> MemoryResult<Option<crate::spill::DurableRestoreRecovery>> {
            self.inner.recover_active_restore()
        }
    }

    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    for commit_before_panic in [false, true] {
        let path = std::env::temp_dir().join(format!(
            "qmp-restore-commit-panic-{}-{}.redb",
            std::process::id(),
            DB_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb"));
        let id = controller.reserve_record_id().expect("reserve");
        let _receipt = controller.commit_spill(id, b"shared\0payload").expect("seed record");
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        let injected = PanicCommitRestore { inner: &controller, commit_before_panic };

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.restore_one_durable_unbounded(&injected, sep23_fup002_decode);
        }));
        assert!(panic.is_err(), "commit callback panic propagates");
        assert_eq!(
            store.pending_durable_restores().expect("fences"),
            vec![id],
            "provisional resident must be fenced, committed={commit_before_panic}"
        );
        assert!(store.reclaim_one().expect("fenced read").is_none());
        let charged = store.committed_bytes().expect("provisional charge");
        assert!(charged > 0);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), charged);

        let resolution = store.reconcile_durable_restore(&controller, id).expect("reconcile");
        assert!(store.pending_durable_restores().expect("resolved").is_empty());
        if commit_before_panic {
            assert_eq!(resolution, DurableRecordResolution::Finalized);
        } else {
            assert!(matches!(resolution, DurableRecordResolution::Present { .. }));
            assert_eq!(store.committed_bytes().expect("retired provisional"), 0);
            let restored = store
                .restore_one_durable_unbounded(&controller, sep23_fup002_decode)
                .expect("ordinary retry");
            assert!(matches!(restored, DurableRestoreOutcome::Restored { sequence: 0 }));
        }
        assert_eq!(store.reclaim_one().expect("read").expect("one entry").0, "shared");
        assert!(store.reclaim_one().expect("no duplicate").is_none());
        assert_eq!(controller.spilled_count(), Ok(0));
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
        drop(controller);
        let _ = std::fs::remove_file(path);
    }
}
#[cfg(feature = "spill-redb")]
#[test]
fn panicking_prepare_after_persisted_claim_has_explicit_recovery_path() {
    use crate::spill::backings::redb::RedbSpillBacking;

    struct PanicAfterPrepare<'a>(&'a RoutedSpillController<RedbSpillBacking>);
    impl SpillController<Vec<u8>> for PanicAfterPrepare<'_> {
        fn spill(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            self.0.spill(handle)
        }
        fn restore(&self) -> MemoryResult<Option<Vec<u8>>> {
            self.0.restore()
        }
    }
    impl crate::spill::DurableSpillController<Vec<u8>> for PanicAfterPrepare<'_> {}
    impl crate::spill::service::sealed::DurableSpillController<Vec<u8>> for PanicAfterPrepare<'_> {}
    impl DurableRestoreController for PanicAfterPrepare<'_> {
        fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
            assert!(self.0.prepare_restore()?.is_some(), "claim was persisted");
            panic!("injected panic after prepare claim");
        }
        fn commit_restore(&self, claim: &DurableRestoreClaim) -> Result<(), DurableCommitFailure> {
            self.0.commit_restore(claim)
        }
        fn finalize_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
            self.0.finalize_restore(claim)
        }
        fn abort_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
            self.0.abort_restore(claim)
        }
        fn reconcile_restore(
            &self,
            claim: &DurableRestoreClaim,
        ) -> MemoryResult<DurableRecordResolution> {
            self.0.reconcile_restore(claim)
        }
        fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
            self.0.active_restore()
        }
        fn recover_active_restore(
            &self,
        ) -> MemoryResult<Option<crate::spill::DurableRestoreRecovery>> {
            self.0.recover_active_restore()
        }
    }

    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-prepare-panic-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb"));
    let id = controller.reserve_record_id().expect("reserve");
    let _receipt = controller.commit_spill(id, b"shared\0payload").expect("seed record");
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store
            .restore_one_durable_unbounded(&PanicAfterPrepare(&controller), sep23_fup002_decode);
    }));
    assert!(panic.is_err());
    assert_eq!(controller.active_restore(), Ok(Some(id)));
    assert!(store.pending_durable_restores().expect("no provisional copy").is_empty());
    assert_eq!(store.committed_bytes().expect("no resident charge"), 0);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);

    let outcome = store
        .recover_active_durable_restore_unbounded(&controller, sep23_fup002_decode)
        .expect("recovery after lost prepare response");
    assert!(matches!(outcome, DurableRestoreOutcome::Restored { sequence: 0 }));
    assert_eq!(store.reclaim_one().expect("read").expect("entry").0, "shared");
    assert!(store.reclaim_one().expect("no duplicate").is_none());
    assert_eq!(controller.active_restore(), Ok(None));
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn sep23_audit_panicking_decode_preserves_committed_recovery_until_retry() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-audit-recovery-decode-panic-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    sep23_fup002_seed_committed_recovery(&path);
    let controller =
        RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb reopen"));
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.recover_active_durable_restore_unbounded(
            &controller,
            |_: &[u8]| -> MemoryResult<(String, Vec<u8>)> { panic!("recovery decode panic") },
        );
    }));
    assert!(panic.is_err(), "decode panic unwinds through recovery");
    // Committed recovery is preserved untouched: still active, still
    // restorable, with no provisional local state (nothing decoded yet).
    assert_eq!(controller.active_restore(), Ok(Some(DurableRecordId::new(0))));
    assert!(store.pending_durable_restores().expect("none fenced").is_empty());
    assert_eq!(store.committed_bytes().expect("bytes"), 0);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);

    // Honest recovery retry publishes the preserved copy exactly once.
    let restore = store
        .recover_active_durable_restore_unbounded(&controller, sep23_fup002_decode)
        .expect("retry");
    assert!(matches!(restore, DurableRestoreOutcome::Restored { sequence: 0 }));
    let (key, value) = store.reclaim_one().expect("entry").expect("restored");
    assert_eq!(key, "shared");
    assert_eq!(value, b"payload");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(controller.active_restore(), Ok(None));
    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn sep23_fup002_stolen_claim_reconcile_retires_provisional_copy() {
    use crate::spill::backings::redb::RedbSpillBacking;
    use std::sync::mpsc;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-fup002-steal-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    sep23_fup002_seed_committed_recovery(&path);

    // Deterministic steal orchestration (no sleeps): A's decode blocks
    // after A minted its claim, B steals and wins while A waits, then A
    // resumes into a stale finalize, fences, and reconciles as superseded.
    let controller =
        RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb reopen"));
    let broker = live_broker(1_000_000);
    let store_a = store_with_broker(&broker);
    let store_b = store_with_broker(&broker);
    let (claimed_tx, claimed_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();

    let outcome_a = thread::scope(|scope| {
        // Move only shared refs plus the worker-owned channel ends into
        // the spawned closure (`Receiver` is `Send` but not `Sync`, so it
        // cannot be captured by reference).
        let worker = scope.spawn({
            let store_a = &store_a;
            let controller = &controller;
            move || {
                store_a.recover_active_durable_restore_unbounded(controller, |bytes| {
                    claimed_tx.send(()).expect("signal claim minted");
                    release_rx.recv().expect("wait for the steal to land");
                    sep23_fup002_decode(bytes)
                })
            }
        });
        claimed_rx.recv().expect("A minted its claim");
        let outcome_b = store_b
            .recover_active_durable_restore_unbounded(&controller, sep23_fup002_decode)
            .expect("B steals and wins");
        assert!(
            matches!(outcome_b, DurableRestoreOutcome::Restored { sequence: 0 }),
            "B must publish exactly once, got {outcome_b:?}"
        );
        release_tx.send(()).expect("release A");
        worker.join().expect("worker A panicked").expect("A result")
    });

    // A resumes with a stale generation: publish attempt fences instead of
    // succeeding, and the fenced copy is never readable.
    assert!(
        matches!(outcome_a, DurableRestoreOutcome::RecoveryRequired { sequence: 0, .. }),
        "stale claimant must fence, got {outcome_a:?}"
    );
    assert!(store_a.reclaim_one().expect("A read").is_none(), "fenced copy not readable");
    assert_eq!(
        store_a.pending_durable_restores().expect("A fenced"),
        vec![DurableRecordId::new(0)]
    );
    assert_eq!(
        store_a
            .reconcile_durable_restore(&controller, DurableRecordId::new(0))
            .expect("A reconcile"),
        DurableRecordResolution::Superseded
    );
    assert!(store_a.pending_durable_restores().expect("A drained").is_empty());
    assert_eq!(store_a.committed_bytes().expect("A charge retired"), 0);

    // Winner owns the single copy; the backend is drained; no charge leaks.
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(store_b.reclaim_one().expect("read B").expect("entry B").0, "shared");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);

    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn sep23_fup002_decode_failure_and_denied_admission_preserve_durable() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-fup002-preserve-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb"));
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("kept".into(), vec![1; 8]).expect("insert");
    let total = store.committed_bytes().expect("bytes");
    let _receipt = store
        .spill_one_durable_unbounded(&controller, |key, value| {
            let mut encoded = key.as_bytes().to_vec();
            encoded.push(0);
            encoded.extend(value);
            Ok(encoded)
        })
        .expect("spill")
        .expect("receipt");

    // Decode failure aborts the Prepared claim; the record stays durable.
    let failed = store.restore_one_durable_unbounded(&controller, |_| {
        Err(MemoryError::report("sep23 fup002 injected decode failure"))
    });
    assert!(matches!(failed, Err(MemoryError::Report(_))), "got {failed:?}");
    assert_eq!(controller.active_restore(), Ok(None));
    assert_eq!(controller.spilled_count(), Ok(1));

    // Admission denial does the same; nothing is charged or published.
    let tiny = live_broker(5);
    let denied_store = store_with_broker(&tiny);
    let denied = denied_store.restore_one_durable_unbounded(&controller, sep23_fup002_decode);
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
    assert_eq!(controller.active_restore(), Ok(None));
    assert_eq!(controller.spilled_count(), Ok(1));
    assert_eq!(tiny.ledger_snapshot().expect("tiny ledger").total_owner_bytes(), 0);

    // A fresh claim still restores the preserved record exactly once.
    let outcome = store
        .restore_one_durable_unbounded(&controller, sep23_fup002_decode)
        .expect("restore preserved");
    assert!(matches!(outcome, DurableRestoreOutcome::Restored { sequence: 0 }));
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(store.committed_bytes().expect("restored bytes"), total);
    assert_eq!(store.reclaim_one().expect("read").expect("entry").0, "kept");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);

    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn sep23_fup002_commit_ack_loss_same_claimant_publishes_on_retry() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-fup002-commitack-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let backing = RedbSpillBacking::create(&path).expect("redb");
    // Inject before wrapping: the flag lives in the backing the controller
    // owns, and fires on the restore commit below (the spill is unaffected).
    backing.inject_lost_restore_commit_ack();
    let controller = RoutedSpillController::new(backing);
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("acked".into(), vec![2; 8]).expect("insert");
    let total = store.committed_bytes().expect("bytes");
    let _receipt = store
        .spill_one_durable_unbounded(&controller, |key, value| {
            let mut encoded = key.as_bytes().to_vec();
            encoded.push(0);
            encoded.extend(value);
            Ok(encoded)
        })
        .expect("spill")
        .expect("receipt");

    // Ambiguous commit fences the provisional copy with its claim intact.
    let outcome = store
        .restore_one_durable_unbounded(&controller, sep23_fup002_decode)
        .expect("restore with lost commit ack");
    assert!(
        matches!(outcome, DurableRestoreOutcome::RecoveryRequired { sequence: 0, .. }),
        "ambiguous commit must fence, got {outcome:?}"
    );
    assert!(store.reclaim_one().expect("fenced read").is_none());
    assert_eq!(store.pending_durable_restores().expect("fenced"), vec![DurableRecordId::new(0)]);

    // Same-claimant reconcile proves the commit landed and publishes.
    assert_eq!(
        store.reconcile_durable_restore(&controller, DurableRecordId::new(0)).expect("reconcile"),
        DurableRecordResolution::Finalized
    );
    assert!(store.pending_durable_restores().expect("resolved").is_empty());
    assert_eq!(store.committed_bytes().expect("published bytes"), total);
    assert_eq!(store.reclaim_one().expect("read").expect("entry").0, "acked");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    assert_eq!(controller.spilled_count(), Ok(0));

    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn denied_durable_restore_leaves_redb_record_authoritative() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-revocable-denied-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb"));
    let record_id = controller.reserve_record_id().expect("reserve durable record");
    let receipt = controller.commit_spill(record_id, b"large\0payload").expect("seed durable");
    assert!(matches!(receipt, SpillReceipt::Durable { .. }));
    let tiny_broker = live_broker(5);
    let store = store_with_broker(&tiny_broker);

    assert!(matches!(
        store.restore_one_durable_unbounded(&controller, |_| {
            Ok(("large".to_string(), vec![0; 100]))
        }),
        Err(MemoryError::Capacity(_))
    ));
    assert_eq!(controller.spilled_count(), Ok(1));
    assert_eq!(store.stats().expect("source unchanged"), StoreStats::default());
    assert_eq!(tiny_broker.ledger_snapshot().expect("ledger unchanged").total_owner_bytes(), 0);
    drop(controller);
    let _ = std::fs::remove_file(path);
}
