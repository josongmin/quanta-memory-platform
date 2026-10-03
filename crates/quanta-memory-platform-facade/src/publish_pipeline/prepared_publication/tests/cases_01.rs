use super::*;

#[test]
fn unbudgeted_admission_never_exceeds_a_complete_gc_root_set() {
    let service = service(None);
    let mut roots = Vec::with_capacity(MAX_REFERENCED_ROOT_TOKENS);
    for index in 0..MAX_REFERENCED_ROOT_TOKENS {
        let identity = format!("root-{index}");
        roots.push(
            service.prepare(request(&identity, b"")).expect("within root bound").token().clone(),
        );
    }
    assert!(service.orphan_tokens(&roots).expect("complete roots").is_empty());
    let before = service.stats().expect("at-cap stats");
    assert!(matches!(
        service.prepare(request("one-too-many", b"")),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::Capacity(_)))
    ));
    assert_eq!(service.stats().expect("denial unchanged"), before);
    roots.pop();
    assert_eq!(service.gc_orphans(&roots).expect("free one slot"), 1);
    service.prepare(request("replacement", b"")).expect("freed slot admits");
}
#[test]
fn prepare_capacity_failure_leaves_all_state_unchanged() {
    let service = service(Some(3));
    let err = service.prepare(request("id-1", b"four")).expect_err("capacity denial");
    assert!(matches!(err, PreparedPublicationErrorV1::Capacity { .. }));
    assert_eq!(service.stats().expect("stats"), PreparedPublicationStatsV1::default());
    assert!(service.orphan_tokens(&[]).expect("orphans").is_empty());
}
#[test]
fn prepared_payload_is_only_reachable_by_exact_token() {
    let service = service(None);
    let receipt = service.prepare(request("id-1", b"payload")).expect("prepare");
    assert_eq!(receipt.disposition(), PrepareDispositionV1::Applied);
    assert_eq!(service.resolve(receipt.token()).expect("resolve"), b"payload".to_vec());
    let forged = PreparedPublicationTokenV1::from_bytes([0xA5; 32]);
    assert!(matches!(
        service.resolve(&forged),
        Err(PreparedPublicationErrorV1::UnknownToken { .. })
    ));
    assert_eq!(service.stats().expect("stats").prepared_count, 1);
}
#[test]
fn identical_prepare_replays_same_token_without_second_charge() {
    let service = service(None);
    let first = service.prepare(request("id-1", b"payload")).expect("first");
    let replay = service.prepare(request("id-1", b"payload")).expect("replay");
    assert_eq!(first.token(), replay.token());
    assert_eq!(first.disposition(), PrepareDispositionV1::Applied);
    assert_eq!(replay.disposition(), PrepareDispositionV1::Replayed);
    assert_eq!(service.stats().expect("stats").charged_bytes, live_cost("id-1", b"payload"));
}
#[test]
fn identity_payload_drift_is_typed_conflict() {
    let service = service(None);
    let original = service.prepare(request("id-1", b"old")).expect("original");
    let err = service.prepare(request("id-1", b"new")).expect_err("payload drift");
    assert!(matches!(err, PreparedPublicationErrorV1::IdentityConflict { .. }));
    assert_eq!(service.resolve(original.token()).expect("original intact"), b"old".to_vec());
    assert_eq!(service.stats().expect("stats").charged_bytes, live_cost("id-1", b"old"));
}
#[test]
fn abort_releases_exact_payload_and_is_retry_safe() {
    let service = service(None);
    let receipt = service.prepare(request("id-1", b"payload")).expect("prepare");
    assert_eq!(service.abort(receipt.token()).expect("abort"), AbortDispositionV1::Aborted);
    assert_eq!(service.abort(receipt.token()).expect("abort replay"), AbortDispositionV1::Replayed);
    assert!(matches!(
        service.resolve(receipt.token()),
        Err(PreparedPublicationErrorV1::AbortedToken { .. })
    ));
    assert_eq!(
        service.stats().expect("stats"),
        PreparedPublicationStatsV1 {
            prepared_count: 0,
            charged_bytes: evidence_cost("id-1"),
            tombstone_count: 1,
            tombstone_bytes: evidence_cost("id-1"),
        }
    );
}
#[test]
fn concurrent_identical_prepare_has_one_applied_winner() {
    const N: usize = 8;
    let service = Arc::new(service(None));
    let barrier = Arc::new(Barrier::new(N));
    let handles: Vec<_> = (0..N)
        .map(|_| {
            let service = Arc::clone(&service);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                prepare_after_busy(&service, request("same", b"payload")).expect("prepare")
            })
        })
        .collect();
    let receipts: Vec<_> = handles.into_iter().map(|h| h.join().expect("join")).collect();
    assert_eq!(
        receipts.iter().filter(|r| r.disposition() == PrepareDispositionV1::Applied).count(),
        1
    );
    assert_eq!(
        receipts.iter().filter(|r| r.disposition() == PrepareDispositionV1::Replayed).count(),
        N - 1
    );
    assert!(receipts.iter().all(|r| r.token() == receipts[0].token()));
    assert_eq!(service.stats().expect("stats").charged_bytes, live_cost("same", b"payload"));
}
#[test]
fn concurrent_conflicting_prepare_has_one_winner() {
    let service = Arc::new(service(None));
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [b"left".to_vec(), b"right".to_vec()]
        .into_iter()
        .map(|payload| {
            let service = Arc::clone(&service);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                prepare_after_busy(&service, request("same", &payload))
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().expect("join")).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(PreparedPublicationErrorV1::IdentityConflict { .. })))
            .count(),
        1
    );
    assert_eq!(service.stats().expect("stats").prepared_count, 1);
}
#[test]
fn root_referenced_token_is_never_enumerated_or_collected_as_an_orphan() {
    let service = service(None);
    let first = service.prepare(request("id-1", b"one")).expect("first");
    let second = service.prepare(request("id-2", b"two")).expect("second");
    let referenced = [second.token().clone()];
    assert_eq!(service.orphan_tokens(&referenced).expect("orphans"), vec![first.token().clone()]);
    assert_eq!(service.gc_orphans(&referenced).expect("gc"), 1);
    assert!(matches!(
        service.resolve(first.token()),
        Err(PreparedPublicationErrorV1::AbortedToken { .. })
    ));
    assert_eq!(service.resolve(second.token()).expect("survivor"), b"two".to_vec());
    assert_eq!(service.stats().expect("stats").prepared_count, 1);
}
#[test]
fn orphan_gc_with_unknown_root_reference_is_batch_atomic() {
    let service = service(None);
    let first = service.prepare(request("id-1", b"one")).expect("first");
    let second = service.prepare(request("id-2", b"two")).expect("second");
    let forged = PreparedPublicationTokenV1::from_bytes([0xA5; 32]);

    let error =
        service.gc_orphans(&[forged]).expect_err("the root snapshot must reject an unknown token");
    assert!(matches!(error, PreparedPublicationErrorV1::UnknownToken { .. }));
    assert_eq!(service.resolve(first.token()).expect("first intact"), b"one".to_vec());
    assert_eq!(service.resolve(second.token()).expect("second intact"), b"two".to_vec());
    assert_eq!(
        service.stats().expect("stats"),
        PreparedPublicationStatsV1 {
            prepared_count: 2,
            charged_bytes: live_cost("id-1", b"one") + live_cost("id-2", b"two"),
            ..PreparedPublicationStatsV1::default()
        }
    );
}
#[test]
fn aborted_root_reference_fails_closed_without_collecting_other_entries() {
    let service = service(None);
    let aborted = service.prepare(request("id-1", b"one")).expect("aborted");
    let survivor = service.prepare(request("id-2", b"two")).expect("survivor");
    service.abort(aborted.token()).expect("abort");

    let error = service
        .gc_orphans(&[aborted.token().clone()])
        .expect_err("an aborted canonical-root reference must fail closed");
    assert!(matches!(error, PreparedPublicationErrorV1::AbortedToken { .. }));
    assert_eq!(service.resolve(survivor.token()).expect("survivor intact"), b"two".to_vec());
    assert_eq!(
        service.stats().expect("stats"),
        PreparedPublicationStatsV1 {
            prepared_count: 1,
            charged_bytes: evidence_cost("id-1") + live_cost("id-2", b"two"),
            tombstone_count: 1,
            tombstone_bytes: evidence_cost("id-1"),
        }
    );
}
#[test]
fn tombstone_limits_bound_n_cycle_churn_and_fail_closed_at_saturation() {
    const MAX_TOMBSTONES: usize = 4;
    const TOMBSTONE_BYTES_PER_CYCLE: u64 = 132;
    let service = service_with_limits(
        None,
        Some((MAX_TOMBSTONES, MAX_TOMBSTONES as u64 * TOMBSTONE_BYTES_PER_CYCLE)),
    );

    for cycle in 0..MAX_TOMBSTONES {
        let identity = format!("id-{cycle}");
        service.prepare(request(&identity, b"x")).expect("prepare");
        assert_eq!(service.gc_orphans(&[]).expect("gc"), 1);
    }
    assert_eq!(
        service.stats().expect("bounded stats"),
        PreparedPublicationStatsV1 {
            prepared_count: 0,
            charged_bytes: MAX_TOMBSTONES as u64 * TOMBSTONE_BYTES_PER_CYCLE,
            tombstone_count: MAX_TOMBSTONES,
            tombstone_bytes: MAX_TOMBSTONES as u64 * TOMBSTONE_BYTES_PER_CYCLE,
        }
    );

    let saturated = service.prepare(request("id-4", b"x")).expect("prepare at limit");
    let error = service.gc_orphans(&[]).expect_err("GC must fail before dropping payload");
    assert!(matches!(
        error,
        PreparedPublicationErrorV1::TombstoneCapacity {
            requested_count: 1,
            available_count: 0,
            requested_bytes: TOMBSTONE_BYTES_PER_CYCLE,
            available_bytes: 0,
        }
    ));
    assert_eq!(service.resolve(saturated.token()).expect("payload intact"), b"x".to_vec());
    assert_eq!(
        service.stats().expect("saturated stats"),
        PreparedPublicationStatsV1 {
            prepared_count: 1,
            charged_bytes: MAX_TOMBSTONES as u64 * TOMBSTONE_BYTES_PER_CYCLE
                + live_cost("id-4", b"x"),
            tombstone_count: MAX_TOMBSTONES,
            tombstone_bytes: MAX_TOMBSTONES as u64 * TOMBSTONE_BYTES_PER_CYCLE,
        }
    );
}
