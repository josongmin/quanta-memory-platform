use super::*;

#[test]
fn concurrent_identical_appends_have_one_applied_winner() {
    const N: usize = 8;
    let store = Arc::new(store());
    let barrier = Arc::new(Barrier::new(N));
    let handles: Vec<_> = (0..N)
        .map(|_| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                loop {
                    match store.append(request("s", 0, "m0", "body")) {
                        Ok(disposition) => break disposition,
                        Err(TranscriptAppendErrorV1::Storage(
                            MemoryError::OperationInProgress(_),
                        )) => thread::yield_now(),
                        Err(error) => panic!("append: {error:?}"),
                    }
                }
            })
        })
        .collect();
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().expect("join")).collect();
    assert_eq!(
        outcomes.iter().filter(|d| **d == TranscriptAppendDispositionV1::Applied).count(),
        1
    );
    assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
    assert_eq!(store.retained_bytes().expect("bytes"), cost("s", &[("m0", "body", true)]));
}
#[test]
fn concurrent_conflicting_appends_have_one_applied_winner() {
    let store = Arc::new(store());
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["left", "right"]
        .into_iter()
        .map(|body| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                loop {
                    match store.append(request("s", 0, "m0", body)) {
                        Err(TranscriptAppendErrorV1::Storage(
                            MemoryError::OperationInProgress(_),
                        )) => thread::yield_now(),
                        result => break result,
                    }
                }
            })
        })
        .collect();
    let outcomes: Vec<_> = handles.into_iter().map(|handle| handle.join().expect("join")).collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Ok(TranscriptAppendDispositionV1::Applied)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                Err(TranscriptAppendErrorV1::PayloadConflict { sequence: 0 })
            ))
            .count(),
        1
    );
    assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
}
#[test]
fn truncate_retains_tombstone_so_late_retry_cannot_reappend() {
    let store = store();
    store.append(request("s", 0, "m0", "old")).expect("first");
    store.append(request("s", 1, "m1", "drop")).expect("second");
    store.truncate_to(&"s".to_string(), 1).expect("truncate");
    assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
    assert_eq!(
        store.append(request("s", 1, "m1", "drop")).expect("late retry"),
        TranscriptAppendDispositionV1::Replayed
    );
    assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
    assert!(matches!(
        store.append(request("s", 1, "m1", "drift")),
        Err(TranscriptAppendErrorV1::PayloadConflict { sequence: 1 })
    ));
}
#[test]
fn truncate_to_zero_retains_all_identity_tombstones() {
    let store = store();
    store.append(request("s", 0, "m0", "zero")).expect("zero");
    store.append(request("s", 2, "m2", "two")).expect("two");
    store.truncate_to(&"s".to_string(), 0).expect("truncate all");

    assert!(store.load(&"s".to_string()).expect("load").is_empty());
    assert_eq!(
        store.retained_bytes().expect("bytes"),
        cost("s", &[("m0", "zero", false), ("m2", "two", false)])
    );
    assert_eq!(
        store.append(request("s", 0, "m0", "zero")).expect("zero replay"),
        TranscriptAppendDispositionV1::Replayed
    );
    assert_eq!(
        store.append(request("s", 2, "m2", "two")).expect("two replay"),
        TranscriptAppendDispositionV1::Replayed
    );
    assert!(store.load(&"s".to_string()).expect("load after retries").is_empty());
}
#[test]
fn zero_body_with_large_ids_and_session_retains_semantic_evidence_cost() {
    let store = store();
    let session = "s".repeat(512);
    let message_id = "m".repeat(1024);
    store.append(request(&session, 0, &message_id, "")).expect("zero-body append");
    let before = cost(&session, &[(&message_id, "", true)]);
    assert_eq!(store.session_bytes(&session).expect("session bytes"), before);
    assert_eq!(store.retained_bytes().expect("total"), before);
    store.truncate_to(&session, 0).expect("truncate body");
    let evidence_only = cost(&session, &[(&message_id, "", false)]);
    assert_eq!(store.session_bytes(&session).expect("persistent evidence"), evidence_only);
    assert_eq!(store.retained_bytes().expect("total evidence"), evidence_only);
    assert_eq!(
        store.append(request(&session, 0, &message_id, "")).expect("late replay"),
        TranscriptAppendDispositionV1::Replayed
    );
    store.close().expect("close evidence");
    assert_eq!(store.retained_bytes().expect("terminal"), 0);
}
#[test]
fn selective_retain_preserves_sparse_rows_and_evicted_identity_tombstones() {
    let store = store();
    store.append(request("s", 0, "m0", "zero")).expect("zero");
    store.append(request("s", 1, "m1", "one")).expect("one");
    store.append(request("s", 2, "m2", "two")).expect("two");

    store.retain_sequences(&"s".to_string(), &[0, 2]).expect("retain sparse set");

    let rows = store.load(&"s".to_string()).expect("load");
    assert_eq!(rows.iter().map(TranscriptRecordV1::sequence).collect::<Vec<_>>(), vec![0, 2]);
    assert_eq!(
        store.retained_bytes().expect("bytes"),
        cost("s", &[("m0", "zero", true), ("m1", "one", false), ("m2", "two", true)])
    );
    assert_eq!(
        store.append(request("s", 1, "m1", "one")).expect("evicted replay"),
        TranscriptAppendDispositionV1::Replayed
    );
    assert_eq!(store.load(&"s".to_string()).expect("load after replay").len(), 2);
}
#[test]
fn selective_retain_rejects_invalid_selector_without_mutation() {
    let store = store();
    store.append(request("s", 0, "m0", "zero")).expect("zero");
    store.append(request("s", 1, "m1", "one")).expect("one");
    let before = store.retained_bytes().expect("before");

    assert!(matches!(
        store.retain_sequences(&"s".to_string(), &[0, 0]),
        Err(MemoryError::InvalidState(_))
    ));
    assert!(matches!(
        store.retain_sequences(&"s".to_string(), &[0, 9]),
        Err(MemoryError::InvalidState(_))
    ));
    assert_eq!(store.load(&"s".to_string()).expect("load").len(), 2);
    assert_eq!(store.retained_bytes().expect("after"), before);
}
#[test]
fn versioned_snapshot_retain_rejects_concurrent_append_without_dropping_rows() {
    let store = store();
    store.append(request("s", 0, "m0", "zero")).expect("zero");
    store.append(request("s", 1, "m1", "one")).expect("one");
    let stale = store.load_versioned_snapshot(&"s".to_string()).expect("snapshot");
    assert_eq!(stale.version(), 2);
    store.append(request("s", 2, "m2", "two")).expect("concurrent append");
    assert_eq!(
        store
            .retain_sequences_if_version(&"s".to_string(), stale.version(), &[0])
            .expect("conditional retain"),
        TranscriptRetainDispositionV1::StaleSnapshot { expected_version: 2, actual_version: 3 }
    );
    assert_eq!(
        store
            .load(&"s".to_string())
            .expect("load")
            .iter()
            .map(TranscriptRecordV1::sequence)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );

    let current = store.load_versioned_snapshot(&"s".to_string()).expect("current snapshot");
    assert_eq!(
        store
            .retain_sequences_if_version(&"s".to_string(), current.version(), &[0, 2])
            .expect("current retain"),
        TranscriptRetainDispositionV1::Applied { version: 4 }
    );
    assert_eq!(
        store
            .load(&"s".to_string())
            .expect("load after retain")
            .iter()
            .map(TranscriptRecordV1::sequence)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
}
#[test]
fn checked_load_reports_poisoned_storage_instead_of_empty_success() {
    let store = Arc::new(store());
    let poisoner = Arc::clone(&store);
    let _ = thread::spawn(move || {
        let _guard = poisoner.state.lock().expect("lock");
        panic!("deterministic poison");
    })
    .join();
    assert!(matches!(store.load(&"s".to_string()), Err(MemoryError::Poisoned(_))));
    assert!(matches!(store.stats(), Err(MemoryError::Poisoned(_))));
}
#[test]
fn ordered_load_sorts_by_caller_sequence_and_allows_declared_gaps() {
    let store = store();
    store.append(request("s", 7, "m7", "seven")).expect("seven");
    store.append(request("s", 2, "m2", "two")).expect("two");
    let rows = store.load(&"s".to_string()).expect("load");
    assert_eq!(rows.iter().map(TranscriptRecordV1::sequence).collect::<Vec<_>>(), vec![2, 7]);
}
