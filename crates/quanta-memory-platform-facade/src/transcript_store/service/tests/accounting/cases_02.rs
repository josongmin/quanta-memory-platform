use super::*;

#[test]
fn compound_hostile_session_ord_panic_cannot_abort_append() {
    use std::sync::atomic::AtomicBool;

    const CHILD: &str = "QMP_TRANSCRIPT_COMPOUND_CHILD";

    #[derive(Clone)]
    struct CompoundSessionKey {
        name: String,
        ord_panics: Arc<AtomicBool>,
        drop_panics: Arc<AtomicBool>,
    }

    impl ByteSized for CompoundSessionKey {
        fn byte_size(&self) -> u64 {
            self.name.len() as u64
        }
    }

    impl PartialEq for CompoundSessionKey {
        fn eq(&self, other: &Self) -> bool {
            self.name == other.name
        }
    }

    impl Eq for CompoundSessionKey {}
    impl PartialOrd for CompoundSessionKey {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for CompoundSessionKey {
        fn cmp(&self, other: &Self) -> Ordering {
            assert!(!self.ord_panics.load(AtomicOrdering::SeqCst), "armed compound-hostile Ord");
            self.name.cmp(&other.name)
        }
    }

    impl Drop for CompoundSessionKey {
        fn drop(&mut self) {
            assert!(!self.drop_panics.load(AtomicOrdering::SeqCst), "armed compound-hostile Drop");
        }
    }

    let scenario = || {
        let broker = broker(1_000_000);
        let ord_panics = Arc::new(AtomicBool::new(false));
        let drop_panics = Arc::new(AtomicBool::new(false));
        let key = CompoundSessionKey {
            name: "session".to_string(),
            ord_panics: Arc::clone(&ord_panics),
            drop_panics: Arc::clone(&drop_panics),
        };
        let store = SimpleSessionTranscriptStore::<
            CompoundSessionKey,
            String,
            String,
            Budgeted,
        >::new_budgeted(
            StoreBindingHandle::new(
                MemoryClass::SessionTranscript,
                "transcript-compound-hostile".to_string(),
                None,
            ),
            TranscriptRetentionPolicyV2::default(),
            broker.clone(),
        );
        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(key.clone(), 0, "m0".to_string()),
                "body0".to_string(),
            ))
            .expect("seed while disarmed");

        ord_panics.store(true, AtomicOrdering::SeqCst);
        drop_panics.store(true, AtomicOrdering::SeqCst);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(key.clone(), 1, "m1".to_string()),
                "body1".to_string(),
            ))
        }));
        assert!(panicked.is_err(), "armed Ord must panic inside append");
        ord_panics.store(false, AtomicOrdering::SeqCst);
        drop_panics.store(false, AtomicOrdering::SeqCst);

        assert_eq!(store.checked_stats().expect("stats after contained panic").item_count, 1);
        assert_eq!(store.load(&key).expect("load after contained panic").len(), 1);
        store.close().expect("close after contained panic");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    };

    if std::env::var_os(CHILD).is_some() {
        scenario();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("--exact")
        .arg(
            "transcript_store::service::tests::accounting::cases_02::\
             compound_hostile_session_ord_panic_cannot_abort_append",
        )
        .arg("--nocapture")
        .env(CHILD, "1")
        .require_one_test()
        .output()
        .expect("isolated compound-hostile append child");
    assert!(
        output.status.success(),
        "a compound hostile session key must not abort append: status={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn session_order_panic_before_mutation_keeps_public_transcript_available() {
    for operation in ["retain", "truncate", "versioned retain"] {
        let broker = broker(1_000_000);
        let trap = ComparisonTrap::new();
        let id = OrderedSessionId { name: "session", trap: Arc::clone(&trap) };
        let store = SimpleSessionTranscriptStore::<
            OrderedSessionId,
            String,
            String,
            Budgeted,
        >::new_budgeted(
            StoreBindingHandle::new(
                MemoryClass::SessionTranscript,
                "transcript-public-panic-test".to_string(),
                None,
            ),
            TranscriptRetentionPolicyV2::default(),
            broker.clone(),
        );
        for sequence in 0..2 {
            store
                .append(TranscriptAppendRequestV1::new(
                    TranscriptAppendIdentityV1::new(id.clone(), sequence, format!("m{sequence}")),
                    format!("body{sequence}"),
                ))
                .expect("seed row");
        }
        let version = store.load_versioned_snapshot(&id).expect("version").version;
        let before = broker.ledger_snapshot().expect("before");
        let calls = trap.calls.load(AtomicOrdering::SeqCst);
        trap.panic_at.store(calls + 1, AtomicOrdering::SeqCst);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match operation {
            "retain" => store.retain_sequences(&id, &[0]),
            "truncate" => store.truncate_to(&id, 1),
            "versioned retain" => store.retain_sequences_if_version(&id, version, &[0]).map(|_| ()),
            _ => unreachable!(),
        }));
        assert!(panicked.is_err(), "{operation}: Ord trap must fire");
        trap.disarm();
        assert_eq!(broker.ledger_snapshot().expect("after"), before);
        assert_eq!(store.load(&id).expect("public load after panic").len(), 2);
        assert_eq!(store.stats().expect("public stats after panic").item_count, 2);
        store.close().expect("public close after panic");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }
}
#[test]
fn session_bytes_ord_panic_preserves_store_and_ledger() {
    let broker = broker(1_000_000);
    let trap = ComparisonTrap::new();
    let id = OrderedSessionId { name: "session", trap: Arc::clone(&trap) };
    let store =
        SimpleSessionTranscriptStore::<OrderedSessionId, String, String, Budgeted>::new_budgeted(
            StoreBindingHandle::new(
                MemoryClass::SessionTranscript,
                "transcript-session-bytes-panic-test".to_string(),
                None,
            ),
            TranscriptRetentionPolicyV2::default(),
            broker.clone(),
        );
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(id.clone(), 0, "m0".to_string()),
            "body0".to_string(),
        ))
        .expect("seed row");
    let before = broker.ledger_snapshot().expect("before");
    // Arm exactly the session_bytes lookup: one seeded session means the
    // BTreeMap probe performs exactly one SessionId::Ord comparison.
    let calls = trap.calls.load(AtomicOrdering::SeqCst);
    trap.panic_at.store(calls + 1, AtomicOrdering::SeqCst);
    let panicked =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.session_bytes(&id)));
    assert!(panicked.is_err(), "Ord trap must fire inside the session_bytes lookup");
    trap.disarm();
    assert_eq!(broker.ledger_snapshot().expect("after panic"), before);
    assert_eq!(
        store.checked_stats().expect("checked_stats after panic").item_count,
        1,
        "a read-side Ord panic must not poison the transcript store"
    );
    assert_eq!(store.load(&id).expect("load after panic").len(), 1);
    assert!(
        store.session_bytes(&id).expect("session_bytes after panic") > 0,
        "same read must succeed once the trap is disarmed"
    );
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(id.clone(), 1, "m1".to_string()),
            "body1".to_string(),
        ))
        .expect("append after panic");
    store.close().expect("close after panic");
    let drained = broker.ledger_snapshot().expect("drained");
    assert_eq!(drained.total_owner_bytes(), 0);
    assert_eq!(drained.live_ticket_count, 0);
}
#[test]
fn drop_contains_multiple_client_destructor_panics_during_outer_unwind() {
    const CHILD_ENV: &str = "QMP_TRANSCRIPT_DROP_UNWIND_CHILD";
    if std::env::var_os(CHILD_ENV).is_some() {
        let broker = broker(1_000_000);
        let store =
            SimpleSessionTranscriptStore::<String, String, PanicOnDropBody, Budgeted>::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "transcript-drop-panic-test".to_string(),
                    None,
                ),
                TranscriptRetentionPolicyV2::default(),
                broker.clone(),
            );
        for sequence in 0..2 {
            store
                .append(TranscriptAppendRequestV1::new(
                    TranscriptAppendIdentityV1::new(
                        "session".to_string(),
                        sequence,
                        format!("message-{sequence}"),
                    ),
                    PanicOnDropBody(vec![0; 3]),
                ))
                .expect("seed charged row");
        }
        assert_eq!(
            broker.ledger_snapshot().expect("live ledger").total_owner_bytes(),
            cost("session", &[("message-0", "one", true), ("message-1", "two", true)])
        );
        let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _store = store;
            panic!("outer unwind");
        }));
        assert!(outer.is_err());
        assert_eq!(broker.ledger_snapshot().expect("drop cleanup").total_owner_bytes(), 0);
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("transcript_store::service::tests::accounting::cases_02::drop_contains_multiple_client_destructor_panics_during_outer_unwind")
            .env(CHILD_ENV, "1")
            .require_one_test()
.output()
            .expect("subprocess test run");
    assert!(
        output.status.success(),
        "store Drop must not abort on client panic: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn retain_retires_multiple_panicking_client_bodies_after_ledger_postimage() {
    let broker = broker(1_000_000);
    let store =
        SimpleSessionTranscriptStore::<String, String, PanicOnDropBody, Budgeted>::new_budgeted(
            StoreBindingHandle::new(
                MemoryClass::SessionTranscript,
                "transcript-retain-panic-test".to_string(),
                None,
            ),
            TranscriptRetentionPolicyV2::default(),
            broker.clone(),
        );
    for sequence in 0..3 {
        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(
                    "session".to_string(),
                    sequence,
                    format!("message-{sequence}"),
                ),
                PanicOnDropBody(vec![0; 3]),
            ))
            .expect("seed row");
    }
    assert_eq!(
        broker.ledger_snapshot().expect("live ledger").total_owner_bytes(),
        cost(
            "session",
            &[("message-0", "one", true), ("message-1", "two", true), ("message-2", "tri", true)]
        )
    );
    store.retain_sequences(&"session".to_string(), &[0]).expect("retain");
    assert_eq!(store.stats().expect("postimage").item_count, 1);
    let expected = cost(
        "session",
        &[("message-0", "one", true), ("message-1", "two", false), ("message-2", "tri", false)],
    );
    assert_eq!(store.retained_bytes().expect("retained bytes"), expected);
    assert_eq!(broker.ledger_snapshot().expect("retained ledger").total_owner_bytes(), expected);
    store.close().expect("close survivor without client panic");
    assert_eq!(broker.ledger_snapshot().expect("closed ledger").total_owner_bytes(), 0);
}
#[test]
fn quota_denial_leaves_row_indexes_and_charge_unchanged() {
    let broker = broker(cost("s", &[("m0", "ok", true)]));
    let store = charged(&broker);
    let denied = store.append(request("s", 0, "m0", "four"));
    assert!(matches!(denied, Err(TranscriptAppendErrorV1::Accounting(MemoryError::Capacity(_)))));
    assert!(store.load(&"s".to_string()).expect("load").is_empty());
    assert_eq!(store.retained_bytes().expect("bytes"), 0);
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
    assert_eq!(
        store.append(request("s", 0, "m0", "ok")).expect("no index residue"),
        TranscriptAppendDispositionV1::Applied
    );
}
#[test]
fn retention_saturation_denies_before_any_new_ledger_charge() {
    let broker = broker(1_000_000);
    let store = charged_with_policy(&broker, TranscriptRetentionPolicyV2::fail_closed(1, u64::MAX));
    store.append(request("s", 0, "m0", "one")).expect("seed");
    store.truncate_to(&"s".to_string(), 0).expect("retain evidence only");
    let before_stats = store.retention_stats().expect("before stats");
    let before_ledger = broker.ledger_snapshot().expect("before ledger");

    assert!(matches!(
        store.append(request("s", 1, "m1", "two")),
        Err(TranscriptAppendErrorV1::RetentionSaturated { .. })
    ));
    assert_eq!(store.retention_stats().expect("after stats"), before_stats);
    assert_eq!(broker.ledger_snapshot().expect("after ledger"), before_ledger);
}
#[test]
fn retention_saturation_emits_one_bounded_tombstone_event() {
    #[derive(Default)]
    struct Events(std::sync::Mutex<Vec<MemoryEvent>>);
    impl MemoryEventSink for Events {
        fn emit(&self, event: MemoryEvent) {
            self.0.lock().expect("events").push(event);
        }
    }

    let events = Arc::new(Events::default());
    let broker = broker_with_events(1_000_000, events.clone());
    let store = charged_with_policy(&broker, TranscriptRetentionPolicyV2::fail_closed(1, u64::MAX));
    store.append(request("s", 0, "m0", "one")).expect("seed");

    assert!(matches!(
        store.append(request("s", 1, "m1", "two")),
        Err(TranscriptAppendErrorV1::RetentionSaturated { .. })
    ));
    let tombstones = events
        .0
        .lock()
        .expect("events")
        .iter()
        .filter(|event| event.operation == MemoryOperation::TombstoneSaturation)
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(
        tombstones,
        vec![MemoryEvent::new(
            MemoryOperation::TombstoneSaturation,
            MemoryOutcome::CapacityDenied,
            None,
        )]
    );
}
#[test]
fn every_conflict_axis_preserves_rows_indexes_and_accounting() {
    let broker = broker(1_000_000);
    let store = charged(&broker);
    store.append(request("s", 4, "m0", "body")).expect("seed");

    let conflicts = [
        store.append(request("s", 4, "m0", "drift")),
        store.append(request("s", 4, "m1", "body")),
        store.append(request("s", 5, "m0", "body")),
    ];
    assert!(matches!(conflicts[0], Err(TranscriptAppendErrorV1::PayloadConflict { sequence: 4 })));
    assert!(matches!(conflicts[1], Err(TranscriptAppendErrorV1::SequenceConflict { sequence: 4 })));
    assert!(matches!(conflicts[2], Err(TranscriptAppendErrorV1::MessageIdConflict)));
    assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
    let expected = cost("s", &[("m0", "body", true)]);
    assert_eq!(store.retained_bytes().expect("bytes"), expected);
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), expected);
}
#[test]
fn exact_retry_charges_once_and_truncate_refunds_only_active_body() {
    let broker = broker(1_000_000);
    let store = charged(&broker);
    store.append(request("s", 0, "m0", "one")).expect("first");
    store.append(request("s", 0, "m0", "one")).expect("replay");
    store.append(request("s", 1, "m1", "four")).expect("second");
    assert_eq!(
        broker.ledger_snapshot().expect("snapshot").total_owner_bytes(),
        cost("s", &[("m0", "one", true), ("m1", "four", true)])
    );
    store.truncate_to(&"s".to_string(), 1).expect("truncate");
    let after = cost("s", &[("m0", "one", true), ("m1", "four", false)]);
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), after);
    assert_eq!(
        store.append(request("s", 1, "m1", "four")).expect("late replay"),
        TranscriptAppendDispositionV1::Replayed
    );
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), after);
}
#[test]
fn truncate_refund_failure_preserves_rows_indexes_and_accounting() {
    let broker = broker(1_000_000);
    let store = charged(&broker);
    store.append(request("s", 0, "m0", "one")).expect("first");
    store.append(request("s", 1, "m1", "four")).expect("second");
    let rebuild_fence2 = broker.begin_rebuild().expect("begin rebuild");
    let denied = store.truncate_to(&"s".to_string(), 1);
    assert!(matches!(denied, Err(MemoryError::InvalidState(_))));
    assert_eq!(store.load(&"s".to_string()).expect("load").len(), 2);
    let expected = cost("s", &[("m0", "one", true), ("m1", "four", true)]);
    assert_eq!(store.retained_bytes().expect("bytes"), expected);
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), expected);
    assert!(rebuild_fence2.finish().expect("finish rebuild").drain_error.is_none());
}
#[test]
fn close_refunds_all_sessions_exactly_once_and_exposes_terminal_empty_reads() {
    let broker = broker(1_000_000);
    let store = charged(&broker);
    store.append(request("a", 0, "m0", "one")).expect("first");
    store.append(request("a", 2, "m2", "four")).expect("second");
    store.append(request("b", 9, "m9", "xx")).expect("third");
    assert_eq!(
        broker.ledger_snapshot().expect("before").total_owner_bytes(),
        cost("a", &[("m0", "one", true), ("m2", "four", true)]) + cost("b", &[("m9", "xx", true)])
    );

    SessionTranscriptStore::close(&store).expect("close");
    assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
    assert_eq!(store.retained_bytes().expect("closed bytes"), 0);
    assert_eq!(store.stats().expect("closed stats").item_count, 0);
    assert!(store.load(&"a".to_string()).expect("closed load").is_empty());
    assert_eq!(
        store.load_versioned_snapshot(&"b".to_string()).expect("closed snapshot").version(),
        0
    );
    store.close().expect("idempotent double close");
    assert_eq!(broker.ledger_snapshot().expect("after repeat").total_owner_bytes(), 0);
    assert!(matches!(
        store.append(request("a", 0, "m0", "one")),
        Err(TranscriptAppendErrorV1::Storage(MemoryError::Closed(_)))
    ));
    assert!(matches!(store.truncate_to(&"a".to_string(), 0), Err(MemoryError::Closed(_))));
    assert!(matches!(
        store.retain_sequences_if_version(&"b".to_string(), 0, &[]),
        Err(MemoryError::Closed(_))
    ));
}
#[test]
fn failed_close_preserves_rows_versions_tombstones_and_all_lease_authority() {
    let broker = broker(1_000_000);
    let store = charged(&broker);
    store.append(request("a", 0, "m0", "one")).expect("first");
    store.append(request("b", 1, "m1", "four")).expect("second");
    let version = store.load_versioned_snapshot(&"a".to_string()).expect("before").version();
    let rebuild_fence3 = broker.begin_rebuild().expect("fence");
    assert!(matches!(store.close(), Err(MemoryError::InvalidState(_))));
    assert_eq!(
        broker.ledger_snapshot().expect("fenced ledger").total_owner_bytes(),
        cost("a", &[("m0", "one", true)]) + cost("b", &[("m1", "four", true)])
    );
    assert_eq!(store.stats().expect("restored stats").item_count, 2);
    assert_eq!(store.load(&"b".to_string()).expect("restored row")[0].body(), "four");
    assert_eq!(
        store.load_versioned_snapshot(&"a".to_string()).expect("version").version(),
        version
    );
    assert_eq!(
        store.append(request("a", 0, "m0", "one")).expect("restored evidence"),
        TranscriptAppendDispositionV1::Replayed
    );
    assert!(rebuild_fence3.finish().expect("open fence").drain_error.is_none());
    store.close().expect("retry close");
    assert_eq!(broker.ledger_snapshot().expect("after retry").total_owner_bytes(), 0);
}
#[test]
fn drop_releases_all_live_rows_even_after_storage_poison() {
    let broker = broker(1_000_000);
    let store = Arc::new(charged(&broker));
    store.append(request("a", 0, "m0", "one")).expect("first");
    store.append(request("b", 1, "m1", "four")).expect("second");
    let poisoner = Arc::clone(&store);
    let _ = thread::spawn(move || {
        let _state = poisoner.state.lock().expect("state");
        panic!("poison store after committed charge");
    })
    .join();
    assert!(matches!(store.close(), Err(MemoryError::Poisoned(_))));
    assert!(matches!(store.stats(), Err(MemoryError::Poisoned(_))));
    assert_eq!(
        broker.ledger_snapshot().expect("charged").total_owner_bytes(),
        cost("a", &[("m0", "one", true)]) + cost("b", &[("m1", "four", true)])
    );
    drop(store);
    assert_eq!(broker.ledger_snapshot().expect("drop refund").total_owner_bytes(), 0);
}
#[test]
fn drop_during_rebuild_defers_exact_release_until_finish() {
    let broker = broker(1_000_000);
    let store = charged(&broker);
    store.append(request("a", 0, "m0", "one")).expect("first");
    store.append(request("b", 1, "m1", "four")).expect("second");
    let rebuild_fence4 = broker.begin_rebuild().expect("fence");
    drop(store);
    assert_eq!(
        broker.ledger_snapshot().expect("fenced").total_owner_bytes(),
        cost("a", &[("m0", "one", true)]) + cost("b", &[("m1", "four", true)])
    );
    assert!(rebuild_fence4.finish().expect("drain deferred charges").drain_error.is_none());
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn mutation_lane_rejects_reentry_and_reads_without_exposing_partial_images() {
    let broker = broker(1_000_000);
    let store = charged(&broker);
    store.append(request("s", 0, "m0", "one")).expect("seed");
    let permit = store.lifecycle.begin_mutation().expect("lane owner");
    assert!(matches!(
        store.append(request("s", 1, "m1", "two")),
        Err(TranscriptAppendErrorV1::Storage(MemoryError::OperationInProgress(_)))
    ));
    assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(store.load(&"s".to_string()), Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
    drop(permit);
    assert_eq!(store.load(&"s".to_string()).expect("stable image").len(), 1);
    assert_eq!(
        broker.ledger_snapshot().expect("inert reentry").total_owner_bytes(),
        cost("s", &[("m0", "one", true)])
    );
}
#[test]
fn checked_byte_and_version_overflow_denials_leave_ledger_and_indexes_intact() {
    let broker = broker(1_000_000);
    let store = charged(&broker);
    {
        let mut state = store.state.lock().expect("state");
        state.retained_bytes = u64::MAX;
    }
    assert!(matches!(
        store.append(request("s", 0, "m0", "x")),
        Err(TranscriptAppendErrorV1::Accounting(MemoryError::Capacity(_)))
    ));
    assert!(store.lock_state().expect("state").sessions.is_empty());
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    {
        let mut state = store.state.lock().expect("state");
        state.retained_bytes = 0;
        state.sessions.insert(
            ClientKey::new("s".to_string()),
            SessionState { version: u64::MAX, ..SessionState::default() },
        );
    }
    assert!(matches!(
        store.append(request("s", 0, "m0", "x")),
        Err(TranscriptAppendErrorV1::Storage(MemoryError::Capacity(_)))
    ));
    let session_id = String::from("s");
    assert_eq!(store.lock_state().expect("state").sessions[&session_id].version, u64::MAX);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn failed_multirow_retain_preserves_positions_tombstones_and_version() {
    let broker = broker(1_000_000);
    let store = charged(&broker);
    for (sequence, body) in [(0, "one"), (2, "two"), (4, "four"), (9, "nine")] {
        store.append(request("s", sequence, &format!("m{sequence}"), body)).expect("seed");
    }
    let before = store.load_versioned_snapshot(&"s".to_string()).expect("snapshot");
    let before_bytes = store.retained_bytes().expect("bytes");
    let rebuild_fence5 = broker.begin_rebuild().expect("fence");
    assert!(matches!(
        store.retain_sequences_if_version(&"s".to_string(), before.version(), &[2, 9]),
        Err(MemoryError::InvalidState(_))
    ));
    assert_eq!(store.load_versioned_snapshot(&"s".to_string()).expect("unchanged"), before);
    assert_eq!(store.retained_bytes().expect("unchanged bytes"), before_bytes);
    assert_eq!(
        broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(),
        before_bytes
    );
    assert!(rebuild_fence5.finish().expect("finish").drain_error.is_none());
    assert_eq!(
        store
            .retain_sequences_if_version(&"s".to_string(), before.version(), &[2, 9])
            .expect("retry"),
        TranscriptRetainDispositionV1::Applied { version: before.version() + 1 }
    );
    assert_eq!(
        store.append(request("s", 0, "m0", "one")).expect("evicted retry"),
        TranscriptAppendDispositionV1::Replayed
    );
    let after_cost = cost(
        "s",
        &[("m0", "one", false), ("m2", "two", true), ("m4", "four", false), ("m9", "nine", true)],
    );
    assert_eq!(store.retained_bytes().expect("after retain"), after_cost);
    assert_eq!(
        broker.ledger_snapshot().expect("after retain ledger").total_owner_bytes(),
        after_cost
    );
}
#[test]
fn racing_close_and_append_have_one_linearizable_terminal_image() {
    for _ in 0..16 {
        let broker = broker(1_000_000);
        let store = Arc::new(charged(&broker));
        let barrier = Arc::new(Barrier::new(2));
        let closer = Arc::clone(&store);
        let closer_barrier = Arc::clone(&barrier);
        let close = thread::spawn(move || {
            closer_barrier.wait();
            closer.close()
        });
        barrier.wait();
        let append = store.append(request("s", 0, "m0", "one"));
        let close = close.join().expect("join close");
        assert!(matches!(
            append,
            Ok(TranscriptAppendDispositionV1::Applied)
                | Err(TranscriptAppendErrorV1::Storage(MemoryError::Closed(_)))
                | Err(TranscriptAppendErrorV1::Storage(MemoryError::OperationInProgress(_)))
        ));
        assert!(matches!(close, Ok(()) | Err(MemoryError::OperationInProgress(_))));
        store.close().expect("terminal retry");
        assert!(store.load(&"s".to_string()).expect("terminal rows").is_empty());
        assert_eq!(store.retained_bytes().expect("terminal bytes"), 0);
        assert_eq!(broker.ledger_snapshot().expect("terminal ledger").total_owner_bytes(), 0);
    }
}
