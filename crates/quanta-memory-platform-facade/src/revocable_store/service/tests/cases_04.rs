use super::*;

#[cfg(feature = "spill")]
#[test]
fn failed_durable_delete_rolls_back_provisional_source_charge() {
    struct DeleteFailTarget;
    impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for DeleteFailTarget {}
    impl SpillInventory for DeleteFailTarget {
        fn retained_count(&self) -> MemoryResult<u64> {
            Ok(1)
        }
    }
    impl SpillTarget<Vec<u8>> for DeleteFailTarget {
        fn is_available(&self) -> bool {
            true
        }
        fn accept(&self, _handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            unreachable!("restore test does not spill")
        }
    }
    impl DurableSpillTarget<Vec<u8>> for DeleteFailTarget {}
    impl DurableRestoreTarget for DeleteFailTarget {
        fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
            Ok(Some(DurableRestoreCandidate::for_test(
                9,
                b"first\0payload".to_vec(),
                DurableRestoreClaim::for_test(0xDE1, 1, 9),
            )))
        }
        fn commit_restore(
            &self,
            claim: &DurableRestoreClaim,
        ) -> Result<(), crate::spill::DurableCommitFailure> {
            assert_eq!(claim.sequence(), 9);
            Err(crate::spill::DurableCommitFailure::retained(MemoryError::report(
                "injected durable delete failure",
            )))
        }

        fn finalize_restore(&self, _claim: &DurableRestoreClaim) -> MemoryResult<()> {
            Ok(())
        }

        fn abort_restore(&self, _claim: &DurableRestoreClaim) -> MemoryResult<()> {
            Ok(())
        }

        fn reconcile_restore(
            &self,
            _claim: &DurableRestoreClaim,
        ) -> MemoryResult<DurableRecordResolution> {
            Ok(DurableRecordResolution::Present { inventory_after: 1 })
        }

        fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
            Ok(Some(DurableRecordId::new(9)))
        }

        fn recover_active_restore(
            &self,
        ) -> MemoryResult<Option<crate::spill::DurableRestoreRecovery>> {
            Ok(None)
        }
    }

    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    let controller = RoutedSpillController::new(DeleteFailTarget);
    assert!(matches!(
        store.restore_one_durable_unbounded(&controller, |_| {
            Ok(("first".to_string(), b"payload".to_vec()))
        }),
        Err(MemoryError::Report(_))
    ));
    assert_eq!(controller.spilled_count(), Ok(1));
    assert_eq!(store.stats().expect("not published"), StoreStats::default());
    assert_eq!(broker.ledger_snapshot().expect("charge rolled back").total_owner_bytes(), 0);
}
#[cfg(feature = "spill")]
#[test]
fn ambiguous_durable_delete_fences_source_until_absence_is_reconciled() {
    struct AmbiguousDeleteTarget {
        present: Arc<AtomicBool>,
    }
    impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for AmbiguousDeleteTarget {}
    impl SpillInventory for AmbiguousDeleteTarget {
        fn retained_count(&self) -> MemoryResult<u64> {
            Ok(u64::from(self.present.load(Ordering::SeqCst)))
        }
    }
    impl SpillTarget<Vec<u8>> for AmbiguousDeleteTarget {
        fn is_available(&self) -> bool {
            true
        }
        fn accept(&self, _handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            unreachable!("restore test does not spill")
        }
    }
    impl DurableSpillTarget<Vec<u8>> for AmbiguousDeleteTarget {}
    impl crate::spill::service::ReconciledDurableSpillTarget for AmbiguousDeleteTarget {
        fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
            unreachable!("restore test does not spill")
        }

        fn commit_spill(
            &self,
            _record_id: DurableRecordId,
            _bytes: &[u8],
        ) -> Result<SpillReceipt, DurableWriteFailure> {
            unreachable!("restore test does not spill")
        }

        fn resolve_record(
            &self,
            _record_id: DurableRecordId,
        ) -> MemoryResult<DurableRecordResolution> {
            if self.present.load(Ordering::SeqCst) {
                Ok(DurableRecordResolution::Present { inventory_after: 1 })
            } else {
                Ok(DurableRecordResolution::Absent)
            }
        }
    }
    impl DurableRestoreTarget for AmbiguousDeleteTarget {
        fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
            Ok(Some(DurableRestoreCandidate::for_test(
                11,
                b"record".to_vec(),
                DurableRestoreClaim::for_test(0xDE1, 1, 11),
            )))
        }
        fn commit_restore(
            &self,
            _claim: &DurableRestoreClaim,
        ) -> Result<(), crate::spill::DurableCommitFailure> {
            Err(crate::spill::DurableCommitFailure::ambiguous(MemoryError::report(
                "commit acknowledgement lost",
            )))
        }

        fn finalize_restore(&self, _claim: &DurableRestoreClaim) -> MemoryResult<()> {
            Ok(())
        }

        fn abort_restore(&self, _claim: &DurableRestoreClaim) -> MemoryResult<()> {
            Ok(())
        }

        fn reconcile_restore(
            &self,
            _claim: &DurableRestoreClaim,
        ) -> MemoryResult<DurableRecordResolution> {
            if self.present.load(Ordering::SeqCst) {
                Ok(DurableRecordResolution::Present { inventory_after: 1 })
            } else {
                Ok(DurableRecordResolution::Finalized)
            }
        }

        fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
            Ok(Some(DurableRecordId::new(11)))
        }

        fn recover_active_restore(
            &self,
        ) -> MemoryResult<Option<crate::spill::DurableRestoreRecovery>> {
            Ok(None)
        }
    }

    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    let present = Arc::new(AtomicBool::new(true));
    let controller =
        RoutedSpillController::new(AmbiguousDeleteTarget { present: Arc::clone(&present) });
    let outcome = store
        .restore_one_durable_unbounded(&controller, |_| {
            Ok(("restored".to_string(), b"payload".to_vec()))
        })
        .expect("ambiguity is a typed recovery outcome");
    assert!(matches!(outcome, DurableRestoreOutcome::RecoveryRequired { sequence: 11, .. }));
    assert_eq!(
        store.pending_durable_restores().expect("fenced IDs"),
        vec![DurableRecordId::new(11)]
    );
    assert!(store.reclaim_one().expect("fenced source is not readable").is_none());
    assert!(broker.ledger_snapshot().expect("source charged").total_owner_bytes() > 0);
    assert_eq!(controller.spilled_count(), Ok(1), "target state is intentionally uncertain");

    present.store(false, Ordering::SeqCst);
    assert_eq!(
        store
            .reconcile_durable_restore(&controller, DurableRecordId::new(11))
            .expect("absent durable record publishes resident source"),
        DurableRecordResolution::Finalized
    );
    assert!(store.pending_durable_restores().expect("resolved IDs").is_empty());
    assert_eq!(store.reclaim_one().expect("published source").expect("entry").0, "restored");
    assert_eq!(broker.ledger_snapshot().expect("released source").total_owner_bytes(), 0);
}
#[cfg(feature = "spill")]
#[test]
fn panicking_finalize_fences_published_source_until_reconciliation() {
    struct PanickingFinalizeTarget {
        finalize_calls: AtomicUsize,
    }

    impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for PanickingFinalizeTarget {}
    impl SpillInventory for PanickingFinalizeTarget {
        fn retained_count(&self) -> MemoryResult<u64> {
            Ok(1)
        }
    }
    impl SpillTarget<Vec<u8>> for PanickingFinalizeTarget {
        fn is_available(&self) -> bool {
            true
        }

        fn accept(&self, _handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            unreachable!("restore test does not spill")
        }
    }
    impl DurableSpillTarget<Vec<u8>> for PanickingFinalizeTarget {}
    impl DurableRestoreTarget for PanickingFinalizeTarget {
        fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
            Ok(Some(DurableRestoreCandidate::for_test(
                13,
                b"restored\0payload".to_vec(),
                DurableRestoreClaim::for_test(0xDE1, 1, 13),
            )))
        }

        fn commit_restore(
            &self,
            claim: &DurableRestoreClaim,
        ) -> Result<(), crate::spill::DurableCommitFailure> {
            assert_eq!(claim.sequence(), 13);
            Ok(())
        }

        fn finalize_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
            assert_eq!(claim.sequence(), 13);
            if self.finalize_calls.fetch_add(1, Ordering::SeqCst) < 2 {
                panic!("injected finalize panic");
            }
            Ok(())
        }

        fn abort_restore(&self, _claim: &DurableRestoreClaim) -> MemoryResult<()> {
            Ok(())
        }

        fn reconcile_restore(
            &self,
            claim: &DurableRestoreClaim,
        ) -> MemoryResult<DurableRecordResolution> {
            assert_eq!(claim.sequence(), 13);
            Ok(DurableRecordResolution::Finalized)
        }

        fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
            Ok(Some(DurableRecordId::new(13)))
        }

        fn recover_active_restore(
            &self,
        ) -> MemoryResult<Option<crate::spill::DurableRestoreRecovery>> {
            Ok(Some(crate::spill::DurableRestoreRecovery {
                candidate: DurableRestoreCandidate::for_test(
                    13,
                    b"restored\0payload".to_vec(),
                    DurableRestoreClaim::for_test(0xDE1, 2, 13),
                ),
                phase: crate::spill::DurableRestorePhase::Committed,
            }))
        }
    }

    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    let controller =
        RoutedSpillController::new(PanickingFinalizeTarget { finalize_calls: AtomicUsize::new(0) });

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store
            .restore_one_durable_unbounded(&controller, |bytes| {
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
            .expect("finalize panic occurs after resident preparation");
    }));
    assert!(panic.is_err());
    assert_eq!(
        store.pending_durable_restores().expect("fenced restore"),
        vec![DurableRecordId::new(13)]
    );
    assert!(store.reclaim_one().expect("fenced source is not readable").is_none());
    assert!(broker.ledger_snapshot().expect("source remains charged").total_owner_bytes() > 0);

    let reconcile_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store
            .reconcile_durable_restore(&controller, DurableRecordId::new(13))
            .expect("second finalize panic occurs after provisional publication");
    }));
    assert!(reconcile_panic.is_err());
    assert_eq!(
        store.pending_durable_restores().expect("refenced restore"),
        vec![DurableRecordId::new(13)]
    );
    assert!(store.reclaim_one().expect("refenced source is not readable").is_none());

    assert_eq!(
        store
            .reconcile_durable_restore(&controller, DurableRecordId::new(13))
            .expect("retry publishes and acknowledges the resident copy"),
        DurableRecordResolution::Finalized
    );
    assert!(store.pending_durable_restores().expect("resolved IDs").is_empty());
    assert_eq!(store.reclaim_one().expect("published source").expect("entry").0, "restored");
    assert_eq!(broker.ledger_snapshot().expect("released source").total_owner_bytes(), 0);
}
#[test]
fn reclaim_one_on_empty_returns_none() {
    let store = store();
    assert!(store.reclaim_one().expect("empty").is_none());
    assert_eq!(store.committed_bytes().expect("bytes"), 0);
}
#[test]
fn reclaim_until_frees_at_least_target_and_reports_reclaimed_bytes() {
    let store = store();
    for key in ["a", "b", "c"] {
        store.insert_checked(key.into(), vec![0; 4]).expect("insert");
    }
    let outcome = store.reclaim_until(expected_entry_cost("a", 4) + 1).expect("reclaim");
    assert_eq!(
        outcome,
        SweepStats { reclaimed_items: 2, reclaimed_bytes: 2 * expected_entry_cost("a", 4) }
    );
    assert_eq!(store.committed_bytes().expect("bytes"), expected_entry_cost("c", 4));
}
#[test]
fn reclaim_until_on_empty_store_reports_zero() {
    assert_eq!(
        store().reclaim_until(100).expect("empty"),
        SweepStats { reclaimed_items: 0, reclaimed_bytes: 0 }
    );
}
#[test]
fn reclaim_until_charges_and_evicts_zero_payload_keys() {
    let store = store();
    store.insert_checked("zero".into(), vec![]).expect("zero");
    store.insert_checked("a".into(), vec![0; 4]).expect("a");
    store.insert_checked("b".into(), vec![0; 4]).expect("b");
    let outcome = store.reclaim_until(expected_entry_cost("zero", 0) + 1).expect("reclaim");
    assert_eq!(
        outcome,
        SweepStats {
            reclaimed_items: 2,
            reclaimed_bytes: expected_entry_cost("zero", 0) + expected_entry_cost("a", 4)
        }
    );
    assert_eq!(store.stats().expect("stats").item_count, 1);
    assert_eq!(store.reclaim_one().expect("last remains").expect("entry").0, "b");
}
#[test]
fn reclaim_until_of_zero_payload_entries_reclaims_their_key_and_index_cost() {
    let store = store();
    for key in ["a", "b", "c"] {
        store.insert_checked(key.into(), vec![]).expect("insert");
    }
    assert_eq!(
        store.reclaim_until(5).expect("reclaim"),
        SweepStats { reclaimed_items: 1, reclaimed_bytes: expected_entry_cost("a", 0) }
    );
    assert_eq!(store.stats().expect("stats").item_count, 2);
}
#[test]
fn revoke_makes_entry_unretrievable_and_decrements_accounting() {
    let store = store();
    store.insert_checked("only".into(), vec![0; 9]).expect("insert");
    let (key, value) = store.reclaim_one().expect("revoke").expect("entry");
    assert_eq!((key.as_str(), value.len()), ("only", 9));
    assert_eq!(store.committed_bytes().expect("bytes"), 0);
    assert!(store.reclaim_one().expect("second revoke").is_none());
}
#[test]
fn empty_reclaim_leaves_state_byte_for_byte_unchanged() {
    let store = store();
    let before = store.stats().expect("before");
    assert!(store.reclaim_one().expect("empty").is_none());
    assert_eq!(store.stats().expect("after"), before);
}
#[test]
fn insert_over_cap_is_denied_and_fails_closed() {
    let broker = live_broker(5);
    let store = store_with_broker(&broker);
    let denied = store.insert_checked("big".into(), vec![0; 9]);
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(store.stats().expect("stats"), StoreStats::default());
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn large_key_with_zero_value_is_charged_and_denied_without_state_change() {
    let broker = live_broker(100);
    let store = store_with_broker(&broker);
    let denied = store.insert_checked("k".repeat(101), Vec::new());
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(store.stats().expect("unchanged state"), StoreStats::default());
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
}
#[test]
fn cost_component_sum_overflow_is_fail_closed_before_charge() {
    struct OversizedKey;
    impl ByteSized for OversizedKey {
        fn byte_size(&self) -> u64 {
            u64::MAX
        }
    }
    let broker = live_broker(1_000);
    let store = SimpleRevocableStore::<OversizedKey, Vec<u8>, Budgeted>::new_budgeted(
        binding(),
        broker.clone(),
    );
    assert!(matches!(store.insert_checked(OversizedKey, vec![1]), Err(MemoryError::Capacity(_))));
    assert_eq!(store.stats().expect("unchanged state"), StoreStats::default());
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
}
#[test]
fn insert_charges_and_reclaim_refunds_the_ledger() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("a".into(), vec![0; 4]).expect("a");
    store.insert_checked("b".into(), vec![0; 6]).expect("b");
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        expected_entry_cost("a", 4) + expected_entry_cost("b", 6)
    );
    store.reclaim_one().expect("one").expect("entry");
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        expected_entry_cost("b", 6)
    );
    store.reclaim_until(100).expect("rest");
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn close_refunds_every_live_entry_exactly_once_and_stays_closed() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("a".into(), vec![0; 4]).expect("a");
    store.insert_checked("b".into(), vec![0; 6]).expect("b");

    store.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    store.close().expect("idempotent close");
    assert!(matches!(store.insert_checked("late".into(), vec![0; 1]), Err(MemoryError::Closed(_))));
    assert!(matches!(store.reclaim_one(), Err(MemoryError::Closed(_))));
    assert_eq!(store.stats().expect("terminal stats"), StoreStats::default());
}
#[test]
fn close_release_failure_preserves_entries_fifo_and_ledger_authority() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("a".into(), vec![0; 4]).expect("a");
    store.insert_checked("b".into(), vec![0; 6]).expect("b");
    let rebuild_fence1 = broker.begin_rebuild().expect("begin rebuild");

    assert!(matches!(store.close(), Err(MemoryError::InvalidState(_))));
    assert_eq!(store.stats().expect("restored stats").item_count, 2);
    let expected = expected_entry_cost("a", 4) + expected_entry_cost("b", 6);
    assert_eq!(store.committed_bytes().expect("restored bytes"), expected);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);

    assert!(rebuild_fence1.finish().expect("finish rebuild").drain_error.is_none());
    assert_eq!(store.reclaim_one().expect("fifo").expect("entry").0, "a");
    store.close().expect("retry close");
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn reclaim_batch_failure_preserves_every_entry_and_charge() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    for (key, bytes) in [("a", 4), ("b", 4), ("c", 4)] {
        store.insert_checked(key.into(), vec![0; bytes]).expect("insert");
    }
    let rebuild_fence2 = broker.begin_rebuild().expect("begin rebuild");
    assert!(matches!(store.reclaim_until(5), Err(MemoryError::InvalidState(_))));
    assert_eq!(store.stats().expect("stats").item_count, 3);
    let expected = 3 * expected_entry_cost("a", 4);
    assert_eq!(store.committed_bytes().expect("bytes"), expected);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);
    assert!(rebuild_fence2.finish().expect("finish rebuild").drain_error.is_none());
    assert_eq!(store.reclaim_one().expect("fifo").expect("entry").0, "a");
}
#[test]
fn reclaim_one_during_rebuild_is_fail_closed() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("a".into(), vec![0; 4]).expect("a");
    store.insert_checked("b".into(), vec![0; 6]).expect("b");
    let rebuild_fence3 = broker.begin_rebuild().expect("begin rebuild");
    assert!(matches!(store.reclaim_one(), Err(MemoryError::InvalidState(_))));
    assert_eq!(store.stats().expect("stats").item_count, 2);
    let expected = expected_entry_cost("a", 4) + expected_entry_cost("b", 6);
    assert_eq!(store.committed_bytes().expect("bytes"), expected);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);
    assert!(rebuild_fence3.finish().expect("finish rebuild").drain_error.is_none());
}
#[test]
fn reclaim_until_during_rebuild_is_fail_closed() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    for key in ["a", "b", "c"] {
        store.insert_checked(key.into(), vec![0; 4]).expect("insert");
    }
    let rebuild_fence4 = broker.begin_rebuild().expect("begin rebuild");
    assert!(matches!(store.reclaim_until(5), Err(MemoryError::InvalidState(_))));
    assert_eq!(store.stats().expect("stats").item_count, 3);
    let expected = 3 * expected_entry_cost("a", 4);
    assert_eq!(store.committed_bytes().expect("bytes"), expected);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);
    assert!(rebuild_fence4.finish().expect("finish rebuild").drain_error.is_none());
}
#[test]
fn drop_without_explicit_close_releases_capacity() {
    let broker = live_broker(1_000_000);
    {
        let store = store_with_broker(&broker);
        store.insert_checked("live".into(), vec![0; 9]).expect("insert");
        assert_eq!(
            broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
            expected_entry_cost("live", 9)
        );
    }
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn drop_during_rebuild_defers_exact_release_until_finish() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("live".into(), vec![0; 9]).expect("insert");
    let rebuild_fence5 = broker.begin_rebuild().expect("begin rebuild");
    drop(store);
    assert_eq!(
        broker.ledger_snapshot().expect("conservative ledger").total_owner_bytes(),
        expected_entry_cost("live", 9)
    );
    assert!(rebuild_fence5.finish().expect("finish and drain").drain_error.is_none());
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn drop_during_outer_unwind_contains_two_client_panics_and_refunds_all_charges() {
    let broker = live_broker(1_000_000);
    let drops = Arc::new(AtomicUsize::new(0));
    let store = SimpleRevocableStore::<String, PanickingValue, Budgeted>::new_budgeted(
        binding(),
        broker.clone(),
    );
    for (key, bytes) in [("first", 4), ("second", 6)] {
        store
            .insert_checked(
                key.into(),
                PanickingValue { bytes: vec![0; bytes], drops: Arc::clone(&drops) },
            )
            .expect("insert before unwind");
    }
    assert_eq!(
        broker.ledger_snapshot().expect("before").total_owner_bytes(),
        expected_entry_cost("first", 4) + expected_entry_cost("second", 6)
    );

    let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _store = store;
        panic!("outer unwind");
    }))
    .expect_err("outer panic must propagate");

    assert_eq!(outer.downcast_ref::<&str>(), Some(&"outer unwind"));
    assert_eq!(drops.load(Ordering::SeqCst), 2, "both client values must be retired");
    assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
}
#[test]
fn concurrent_insert_and_close_have_one_linearized_outcome() {
    let broker = live_broker(1_000_000);
    let store = Arc::new(store_with_broker(&broker));
    let barrier = Arc::new(Barrier::new(2));
    let worker_store = Arc::clone(&store);
    let worker_barrier = Arc::clone(&barrier);
    let worker = thread::spawn(move || {
        worker_barrier.wait();
        worker_store.insert_checked("racing".into(), vec![0; 7])
    });

    barrier.wait();
    loop {
        match store.close() {
            Ok(()) => break,
            Err(MemoryError::OperationInProgress(_)) => thread::yield_now(),
            Err(error) => panic!("unexpected close error: {error}"),
        }
    }
    let insert = worker.join().expect("worker");
    assert!(
        insert.is_ok()
            || matches!(insert, Err(MemoryError::Closed(_) | MemoryError::OperationInProgress(_))),
        "a contending mutation may be rejected as Busy before it acquires the lifecycle lane"
    );
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    assert_eq!(store.stats().expect("terminal stats"), StoreStats::default());
}
#[test]
fn counter_overflow_is_rejected_before_broker_charge_or_mutation() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.state.lock().expect("state").committed_bytes = u64::MAX;

    let denied = store.insert_checked("overflow".into(), vec![0; 1]);
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(store.state.lock().expect("state").entries.len(), 0);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn poisoned_stats_is_error_not_empty_and_drop_still_releases() {
    let broker = live_broker(1_000_000);
    let store = Arc::new(store_with_broker(&broker));
    store.insert_checked("live".into(), vec![0; 5]).expect("insert");
    let poison = Arc::clone(&store);
    assert!(thread::spawn(move || {
        let _guard = poison.state.lock().expect("state");
        panic!("intentional poison");
    })
    .join()
    .is_err());

    assert!(matches!(store.stats(), Err(MemoryError::Poisoned(_))));
    drop(store);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn reentrant_read_and_close_under_state_lock_return_typed_busy() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("live".into(), vec![0; 4]).expect("insert");
    let held = store.state.lock().expect("simulated client callback holds state lock");
    assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
    drop(held);
    assert_eq!(
        store.stats().expect("open postimage").estimated_bytes,
        expected_entry_cost("live", 4)
    );
    store.close().expect("retry after callback");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn constructed_budgeted_store_exposes_its_single_broker_authority() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    assert!(store.broker().is_some());
}
#[test]
fn reclaim_eviction_order_is_deterministic_fifo_at_capacity_boundary() {
    let store = store();
    store.insert_checked("oldest".into(), vec![0; 4]).expect("oldest");
    store.insert_checked("middle".into(), vec![0; 4]).expect("middle");
    store.insert_checked("newest".into(), vec![0; 4]).expect("newest");
    assert_eq!(
        store.reclaim_until(1).expect("reclaim"),
        SweepStats { reclaimed_items: 1, reclaimed_bytes: expected_entry_cost("oldest", 4) }
    );
    assert_eq!(store.reclaim_one().expect("next").expect("entry").0, "middle");
}
#[cfg(feature = "unbudgeted-stores")]
#[test]
fn unbudgeted_store_is_a_distinct_explicit_product_type() {
    let store = SimpleRevocableStore::<String, Vec<u8>, Unbudgeted>::new_unbudgeted(binding());
    store.insert_checked("fixture".into(), vec![0; 3]).expect("insert");
    assert_eq!(store.stats().expect("stats").estimated_bytes, expected_entry_cost("fixture", 3));
    store.close().expect("close");
}
