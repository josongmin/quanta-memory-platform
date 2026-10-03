use super::*;

#[test]
fn empty_value_with_huge_retained_key_is_denied_without_ledger_or_map_mutation() {
    let broker = live_broker(100);
    let store = charged(&broker);
    let huge_key = "k".repeat(1_024);
    assert!(matches!(
        store.insert_checked(huge_key.clone(), Vec::new()),
        Err(MemoryError::Capacity(_))
    ));
    assert_eq!(store.stats_checked().expect("unchanged stats"), StoreStats::default());
    assert_eq!(store.get_checked(&huge_key).expect("absent"), None);
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
}
#[test]
fn key_measurement_panic_precedes_accounting_and_preserves_state() {
    let broker = live_broker(1_000);
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "key-measure-panic-owner".into(), None);
    let store: SimpleRetainedMapStore<PanicMeasuredKey, Vec<u8>, Budgeted> =
        SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.insert_checked(PanicMeasuredKey(1), vec![1]);
    }));
    assert!(caught.is_err());
    assert_eq!(store.stats_checked().expect("state not poisoned"), StoreStats::default());
    assert_eq!(broker.ledger_snapshot().expect("no charge").total_owner_bytes(), 0);
}
#[test]
fn counter_reads_show_open_busy_and_closed_images() {
    let broker = live_broker(1_000_000);
    let store = charged(&broker);
    store.insert_checked("key".to_string(), vec![1, 2, 3]).expect("insert");
    // Open image: the plain counters mirror the authoritative ledger charge.
    let charged = broker.ledger_snapshot().expect("open ledger").total_owner_bytes();
    assert!(charged > 0, "seed entry must hold a live charge");
    assert_eq!(
        store.stats_checked().expect("open stats"),
        StoreStats { item_count: 1, estimated_bytes: charged }
    );
    assert_eq!(store.committed_bytes().expect("open bytes"), charged);

    // Busy image: while the mutation lane is held, counter reads are
    // typed contention — never an empty success masking a detached plan.
    let permit = store.lifecycle.begin_mutation().expect("own lane");
    assert!(matches!(store.stats_checked(), Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(store.committed_bytes(), Err(MemoryError::OperationInProgress(_))));
    drop(permit);

    // Closed image: the terminal empty image stays readable and consistent.
    store.close().expect("close");
    assert_eq!(store.stats_checked().expect("closed stats"), StoreStats::default());
    assert_eq!(store.committed_bytes().expect("closed bytes"), 0);
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
}
#[test]
fn overwrite_preserves_retained_original_key_cost_and_exact_ledger() {
    let broker = live_broker(100);
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "alias-key-owner".into(), None);
    let store: SimpleRetainedMapStore<AliasKey, Vec<u8>, Budgeted> =
        SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
    let original = AliasKey { id: 1, declared_bytes: 2 };
    store.insert_checked(original.clone(), vec![1; 4]).expect("seed");
    let initial = 4 + 2 + 24 + 16;
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), initial);
    let equal_but_huge = AliasKey { id: 1, declared_bytes: 1_024 };
    store.insert_checked(equal_but_huge, vec![2; 6]).expect("original key remains retained");
    let expected = initial + 2;
    assert_eq!(store.committed_bytes().expect("store"), expected);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), expected);
    assert_eq!(store.get_checked(&original).expect("lookup"), Some(vec![2; 6]));
}
#[test]
fn aggregate_cost_overflow_is_denied_before_new_entry_or_ledger_charge() {
    let broker = live_broker(1_000);
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "overflow-owner".into(), None);
    let store: SimpleRetainedMapStore<String, DeclaredBytesV1, Budgeted> =
        SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
    assert!(matches!(
        store.insert_checked("k".into(), DeclaredBytesV1 { marker: 1, retained_bytes: u64::MAX }),
        Err(MemoryError::Capacity(_))
    ));
    assert_eq!(store.stats_checked().expect("unchanged stats"), StoreStats::default());
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
}
#[test]
fn panic_in_old_value_drop_cannot_split_committed_bytes_from_ledger() {
    let broker = live_broker(1_000);
    let binding = StoreBindingHandle::new(MemoryClass::MemoResidency, "panic-owner".into(), None);
    let store: SimpleRetainedMapStore<String, PanicOnRetire, Budgeted> =
        SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
    let panic_once = Arc::new(AtomicBool::new(true));
    store
        .insert_checked(
            "key".into(),
            PanicOnRetire { bytes: 4, panic_once: Arc::clone(&panic_once) },
        )
        .expect("seed entry");

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.insert_checked(
            "key".into(),
            PanicOnRetire { bytes: 8, panic_once: Arc::new(AtomicBool::new(false)) },
        )
    }));
    assert!(caught.is_err(), "client destructor did panic");
    assert_eq!(
        store.stats_checked().expect("complete postimage").estimated_bytes,
        retained_cost("key", 8)
    );
    assert_eq!(store.committed_bytes().expect("committed postimage"), retained_cost("key", 8));
    assert_eq!(
        broker.ledger_snapshot().expect("ledger postimage").total_owner_bytes(),
        retained_cost("key", 8)
    );
    store.close().expect("close remains possible after client panic");
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
}
#[test]
fn close_contains_client_destructor_panic_and_keeps_terminal_image() {
    let broker = live_broker(1_000);
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "close-panic-owner".into(), None);
    let store: SimpleRetainedMapStore<String, PanicOnRetire, Budgeted> =
        SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
    store
        .insert_checked(
            "key".into(),
            PanicOnRetire { bytes: 9, panic_once: Arc::new(AtomicBool::new(true)) },
        )
        .expect("seed entry");
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.close()));
    assert!(caught.expect("client destructor must not escape close").is_ok());
    assert_eq!(store.stats_checked().expect("terminal image").item_count, 0);
    assert_eq!(store.committed_bytes().expect("terminal bytes"), 0);
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
    store.close().expect("double close is idempotent after client panic");
    assert!(matches!(
        store.insert_checked(
            "late".into(),
            PanicOnRetire { bytes: 1, panic_once: Arc::new(AtomicBool::new(false)) }
        ),
        Err(MemoryError::Closed(_))
    ));
}
#[test]
fn drop_during_unwind_contains_two_client_destructor_panics_and_releases_all() {
    let broker = live_broker(1_000);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "double-panic-owner".into(), None);
        let store: SimpleRetainedMapStore<String, PanicOnRetire, Budgeted> =
            SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
        for key in ["a", "b"] {
            store
                .insert_checked(
                    key.into(),
                    PanicOnRetire { bytes: 3, panic_once: Arc::new(AtomicBool::new(true)) },
                )
                .expect("charged entry");
        }
        panic!("outer unwind");
    }));
    assert!(caught.is_err());
    assert_eq!(broker.ledger_snapshot().expect("all leases released").total_owner_bytes(), 0);
}
#[test]
fn sweep_never_hashes_client_key_after_refund() {
    let broker = live_broker(1_000);
    let binding = StoreBindingHandle::new(
        MemoryClass::MemoResidency,
        "sweep-hash-after-refund-owner".into(),
        None,
    );
    let store: SimpleRetainedMapStore<AssertChargedDuringHashKey, Vec<u8>, Budgeted> =
        SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
    let armed = Arc::new(AtomicBool::new(false));
    let callbacks = Arc::new(AtomicUsize::new(0));
    store
        .insert_checked(
            AssertChargedDuringHashKey {
                id: 1,
                broker: broker.clone(),
                armed: Arc::clone(&armed),
                callbacks: Arc::clone(&callbacks),
            },
            vec![7; 5],
        )
        .expect("charged insert");
    armed.store(true, Ordering::SeqCst);
    let swept = store.sweep(SweepHint { target_bytes: 5 }).expect("sweep succeeds");
    assert_eq!(swept.reclaimed_bytes, retained_cost("x1234567", 5));
    assert_eq!(callbacks.load(Ordering::SeqCst), 1);
    assert_eq!(store.stats_checked().expect("postimage").item_count, 0);
    assert_eq!(broker.ledger_snapshot().expect("refunded ledger").total_owner_bytes(), 0);
}
#[test]
fn sweep_hash_panic_before_refund_preserves_charged_preimage() {
    let broker = live_broker(1_000);
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "sweep-panic-owner".into(), None);
    let store: SimpleRetainedMapStore<PanicAfterSweepPlanKey, Vec<u8>, Budgeted> =
        SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
    let hash_must_panic = Arc::new(AtomicBool::new(false));
    store
        .insert_checked(
            PanicAfterSweepPlanKey { id: 1, hash_must_panic: Arc::clone(&hash_must_panic) },
            vec![7; 5],
        )
        .expect("charged insert");
    hash_must_panic.store(true, Ordering::SeqCst);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.sweep(SweepHint { target_bytes: 5 })
    }));
    assert!(result.is_err(), "client Hash panic occurs before refund");
    assert_eq!(
        store.stats_checked().expect("charged preimage").estimated_bytes,
        retained_cost("x1234567", 5)
    );
    assert_eq!(
        broker.ledger_snapshot().expect("charged ledger").total_owner_bytes(),
        retained_cost("x1234567", 5)
    );
    hash_must_panic.store(false, Ordering::SeqCst);
    let stats = store.sweep(SweepHint { target_bytes: 5 }).expect("retry succeeds");
    assert_eq!(stats.reclaimed_bytes, retained_cost("x1234567", 5));
    assert_eq!(store.stats_checked().expect("postimage").estimated_bytes, 0);
    assert_eq!(broker.ledger_snapshot().expect("postimage ledger").total_owner_bytes(), 0);
}
#[test]
fn panic_in_sweep_value_drop_is_contained_after_exact_refund() {
    let broker = live_broker(1_000);
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "sweep-value-panic-owner".into(), None);
    let store: SimpleRetainedMapStore<String, PanicOnRetire, Budgeted> =
        SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
    store
        .insert_checked(
            "key".into(),
            PanicOnRetire { bytes: 5, panic_once: Arc::new(AtomicBool::new(true)) },
        )
        .expect("charged insert");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.sweep(SweepHint { target_bytes: 5 })
    }));
    assert_eq!(
        result.expect("client destructor panic is contained").expect("sweep"),
        SweepStats { reclaimed_items: 1, reclaimed_bytes: retained_cost("key", 5) }
    );
    assert_eq!(store.stats_checked().expect("postimage").item_count, 0);
    assert_eq!(store.committed_bytes().expect("postimage bytes"), 0);
    assert_eq!(broker.ledger_snapshot().expect("postimage ledger").total_owner_bytes(), 0);
}
#[test]
fn sweep_multiple_panicking_destructors_does_not_abort_process() {
    const CHILD: &str = "QMP_RETAINED_SWEEP_MULTI_PANIC_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let broker = live_broker(1_000);
        let binding = StoreBindingHandle::new(
            MemoryClass::MemoResidency,
            "sweep-multi-panic-owner".into(),
            None,
        );
        let store: SimpleRetainedMapStore<PanicAfterSweepDropKey, PanicOnRetire, Budgeted> =
            SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
        let mut key_panics = Vec::new();
        let mut value_panics = Vec::new();
        for (key, bytes, panic) in [("survivor", 7, false), ("a", 5, true), ("b", 5, true)] {
            let key_panic = Arc::new(AtomicBool::new(false));
            let value_panic = Arc::new(AtomicBool::new(panic));
            store
                .insert_checked(
                    PanicAfterSweepDropKey { id: key, panic_once: Arc::clone(&key_panic) },
                    PanicOnRetire { bytes, panic_once: Arc::clone(&value_panic) },
                )
                .expect("seed charged entry");
            key_panics.push(key_panic);
            value_panics.push(value_panic);
        }
        key_panics[1].store(true, Ordering::SeqCst);
        key_panics[2].store(true, Ordering::SeqCst);
        struct SweepOnOuterUnwind<'a> {
            store: &'a SimpleRetainedMapStore<PanicAfterSweepDropKey, PanicOnRetire, Budgeted>,
            result: &'a mut Option<Result<SweepStats, MemoryError>>,
        }

        impl Drop for SweepOnOuterUnwind<'_> {
            fn drop(&mut self) {
                *self.result =
                    Some(self.store.sweep(SweepHint { target_bytes: retained_cost("a", 5) + 1 }));
            }
        }

        let mut result = None;
        let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _sweep = SweepOnOuterUnwind { store: &store, result: &mut result };
            panic!("outer unwind before sweep destructor");
        }));
        assert!(outer.is_err(), "outer unwind reached sweep");
        let swept = result.expect("sweep ran during outer unwind").expect("sweep completes");
        assert_eq!(swept.reclaimed_items, 2);
        assert_eq!(swept.reclaimed_bytes, retained_cost("a", 5) + retained_cost("b", 5));
        assert_eq!(store.stats_checked().expect("complete postimage").item_count, 1);
        assert_eq!(
            store.committed_bytes().expect("complete postimage"),
            retained_cost("survivor", 7)
        );
        assert_eq!(
            broker.ledger_snapshot().expect("exact ledger").total_owner_bytes(),
            retained_cost("survivor", 7)
        );
        assert!(key_panics[1..].iter().all(|flag| !flag.load(Ordering::SeqCst)));
        assert!(value_panics[1..].iter().all(|flag| !flag.load(Ordering::SeqCst)));
        store.close().expect("survivor remains closable");
        assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("--exact")
        .arg("retained_map::service::tests::charged::cases_01::sweep_multiple_panicking_destructors_does_not_abort_process")
        .arg("--nocapture")
        .env(CHILD, "1")
        .require_one_test()
        .output()
        .expect("run isolated destructor-panic scenario");
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "sweep must not abort or unwind with two client Drop panics: status={:?}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn rebuild_denied_recharge_preserves_charged_preimage_v1() {
    let broker = live_broker(1_000);
    let store = charged(&broker);
    store.insert_checked("key".to_string(), vec![1u8; 30]).expect("seed charged entry");
    let rebuild_fence1 = broker.begin_rebuild().expect("begin rebuild");
    let error = store
        .insert_checked("key".to_string(), vec![2u8; 20])
        .expect_err("fenced recharge must fail closed");
    assert!(matches!(error, MemoryError::InvalidState(_)));
    assert_eq!(
        store.get_checked(&"key".to_string()).expect("checked read after recovery"),
        Some(vec![1u8; 30]),
        "managed recharge denial preserves the charged preimage"
    );
    assert_eq!(
        store.stats_checked().expect("checked stats").estimated_bytes,
        retained_cost("key", 30)
    );
    assert_eq!(
        broker.ledger_snapshot().expect("ledger after recovery").total_owner_bytes(),
        retained_cost("key", 30),
        "store and ledger retain the same charge authority"
    );
    assert!(rebuild_fence1.finish().expect("finish rebuild").drain_error.is_none());
}
#[test]
fn charged_insert_routes_bytes_through_the_ledger() {
    // (a) charged mode: the ledger (SSOT) sees the map's real bytes, an
    // overwrite re-charges to the new size without double-counting, and
    // remove refunds the ledger.
    let broker = live_broker(1_000_000);
    let store = charged(&broker);

    store.insert_checked("k".to_string(), vec![0u8; 100]).expect("insert within cap");
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        retained_cost("k", 100),
        "the ledger must see the charged map's bytes"
    );

    // Overwrite in place with a SMALLER value: the ledger tracks the live
    // charge only (40), never the sum of both inserts.
    store.insert_checked("k".to_string(), vec![0u8; 40]).expect("overwrite");
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        retained_cost("k", 40),
        "an overwrite must re-charge to the new size, not double-count"
    );

    // remove refunds the live charge back to the ledger.
    store.remove(&"k".to_string()).expect("remove");
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        0,
        "remove must refund the ledger"
    );
}
#[test]
fn atomic_insert_if_absent_never_recharges_or_overwrites_an_existing_entry_v1() {
    let broker = live_broker(100);
    let store = charged(&broker);
    store.insert_checked("canonical".to_string(), vec![1u8; 30]).expect("seed");

    // The losing candidate is larger than the remaining budget. An
    // occupied decision must happen before charging, so it still returns
    // AlreadyPresent and leaves both authorities byte-for-byte intact.
    assert_eq!(
        store
            .insert_if_absent_checked("canonical".to_string(), vec![2u8; 60])
            .expect("occupied publication is not an admission attempt"),
        RetainedMapInsertOutcomeV1::AlreadyPresent
    );
    assert_eq!(
        store.get_checked(&"canonical".to_string()).expect("checked read"),
        Some(vec![1u8; 30]),
        "a losing publisher must not replace the canonical payload"
    );
    assert_eq!(
        store.stats_checked().expect("checked stats").estimated_bytes,
        retained_cost("canonical", 30)
    );
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        retained_cost("canonical", 30),
        "a losing publisher must not charge or refund the ledger"
    );
}
#[test]
fn denied_atomic_insert_if_absent_publishes_neither_payload_nor_charge_v1() {
    let broker = live_broker(100);
    let store = charged(&broker);
    store.insert_checked("keep".to_string(), vec![0u8; 40]).expect("seed");
    let ledger_before = broker.ledger_snapshot().expect("snap").total_owner_bytes();
    let stats_before = store.stats_checked().expect("checked stats before");

    let denied = store.insert_if_absent_checked("denied".to_string(), vec![0u8; 20]);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "a denied atomic publication must preserve the typed error: {denied:?}"
    );
    assert_eq!(store.stats_checked().expect("checked stats after"), stats_before);
    assert_eq!(
        store.get_checked(&"denied".to_string()).expect("checked absent read"),
        None,
        "charge denial must not publish a payload"
    );
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        ledger_before,
        "charge denial must not mutate the accounting authority"
    );
}
#[test]
fn charged_insert_over_cap_is_denied_and_fails_closed() {
    // (b) over-cap insert is denied and fails closed: capture ledger + store
    // state before the denied op and assert both are byte-for-byte unchanged.
    let broker = live_broker(100); // MemoResidency hard cap 100 semantic bytes
    let store = charged(&broker);

    // Seed a surviving entry so the denied op cannot quietly drop it.
    store.insert_checked("keep".to_string(), vec![0u8; 40]).expect("seed within cap");
    let ledger_before = broker.ledger_snapshot().expect("snap").total_owner_bytes();
    let committed_before = store.committed_bytes().expect("committed before");
    let stats_before = store.stats();
    assert_eq!(ledger_before, retained_cost("keep", 40));

    // Both retained key/index charges make the combined image exceed the cap.
    let denied = store.insert_checked("big".to_string(), vec![0u8; 20]);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "an over-cap insert must be a typed capacity error, got {denied:?}"
    );
    // Fail-closed: ledger, store counter, and stats all unchanged; the
    // surviving entry is intact.
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        ledger_before,
        "a denied insert must leave the ledger byte-for-byte unchanged"
    );
    assert_eq!(store.committed_bytes().expect("committed after"), committed_before);
    assert_eq!(store.stats(), stats_before);
    assert_eq!(store.get(&"keep".to_string()), Some(vec![0u8; 40]), "survivor intact");
}
#[test]
fn charged_overwrite_denial_leaves_existing_entry_unchanged() {
    // (b, overwrite variant) a denied overwrite of an existing key must NOT
    // disturb the existing entry's value or its ledger charge — fail-closed
    // before any mutation or refund of the old charge.
    let broker = live_broker(100);
    let store = charged(&broker);
    store.insert_checked("k".to_string(), vec![1u8; 30]).expect("seed");
    let ledger_before = broker.ledger_snapshot().expect("snap").total_owner_bytes();
    assert_eq!(ledger_before, retained_cost("k", 30));

    // Re-insert the same key with a value that, charged BEFORE the old refund,
    // would exceed the semantic cap: must be denied.
    let denied = store.insert_checked("k".to_string(), vec![2u8; 60]);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "over-cap overwrite must be denied, got {denied:?}"
    );
    // The old entry, its value, and its ledger charge survive untouched.
    assert_eq!(store.get(&"k".to_string()), Some(vec![1u8; 30]), "old value survives");
    assert_eq!(store.committed_bytes().expect("committed"), retained_cost("k", 30));
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        ledger_before,
        "a denied overwrite must not refund the old charge"
    );
}
#[test]
fn entry_remains_refundable_after_a_denied_overwrite() {
    // After a denied overwrite the entry must still hold a LIVE ledger
    // ticket (the original, or the broker-restored one) — never a stranded
    // non-live ticket. Proven end-to-end: a follow-up remove drains the
    // ledger to zero (a non-live ticket would be rejected and strand bytes).
    let broker = live_broker(100);
    let store = charged(&broker);
    store.insert_checked("k".to_string(), vec![1u8; 30]).expect("seed");

    let denied = store.insert_checked("k".to_string(), vec![2u8; 60]);
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "over-cap overwrite denied");
    assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), retained_cost("k", 30));

    // The surviving entry's ticket is refundable: removing it drains the ledger.
    let removed = store.remove(&"k".to_string()).expect("remove must succeed");
    assert_eq!(removed, Some(vec![1u8; 30]), "remove returns the surviving value");
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        0,
        "the post-denial ticket was live and refundable (no strand)"
    );
}
#[test]
fn charged_overwrite_during_rebuild_is_fail_closed_and_leaves_entry_intact() {
    // A charged overwrite while the broker is rebuilding must fail closed
    // (the recharge's release-old hits the rebuild gate), leaving the old
    // value and its ledger charge byte-for-byte intact — and the entry's
    // ticket still live so the value survives and is refundable after.
    let broker = live_broker(1_000_000);
    let store = charged(&broker);
    store.insert_checked("k".to_string(), vec![1u8; 40]).expect("seed");
    assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), retained_cost("k", 40));

    let rebuild_fence2 = broker.begin_rebuild().expect("begin rebuild");
    let denied = store.insert_checked("k".to_string(), vec![2u8; 20]);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "overwrite during rebuild must be rejected, got {denied:?}"
    );
    assert_eq!(store.get(&"k".to_string()), Some(vec![1u8; 40]), "old value intact");
    assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), retained_cost("k", 40));

    // After rebuild the entry is still live: it overwrites and removes cleanly.
    assert!(rebuild_fence2.finish().expect("finish").drain_error.is_none());
    store.insert_checked("k".to_string(), vec![2u8; 20]).expect("overwrite resumes");
    assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), retained_cost("k", 20));
    store.remove(&"k".to_string()).expect("remove");
    assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
}
#[test]
fn charged_sweep_refunds_reclaimed_bytes_to_the_ledger() {
    // sweep must refund every reclaimed entry's bytes to the ledger.
    let broker = live_broker(1_000_000);
    let store = charged(&broker);
    store.insert_checked("a".to_string(), vec![0u8; 5]).expect("a");
    store.insert_checked("b".to_string(), vec![0u8; 5]).expect("b");
    store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        3 * retained_cost("a", 5)
    );

    let stats = store.sweep(SweepHint { target_bytes: retained_cost("a", 5) + 1 }).expect("sweep");
    assert_eq!(stats.reclaimed_items, 2, "two retained entries cover the target");
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        retained_cost("a", 5),
        "sweep must refund the reclaimed bytes to the ledger"
    );
}
#[test]
fn charged_full_eviction_leaves_no_entry_key_ticket_or_accounting_residue_v1() {
    let broker = live_broker(1_000_000);
    let store = charged(&broker);
    store.insert_checked("a".to_string(), vec![0u8; 2]).expect("a");
    store.insert_checked("b".to_string(), vec![0u8; 3]).expect("b");
    store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
    let full_cost = retained_cost("a", 2) + retained_cost("b", 3) + retained_cost("c", 5);
    assert_eq!(broker.ledger_snapshot().expect("before").total_owner_bytes(), full_cost);

    let swept = store.sweep(SweepHint { target_bytes: full_cost }).expect("full sweep");
    assert_eq!(swept, SweepStats { reclaimed_items: 3, reclaimed_bytes: full_cost });
    assert_eq!(store.stats_checked().expect("checked empty stats"), StoreStats::default());
    assert!(store.keys().is_empty(), "full eviction leaves no reachable key");
    assert!(store.entries().is_empty(), "full eviction leaves no reachable entry");
    assert_eq!(store.committed_bytes().expect("empty committed bytes"), 0);
    assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
    assert!(
        store.state.lock().expect("state").entries.is_empty(),
        "the canonical owner contains no entry and therefore no live ticket"
    );
}
#[test]
fn charged_remove_during_rebuild_is_fail_closed() {
    let broker = live_broker(1_000_000);
    let store = charged(&broker);
    store.insert_checked("k".to_string(), vec![0u8; 7]).expect("seed");
    let _rebuild_fence3 = broker.begin_rebuild().expect("begin rebuild");

    let denied = store.remove(&"k".to_string());
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "remove during rebuild must fail closed, got {denied:?}"
    );
    assert_eq!(store.get(&"k".to_string()), Some(vec![0u8; 7]), "entry must survive denial");
    assert_eq!(store.committed_bytes().expect("committed"), retained_cost("k", 7));
    assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), retained_cost("k", 7));
}
#[test]
fn charged_remove_rejects_a_ticketless_entry_without_mutating_state() {
    let broker = live_broker(1_000_000);
    let store = charged(&broker);
    store.insert_checked("k".to_string(), vec![0u8; 7]).expect("seed");
    let held = store
        .state
        .lock()
        .expect("state")
        .entries
        .get_mut("k")
        .expect("entry")
        .charge
        .take()
        .expect("budgeted charge");
    let denied = store.remove(&"k".to_string());
    assert!(matches!(denied, Err(MemoryError::InvalidState(_))), "got {denied:?}");
    assert_eq!(store.get(&"k".to_string()), Some(vec![0u8; 7]));
    assert_eq!(store.committed_bytes().expect("committed"), retained_cost("k", 7));
    assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), retained_cost("k", 7));
    store.state.lock().expect("state").entries.get_mut("k").expect("entry").charge = Some(held);
    store.close().expect("cleanup recovered charge");
    assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
}
