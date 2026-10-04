use super::*;

#[test]
fn checked_store_and_session_sequences_never_wrap() {
    let counter = std::sync::atomic::AtomicU64::new(u64::MAX - 1);
    assert_eq!(allocate_store_nonce(&counter).expect("last nonce"), u64::MAX - 1);
    assert!(matches!(allocate_store_nonce(&counter), Err(MemoryError::Capacity(_))));
    let store = store();
    store.state.lock().expect("state").next_session_sequence = u64::MAX;
    assert!(matches!(store.open_session("s".to_string()), Err(MemoryError::Capacity(_))));
    let state = store.state.lock().expect("unchanged");
    assert_eq!(state.next_session_sequence, u64::MAX);
    assert!(state.sessions.is_empty());
}

#[test]
fn empty_sessions_consume_limits_and_reopen_never_reuses_handle() {
    let key_cost = cost("a", &[]);
    let store = store_with_policy(
        TranscriptRetentionPolicyV2::default().with_session_limits(2, 2 * key_cost),
    );
    let a = store.open_session("a".to_string()).expect("first empty session");
    let same_name = store.open_session("a".to_string()).expect("independent same-name session");
    assert_ne!(a, same_name);
    let before = store.retention_stats().expect("two empty sessions");
    assert_eq!(before.session_count, 2);
    assert_eq!(before.session_key_bytes, 2 * key_cost);
    assert_eq!(before.total_retained_bytes, 2 * key_cost);
    assert_eq!(before.active_row_count, 0);
    assert!(matches!(store.open_session("a".to_string()), Err(MemoryError::Capacity(_))));
    assert_eq!(store.retention_stats().expect("unchanged"), before);

    store.retire_session(&a).expect("retire first incarnation");
    assert!(matches!(
        store.append(request(a, 0, "late", "body")),
        Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(_)))
    ));
    assert!(matches!(store.load(&a), Err(MemoryError::InvalidState(_))));
    assert_eq!(store.retention_stats().expect("one empty session").session_count, 1);
    let reopened = store.open_session("a".to_string()).expect("reopen same metadata");
    assert_ne!(reopened, a);
    assert_ne!(reopened, same_name);
    store.append(request(reopened, 0, "new", "body")).expect("new incarnation");
    assert!(store.load(&same_name).expect("same name independent").is_empty());
    assert_eq!(store.load(&reopened).expect("new row").len(), 1);
    assert!(matches!(
        store.try_reconfigure_retention(
            TranscriptRetentionPolicyV2::default().with_session_limits(1, u64::MAX)
        ),
        Err(TranscriptRetentionReconfigureErrorV2::BelowLiveSessions { .. })
    ));
}

#[cfg(feature = "core-live-accounting")]
#[test]
fn budgeted_empty_session_retire_and_close_refund_metadata_exactly() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("broker");
    let store = SimpleSessionTranscriptStore::<String, String, String, Budgeted>::new_budgeted(
        StoreBindingHandle::new(MemoryClass::SessionTranscript, "empty-retire".into(), None),
        TranscriptRetentionPolicyV2::default(),
        broker.clone(),
    )
    .expect("store nonce");
    let first = store.open_session("s".to_string()).expect("first");
    let key_cost = cost("s", &[]);
    let stats = store.retention_stats().expect("open stats");
    assert_eq!(stats.session_count, 1);
    assert_eq!(stats.session_key_bytes, key_cost);
    assert_eq!(stats.total_retained_bytes, key_cost);
    assert_eq!(
        broker.ledger_snapshot().expect("open ledger").total_retained_metadata_bytes(),
        key_cost
    );
    store.retire_session(&first).expect("retire empty");
    assert_eq!(store.retention_stats().expect("retired stats").total_retained_bytes, 0);
    assert_eq!(broker.ledger_snapshot().expect("retired ledger").total_owner_bytes(), 0);
    let second = store.open_session("s".to_string()).expect("reopen");
    assert_ne!(first, second);
    store.close().expect("close empty");
    assert_eq!(broker.ledger_snapshot().expect("closed ledger").total_owner_bytes(), 0);
}

#[cfg(feature = "core-live-accounting")]
#[test]
fn count_and_key_byte_limits_independently_deny_open_before_charge() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("broker");
    let binding = || {
        StoreBindingHandle::new(MemoryClass::SessionTranscript, "independent-limit".into(), None)
    };

    let by_bytes = SimpleSessionTranscriptStore::<String, String, String, Budgeted>::new_budgeted(
        binding(),
        TranscriptRetentionPolicyV2::default().with_session_limits(100, cost("a", &[])),
        broker.clone(),
    )
    .expect("byte-limited store");
    by_bytes.open_session("a".to_string()).expect("first key");
    let before_stats = by_bytes.retention_stats().expect("before byte denial");
    let before_ledger = broker.ledger_snapshot().expect("before byte ledger");
    assert!(matches!(by_bytes.open_session("b".to_string()), Err(MemoryError::Capacity(_))));
    assert_eq!(by_bytes.retention_stats().expect("byte postimage"), before_stats);
    assert_eq!(broker.ledger_snapshot().expect("byte ledger postimage"), before_ledger);
    by_bytes.close().expect("drain byte-limited store");

    let by_count = SimpleSessionTranscriptStore::<String, String, String, Budgeted>::new_budgeted(
        binding(),
        TranscriptRetentionPolicyV2::default().with_session_limits(1, u64::MAX),
        broker.clone(),
    )
    .expect("count-limited store");
    by_count.open_session("a".to_string()).expect("first session");
    let before_stats = by_count.retention_stats().expect("before count denial");
    let before_ledger = broker.ledger_snapshot().expect("before count ledger");
    assert!(matches!(by_count.open_session("b".to_string()), Err(MemoryError::Capacity(_))));
    assert_eq!(by_count.retention_stats().expect("count postimage"), before_stats);
    assert_eq!(broker.ledger_snapshot().expect("count ledger postimage"), before_ledger);
    by_count.close().expect("drain count-limited store");
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
}

#[test]
fn cursor_limits_staleness_and_max_sequence_are_fail_closed() {
    let store = store();
    let handle = store.open_session("s".to_string()).expect("open");
    store.append(request(handle, 1, "m1", "one")).expect("first");
    store.append(request(handle, u64::MAX, "m2", "two")).expect("last");
    let before = store.retention_stats().expect("before invalid pages");
    assert!(matches!(store.retain_sequences(&handle, &[1, 1, 1]), Err(MemoryError::Capacity(_))));
    assert!(matches!(store.read_page(&handle, None, 0, u64::MAX), Err(MemoryError::Capacity(_))));
    assert!(matches!(store.read_page(&handle, None, 1, 0), Err(MemoryError::Capacity(_))));
    // Independent active-row oracle: 3 body + 2 id + 24 index + 16 entry.
    assert!(matches!(store.read_page(&handle, None, 1, 44), Err(MemoryError::Capacity(_))));
    assert_eq!(store.retention_stats().expect("no advance"), before);

    let first = store.read_page(&handle, None, 1, 45).expect("first page");
    assert_eq!(first.handle(), handle);
    assert_eq!(first.records()[0].sequence(), 1);
    let cursor = first.next_cursor().expect("second page remains");
    assert_eq!(cursor.last_sequence(), 1);
    let final_page = store.read_page(&handle, Some(&cursor), 1, 45).expect("last page");
    assert_eq!(final_page.records()[0].sequence(), u64::MAX);
    assert_eq!(final_page.next_cursor(), None);

    let other = store.open_session("s".to_string()).expect("other incarnation");
    assert!(matches!(
        store.read_page(&other, Some(&cursor), 1, 45),
        Err(MemoryError::InvalidState(_))
    ));
    store.append(request(handle, 2, "m3", "new")).expect("mutation invalidates cursor");
    assert!(matches!(
        store.read_page(&handle, Some(&cursor), 1, 45),
        Err(MemoryError::InvalidState(_))
    ));
    assert_eq!(store.retention_stats().expect("stale cursor inert").active_row_count, 3);
}

#[cfg(feature = "core-live-accounting")]
#[test]
fn failed_retire_restores_all_charges_and_handle_then_success_ends_replay() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("broker");
    let store = SimpleSessionTranscriptStore::<String, String, String, Budgeted>::new_budgeted(
        StoreBindingHandle::new(MemoryClass::SessionTranscript, "retirement".into(), None),
        TranscriptRetentionPolicyV2::default(),
        broker.clone(),
    )
    .expect("store nonce");
    let handle = store.open_session("s".to_string()).expect("open");
    store.append(request(handle, 0, "m", "body")).expect("append");
    let before_stats = store.retention_stats().expect("before");
    let before_ledger = broker.ledger_snapshot().expect("before ledger");
    let fence = broker.begin_rebuild().expect("fence release");
    assert!(matches!(store.retire_session(&handle), Err(MemoryError::InvalidState(_))));
    assert_eq!(store.retention_stats().expect("restored stats"), before_stats);
    assert_eq!(broker.ledger_snapshot().expect("restored ledger"), before_ledger);
    assert_eq!(
        store.append(request(handle, 0, "m", "body")).expect("replay after denial"),
        TranscriptAppendDispositionV1::Replayed
    );
    assert!(fence.finish().expect("finish").drain_error.is_none());
    store.retire_session(&handle).expect("retry retirement");
    let retired = store.retention_stats().expect("retired stats");
    assert_eq!(retired.session_count, 0);
    assert_eq!(retired.session_key_bytes, 0);
    assert_eq!(retired.evidence_count, 0);
    assert_eq!(retired.active_row_count, 0);
    assert_eq!(retired.total_retained_bytes, 0);
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    assert!(matches!(
        store.append(request(handle, 0, "m", "body")),
        Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(_)))
    ));
}

#[cfg(feature = "core-live-accounting")]
#[test]
fn detached_slow_clone_allows_other_append_and_pins_target_charge_until_read_ends() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct BlockingBody {
        bytes: Vec<u8>,
        armed: Arc<AtomicBool>,
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
    }
    impl Clone for BlockingBody {
        fn clone(&self) -> Self {
            if self.armed.load(Ordering::SeqCst) {
                self.entered.wait();
                self.release.wait();
            }
            Self {
                bytes: self.bytes.clone(),
                armed: Arc::clone(&self.armed),
                entered: Arc::clone(&self.entered),
                release: Arc::clone(&self.release),
            }
        }
    }
    impl ByteSized for BlockingBody {
        fn byte_size(&self) -> u64 {
            self.bytes.len() as u64
        }
    }
    impl CanonicalKey for BlockingBody {
        fn canonical_key(&self) -> Vec<u8> {
            self.bytes.clone()
        }
    }

    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("broker");
    let store = Arc::new(
        SimpleSessionTranscriptStore::<String, String, BlockingBody, Budgeted>::new_budgeted(
            StoreBindingHandle::new(MemoryClass::SessionTranscript, "detached-clone".into(), None),
            TranscriptRetentionPolicyV2::default(),
            broker.clone(),
        )
        .expect("store nonce"),
    );
    let a = store.open_session("a".to_string()).expect("a");
    let b = store.open_session("b".to_string()).expect("b");
    let armed = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let body = |value: u8| BlockingBody {
        bytes: vec![value],
        armed: Arc::clone(&armed),
        entered: Arc::clone(&entered),
        release: Arc::clone(&release),
    };
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(a, 0, "a0".to_string()),
            body(1),
        ))
        .expect("seed a");
    armed.store(true, Ordering::SeqCst);
    let reading = Arc::clone(&store);
    let reader = thread::spawn(move || reading.load(&a));
    entered.wait();
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(b, 0, "b0".to_string()),
            body(2),
        ))
        .expect("other session append during clone");
    assert!(matches!(store.retain_sequences(&a, &[]), Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(store.retire_session(&a), Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
    let pinned_ledger = broker.ledger_snapshot().expect("both rows remain charged");
    assert!(pinned_ledger.total_owner_bytes() > 0);
    armed.store(false, Ordering::SeqCst);
    release.wait();
    assert_eq!(reader.join().expect("reader join").expect("reader result").len(), 1);
    store.retire_session(&a).expect("retire after read");
    store.close().expect("close after read");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}

#[cfg(feature = "core-live-accounting")]
#[test]
fn closing_phase_prevents_new_read_pin_and_clone_then_retry_succeeds() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountedBody {
        bytes: Vec<u8>,
        clones: Arc<AtomicUsize>,
    }
    impl Clone for CountedBody {
        fn clone(&self) -> Self {
            self.clones.fetch_add(1, Ordering::SeqCst);
            Self { bytes: self.bytes.clone(), clones: Arc::clone(&self.clones) }
        }
    }
    impl ByteSized for CountedBody {
        fn byte_size(&self) -> u64 {
            self.bytes.len() as u64
        }
    }
    impl CanonicalKey for CountedBody {
        fn canonical_key(&self) -> Vec<u8> {
            self.bytes.clone()
        }
    }

    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("broker");
    let store =
        SimpleSessionTranscriptStore::<String, String, CountedBody, Budgeted>::new_budgeted(
            StoreBindingHandle::new(MemoryClass::SessionTranscript, "closing-first".into(), None),
            TranscriptRetentionPolicyV2::default(),
            broker.clone(),
        )
        .expect("store nonce");
    let handle = store.open_session("s".to_string()).expect("open");
    let clones = Arc::new(AtomicUsize::new(0));
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(handle, 0, "m".to_string()),
            CountedBody { bytes: vec![1], clones: Arc::clone(&clones) },
        ))
        .expect("append");
    let before = broker.ledger_snapshot().expect("before closing permit");
    let permit = match store.lifecycle.begin_close().expect("closing permit") {
        BeginClose::Acquired(permit) => permit,
        BeginClose::AlreadyClosed => panic!("new store must be open"),
    };
    assert!(matches!(store.load(&handle), Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(
        store.read_page(&handle, None, 1, u64::MAX),
        Err(MemoryError::OperationInProgress(_))
    ));
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    let state = store.state.lock().expect("state while closing");
    let row = state.sessions[&handle].rows.get(&0).expect("row remains");
    assert_eq!(Arc::strong_count(&row.data), 1, "closing-first read acquired no pin");
    drop(state);
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger"), before);
    drop(permit);
    assert_eq!(store.load(&handle).expect("read after failed close").len(), 1);
    assert_eq!(clones.load(Ordering::SeqCst), 1);
    store.close().expect("terminal close");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
