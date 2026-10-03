use super::*;

#[test]
fn insert_commits_envelope_bytes() {
    let store = store();
    store.insert_checked("k1".to_string(), vec![1u8, 2, 3]).expect("insert");
    // The committed charge includes the retained key and index, not just the value.
    assert_eq!(store.committed_bytes().expect("committed"), retained_cost("k1", 3));
    assert_eq!(
        store.stats(),
        StoreStats { item_count: 1, estimated_bytes: retained_cost("k1", 3) }
    );
    assert_eq!(store.get(&"k1".to_string()), Some(vec![1u8, 2, 3]));
}
#[test]
fn committed_byte_overflow_rejects_before_payload_or_accounting_mutation_v1() {
    let store = declared_bytes_store_v1();
    let original = DeclaredBytesV1 { marker: 1, retained_bytes: 1 };
    store.insert_checked("stable".to_string(), original.clone()).expect("first insert");
    store.insert_checked("other".to_string(), original.clone()).expect("second insert");

    let replacement = DeclaredBytesV1 { marker: 9, retained_bytes: u64::MAX };
    let overwrite_error = store
        .insert_checked("stable".to_string(), replacement)
        .expect_err("overflowing overwrite must fail closed");
    assert!(matches!(overwrite_error, MemoryError::Capacity(_)));
    assert_eq!(store.get_checked(&"stable".to_string()).expect("stable read"), Some(original));
    assert_eq!(
        store.committed_bytes().expect("unchanged committed bytes"),
        retained_cost("stable", 1) + retained_cost("other", 1)
    );
    assert_eq!(store.stats_checked().expect("unchanged stats").item_count, 2);

    let max_store = declared_bytes_store_v1();
    max_store
        .insert_if_absent_checked(
            "max".to_string(),
            DeclaredBytesV1 { marker: 7, retained_bytes: u64::MAX - retained_cost("max", 0) },
        )
        .expect("the first maximum-sized declaration is representable");
    let insert_error = max_store
        .insert_if_absent_checked(
            "overflow".to_string(),
            DeclaredBytesV1 { marker: 8, retained_bytes: 1 },
        )
        .expect_err("overflowing vacant insert must fail closed");
    assert!(matches!(insert_error, MemoryError::Capacity(_)));
    assert_eq!(max_store.get_checked(&"overflow".to_string()).expect("absent read"), None);
    assert_eq!(max_store.committed_bytes().expect("maximum committed bytes"), u64::MAX);
    assert_eq!(max_store.stats_checked().expect("maximum stats").item_count, 1);
}
#[test]
fn concurrent_insert_if_absent_has_one_creator_and_never_overwrites_v1() {
    const WRITERS: usize = 12;
    let store = Arc::new(store());
    let barrier = Arc::new(Barrier::new(WRITERS));
    let mut joins = Vec::new();
    for writer in 0..WRITERS {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        joins.push(std::thread::spawn(move || {
            barrier.wait();
            let writer_byte = u8::try_from(writer).expect("writer index fits u8");
            for _ in 0..10_000 {
                match store.insert_if_absent_checked("canonical".to_string(), vec![writer_byte]) {
                    Ok(outcome) => return outcome,
                    Err(MemoryError::OperationInProgress(_)) => std::thread::yield_now(),
                    Err(error) => panic!("unexpected insert failure: {error}"),
                }
            }
            panic!("mutation lane never became available")
        }));
    }
    let outcomes: Vec<_> =
        joins.into_iter().map(|join| join.join().expect("writer joins")).collect();
    assert_eq!(
        outcomes.iter().filter(|outcome| **outcome == RetainedMapInsertOutcomeV1::Inserted).count(),
        1,
        "exactly one caller owns creation"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == RetainedMapInsertOutcomeV1::AlreadyPresent)
            .count(),
        WRITERS - 1,
        "all losing callers observe an existing row"
    );
    assert_eq!(store.stats().item_count, 1);
    assert_eq!(store.committed_bytes().expect("committed bytes"), retained_cost("canonical", 1));
}
#[test]
fn concurrent_checked_read_observes_only_complete_old_or_new_payloads_v1() {
    const READERS: usize = 4;
    const ITERATIONS: usize = 2_000;
    const PAYLOAD_BYTES: usize = 4_096;

    let store = Arc::new(store());
    let key = "concurrent-overwrite".to_string();
    let old = vec![0x11; PAYLOAD_BYTES];
    let new = vec![0xee; PAYLOAD_BYTES];
    store.insert_checked(key.clone(), old.clone()).expect("seed old payload");

    let barrier = Arc::new(Barrier::new(READERS + 1));
    let old_observations = Arc::new(AtomicUsize::new(0));
    let new_observations = Arc::new(AtomicUsize::new(0));
    let invalid_observations = Arc::new(AtomicUsize::new(0));
    let mut readers = Vec::new();
    for _ in 0..READERS {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let key = key.clone();
        let old = old.clone();
        let new = new.clone();
        let old_observations = Arc::clone(&old_observations);
        let new_observations = Arc::clone(&new_observations);
        let invalid_observations = Arc::clone(&invalid_observations);
        readers.push(std::thread::spawn(move || {
            for _ in 0..ITERATIONS {
                // Reusable two-phase barrier: every read races exactly one
                // overwrite epoch instead of merely starting near the writer.
                barrier.wait();
                let mut observed = None;
                for _ in 0..10_000 {
                    match store.get_checked(&key) {
                        Err(MemoryError::OperationInProgress(_)) => {
                            std::thread::yield_now();
                        }
                        outcome => {
                            observed = Some(outcome);
                            break;
                        }
                    }
                }
                match observed.expect("checked read must leave the in-flight lane") {
                    Ok(Some(observed)) if observed == old => {
                        old_observations.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Some(observed)) if observed == new => {
                        new_observations.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Some(_)) | Ok(None) | Err(_) => {
                        invalid_observations.fetch_add(1, Ordering::Relaxed);
                    }
                }
                barrier.wait();
            }
        }));
    }

    let mut write_failures = 0usize;
    for iteration in 0..ITERATIONS {
        barrier.wait();
        let payload = if iteration % 2 == 0 { new.clone() } else { old.clone() };
        let mut published = false;
        for _ in 0..10_000 {
            match store.insert_checked(key.clone(), payload.clone()) {
                Ok(()) => {
                    published = true;
                    break;
                }
                Err(MemoryError::OperationInProgress(_)) => std::thread::yield_now(),
                Err(_) => break,
            }
        }
        if !published {
            write_failures += 1;
        }
        barrier.wait();
    }
    for reader in readers {
        reader.join().expect("reader joins without invalid observation");
    }

    assert_eq!(
        old_observations.load(Ordering::Relaxed) + new_observations.load(Ordering::Relaxed),
        READERS * ITERATIONS,
        "every checked read must classify as one complete published payload"
    );
    assert_eq!(write_failures, 0, "every synchronized overwrite must publish successfully");
    assert_eq!(
        invalid_observations.load(Ordering::Relaxed),
        0,
        "no checked read may observe absence, error, torn bytes, or a foreign generation"
    );
    assert!(
        old_observations.load(Ordering::Relaxed) > 0
            && new_observations.load(Ordering::Relaxed) > 0,
        "the synchronized overwrite epochs must expose both complete generations"
    );
}
#[test]
fn compound_hostile_remove_panic_cannot_abort_process() {
    if std::env::var_os(COMPOUND_CHILD).is_some() {
        compound_scenario_remove();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("--exact")
        .arg("retained_map::service::tests::cases_01::compound_hostile_remove_panic_cannot_abort_process")
        .arg("--nocapture")
        .env(COMPOUND_CHILD, "remove")
        .require_one_test()
        .output()
        .expect("isolated compound-hostile remove child");
    assert!(
        output.status.success(),
        "a compound hostile caller type must not abort remove: status={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn compound_hostile_insert_panic_cannot_abort_process() {
    if std::env::var_os(COMPOUND_CHILD).is_some() {
        compound_scenario_insert();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("--exact")
        .arg("retained_map::service::tests::cases_01::compound_hostile_insert_panic_cannot_abort_process")
        .arg("--nocapture")
        .env(COMPOUND_CHILD, "insert")
        .require_one_test()
        .output()
        .expect("isolated compound-hostile insert child");
    assert!(
        output.status.success(),
        "a compound hostile caller type must not abort insert: status={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn read_hash_callback_reentry_is_typed_busy_and_preserves_exact_postimage() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};

    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("broker");
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-read-reentry".to_string(), None);
    let store = Arc::new(ReentrantReadStore::new_budgeted(binding, broker.clone()));
    let armed = Arc::new(AtomicBool::new(false));
    let callbacks = Arc::new(AtomicUsize::new(0));
    let owner = Arc::new(OnceLock::new());
    owner.set(Arc::downgrade(&store)).expect("install weak store once");
    let key = ReentrantReadKey {
        id: 7,
        armed: Arc::clone(&armed),
        store: owner,
        callbacks: Arc::clone(&callbacks),
    };
    store.insert_checked(key.clone(), vec![1, 2, 3]).expect("seed");
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        retained_cost("x", 3)
    );

    armed.store(true, Ordering::SeqCst);
    assert_eq!(store.get_checked(&key).expect("outer read"), Some(vec![1, 2, 3]));
    armed.store(false, Ordering::SeqCst);
    assert_eq!(callbacks.load(Ordering::SeqCst), 1, "lookup must invoke the client hash");
    assert_eq!(
        store.stats_checked().expect("published state"),
        StoreStats { item_count: 1, estimated_bytes: retained_cost("x", 3) }
    );
    assert_eq!(
        broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(),
        retained_cost("x", 3)
    );
    store.close().expect("closing phase restored after reentry");
    assert_eq!(broker.ledger_snapshot().expect("released ledger").total_owner_bytes(), 0);
}
#[test]
fn sweep_hash_callback_reentry_is_typed_busy_before_refund() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};

    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("broker");
    let binding = StoreBindingHandle::new(
        MemoryClass::MemoResidency,
        "owner-sweep-reentry".to_string(),
        None,
    );
    let store = Arc::new(ReentrantReadStore::new_budgeted(binding, broker.clone()));
    let armed = Arc::new(AtomicBool::new(false));
    let callbacks = Arc::new(AtomicUsize::new(0));
    let owner = Arc::new(OnceLock::new());
    owner.set(Arc::downgrade(&store)).expect("install weak store once");
    let key = ReentrantReadKey {
        id: 8,
        armed: Arc::clone(&armed),
        store: owner,
        callbacks: Arc::clone(&callbacks),
    };
    store.insert_checked(key, vec![1, 2, 3]).expect("charged preimage");
    assert_eq!(
        broker.ledger_snapshot().expect("preimage ledger").total_owner_bytes(),
        retained_cost("x", 3)
    );

    armed.store(true, Ordering::SeqCst);
    let swept = store.sweep(SweepHint { target_bytes: 3 }).expect("sweep succeeds");
    assert_eq!(swept.reclaimed_bytes, retained_cost("x", 3));
    assert_eq!(callbacks.load(Ordering::SeqCst), 1, "planning invokes client Hash once");
    assert_eq!(store.stats_checked().expect("complete postimage").item_count, 0);
    assert_eq!(broker.ledger_snapshot().expect("refunded ledger").total_owner_bytes(), 0);
}
#[test]
fn hash_unwind_during_detached_mutation_restores_owned_state_and_charge() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};

    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("broker");
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-hash-unwind".to_string(), None);
    let store: SimpleRetainedMapStore<PanickingHashKey, Vec<u8>, Budgeted> =
        SimpleRetainedMapStore::new_budgeted(binding, broker.clone());
    let armed = Arc::new(AtomicBool::new(false));
    let key = PanickingHashKey { id: 1, armed: Arc::clone(&armed) };
    store.insert_checked(key.clone(), vec![1, 2, 3]).expect("seed");
    armed.store(true, Ordering::SeqCst);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.insert_if_absent_checked(
            PanickingHashKey { id: 2, armed: Arc::clone(&armed) },
            vec![9],
        );
    }));
    assert!(panic.is_err(), "hash callback must unwind after state extraction");
    armed.store(false, Ordering::SeqCst);

    assert_eq!(store.get_checked(&key).expect("restored read"), Some(vec![1, 2, 3]));
    assert_eq!(
        store.stats_checked().expect("restored stats").estimated_bytes,
        retained_cost("x", 3)
    );
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        retained_cost("x", 3)
    );
    store.close().expect("terminal release");
    assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
}
#[test]
fn checked_reads_propagate_client_clone_panics_without_poisoning_owner_mutex_v1() {
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-poison".to_string(), None);
    let store: UnbudgetedTestStore<String, ClonePanicsWhenArmed> =
        SimpleRetainedMapStore::new_unbudgeted_fixture(binding);
    let armed = Arc::new(AtomicBool::new(false));
    let key = "poison".to_string();
    store
        .insert_checked(
            key.clone(),
            ClonePanicsWhenArmed { bytes: vec![1, 2, 3], armed: Arc::clone(&armed) },
        )
        .expect("seed panic-capable payload");

    armed.store(true, Ordering::SeqCst);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.get_checked(&key);
    }));
    assert!(panic.is_err(), "clone panic must remain visible to the caller");
    armed.store(false, Ordering::SeqCst);

    assert!(
        store.get_checked(&key).expect("owner lock remains usable after caller panic").is_some(),
        "checked point read remains available after an isolated clone panic"
    );
    assert!(
        store.stats_checked().is_ok(),
        "checked stats must remain available after an isolated clone panic"
    );
    assert!(
        store.keys_checked().is_ok(),
        "checked key enumeration must remain available after an isolated clone panic"
    );
    assert!(
        store.entries_checked().is_ok(),
        "checked entry enumeration must remain available after an isolated clone panic"
    );

    armed.store(true, Ordering::SeqCst);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.entries_checked();
    }));
    assert!(panic.is_err(), "entry enumeration must propagate client clone panic");
    armed.store(false, Ordering::SeqCst);
    assert!(
        store.stats_checked().is_ok(),
        "entry-clone panic must release the read guard without poisoning the owner mutex"
    );
}
#[test]
fn partial_entry_read_clone_and_drop_panics_cannot_abort() {
    const CHILD: &str = "QMP_RETAINED_PARTIAL_READ_CHILD";
    struct HostileReadValue {
        calls: Arc<AtomicUsize>,
        drop_panics: Arc<AtomicBool>,
    }
    impl Clone for HostileReadValue {
        fn clone(&self) -> Self {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            assert_ne!(call, 2, "second value clone trap");
            Self { calls: Arc::clone(&self.calls), drop_panics: Arc::clone(&self.drop_panics) }
        }
    }
    impl ByteSized for HostileReadValue {
        fn byte_size(&self) -> u64 {
            1
        }
    }
    impl Drop for HostileReadValue {
        fn drop(&mut self) {
            assert!(!self.drop_panics.load(Ordering::SeqCst), "partial clone drop trap");
        }
    }
    if std::env::var_os(CHILD).is_some() {
        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "owner:partial-read".into(), None);
        let store: UnbudgetedTestStore<String, HostileReadValue> =
            SimpleRetainedMapStore::new_unbudgeted_fixture(binding);
        let calls = Arc::new(AtomicUsize::new(0));
        let drop_panics = Arc::new(AtomicBool::new(false));
        for key in ["a", "b"] {
            store
                .insert_checked(
                    key.into(),
                    HostileReadValue {
                        calls: Arc::clone(&calls),
                        drop_panics: Arc::clone(&drop_panics),
                    },
                )
                .expect("seed");
        }
        drop_panics.store(true, Ordering::SeqCst);
        let caught =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.entries_checked()));
        assert!(caught.is_err(), "later Clone must panic");
        drop_panics.store(false, Ordering::SeqCst);
        assert_eq!(store.stats_checked().expect("read after unwind").item_count, 2);
        store.close().expect("close after unwind");
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("retained_map::service::tests::cases_01::partial_entry_read_clone_and_drop_panics_cannot_abort")
            .env(CHILD, "1")
            .require_one_test()
.output()
            .expect("isolated partial-read child");
    assert!(
        output.status.success(),
        "partial read must not abort: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn put_get_remove_get_none_round_trip_corrects_bytes() {
    // scenario F: Simple* store put -> get -> remove -> get(None) round-trip
    // with byte-accounting correction at each step.
    let store = store();
    // get on an empty store is None and accounting is zero.
    assert_eq!(store.get(&"k".to_string()), None);
    assert_eq!(store.committed_bytes().expect("empty"), 0);

    store.insert_checked("k".to_string(), vec![1u8, 2, 3, 4]).expect("put");
    // get returns the stored value; bytes reflect exactly the value size.
    assert_eq!(store.get(&"k".to_string()), Some(vec![1u8, 2, 3, 4]));
    assert_eq!(store.committed_bytes().expect("after put"), retained_cost("k", 4));

    let removed = store.remove(&"k".to_string()).expect("remove");
    assert_eq!(removed, Some(vec![1u8, 2, 3, 4]), "remove returns the stored value");
    // After remove: get(None) and bytes corrected back to zero.
    assert_eq!(store.get(&"k".to_string()), None, "removed key must not be retrievable");
    assert_eq!(store.committed_bytes().expect("after remove"), 0);
    assert_eq!(store.stats(), StoreStats { item_count: 0, estimated_bytes: 0 });
}
#[test]
fn equal_hash_key_duplicate_overwrites_and_does_not_double_count() {
    // scenario F (audit U): two keys equal under Eq but not bit-identical
    // ("Key" vs "KEY") must collapse to one live entry with one live charge.
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-partialeq".to_string(), None);
    let store: UnbudgetedTestStore<CaseInsensitiveKey, Vec<u8>> =
        SimpleRetainedMapStore::new_unbudgeted_fixture(binding);

    store
        .insert_checked(CaseInsensitiveKey { raw: "Key".to_string() }, vec![0u8; 10])
        .expect("first put");
    assert_eq!(store.committed_bytes().expect("c1"), retained_cost("Key", 10));
    assert_eq!(store.stats().item_count, 1);

    // Re-put under an equal-but-not-identical key form: overwrite in place.
    store
        .insert_checked(CaseInsensitiveKey { raw: "KEY".to_string() }, vec![0u8; 3])
        .expect("overwrite put");
    assert_eq!(
        store.committed_bytes().expect("c2"),
        retained_cost("Key", 3),
        "Eq-equal duplicate must overwrite, never double-count bytes"
    );
    assert_eq!(store.stats().item_count, 1, "equal keys must collapse to one entry");
    // The new value is observable under either equal key form.
    assert_eq!(
        store.get(&CaseInsensitiveKey { raw: "key".to_string() }),
        Some(vec![0u8; 3]),
        "get under any equal key form returns the overwritten value"
    );
}
#[test]
fn first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1() {
    exercise_endpoint_point_operations_without_linear_probe_growth(0);
    exercise_endpoint_point_operations_without_linear_probe_growth(4_095);
}
#[test]
fn reinsert_same_key_recharges_to_new_size_not_doubled() {
    let store = store();
    store.insert_checked("k".to_string(), vec![0u8; 10]).expect("insert");
    assert_eq!(store.committed_bytes().expect("c1"), retained_cost("k", 10));
    // Replace with a smaller value: committed bytes track the live charge only,
    // never the sum of both inserts.
    store.insert_checked("k".to_string(), vec![0u8; 4]).expect("reinsert");
    assert_eq!(
        store.committed_bytes().expect("c2"),
        retained_cost("k", 4),
        "replacement must not double-charge the key"
    );
    assert_eq!(store.stats().item_count, 1);
}
#[test]
fn overwrite_drop_reentry_is_bounded_and_other_store_sees_exact_ledger() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};

    const CHILD: &str = "QMP_RETAINED_OVERWRITE_DROP_REENTRY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("overwrite_drop_reentry_is_bounded_and_other_store_sees_exact_ledger")
            .env(CHILD, "1")
            .require_one_test()
            .spawn()
            .expect("spawn bounded reentry test");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().expect("poll child") {
                assert!(status.success(), "reentry child failed: {status}");
                return;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().expect("terminate deadlocked child");
                child.wait().expect("reap child");
                panic!("reentry child exceeded bounded deadline");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("broker");
    let home = Arc::new(DropReentryStore::new_budgeted(
        StoreBindingHandle::new(MemoryClass::MemoResidency, "drop-home".into(), None),
        broker.clone(),
    ));
    let peer = Arc::new(DropReentryStore::new_budgeted(
        StoreBindingHandle::new(MemoryClass::MemoResidency, "drop-peer".into(), None),
        broker.clone(),
    ));
    let home_slot = Arc::new(OnceLock::new());
    let peer_slot = Arc::new(OnceLock::new());
    home_slot.set(Arc::downgrade(&home)).expect("install home");
    peer_slot.set(Arc::downgrade(&peer)).expect("install peer");
    let hits = Arc::new(AtomicUsize::new(0));
    let armed = Arc::new(AtomicBool::new(false));
    let value = |id, bytes, armed| DropReentryValue {
        id,
        bytes,
        armed,
        home: Arc::clone(&home_slot),
        peer: Arc::clone(&peer_slot),
        hits: Arc::clone(&hits),
    };
    home.insert_checked("home".to_string(), value(1, 4, Arc::clone(&armed))).expect("seed");
    armed.store(true, Ordering::SeqCst);
    home.insert_checked("home".to_string(), value(2, 7, Arc::new(AtomicBool::new(false))))
        .expect("overwrite and retire old value");

    assert_eq!(hits.load(Ordering::SeqCst), 1, "old value drop ran exactly once");
    assert_eq!(home.get_checked(&"home".to_string()).expect("home read").expect("row").id, 2);
    assert_eq!(peer.get_checked(&"peer".to_string()).expect("peer read").expect("row").id, 3);
    let expected_home = retained_cost("home", 7);
    let expected_peer = retained_cost("peer", 2);
    assert_eq!(home.stats_checked().expect("home stats").estimated_bytes, expected_home);
    assert_eq!(peer.stats_checked().expect("peer stats").estimated_bytes, expected_peer);
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        expected_home + expected_peer
    );
    home.close().expect("close home");
    peer.close().expect("close peer");
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
}
#[test]
fn remove_releases_committed_bytes() {
    let store = store();
    store.insert_checked("k".to_string(), vec![9u8; 7]).expect("insert");
    let removed = store.remove(&"k".to_string()).expect("remove");
    assert_eq!(removed, Some(vec![9u8; 7]));
    // NEGATIVE-PAIR: removing the only entry must drop committed bytes to 0.
    assert_eq!(store.committed_bytes().expect("released"), 0);
    assert_eq!(store.stats().item_count, 0);
    // Removing again returns None and keeps accounting at 0.
    assert_eq!(store.remove(&"k".to_string()).expect("remove missing"), None);
    assert_eq!(store.committed_bytes().expect("still released"), 0);
}
#[test]
fn sweep_reclaims_at_least_target_bytes_and_releases_them() {
    let store = store();
    store.insert_checked("a".to_string(), vec![0u8; 5]).expect("a");
    store.insert_checked("b".to_string(), vec![0u8; 5]).expect("b");
    store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
    assert_eq!(store.committed_bytes().expect("before"), 3 * retained_cost("a", 5));

    let stats = store.sweep(SweepHint { target_bytes: retained_cost("a", 5) + 1 }).expect("sweep");
    // Must reclaim until >= target: two retained entries cover a one-entry-plus-one target.
    assert_eq!(stats.reclaimed_items, 2);
    assert_eq!(stats.reclaimed_bytes, 2 * retained_cost("a", 5));
    assert_eq!(store.committed_bytes().expect("after sweep"), retained_cost("a", 5));
    assert_eq!(store.stats().item_count, 1);
}
#[test]
fn overwrite_preserves_original_insertion_position_for_newest_first_sweep() {
    let store = store();
    store.insert_checked("a".to_string(), vec![1]).expect("a");
    store.insert_checked("b".to_string(), vec![2]).expect("b");
    store.insert_checked("c".to_string(), vec![3]).expect("c");

    // Updating b must not turn it into the newest entry.
    store.insert_checked("b".to_string(), vec![9, 9]).expect("overwrite b");
    let swept = store.sweep(SweepHint { target_bytes: 1 }).expect("sweep newest");

    assert_eq!(swept, SweepStats { reclaimed_items: 1, reclaimed_bytes: retained_cost("c", 1) });
    assert_eq!(store.get(&"b".to_string()), Some(vec![9, 9]), "overwritten b stays resident");
    assert_eq!(store.get(&"c".to_string()), None, "original newest c is evicted first");
    assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string()]);
}
#[test]
fn arbitrary_remove_preserves_remaining_newest_first_sweep_order() {
    let store = store();
    for key in ["a", "b", "c", "d"] {
        store.insert_checked(key.to_string(), vec![1]).expect("seed");
    }

    assert_eq!(store.remove(&"b".to_string()).expect("remove b"), Some(vec![1]));
    store.sweep(SweepHint { target_bytes: 1 }).expect("sweep d");
    assert_eq!(store.get(&"d".to_string()), None, "d remains newest after removing b");
    assert!(store.get(&"c".to_string()).is_some(), "c must survive the first sweep");

    store.sweep(SweepHint { target_bytes: 1 }).expect("sweep c");
    assert_eq!(store.get(&"c".to_string()), None, "c is next newest");
    assert_eq!(store.keys(), vec!["a".to_string()]);
}
#[test]
fn byte_targeted_sweep_reclaims_full_cost_of_newest_entry() {
    // Empty values still retain keys and index state. A small byte target
    // must reclaim only the newest entry's full semantic cost.
    let store = store();
    store.insert_checked("a".to_string(), vec![]).expect("a"); // 0 bytes
    store.insert_checked("b".to_string(), vec![]).expect("b"); // 0 bytes
    store.insert_checked("c".to_string(), vec![0u8; 8]).expect("c"); // 8 bytes
    assert_eq!(
        store.committed_bytes().expect("before"),
        retained_cost("a", 0) + retained_cost("b", 0) + retained_cost("c", 8)
    );

    let stats = store.sweep(SweepHint { target_bytes: 6 }).expect("sweep");
    assert_eq!(stats.reclaimed_items, 1, "only the newest entry is swept");
    assert_eq!(stats.reclaimed_bytes, retained_cost("c", 8));
    assert_eq!(store.stats().item_count, 2, "older entries survive the byte sweep");
    assert!(
        store.get(&"a".to_string()).is_some() && store.get(&"b".to_string()).is_some(),
        "older entries are retained, not over-evicted"
    );
}
#[test]
fn byte_targeted_sweep_of_empty_values_reclaims_key_and_index_cost() {
    // An empty value is not a free retained entry: key identity and index
    // state must be charged and returned by the sweep.
    let store = store();
    for k in ["a", "b", "c"] {
        store.insert_checked(k.to_string(), vec![]).expect("zero-byte insert");
    }
    let stats = store.sweep(SweepHint { target_bytes: 6 }).expect("sweep");
    assert_eq!(stats.reclaimed_items, 1);
    assert_eq!(stats.reclaimed_bytes, retained_cost("c", 0));
    assert_eq!(
        store.stats().item_count,
        2,
        "a positive byte target must remove only the byte-bearing entry needed"
    );
}
