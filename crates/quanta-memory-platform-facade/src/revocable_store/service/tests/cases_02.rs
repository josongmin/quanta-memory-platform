use super::*;

#[cfg(feature = "spill-redb")]
#[test]
fn bounded_codec_rejects_malformed_durable_record_without_publication() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-bounded-malformed-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    assert!(!path.exists(), "test needs a fresh backing");
    let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("backing"));
    let record_id = controller.reserve_record_id().expect("id");
    // Valid V1 lengths with an unsupported version: a format migration
    // must not publish or delete an older/unknown durable authority.
    const UNKNOWN_VERSION: [u8; 22] =
        [b'Q', b'M', b'R', b'2', 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, b'k', b'v'];
    let _receipt = controller.commit_spill(record_id, &UNKNOWN_VERSION).expect("seed");
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    let denied = store.restore_one_durable_bounded(&controller);
    assert!(matches!(denied, Err(MemoryError::InvalidState(_))), "{denied:?}");
    assert_eq!(controller.spilled_count(), Ok(1));
    assert_eq!(controller.active_restore(), Ok(None));
    assert_eq!(store.stats(), Ok(StoreStats::default()));
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    drop(controller);
    std::fs::remove_file(path).expect("remove test backing");
}
#[cfg(feature = "spill-durable")]
#[test]
fn bounded_codec_keeps_output_charged_until_durable_commit_returns() {
    struct ObservingTarget {
        broker: LiveProcessMemoryBroker,
        expected_peak: u64,
        reservations: Arc<AtomicUsize>,
    }
    impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for ObservingTarget {}
    impl SpillInventory for ObservingTarget {
        fn retained_count(&self) -> MemoryResult<u64> {
            Ok(0)
        }
    }
    impl SpillTarget<Vec<u8>> for ObservingTarget {
        fn is_available(&self) -> bool {
            true
        }
        fn accept(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            Err(SpillFailure::new(MemoryError::InvalidState("use commit_spill".into()), handle))
        }
    }
    impl DurableSpillTarget<Vec<u8>> for ObservingTarget {}
    impl crate::spill::service::ReconciledDurableSpillTarget for ObservingTarget {
        fn max_record_bytes(&self) -> Option<u64> {
            Some(100)
        }
        fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
            self.reservations.fetch_add(1, Ordering::SeqCst);
            Ok(DurableRecordId::new(1))
        }
        fn commit_spill(
            &self,
            record_id: DurableRecordId,
            bytes: &[u8],
        ) -> Result<SpillReceipt, DurableWriteFailure> {
            assert_eq!(bytes.len(), 41);
            assert_eq!(
                self.broker.ledger_snapshot().expect("commit ledger").total_owner_bytes(),
                self.expected_peak,
                "source and codec output must both remain charged through commit"
            );
            Ok(SpillReceipt::Durable { sequence: record_id.get(), inventory_after: 1 })
        }
        fn resolve_record(
            &self,
            _record_id: DurableRecordId,
        ) -> MemoryResult<DurableRecordResolution> {
            Ok(DurableRecordResolution::Present { inventory_after: 1 })
        }
    }

    let broker = live_broker(100);
    let store = store_with_broker(&broker);
    store.insert_checked("k".into(), vec![7; 20]).expect("source");
    let reservations = Arc::new(AtomicUsize::new(0));
    let target = ObservingTarget {
        broker: broker.clone(),
        expected_peak: 94,
        reservations: Arc::clone(&reservations),
    };
    let controller = RoutedSpillController::new(target);
    assert!(matches!(
        store.spill_one_durable_bounded(&controller),
        Ok(Some(SpillReceipt::Durable { .. }))
    ));
    assert_eq!(reservations.load(Ordering::SeqCst), 1);
    assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
}
#[cfg(feature = "spill-redb")]
#[test]
fn bounded_codec_committed_recovery_retries_after_capacity_denial() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-bounded-recovery-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    assert!(!path.exists(), "test needs a fresh backing");
    let record_id = {
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(&path).expect("backing"));
        let len = durable_codec::encoded_len("k", &[7; 20], 100).expect("wire length");
        let bytes = durable_codec::encode("k", &[7; 20], len).expect("encode seed");
        let record_id = controller.reserve_record_id().expect("id");
        let _receipt = controller.commit_spill(record_id, &bytes).expect("seed");
        let candidate = controller.prepare_restore().expect("prepare").expect("candidate");
        controller.commit_restore(&candidate.claim).expect("committed recovery");
        record_id
    };
    let controller =
        RoutedSpillController::new(RedbSpillBacking::create(&path).expect("cold reopen"));
    let tight_broker = live_broker(52);
    let tight_store = store_with_broker(&tight_broker);
    let denied = tight_store.recover_active_durable_restore_bounded(&controller);
    assert!(matches!(denied, Err(MemoryError::OperationInProgress(_))), "{denied:?}");
    assert_eq!(controller.active_restore(), Ok(Some(record_id)));
    assert_eq!(tight_broker.ledger_snapshot().expect("denied ledger").total_owner_bytes(), 0);

    let broker = live_broker(100);
    let store = store_with_broker(&broker);
    assert!(matches!(
        store.recover_active_durable_restore_bounded(&controller),
        Ok(DurableRestoreOutcome::Restored { .. })
    ));
    assert_eq!(store.reclaim_one().expect("read"), Some(("k".into(), vec![7; 20])));
    assert_eq!(controller.active_restore(), Ok(None));
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    drop(controller);
    std::fs::remove_file(path).expect("remove test backing");
}
#[cfg(feature = "spill-durable")]
#[test]
fn panicking_durable_encoder_restores_source_before_unwind() {
    struct DurableSink;
    impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for DurableSink {}
    impl SpillInventory for DurableSink {
        fn retained_count(&self) -> MemoryResult<u64> {
            Ok(0)
        }
    }
    impl SpillTarget<Vec<u8>> for DurableSink {
        fn is_available(&self) -> bool {
            true
        }
        fn accept(&self, _handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            Ok(SpillReceipt::Durable { sequence: 0, inventory_after: 1 })
        }
    }
    impl DurableSpillTarget<Vec<u8>> for DurableSink {}
    impl crate::spill::service::ReconciledDurableSpillTarget for DurableSink {
        fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
            Ok(DurableRecordId::new(0))
        }

        fn commit_spill(
            &self,
            record_id: DurableRecordId,
            _bytes: &[u8],
        ) -> Result<SpillReceipt, DurableWriteFailure> {
            Ok(SpillReceipt::Durable { sequence: record_id.get(), inventory_after: 1 })
        }

        fn resolve_record(
            &self,
            _record_id: DurableRecordId,
        ) -> MemoryResult<DurableRecordResolution> {
            Ok(DurableRecordResolution::Present { inventory_after: 1 })
        }
    }

    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("first".into(), vec![1; 4]).expect("insert");
    let total = store.committed_bytes().expect("before");
    let controller = RoutedSpillController::new(DurableSink);

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.spill_one_durable_unbounded(&controller, |_, _| -> MemoryResult<Vec<u8>> {
            panic!("encoder panic")
        });
    }));
    assert!(panic.is_err());
    assert_eq!(store.committed_bytes().expect("restored"), total);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), total);
    assert_eq!(store.reclaim_one().expect("source").expect("entry").0, "first");
}
#[cfg(feature = "spill-durable")]
#[test]
fn panicking_durable_commit_fences_source_until_exact_record_resolution() {
    struct PanicCommitTarget {
        persist_before_panic: bool,
        retained: Mutex<Option<Vec<u8>>>,
    }

    impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for PanicCommitTarget {}
    impl SpillInventory for PanicCommitTarget {
        fn retained_count(&self) -> MemoryResult<u64> {
            Ok(u64::from(self.retained.lock().expect("retained").is_some()))
        }
    }
    impl SpillTarget<Vec<u8>> for PanicCommitTarget {
        fn is_available(&self) -> bool {
            true
        }
        fn accept(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            Err(SpillFailure::new(
                MemoryError::InvalidState("use reconciled commit path".into()),
                handle,
            ))
        }
    }
    impl DurableSpillTarget<Vec<u8>> for PanicCommitTarget {}
    impl crate::spill::service::ReconciledDurableSpillTarget for PanicCommitTarget {
        fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
            Ok(DurableRecordId::new(0))
        }

        fn commit_spill(
            &self,
            record_id: DurableRecordId,
            bytes: &[u8],
        ) -> Result<SpillReceipt, DurableWriteFailure> {
            assert_eq!(record_id, DurableRecordId::new(0));
            if self.persist_before_panic {
                *self.retained.lock().expect("retained") = Some(bytes.to_vec());
            }
            panic!("injected commit callback panic");
        }

        fn resolve_record(
            &self,
            record_id: DurableRecordId,
        ) -> MemoryResult<DurableRecordResolution> {
            assert_eq!(record_id, DurableRecordId::new(0));
            Ok(if self.retained.lock().expect("retained").is_some() {
                DurableRecordResolution::Present { inventory_after: 1 }
            } else {
                DurableRecordResolution::Absent
            })
        }
    }

    for persist_before_panic in [false, true] {
        let broker = live_broker(1_000_000);
        let store = store_with_broker(&broker);
        store.insert_checked("source".into(), vec![1; 4]).expect("source insert");
        let source_bytes = store.committed_bytes().expect("source charge");
        let controller = RoutedSpillController::new(PanicCommitTarget {
            persist_before_panic,
            retained: Mutex::new(None),
        });

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ =
                store.spill_one_durable_unbounded(&controller, |_, _| Ok(b"source\0data".to_vec()));
        }));
        assert!(panic.is_err(), "commit callback panic propagates");
        assert_eq!(
            store.pending_durable_spills().expect("pending"),
            vec![DurableRecordId::new(0)],
            "uncertain commit must fence source, persist={persist_before_panic}"
        );
        assert!(store.reclaim_one().expect("fenced read").is_none());
        assert_eq!(store.committed_bytes().expect("charged source"), source_bytes);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), source_bytes);

        let resolution = store
            .reconcile_durable_spill(&controller, DurableRecordId::new(0))
            .expect("resolve exact record");
        assert!(store.pending_durable_spills().expect("resolved").is_empty());
        if persist_before_panic {
            assert_eq!(resolution, DurableRecordResolution::Present { inventory_after: 1 });
            assert_eq!(store.committed_bytes().expect("source retired"), 0);
            assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
        } else {
            assert_eq!(resolution, DurableRecordResolution::Absent);
            assert_eq!(store.committed_bytes().expect("source restored"), source_bytes);
            assert_eq!(store.reclaim_one().expect("restored read").expect("source").0, "source");
            assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
        }
    }
}
#[cfg(feature = "spill-redb")]
#[test]
fn durable_redb_round_trip_transfers_charge_exactly_once() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-revocable-roundtrip-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("first".into(), vec![1, 2, 3, 4]).expect("insert");
    let total = store.committed_bytes().expect("before");
    let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb"));

    let receipt = store.spill_one_durable_bounded(&controller).expect("spill").expect("entry");
    assert_eq!(receipt, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });
    assert_eq!(broker.ledger_snapshot().expect("off-process").total_owner_bytes(), 0);
    assert_eq!(controller.spilled_count(), Ok(1));

    let restore = store.restore_one_durable_bounded(&controller).expect("restore");
    assert!(matches!(restore, DurableRestoreOutcome::Restored { sequence: 0 }));
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(store.committed_bytes().expect("restored bytes"), total);
    assert_eq!(broker.ledger_snapshot().expect("restored charge").total_owner_bytes(), total);
    assert_eq!(store.reclaim_one().expect("entry").expect("restored").0, "first");
    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn oversized_encoded_record_preserves_charged_source_and_no_durable_payload() {
    use crate::spill::backings::redb::{RedbSpillBacking, RedbSpillLimits};
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-revocable-record-cap-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let limits = RedbSpillLimits {
        max_record_bytes: 3,
        max_retained_payload_bytes: 3,
        max_inflight_restore_bytes: 6,
        ..RedbSpillLimits::default()
    };
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("source".into(), vec![1, 2, 3, 4]).expect("insert");
    let charged = store.committed_bytes().expect("initial charge");
    let controller = RoutedSpillController::new(
        RedbSpillBacking::create_with_limits(&path, limits).expect("open"),
    );

    let denied = store.spill_one_durable_unbounded(&controller, |_, value| Ok(value.clone()));
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
    assert_eq!(store.pending_durable_spills(), Ok(Vec::new()));
    assert_eq!(store.committed_bytes(), Ok(charged));
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), charged);
    assert_eq!(controller.spilled_count(), Ok(0));
    drop(controller);

    let reopened = RedbSpillBacking::create_with_limits(&path, limits).expect("cold reopen");
    assert_eq!(reopened.retained_count(), Ok(0));
    assert_eq!(
        store.reclaim_one().expect("resident source"),
        Some(("source".into(), vec![1, 2, 3, 4]))
    );
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    drop(reopened);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn redb_file_cap_denial_keeps_store_source_charged_until_exact_resolution() {
    use crate::spill::backings::redb::{RedbSpillBacking, RedbSpillLimits};
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-revocable-file-cap-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let limits = RedbSpillLimits {
        max_record_bytes: 512 * 1024,
        max_retained_records: 64,
        max_retained_payload_bytes: 32 * 1024 * 1024,
        max_database_file_bytes: 4 * 1024 * 1024,
        cache_bytes: 4 * 1024 * 1024,
        max_inflight_restore_bytes: 1024 * 1024,
        max_inflight_restore_buffers: 2,
    };
    let broker = live_broker(2_000_000);
    let store = store_with_broker(&broker);
    let controller = RoutedSpillController::new(
        RedbSpillBacking::create_with_limits(&path, limits).expect("open"),
    );
    let payload = vec![0x5A; 512 * 1024];
    let mut denied = false;
    for _ in 0..63 {
        store.insert_checked("source".into(), payload.clone()).expect("insert source");
        let charge = store.committed_bytes().expect("charged source");
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), charge);
        match store.spill_one_durable_unbounded(&controller, |_, value| Ok(value.clone())) {
            Ok(Some(_)) => {
                assert_eq!(store.committed_bytes(), Ok(0));
                assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
            }
            Err(error) => {
                assert!(
                    format!("{error}").contains("database file"),
                    "unexpected spill denial: {error}"
                );
                assert_eq!(store.committed_bytes(), Ok(charge));
                assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), charge);
                denied = true;
                break;
            }
            Ok(None) => panic!("live source vanished before spill"),
        }
    }
    assert!(denied, "file cap must reject before logical record/payload caps");
    drop(controller);
    let larger = RedbSpillLimits { max_database_file_bytes: 64 * 1024 * 1024, ..limits };
    let reopened = RoutedSpillController::new(
        RedbSpillBacking::create_with_limits(&path, larger).expect("reopen"),
    );
    let pending = store.pending_durable_spills().expect("pending ids");
    if let Some(id) = pending.first() {
        assert_eq!(pending.len(), 1);
        let resolution = store.reconcile_durable_spill(&reopened, *id).expect("reconcile");
        assert!(matches!(
            resolution,
            DurableRecordResolution::Present { .. }
                | DurableRecordResolution::Finalized
                | DurableRecordResolution::Absent
        ));
    }
    if store.committed_bytes().expect("post-resolution charge") != 0 {
        let (key, value) = store.reclaim_one().expect("read surviving source").expect("source");
        assert_eq!(key, "source");
        assert_eq!(value, payload);
    }
    assert!(store.pending_durable_spills().expect("resolved").is_empty());
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    drop(reopened);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn durable_redb_committed_restore_recovers_after_process_restart() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-revocable-crash-recovery-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    {
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb 1"));
        let record_id = controller.reserve_record_id().expect("reserve");
        let _receipt =
            controller.commit_spill(record_id, b"recovered\0payload").expect("seed durable");
        let candidate = controller.prepare_restore().expect("prepare").expect("candidate");
        controller
            .commit_restore(&candidate.claim)
            .expect("persist recovery payload before simulated crash");
        assert_eq!(controller.active_restore(), Ok(Some(record_id)));
        // No finalize: dropping this controller simulates a process that
        // dies after durable delete but before resident acknowledgement.
    }

    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb 2"));
    let outcome = store
        .recover_active_durable_restore_unbounded(&controller, |bytes| {
            let separator = bytes
                .iter()
                .position(|byte| *byte == 0)
                .ok_or_else(|| MemoryError::report("missing separator"))?;
            Ok((
                String::from_utf8(bytes[..separator].to_vec())
                    .map_err(|error| MemoryError::report(format!("invalid key: {error}")))?,
                bytes[separator + 1..].to_vec(),
            ))
        })
        .expect("restart recovery");
    assert!(matches!(outcome, DurableRestoreOutcome::Restored { sequence: 0 }));
    assert_eq!(controller.active_restore(), Ok(None));
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(store.reclaim_one().expect("read recovered").expect("entry").0, "recovered");
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);

    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn durable_redb_prepared_restore_recovers_after_process_restart() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-revocable-prepared-recovery-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    {
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb 1"));
        let record_id = controller.reserve_record_id().expect("reserve");
        let _receipt =
            controller.commit_spill(record_id, b"prepared\0payload").expect("seed durable");
        let candidate = controller.prepare_restore().expect("prepare").expect("candidate");
        assert_eq!(candidate.sequence, record_id.get());
        // No commit: the durable queue plus persisted claim must survive.
    }

    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    let controller = RoutedSpillController::new(RedbSpillBacking::create(&path).expect("redb 2"));
    let outcome = store
        .recover_active_durable_restore_unbounded(&controller, |bytes| {
            let separator = bytes
                .iter()
                .position(|byte| *byte == 0)
                .ok_or_else(|| MemoryError::report("missing separator"))?;
            Ok((
                String::from_utf8(bytes[..separator].to_vec())
                    .map_err(|error| MemoryError::report(format!("invalid key: {error}")))?,
                bytes[separator + 1..].to_vec(),
            ))
        })
        .expect("restart recovery");
    assert!(matches!(outcome, DurableRestoreOutcome::Restored { sequence: 0 }));
    assert_eq!(controller.active_restore(), Ok(None));
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(store.reclaim_one().expect("read recovered").expect("entry").0, "prepared");
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);

    drop(controller);
    let _ = std::fs::remove_file(path);
}
#[cfg(feature = "spill-redb")]
#[test]
fn sep23_fup001_uncertain_commit_keeps_source_fenced_and_blocks_wrong_retire() {
    use crate::spill::backings::redb::RedbSpillBacking;
    use crate::spill::service::ReconciledDurableSpillTarget;
    use crate::spill::{
        DurableCommitFailure, DurableRecordId, DurableRecordResolution, DurableRestoreCandidate,
        DurableRestoreClaim, DurableRestoreController, DurableRestoreRecovery,
        DurableRestoreTarget, DurableSpillController, DurableWriteFailure,
        ReconciledDurableSpillController, SpillFailure, SpillTarget,
    };
    use std::sync::atomic::AtomicBool;

    /// Real redb backing with one scripted double-uncertain commit: the
    /// commit acknowledgement AND the follow-up probe both fail, so the
    /// store must fence the source. All resolution runs against the real
    /// persisted state.
    struct UncertainOnce {
        backing: RedbSpillBacking,
        fail_next_commit: AtomicBool,
    }
    impl SpillController<Vec<u8>> for UncertainOnce {
        fn spill(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            self.backing.accept(handle)
        }
        fn restore(&self) -> MemoryResult<Option<Vec<u8>>> {
            self.backing.restore()
        }
    }
    impl DurableSpillController<Vec<u8>> for UncertainOnce {}
    impl crate::spill::service::sealed::DurableSpillController<Vec<u8>> for UncertainOnce {}
    impl ReconciledDurableSpillController for UncertainOnce {
        fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
            self.backing.reserve_record_id()
        }
        fn commit_spill(
            &self,
            record_id: DurableRecordId,
            bytes: &[u8],
        ) -> Result<SpillReceipt, DurableWriteFailure> {
            if self.fail_next_commit.swap(false, Ordering::SeqCst) {
                let _ = bytes;
                return Err(DurableWriteFailure::RecoveryRequired {
                    record_id,
                    commit_error: Box::new(MemoryError::report(
                        "sep23 fup001 scripted lost commit ack",
                    )),
                    probe_error: Box::new(MemoryError::report("sep23 fup001 scripted lost probe")),
                });
            }
            self.backing.commit_spill(record_id, bytes)
        }
        fn resolve_record(
            &self,
            record_id: DurableRecordId,
        ) -> MemoryResult<DurableRecordResolution> {
            self.backing.resolve_record(record_id)
        }
    }
    impl DurableRestoreController for UncertainOnce {
        fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
            self.backing.prepare_restore()
        }
        fn commit_restore(&self, claim: &DurableRestoreClaim) -> Result<(), DurableCommitFailure> {
            self.backing.commit_restore(claim)
        }
        fn finalize_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
            self.backing.finalize_restore(claim)
        }
        fn abort_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
            self.backing.abort_restore(claim)
        }
        fn reconcile_restore(
            &self,
            claim: &DurableRestoreClaim,
        ) -> MemoryResult<DurableRecordResolution> {
            self.backing.reconcile_restore(claim)
        }
        fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
            self.backing.active_restore()
        }
        fn recover_active_restore(&self) -> MemoryResult<Option<DurableRestoreRecovery>> {
            self.backing.recover_active_restore()
        }
    }

    fn encode(key: &String, value: &Vec<u8>) -> MemoryResult<Vec<u8>> {
        let mut encoded = key.as_bytes().to_vec();
        encoded.push(0);
        encoded.extend(value);
        Ok(encoded)
    }
    fn decode(bytes: &[u8]) -> MemoryResult<(String, Vec<u8>)> {
        let separator = bytes
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| MemoryError::report("missing durable record separator"))?;
        let key = String::from_utf8(bytes[..separator].to_vec())
            .map_err(|error| MemoryError::report(format!("invalid key: {error}")))?;
        Ok((key, bytes[separator + 1..].to_vec()))
    }

    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-fup001-fenced-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let controller = UncertainOnce {
        backing: RedbSpillBacking::create(&path).expect("redb"),
        fail_next_commit: AtomicBool::new(true),
    };
    let broker = live_broker(1_000_000);
    let store_a = store_with_broker(&broker);
    let store_b = store_with_broker(&broker);

    store_a.insert_checked("fenced".into(), vec![7; 8]).expect("insert a");
    let total_a = store_a.committed_bytes().expect("a bytes");
    // The first commit goes double-uncertain: the source stays fenced AND
    // charged — nothing is retired on an unknown outcome.
    let spilled = store_a.spill_one_durable_unbounded(&controller, encode);
    assert!(
        matches!(spilled, Err(MemoryError::OperationInProgress(_))),
        "double-uncertain commit must fence, got {spilled:?}"
    );
    assert_eq!(store_a.pending_durable_spills().expect("pending"), vec![DurableRecordId::new(0)]);
    assert_eq!(store_a.committed_bytes().expect("still charged"), total_a);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), total_a);

    // A second store commits AND consumes a LATER id on the same backend.
    store_b.insert_checked("later".into(), vec![9; 8]).expect("insert b");
    let total_b = store_b.committed_bytes().expect("b bytes");
    let receipt = store_b
        .spill_one_durable_unbounded(&controller, encode)
        .expect("spill b")
        .expect("receipt b");
    assert_eq!(receipt, SpillReceipt::Durable { sequence: 1, inventory_after: 1 });
    let outcome = store_b.restore_one_durable_unbounded(&controller, decode).expect("restore b");
    assert!(matches!(outcome, DurableRestoreOutcome::Restored { sequence: 1 }));

    // Reconciling the fenced id-0 source must observe the HOLE (Absent):
    // the later finalize must not retire a source that never became
    // durable. The watermark oracle misread this as `Finalized`.
    let resolution =
        store_a.reconcile_durable_spill(&controller, DurableRecordId::new(0)).expect("reconcile");
    assert_eq!(resolution, DurableRecordResolution::Absent);
    assert!(store_a.pending_durable_spills().expect("drained").is_empty());
    assert_eq!(store_a.committed_bytes().expect("charge intact"), total_a);
    assert_eq!(
        broker.ledger_snapshot().expect("ledger intact").total_owner_bytes(),
        total_a + total_b
    );
    assert_eq!(store_a.reclaim_one().expect("read a").expect("entry a").0, "fenced");
    assert_eq!(store_b.reclaim_one().expect("read b").expect("entry b").0, "later");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);

    drop(controller);
    let _ = std::fs::remove_file(path);
}
