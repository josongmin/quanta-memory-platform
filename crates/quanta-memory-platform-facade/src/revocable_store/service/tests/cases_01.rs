use super::*;

#[test]
fn sparse_reclaim_partition_and_rollback_preserve_large_fifo() {
    let count = 16_384usize;
    // The first shape is the ordinary oldest-half reclaim. The second
    // exercises sparse selection, which is also accepted by the partition
    // helpers used for exact rollback.
    for selected in [(0..count / 2).collect::<Vec<_>>(), (0..count).step_by(2).collect::<Vec<_>>()]
    {
        let mut entries = (0..count)
            .map(|key| RevocableEntry {
                key,
                value: (),
                charged_bytes: 1,
                charge: None,
                #[cfg(feature = "spill-durable")]
                restore_claim: None,
            })
            .collect::<VecDeque<_>>();
        let plan = extract_entries(&mut entries, &selected);
        assert_eq!(plan.len(), count / 2);
        let mut selected_flags = vec![false; count];
        for &index in &selected {
            selected_flags[index] = true;
        }
        assert!(entries
            .iter()
            .map(|entry| entry.key)
            .eq((0..count).filter(|index| !selected_flags[*index])));
        restore_entries(&mut entries, plan);
        assert!(entries.iter().map(|entry| entry.key).eq(0..count));
    }
}
#[test]
fn compound_measurement_and_drop_panic_does_not_abort_insert() {
    const CHILD: &str = "QMP_REVOCABLE_MEASURE_DROP_CHILD";
    struct HostileValue {
        measure: Arc<AtomicBool>,
        drop: Arc<AtomicBool>,
    }
    impl ByteSized for HostileValue {
        fn byte_size(&self) -> u64 {
            assert!(!self.measure.load(Ordering::SeqCst), "measurement trap");
            1
        }
    }
    impl Drop for HostileValue {
        fn drop(&mut self) {
            assert!(!self.drop.load(Ordering::SeqCst), "drop trap");
        }
    }
    if std::env::var_os(CHILD).is_some() {
        let broker = live_broker(1_000);
        let store = SimpleRevocableStore::<String, HostileValue, Budgeted>::new_budgeted(
            binding(),
            broker.clone(),
        );
        let measure = Arc::new(AtomicBool::new(true));
        let drop = Arc::new(AtomicBool::new(true));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.insert_checked(
                "x".into(),
                HostileValue { measure: Arc::clone(&measure), drop: Arc::clone(&drop) },
            )
        }));
        assert!(caught.is_err());
        measure.store(false, Ordering::SeqCst);
        drop.store(false, Ordering::SeqCst);
        assert_eq!(store.stats().expect("usable after panic").item_count, 0);
        assert_eq!(broker.ledger_snapshot().expect("uncharged").live_ticket_count, 0);
        store.close().expect("close after panic");
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("--exact")
        .arg("revocable_store::service::tests::cases_01::compound_measurement_and_drop_panic_does_not_abort_insert")
        .env(CHILD, "1")
        .require_one_test()
        .output()
        .expect("isolated measurement child");
    assert!(
        output.status.success(),
        "measurement+Drop must not abort: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn insert_tracks_reclaimable_bytes() {
    let store = store();
    store.insert_checked("a".into(), vec![0; 4]).expect("a");
    store.insert_checked("b".into(), vec![0; 6]).expect("b");
    let expected = expected_entry_cost("a", 4) + expected_entry_cost("b", 6);
    assert_eq!(store.committed_bytes().expect("bytes"), expected);
    assert_eq!(
        store.stats().expect("stats"),
        StoreStats { item_count: 2, estimated_bytes: expected }
    );
}
#[test]
fn reclaim_one_is_fifo_and_releases_its_bytes() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("first".to_string(), vec![0; 4]).expect("first");
    store.insert_checked("second".to_string(), vec![0; 6]).expect("second");
    let expected = expected_entry_cost("first", 4) + expected_entry_cost("second", 6);
    assert_eq!(store.stats().expect("stats").estimated_bytes, expected);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);

    let (key, value) = store.reclaim_one().expect("reclaim").expect("entry");
    assert_eq!((key.as_str(), value.len()), ("first", 4));
    assert_eq!(store.committed_bytes().expect("bytes"), expected_entry_cost("second", 6));
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        expected_entry_cost("second", 6)
    );
}
#[test]
fn same_store_reclaimer_returns_busy_without_losing_fifo() {
    use quanta_memory_platform_core::{
        MemoryError, MemoryReclaimer, MemoryResult, ReclaimOutcome, ReclaimRequest, ReclaimScope,
        ReclaimUnit,
    };
    use std::sync::OnceLock;

    struct SameStoreReclaimer {
        store: OnceLock<Arc<Store>>,
        inner_results: Mutex<Vec<MemoryResult<SweepStats>>>,
    }

    impl MemoryReclaimer for SameStoreReclaimer {
        fn reclaim(&self, request: &ReclaimRequest) -> MemoryResult<ReclaimOutcome> {
            let store = self.store.get().expect("store registered before use");
            // This fixture's only pressure is its configured class cap. Check
            // the victim scope and unit before interpreting the reduction.
            assert_eq!(request.target.unit, ReclaimUnit::Bytes);
            assert_eq!(
                request.target.scope,
                ReclaimScope::Class(MemoryClass::ExecutionScratchRevocable)
            );
            const WORK_BUDGET_BYTES: u64 = 64 * 1024;
            let target = request
                .target
                .minimum_reduction
                .unwrap_or(WORK_BUDGET_BYTES)
                .min(WORK_BUDGET_BYTES);
            // The inserting thread holds this store's mutation lane while
            // the broker runs the callback, so a synchronous same-store
            // reclaim observes typed Busy instead of blocking or evicting.
            let inner = store.reclaim_until(target);
            let busy = matches!(inner, Err(MemoryError::OperationInProgress(_)));
            self.inner_results.lock().expect("results").push(inner);
            if busy {
                return Err(MemoryError::operation_in_progress(
                    "same-store reclaim during its own admission",
                ));
            }
            Ok(ReclaimOutcome { reclaimed_bytes: 0 })
        }
    }

    let reclaimer = Arc::new(SameStoreReclaimer {
        store: OnceLock::new(),
        inner_results: Mutex::new(Vec::new()),
    });
    let broker = ProcessMemoryBrokerBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            ResidencyPolicy::new(10_000_000).with_class_budget(
                MemoryClass::ExecutionScratchRevocable,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: 100,
                    hard_cap_bytes: 100,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::First,
                },
            ),
        ))
        .with_reclaimer(reclaimer.clone() as Arc<dyn MemoryReclaimer>)
        .build()
        .expect("valid broker");
    let store = Arc::new(store_with_broker(&broker));
    assert!(reclaimer.store.set(Arc::clone(&store)).is_ok());

    // Two FIFO entries consume 74 of the 100-byte class budget.
    store.insert_checked("a".to_string(), vec![0; 4]).expect("a");
    store.insert_checked("b".to_string(), vec![0; 4]).expect("b");
    let live = expected_entry_cost("a", 4) + expected_entry_cost("b", 4);
    assert_eq!(live, 74);

    // The third insert is hard-cap denied. The broker callback runs while
    // this thread still owns the mutation lane, so the same-store reclaim
    // is typed Busy, no bytes are freed, and admission stays a Capacity
    // denial with the FIFO and charges byte-identical.
    let denied = store.insert_checked("c".to_string(), vec![0; 4]);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "hard-cap denial without freed bytes must surface as Capacity, got {denied:?}"
    );
    let inner_results = reclaimer.inner_results.lock().expect("results").clone();
    assert_eq!(inner_results.len(), 1, "exactly one reclaim callback");
    assert!(
        matches!(&inner_results[0], Err(MemoryError::OperationInProgress(_))),
        "same-store reclaim inside the admission callback must be typed Busy"
    );
    assert_eq!(store.stats().expect("stats preserved").estimated_bytes, live);
    assert_eq!(store.committed_bytes().expect("committed preserved"), live);
    assert_eq!(broker.ledger_snapshot().expect("ledger preserved").total_owner_bytes(), live);

    // Outside the denied admission, the same store reclaims its FIFO head
    // and the retried insert is admitted.
    let swept = store.reclaim_until(expected_entry_cost("a", 4)).expect("reclaim after denial");
    assert_eq!(swept.reclaimed_items, 1);
    assert_eq!(swept.reclaimed_bytes, expected_entry_cost("a", 4));
    store.insert_checked("c".to_string(), vec![0; 4]).expect("insert after reclaim");
    assert_eq!(store.stats().expect("retried stats").item_count, 2);

    // FIFO order survived the denial: the head is now the untouched "b".
    let (key, _) = store.reclaim_one().expect("reclaim").expect("b still ahead of c");
    assert_eq!(key.as_str(), "b");
    store.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[cfg(feature = "spill")]
#[test]
fn reclaim_to_in_memory_tier_keeps_bytes_charged() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("first".into(), vec![1; 4]).expect("first");
    store.insert_checked("second".into(), vec![2; 6]).expect("second");
    let total = expected_entry_cost("first", 4) + expected_entry_cost("second", 6);
    let controller = RoutedSpillController::new(InMemorySpillTarget::available());

    assert!(store.spill_one_into(&controller).expect("transfer"));
    assert_eq!(store.stats().expect("source stats").item_count, 1);
    assert_eq!(store.committed_bytes().expect("source bytes"), expected_entry_cost("second", 6));
    assert_eq!(controller.spilled_count(), Ok(1));
    assert_eq!(broker.ledger_snapshot().expect("retained tier charge").total_owner_bytes(), total);

    let handle = controller.restore().expect("restore").expect("retained handle");
    assert_eq!(handle.key(), "first");
    assert_eq!(handle.value(), &vec![1; 4]);
    assert_eq!(handle.charged_bytes(), expected_entry_cost("first", 4));
    assert_eq!(controller.spilled_count(), Ok(0));
    store.accept_restored(handle).expect("reattach original authority");
    assert_eq!(store.stats().expect("restored stats").item_count, 2);
    assert_eq!(broker.ledger_snapshot().expect("restored ledger").total_owner_bytes(), total);
    assert_eq!(store.reclaim_one().expect("remaining FIFO").expect("entry").0, "second");
    assert_eq!(store.reclaim_one().expect("restored entry").expect("entry").0, "first");
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
}
#[cfg(feature = "spill")]
#[test]
fn many_spills_share_large_binding_identity_without_unbilled_clones() {
    const ENTRY_COUNT: usize = 128;
    let broker = live_broker(10_000_000);
    // Owner identifiers now have a public 256-byte cap because they are
    // copied into live ledger authority. Keep this binding at that valid
    // boundary and retain the large runtime ID to test Arc-shared identity.
    let large_owner = "o".repeat(quanta_memory_platform_core::MAX_OWNER_KEY_BYTES);
    let large_runtime = "runtime-".to_string() + &"r".repeat(64 * 1024);
    let store = SimpleRevocableStore::new_budgeted(
        StoreBindingHandle::new(
            MemoryClass::ExecutionScratchRevocable,
            large_owner,
            Some(large_runtime),
        ),
        broker.clone(),
    );
    let controller = RoutedSpillController::new(InMemorySpillTarget::available());

    let mut expected_total = 0u64;
    for index in 0..ENTRY_COUNT {
        let key = format!("key-{index}");
        expected_total += expected_entry_cost(&key, 1);
        store.insert_checked(key, vec![index as u8]).expect("insert");
    }
    assert_eq!(
        broker.ledger_snapshot().expect("initial ledger").total_owner_bytes(),
        expected_total
    );

    for spilled in 1..=ENTRY_COUNT {
        assert!(store.spill_one_into(&controller).expect("spill"));
        assert_eq!(controller.spilled_count(), Ok(spilled as u64));
        assert_eq!(
            broker.ledger_snapshot().expect("spill ledger").total_owner_bytes(),
            expected_total,
            "in-process transfer must preserve the exact semantic charge"
        );
    }
    assert_eq!(store.committed_bytes(), Ok(0));

    let mut handles = Vec::with_capacity(ENTRY_COUNT);
    while let Some(handle) = controller.restore().expect("restore from tier") {
        assert!(
            Arc::ptr_eq(&handle.binding, &store.binding),
            "every spill handle must share the store identity allocation"
        );
        handles.push(handle);
    }
    assert_eq!(handles.len(), ENTRY_COUNT);
    assert_eq!(Arc::strong_count(&store.binding), ENTRY_COUNT + 1);
    assert_eq!(
        broker.ledger_snapshot().expect("restored handles ledger").total_owner_bytes(),
        expected_total
    );

    for handle in handles {
        store.accept_restored(handle).expect("reattach shared binding handle");
        assert_eq!(
            broker.ledger_snapshot().expect("reattach ledger").total_owner_bytes(),
            expected_total
        );
    }
    assert_eq!(Arc::strong_count(&store.binding), 1);
    drop(controller);
    assert_eq!(
        broker.ledger_snapshot().expect("empty tier drop ledger").total_owner_bytes(),
        expected_total
    );
    store.close().expect("close restored source");
    assert_eq!(broker.ledger_snapshot().expect("closed ledger").total_owner_bytes(), 0);
}
#[cfg(feature = "spill")]
#[test]
fn spill_and_close_emit_bounded_facade_events() {
    let events = Arc::new(RecordingEvents::default());
    let broker = live_broker_with_events(1_000_000, events.clone());
    let store = store_with_broker(&broker);
    store.insert_checked("first".into(), vec![1; 4]).expect("insert");
    let controller = RoutedSpillController::new(InMemorySpillTarget::available());

    assert!(store.spill_one_into(&controller).expect("spill"));
    store.close().expect("close empty source");

    let recorded = events.0.lock().expect("events").clone();
    assert!(recorded.contains(&MemoryEvent::new(
        MemoryOperation::SpillInProcess,
        MemoryOutcome::Succeeded,
        None,
    )));
    assert!(recorded.contains(&MemoryEvent::new(
        MemoryOperation::Close,
        MemoryOutcome::Succeeded,
        None,
    )));
}
#[cfg(feature = "spill")]
#[test]
fn failed_destination_write_preserves_source_entry_and_charge() {
    struct RejectingTarget;
    impl<Handle> crate::spill::service::sealed::InProcessSpillTarget<Handle> for RejectingTarget {}
    impl<Handle> SpillTarget<Handle> for RejectingTarget {
        fn is_available(&self) -> bool {
            true
        }
        fn accept(&self, handle: Handle) -> Result<SpillReceipt, SpillFailure<Handle>> {
            Err(SpillFailure::new(MemoryError::capacity("injected write failure"), handle))
        }
    }
    impl crate::spill::SpillInventory for RejectingTarget {
        fn retained_count(&self) -> MemoryResult<u64> {
            Ok(0)
        }
    }
    impl<Handle> InProcessSpillTarget<Handle> for RejectingTarget {}
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("first".into(), vec![1; 4]).expect("first");
    store.insert_checked("second".into(), vec![2; 6]).expect("second");
    let total = store.committed_bytes().expect("before");
    let controller = RoutedSpillController::new(RejectingTarget);

    assert!(matches!(store.spill_one_into(&controller), Err(MemoryError::Capacity(_))));
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(store.committed_bytes().expect("unchanged"), total);
    assert_eq!(store.stats().expect("source stats").item_count, 2);
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), total);
    assert_eq!(store.reclaim_one().expect("FIFO").expect("first").0, "first");
}
#[cfg(feature = "spill")]
#[test]
fn noop_spill_does_not_discard_source() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("first".into(), vec![1; 4]).expect("insert");
    let total = store.committed_bytes().expect("before");
    assert!(matches!(store.spill_one_into(&NoopSpillController), Err(MemoryError::Capacity(_))));
    assert_eq!(store.stats().expect("after").item_count, 1);
    assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), total);
    assert_eq!(store.reclaim_one().expect("source").expect("entry").0, "first");
}
#[cfg(feature = "spill")]
#[test]
fn foreign_store_cannot_adopt_spill_charge() {
    let broker = live_broker(1_000_000);
    let source = store_with_broker(&broker);
    let foreign = store_with_broker(&live_broker(1_000_000));
    source.insert_checked("first".into(), vec![1; 4]).expect("insert");
    let total = source.committed_bytes().expect("before");
    let controller = RoutedSpillController::new(InMemorySpillTarget::available());
    assert!(source.spill_one_into(&controller).expect("spill"));

    let handle = controller.restore().expect("restore").expect("entry");
    let (error, handle) =
        foreign.accept_restored(handle).expect_err("foreign broker must reject").into_parts();
    assert!(matches!(error, MemoryError::InvalidState(_)));
    assert_eq!(broker.ledger_snapshot().expect("original charge").total_owner_bytes(), total);
    source.accept_restored(handle).expect("original owner can restore");
    assert_eq!(source.stats().expect("source").item_count, 1);
    assert_eq!(broker.ledger_snapshot().expect("charge").total_owner_bytes(), total);
}
#[cfg(feature = "spill")]
#[test]
fn source_close_does_not_refund_live_in_process_tier() {
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("first".into(), vec![1; 4]).expect("insert");
    let total = store.committed_bytes().expect("before");
    let controller = RoutedSpillController::new(InMemorySpillTarget::available());

    assert!(store.spill_one_into(&controller).expect("spill"));
    store.close().expect("close source");
    assert_eq!(broker.ledger_snapshot().expect("tier authority").total_owner_bytes(), total);
    drop(controller);
    assert_eq!(broker.ledger_snapshot().expect("tier retired").total_owner_bytes(), 0);
}
#[cfg(feature = "spill")]
#[test]
fn closed_source_rejects_restored_handle_without_losing_its_charge() {
    let broker = live_broker(1_000_000);
    let source = store_with_broker(&broker);
    source.insert_checked("first".into(), vec![1; 4]).expect("insert");
    let charged = expected_entry_cost("first", 4);
    let controller = RoutedSpillController::new(InMemorySpillTarget::available());

    assert!(source.spill_one_into(&controller).expect("spill"));
    source.close().expect("close empty source");
    let handle = controller.restore().expect("restore from tier").expect("live handle");
    assert_eq!(handle.key(), "first");
    assert_eq!(handle.value(), &vec![1; 4]);
    assert_eq!(handle.charged_bytes(), charged);
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(broker.ledger_snapshot().expect("handle ledger").total_owner_bytes(), charged);

    let (error, handle) =
        source.accept_restored(handle).expect_err("source is closed").into_parts();
    assert!(matches!(error, MemoryError::Closed(_)));
    assert_eq!(handle.key(), "first", "rejected restore retains the original payload");
    assert_eq!(handle.charged_bytes(), charged);
    assert_eq!(source.stats().expect("closed source stats").item_count, 0);
    assert_eq!(
        broker.ledger_snapshot().expect("rejected handle ledger").total_owner_bytes(),
        charged
    );

    drop(controller);
    assert_eq!(
        broker.ledger_snapshot().expect("handle outlives tier").total_owner_bytes(),
        charged
    );
    drop(handle);
    assert_eq!(broker.ledger_snapshot().expect("retired handle ledger").total_owner_bytes(), 0);
}
#[cfg(feature = "spill-durable")]
#[test]
fn durable_spill_releases_source_only_after_commit() {
    struct ObservingDurableTarget {
        broker: LiveProcessMemoryBroker,
        expected_live: u64,
        retained: Mutex<Vec<Vec<u8>>>,
    }
    impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for ObservingDurableTarget {}
    impl SpillInventory for ObservingDurableTarget {
        fn retained_count(&self) -> MemoryResult<u64> {
            Ok(self.retained.lock().expect("retained").len() as u64)
        }
    }
    impl SpillTarget<Vec<u8>> for ObservingDurableTarget {
        fn is_available(&self) -> bool {
            true
        }
        fn accept(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            assert_eq!(
                self.broker.ledger_snapshot().expect("pre-commit ledger").total_owner_bytes(),
                self.expected_live,
                "source charge must remain live until durable accept commits"
            );
            let mut retained = self.retained.lock().expect("retained");
            retained.push(handle);
            Ok(SpillReceipt::Durable { sequence: 7, inventory_after: retained.len() as u64 })
        }
    }
    impl DurableSpillTarget<Vec<u8>> for ObservingDurableTarget {}
    impl crate::spill::service::ReconciledDurableSpillTarget for ObservingDurableTarget {
        fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
            Ok(DurableRecordId::new(7))
        }

        fn commit_spill(
            &self,
            record_id: DurableRecordId,
            bytes: &[u8],
        ) -> Result<SpillReceipt, DurableWriteFailure> {
            assert_eq!(record_id.get(), 7);
            assert_eq!(
                self.broker.ledger_snapshot().expect("pre-commit ledger").total_owner_bytes(),
                self.expected_live,
                "source charge must remain live until durable accept commits"
            );
            let mut retained = self.retained.lock().expect("retained");
            retained.push(bytes.to_vec());
            Ok(SpillReceipt::Durable {
                sequence: record_id.get(),
                inventory_after: retained.len() as u64,
            })
        }

        fn resolve_record(
            &self,
            _record_id: DurableRecordId,
        ) -> MemoryResult<DurableRecordResolution> {
            Ok(DurableRecordResolution::Present {
                inventory_after: self.retained.lock().expect("retained").len() as u64,
            })
        }
    }

    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("first".into(), vec![1; 4]).expect("insert");
    let total = store.committed_bytes().expect("before");
    let controller = RoutedSpillController::new(ObservingDurableTarget {
        broker: broker.clone(),
        expected_live: total,
        retained: Mutex::new(Vec::new()),
    });

    let receipt = store
        .spill_one_durable_unbounded(&controller, |key, value| {
            let mut bytes = key.as_bytes().to_vec();
            bytes.extend(value);
            Ok(bytes)
        })
        .expect("spill")
        .expect("entry");
    assert_eq!(receipt, SpillReceipt::Durable { sequence: 7, inventory_after: 1 });
    assert_eq!(store.stats().expect("source empty"), StoreStats::default());
    assert_eq!(broker.ledger_snapshot().expect("released after commit").total_owner_bytes(), 0);
    assert_eq!(controller.spilled_count(), Ok(1));
}
#[cfg(feature = "spill-durable")]
#[test]
fn failed_durable_write_and_encoder_error_restore_source_fifo_and_charge() {
    struct RejectingDurableTarget {
        reservations: Arc<AtomicUsize>,
    }
    impl crate::spill::service::sealed::DurableSpillTarget<Vec<u8>> for RejectingDurableTarget {}
    impl SpillInventory for RejectingDurableTarget {
        fn retained_count(&self) -> MemoryResult<u64> {
            Ok(0)
        }
    }
    impl SpillTarget<Vec<u8>> for RejectingDurableTarget {
        fn is_available(&self) -> bool {
            true
        }
        fn accept(&self, handle: Vec<u8>) -> Result<SpillReceipt, SpillFailure<Vec<u8>>> {
            Err(SpillFailure::new(MemoryError::capacity("injected durable failure"), handle))
        }
    }
    impl DurableSpillTarget<Vec<u8>> for RejectingDurableTarget {}
    impl crate::spill::service::ReconciledDurableSpillTarget for RejectingDurableTarget {
        fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
            self.reservations.fetch_add(1, Ordering::SeqCst);
            Ok(DurableRecordId::new(1))
        }

        fn commit_spill(
            &self,
            _record_id: DurableRecordId,
            _bytes: &[u8],
        ) -> Result<SpillReceipt, DurableWriteFailure> {
            Err(DurableWriteFailure::NotCommitted(Box::new(MemoryError::capacity(
                "injected durable failure",
            ))))
        }

        fn resolve_record(
            &self,
            _record_id: DurableRecordId,
        ) -> MemoryResult<DurableRecordResolution> {
            Ok(DurableRecordResolution::Absent)
        }
    }

    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    store.insert_checked("first".into(), vec![1; 4]).expect("first");
    store.insert_checked("second".into(), vec![2; 6]).expect("second");
    let total = store.committed_bytes().expect("before");
    let before = broker.ledger_snapshot().expect("before bounded denial");
    let reservations = Arc::new(AtomicUsize::new(0));
    let controller = RoutedSpillController::new(RejectingDurableTarget {
        reservations: Arc::clone(&reservations),
    });

    assert!(matches!(
        store.spill_one_durable_bounded(&controller),
        Err(MemoryError::Unsupported(_))
    ));
    assert_eq!(store.committed_bytes(), Ok(total));
    assert_eq!(broker.ledger_snapshot().expect("after bounded denial"), before);
    assert_eq!(reservations.load(Ordering::SeqCst), 0, "no record id may be reserved");

    assert!(matches!(
        store.spill_one_durable_unbounded(&controller, |_, _| Ok(vec![9])),
        Err(MemoryError::Capacity(_))
    ));
    assert_eq!(reservations.load(Ordering::SeqCst), 1);
    assert_eq!(store.committed_bytes().expect("write failure restored"), total);
    assert_eq!(broker.ledger_snapshot().expect("write failure ledger").total_owner_bytes(), total);

    assert!(matches!(
        store.spill_one_durable_unbounded(&controller, |_, _| Err(MemoryError::report(
            "encode failed"
        ))),
        Err(MemoryError::Report(_))
    ));
    assert_eq!(store.committed_bytes().expect("encoder failure restored"), total);
    assert_eq!(store.reclaim_one().expect("FIFO").expect("first").0, "first");
}
#[cfg(feature = "spill-redb")]
#[test]
fn bounded_codec_spill_preflights_backend_and_broker_before_allocating() {
    use crate::spill::backings::redb::{RedbSpillBacking, RedbSpillLimits};
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-bounded-spill-denial-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    assert!(!path.exists(), "test needs a fresh backing");
    let limits = RedbSpillLimits { max_record_bytes: 40, ..RedbSpillLimits::default() };
    let controller = RoutedSpillController::new(
        RedbSpillBacking::create_with_limits(&path, limits).expect("backing"),
    );
    let broker = live_broker(93);
    let store = store_with_broker(&broker);
    store.insert_checked("k".into(), vec![7; 20]).expect("source");
    let source_charge = store.committed_bytes().expect("source cost");
    assert_eq!(source_charge, 53);

    let denied = store.spill_one_durable_bounded(&controller);
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "{denied:?}");
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(store.pending_durable_spills(), Ok(Vec::new()));
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), source_charge);

    drop(controller);
    let controller = RoutedSpillController::new(
        RedbSpillBacking::create_with_limits(
            &path,
            RedbSpillLimits { max_record_bytes: 41, ..limits },
        )
        .expect("larger record cap"),
    );
    let denied = store.spill_one_durable_bounded(&controller);
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "{denied:?}");
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(store.pending_durable_spills(), Ok(Vec::new()));
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), source_charge);
    assert_eq!(store.reclaim_one().expect("source"), Some(("k".into(), vec![7; 20])));
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    drop(controller);
    std::fs::remove_file(path).expect("remove test backing");
}
#[cfg(feature = "spill-redb")]
#[test]
fn bounded_codec_cold_reopen_restore_denial_and_retry_keep_one_authority() {
    use crate::spill::backings::redb::RedbSpillBacking;
    static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "qmp-bounded-cold-reopen-{}-{}.redb",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    assert!(!path.exists(), "test needs a fresh backing");
    let broker = live_broker(1_000_000);
    let store = store_with_broker(&broker);
    let payload = vec![0xa5; 64];
    store.insert_checked("source".into(), payload.clone()).expect("source");
    let source_charge = store.committed_bytes().expect("source charge");
    {
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(&path).expect("backing"));
        assert!(matches!(
            store.spill_one_durable_bounded(&controller),
            Ok(Some(SpillReceipt::Durable { .. }))
        ));
        assert_eq!(controller.spilled_count(), Ok(1));
        assert_eq!(broker.ledger_snapshot().expect("released source").total_owner_bytes(), 0);
    }

    let controller =
        RoutedSpillController::new(RedbSpillBacking::create(&path).expect("cold reopen"));
    let tight_broker = live_broker(source_charge - 1);
    let tight_store = store_with_broker(&tight_broker);
    let denied = tight_store.restore_one_durable_bounded(&controller);
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "{denied:?}");
    assert_eq!(controller.spilled_count(), Ok(1));
    assert_eq!(controller.active_restore(), Ok(None));
    assert_eq!(tight_broker.ledger_snapshot().expect("denied ledger").total_owner_bytes(), 0);

    assert!(matches!(
        store.restore_one_durable_bounded(&controller),
        Ok(DurableRestoreOutcome::Restored { .. })
    ));
    assert_eq!(broker.ledger_snapshot().expect("restored").total_owner_bytes(), source_charge);
    assert_eq!(store.reclaim_one().expect("read"), Some(("source".into(), payload)));
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    drop(controller);
    std::fs::remove_file(path).expect("remove test backing");
}
