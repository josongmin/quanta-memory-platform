use super::*;

#[test]
fn append_message_id_clone_and_order_reentry_is_typed_busy() {
    let broker = broker(1_000_000);
    let store = Arc::new(
        ReentrantIdStore::new_budgeted(
            StoreBindingHandle::new(
                MemoryClass::SessionTranscript,
                "transcript-append-reentry-test".to_string(),
                None,
            ),
            TranscriptRetentionPolicyV2::default(),
            broker.clone(),
        )
        .expect("transcript store nonce"),
    );
    let s = store.open_session("s".to_string()).expect("open session");
    let owner = Arc::new(OnceLock::new());
    owner.set(Arc::downgrade(&store)).expect("one owner");
    let busy_callbacks = Arc::new(AtomicUsize::new(0));
    for (sequence, name, body) in [(0, "m0", "one"), (1, "m1", "four")] {
        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(
                    s,
                    sequence,
                    ReentrantMessageId {
                        name,
                        owner: Arc::clone(&owner),
                        busy_callbacks: Arc::clone(&busy_callbacks),
                    },
                ),
                body.to_string(),
            ))
            .expect("append after typed busy callbacks");
    }
    assert!(busy_callbacks.load(AtomicOrdering::SeqCst) >= 4);
    assert_eq!(store.stats().expect("committed postimage").item_count, 2);
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        cost("s", &[("m0", "one", true), ("m1", "four", true)])
    );
    store.close().expect("close after callbacks");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn panicking_message_id_clone_at_each_publication_point_leaves_append_atomic() {
    for panic_after in [1, 2, 3] {
        let broker = broker(1_000_000);
        let store = callback_store(&broker);
        let s = store.open_session("s".to_string()).expect("open session");
        let clones = CallbackTrap::new();
        let comparisons = CallbackTrap::new();
        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(s, 0, callback_id("m0", &clones, &comparisons)),
                "one".to_string(),
            ))
            .expect("seed row");
        clones.arm_after(panic_after);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(s, 1, callback_id("m1", &clones, &comparisons)),
                "four".to_string(),
            ));
        }));
        assert!(panicked.is_err(), "clone trap must fire at point {panic_after}");
        clones.disarm();
        assert_seed_only_after_append_panic(&store, &broker);
        assert_eq!(store.load(&s).expect("readable after callback panic").len(), 1);
        store.close().expect("close after callback panic");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }
}
#[test]
fn panicking_message_id_clone_preserves_explicit_empty_session() {
    for panic_after in [1, 2, 3] {
        let broker = broker(1_000_000);
        let store = callback_store(&broker);
        let new = store.open_session("new".to_string()).expect("open empty session");
        let clones = CallbackTrap::new();
        let comparisons = CallbackTrap::new();
        clones.arm_after(panic_after);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = store.append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(new, 0, callback_id("m0", &clones, &comparisons)),
                "body".to_string(),
            ));
        }));
        assert!(panicked.is_err(), "clone trap must fire at point {panic_after}");
        clones.disarm();
        assert_eq!(store.state.lock().unwrap_or_else(PoisonError::into_inner).sessions.len(), 1);
        assert_eq!(store.stats().expect("readable after callback panic").item_count, 0);
        assert_eq!(store.retained_bytes().expect("bytes"), cost("new", &[]));
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), cost("new", &[]));
        assert_eq!(
            store
                .append(TranscriptAppendRequestV1::new(
                    TranscriptAppendIdentityV1::new(
                        new,
                        0,
                        callback_id("m0", &clones, &comparisons),
                    ),
                    "body".to_string(),
                ))
                .expect("fresh retry"),
            TranscriptAppendDispositionV1::Applied
        );
        store.close().expect("close after retry");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }
}
#[test]
fn session_handles_do_not_compare_caller_metadata_after_open() {
    let broker = broker(1_000_000);
    let trap = ComparisonTrap::new();
    let store =
        SimpleSessionTranscriptStore::<OrderedSessionId, String, String, Budgeted>::new_budgeted(
            StoreBindingHandle::new(MemoryClass::SessionTranscript, "handle-order".into(), None),
            TranscriptRetentionPolicyV2::default(),
            broker.clone(),
        )
        .expect("transcript store nonce");
    let old =
        store.open_session(OrderedSessionId { name: "old", trap: Arc::clone(&trap) }).expect("old");
    let new =
        store.open_session(OrderedSessionId { name: "new", trap: Arc::clone(&trap) }).expect("new");
    trap.panic_at.store(1, AtomicOrdering::SeqCst);
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(old, 0, "m0".to_string()),
            "one".to_string(),
        ))
        .expect("old append");
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(new, 0, "m1".to_string()),
            "four".to_string(),
        ))
        .expect("new append");
    assert_eq!(trap.calls.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(store.load(&old).expect("old rows").len(), 1);
    assert_eq!(store.load(&new).expect("new rows").len(), 1);
    store.retire_session(&old).expect("retire old");
    assert_eq!(store.load(&new).expect("new survives").len(), 1);
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        cost("new", &[("m1", "four", true)])
    );
    store.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn foreign_handle_append_fails_before_accounting() {
    let broker = broker(1_000_000);
    let a = charged(&broker);
    let b = charged(&broker);
    let handle = a.open_session("same".to_string()).expect("a session");
    let before = broker.ledger_snapshot().expect("before");
    assert!(matches!(
        b.append(request(handle, 0, "m", "body")),
        Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(_)))
    ));
    assert_eq!(broker.ledger_snapshot().expect("after"), before);
    assert!(a.load(&handle).expect("a remains").is_empty());
}
#[test]
fn append_preflight_message_order_panic_keeps_public_store_available() {
    let broker = broker(1_000_000);
    let store = callback_store(&broker);
    let s = store.open_session("s".to_string()).expect("open session");
    let clones = CallbackTrap::new();
    let comparisons = CallbackTrap::new();
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(s, 0, callback_id("m0", &clones, &comparisons)),
            "one".to_string(),
        ))
        .expect("seed row");
    let before = broker.ledger_snapshot().expect("before");
    comparisons.arm_after(1);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(s, 1, callback_id("m1", &clones, &comparisons)),
            "two".to_string(),
        ));
    }));
    assert!(panicked.is_err(), "MessageId Ord trap must fire in preflight");
    comparisons.disarm();
    assert_eq!(broker.ledger_snapshot().expect("after"), before);
    assert_eq!(store.load(&s).expect("public load after panic").len(), 1);
    store.close().expect("public close after panic");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn append_replay_canonical_key_panic_keeps_public_store_available() {
    #[derive(Clone)]
    struct Body {
        bytes: Vec<u8>,
        trap: Arc<CallbackTrap>,
    }
    impl ByteSized for Body {
        fn byte_size(&self) -> u64 {
            self.bytes.len() as u64
        }
    }
    impl CanonicalKey for Body {
        fn canonical_key(&self) -> Vec<u8> {
            self.trap.hit();
            self.bytes.clone()
        }
    }

    let broker = broker(1_000_000);
    let trap = CallbackTrap::new();
    let store = SimpleSessionTranscriptStore::<String, String, Body, Budgeted>::new_budgeted(
        StoreBindingHandle::new(
            MemoryClass::SessionTranscript,
            "transcript-canonical-panic".to_string(),
            None,
        ),
        TranscriptRetentionPolicyV2::default(),
        broker.clone(),
    )
    .expect("transcript store nonce");
    let s = store.open_session("s".to_string()).expect("open session");
    let body = Body { bytes: b"body".to_vec(), trap: Arc::clone(&trap) };
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(s, 0, "m0".to_string()),
            body.clone(),
        ))
        .expect("seed row");
    let before = broker.ledger_snapshot().expect("before");
    // Incoming body canonicalization is outside the mutex; the second
    // call checks the retained row while resolving an exact replay.
    trap.arm_after(2);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(s, 0, "m0".to_string()),
            body.clone(),
        ));
    }));
    assert!(panicked.is_err(), "retained CanonicalKey trap must fire");
    trap.disarm();
    assert_eq!(broker.ledger_snapshot().expect("after"), before);
    assert_eq!(store.load(&s).expect("public load after panic").len(), 1);
    store.close().expect("public close after panic");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn panicking_message_id_order_at_publication_leaves_append_atomic() {
    let broker = broker(1_000_000);
    let store = callback_store(&broker);
    let s = store.open_session("s".to_string()).expect("open session");
    let clones = CallbackTrap::new();
    let comparisons = CallbackTrap::new();
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(s, 0, callback_id("m0", &clones, &comparisons)),
            "one".to_string(),
        ))
        .expect("seed row");
    // One comparison is the preflight lookup; the next is publication.
    comparisons.arm_after(2);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(s, 1, callback_id("m1", &clones, &comparisons)),
            "four".to_string(),
        ));
    }));
    assert!(panicked.is_err(), "order trap must fire at publication");
    comparisons.disarm();
    assert_seed_only_after_append_panic(&store, &broker);
    assert_eq!(store.load(&s).expect("readable after callback panic").len(), 1);
    let before_retry = comparisons.calls.load(AtomicOrdering::SeqCst);
    // Three MessageId comparisons follow on the retry (preflight
    // probes and the publication slot resolution); arm past all of
    // them so the disarmed retry completes.
    comparisons.arm_after(4);
    assert_eq!(
        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(s, 1, callback_id("m1", &clones, &comparisons),),
                "four".to_string(),
            ))
            .expect("retry after callback panic"),
        TranscriptAppendDispositionV1::Applied
    );
    assert_eq!(
        comparisons.calls.load(AtomicOrdering::SeqCst),
        before_retry + 3,
        "append compares MessageId during preflight and the publication slot resolution"
    );
    comparisons.disarm();
    let expected = cost("s", &[("m0", "one", true), ("m1", "four", true)]);
    assert_eq!(store.retained_bytes().expect("committed bytes"), expected);
    assert_eq!(broker.ledger_snapshot().expect("committed ledger").total_owner_bytes(), expected);
    store.close().expect("close after callback panic");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn message_id_keyed_insert_panic_rolls_back_publication_completely() {
    let broker = broker(1_000_000);
    let store = callback_store(&broker);
    let s = store.open_session("s".to_string()).expect("open session");
    let clones = CallbackTrap::new();
    let comparisons = CallbackTrap::new();
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(s, 0, callback_id("m0", &clones, &comparisons)),
            "one".to_string(),
        ))
        .expect("seed row");
    comparisons.arm_after(3);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(s, 1, callback_id("m1", &clones, &comparisons)),
            "four".to_string(),
        ));
    }));
    assert!(panicked.is_err(), "trap must fire inside publication");
    comparisons.disarm();

    // Atomicity: the seed-only image must hold and the store must
    // remain fully closable (no half-published row/evidence).
    assert_seed_only_after_append_panic(&store, &broker);
    assert_eq!(
        store.load(&s).expect("readable").len(),
        1,
        "no phantom row may survive the publication panic"
    );

    // The rolled-back append must retry cleanly, not be swallowed as
    // a replay and not double-publish the message id.
    assert_eq!(
        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(s, 1, callback_id("m1", &clones, &comparisons),),
                "four".to_string(),
            ))
            .expect("clean retry after rollback"),
        TranscriptAppendDispositionV1::Applied
    );
    let expected = cost("s", &[("m0", "one", true), ("m1", "four", true)]);
    assert_eq!(store.retained_bytes().expect("committed bytes"), expected);
    assert_eq!(broker.ledger_snapshot().expect("committed ledger").total_owner_bytes(), expected);
    store.close().expect("close must reconcile after the panic");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn semantic_identity_component_overflow_denies_before_ledger_or_index_mutation() {
    let broker = broker(1_000_000);
    let store = SimpleSessionTranscriptStore::<SizedId, SizedId, Vec<u8>, Budgeted>::new_budgeted(
        StoreBindingHandle::new(MemoryClass::SessionTranscript, "overflow".into(), None),
        TranscriptRetentionPolicyV2::default(),
        broker.clone(),
    )
    .expect("transcript store nonce");
    assert!(matches!(
        store.open_session(SizedId { name: "huge", bytes: u64::MAX }),
        Err(MemoryError::Capacity(_))
    ));
    assert!(store.lock_state().expect("state").sessions.is_empty());
    assert_eq!(broker.ledger_snapshot().expect("inert ledger").total_owner_bytes(), 0);

    let handle = store.open_session(SizedId { name: "session", bytes: 1 }).expect("small metadata");
    let before = broker.ledger_snapshot().expect("key-only ledger");
    let err = store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(
                handle,
                0,
                SizedId { name: "message", bytes: u64::MAX },
            ),
            Vec::new(),
        ))
        .expect_err("message cost overflow");
    assert!(matches!(err, TranscriptAppendErrorV1::Accounting(MemoryError::Capacity(_))));
    assert!(store.load(&handle).expect("zero rows").is_empty());
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger"), before);
}
#[test]
fn zero_body_large_identity_and_session_are_charged_until_evidence_retirement() {
    let broker = broker(10_000);
    let store = charged(&broker);
    let session = "s".repeat(512);
    let session_handle = store.open_session(session.clone()).expect("open large session");
    let message_id = "m".repeat(1024);
    store.append(request(session_handle, 0, &message_id, "")).expect("zero-body append");
    let before = cost(&session, &[(&message_id, "", true)]);
    assert_eq!(store.session_bytes(&session_handle).expect("session bytes"), before);
    assert_eq!(store.stats().expect("stats").estimated_bytes, before);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), before);
    store.truncate_to(&session_handle, 0).expect("truncate active row");
    let after = cost(&session, &[(&message_id, "", false)]);
    assert_eq!(store.session_bytes(&session_handle).expect("evidence bytes"), after);
    assert_eq!(store.retained_bytes().expect("total evidence"), after);
    assert_eq!(broker.ledger_snapshot().expect("evidence ledger").total_owner_bytes(), after);
    assert_eq!(
        store.append(request(session_handle, 0, &message_id, "")).expect("late replay"),
        TranscriptAppendDispositionV1::Replayed
    );
    store.close().expect("close evidence");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn retain_rollback_and_commit_do_not_compare_caller_metadata() {
    let broker = broker(1_000_000);
    let trap = ComparisonTrap::new();
    let store =
        SimpleSessionTranscriptStore::<OrderedSessionId, String, String, Budgeted>::new_budgeted(
            StoreBindingHandle::new(MemoryClass::SessionTranscript, "retain-handle".into(), None),
            TranscriptRetentionPolicyV2::default(),
            broker.clone(),
        )
        .expect("transcript store nonce");
    let handle = store
        .open_session(OrderedSessionId { name: "session", trap: Arc::clone(&trap) })
        .expect("open");
    for (sequence, body) in [(0, "one"), (1, "four")] {
        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(handle, sequence, format!("m{sequence}")),
                body.to_string(),
            ))
            .expect("seed");
    }
    let before = broker.ledger_snapshot().expect("before");
    trap.panic_at.store(1, AtomicOrdering::SeqCst);
    let fence = broker.begin_rebuild().expect("fence");
    assert!(matches!(store.retain_sequences(&handle, &[0]), Err(MemoryError::InvalidState(_))));
    assert_eq!(broker.ledger_snapshot().expect("rollback"), before);
    assert_eq!(store.load(&handle).expect("rows").len(), 2);
    assert!(fence.finish().expect("finish").drain_error.is_none());
    store.retain_sequences(&handle, &[0]).expect("retry");
    assert_eq!(store.load(&handle).expect("retained").len(), 1);
    assert_eq!(trap.calls.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        cost("session", &[("m0", "one", true), ("m1", "four", false)])
    );
}
#[test]
fn retain_by_handle_skips_caller_order_before_detach() {
    let broker = broker(1_000_000);
    let trap = ComparisonTrap::new();
    let store =
        SimpleSessionTranscriptStore::<OrderedSessionId, String, String, Budgeted>::new_budgeted(
            StoreBindingHandle::new(MemoryClass::SessionTranscript, "retain-no-order".into(), None),
            TranscriptRetentionPolicyV2::default(),
            broker.clone(),
        )
        .expect("transcript store nonce");
    let handle = store
        .open_session(OrderedSessionId { name: "session", trap: Arc::clone(&trap) })
        .expect("open");
    for (sequence, body) in [(0, "one"), (1, "four")] {
        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(handle, sequence, format!("m{sequence}")),
                body.to_string(),
            ))
            .expect("seed");
    }
    trap.panic_at.store(1, AtomicOrdering::SeqCst);
    store.retain_sequences(&handle, &[0]).expect("retain without metadata order");
    assert_eq!(trap.calls.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(store.load(&handle).expect("rows").len(), 1);
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        cost("session", &[("m0", "one", true), ("m1", "four", false)])
    );
}
#[test]
fn active_row_and_session_evidence_own_separate_kind_buckets() {
    let broker = broker(1_000_000);
    let store = SimpleSessionTranscriptStore::<String, String, String, Budgeted>::new_budgeted(
        StoreBindingHandle::new(
            MemoryClass::SessionTranscript,
            "transcript-kind-bucket-test".to_string(),
            None,
        ),
        TranscriptRetentionPolicyV2::default(),
        broker.clone(),
    )
    .expect("transcript store nonce");
    let session = store.open_session("session".to_string()).expect("open session");
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(session, 0, "m0".to_string()),
            "body".to_string(),
        ))
        .expect("append");

    // Two owned lifetime components, two buckets: the active row is
    // RetainedPayload; the session key and persistent identity
    // evidence are RetainedMetadata. Cross-check each bucket against
    // the independent retention component counters.
    let retention = store.retention_stats().expect("retention stats");
    let snap = broker.ledger_snapshot().expect("snapshot");
    assert!(retention.active_bytes > 0);
    assert!(retention.evidence_bytes + retention.session_key_bytes > 0);
    assert_eq!(snap.total_retained_payload_bytes(), retention.active_bytes);
    assert_eq!(
        snap.total_retained_metadata_bytes(),
        retention.evidence_bytes + retention.session_key_bytes
    );
    assert_eq!(snap.total_live_scratch_bytes(), 0);
    assert_eq!(
        snap.total_owner_bytes(),
        retention.active_bytes + retention.evidence_bytes + retention.session_key_bytes
    );
    let usage = snap.owner_usage.values().next().expect("sole owner");
    assert_eq!(usage.retained_payload_bytes, retention.active_bytes);
    assert_eq!(
        usage.retained_metadata_bytes,
        retention.evidence_bytes + retention.session_key_bytes
    );
    assert_eq!(usage.live_scratch_bytes, 0);

    store.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
