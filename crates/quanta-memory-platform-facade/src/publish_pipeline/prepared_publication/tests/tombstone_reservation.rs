use super::*;

#[test]
fn count_limit_rejects_fresh_identity_before_stranding_live_payload() {
    let service = service_with_limits(None, Some((2, u64::MAX)));
    let aborted = service.prepare(request("id-0", b"a")).expect("first prepare");
    assert_eq!(service.abort(aborted.token()).expect("first abort"), AbortDispositionV1::Aborted);
    let live = service.prepare(request("id-1", b"b")).expect("second prepare");
    assert_identity_evidence_invariant(&service);
    let before = service.stats().expect("before denial");
    assert_eq!(before.prepared_count, 1);
    assert_eq!(before.tombstone_count, 1);
    assert_eq!(before.charged_bytes, evidence_cost("id-0") + live_cost("id-1", b"b"));

    assert!(matches!(
        service.prepare(request("id-2", b"c")),
        Err(PreparedPublicationErrorV1::IdentityEvidenceCapacity {
            requested_count: 1,
            available_count: 0,
            requested_bytes: 132,
            ..
        })
    ));
    assert_eq!(service.stats().expect("denial leaves state unchanged"), before);
    assert_identity_evidence_invariant(&service);
    assert_eq!(service.resolve(live.token()).expect("live payload remains"), b"b".to_vec());

    let replay = service.prepare(request("id-1", b"b")).expect("live replay at capacity");
    assert_eq!(replay.disposition(), PrepareDispositionV1::Replayed);
    assert_eq!(replay.token(), live.token());
    assert!(matches!(
        service.prepare(request("id-1", b"different")),
        Err(PreparedPublicationErrorV1::IdentityConflict { .. })
    ));
    assert!(matches!(
        service.prepare(request("id-0", b"a")),
        Err(PreparedPublicationErrorV1::AbortedToken { .. })
    ));
    assert_eq!(service.stats().expect("replay and conflicts unchanged"), before);

    assert_eq!(
        service.abort(live.token()).expect("abort at full reservation"),
        AbortDispositionV1::Aborted
    );
    assert_eq!(service.stats().expect("retired stats").tombstone_count, 2);
    assert_identity_evidence_invariant(&service);
}

#[test]
fn byte_limit_counts_live_and_aborted_identity_evidence_independently() {
    let evidence = evidence_cost("id-0");
    let service = service_with_limits(None, Some((3, 2 * evidence)));
    let first = service.prepare(request("id-0", b"a")).expect("first prepare");
    service.abort(first.token()).expect("first abort");
    let second = service.prepare(request("id-1", b"b")).expect("second prepare");
    assert_identity_evidence_invariant(&service);
    let before = service.stats().expect("before byte denial");

    assert!(matches!(
        service.prepare(request("id-2", b"c")),
        Err(PreparedPublicationErrorV1::IdentityEvidenceCapacity {
            requested_count: 1,
            available_count: 1,
            requested_bytes: 132,
            available_bytes: 0,
        })
    ));
    assert_eq!(service.stats().expect("byte denial unchanged"), before);
    assert_eq!(service.resolve(second.token()).expect("live payload"), b"b".to_vec());
    assert_eq!(
        service.abort(second.token()).expect("abort at byte limit"),
        AbortDispositionV1::Aborted
    );
    assert_eq!(service.stats().expect("after abort").tombstone_bytes, 2 * evidence);
    assert_identity_evidence_invariant(&service);
}

#[test]
fn full_live_reservation_allows_atomic_mixed_batch_gc() {
    let evidence = evidence_cost("id-0");
    let service = service_with_limits(None, Some((3, 3 * evidence)));
    let first = service.prepare(request("id-0", b"a")).expect("first");
    let second = service.prepare(request("id-1", b"b")).expect("second");
    let third = service.prepare(request("id-2", b"c")).expect("third");
    service.abort(first.token()).expect("first abort");
    let before = service.stats().expect("before batch");
    assert_identity_evidence_invariant(&service);
    assert_eq!(before.prepared_count, 2);
    assert_eq!(before.tombstone_count, 1);
    assert_eq!(service.gc_orphans(&[]).expect("batch at full reservation"), 2);
    assert_identity_evidence_invariant(&service);
    assert!(matches!(
        service.resolve(second.token()),
        Err(PreparedPublicationErrorV1::AbortedToken { .. })
    ));
    assert!(matches!(
        service.resolve(third.token()),
        Err(PreparedPublicationErrorV1::AbortedToken { .. })
    ));
    assert_eq!(
        service.stats().expect("all retired"),
        PreparedPublicationStatsV1 {
            prepared_count: 0,
            charged_bytes: 3 * evidence,
            identity_evidence_count: 3,
            identity_evidence_bytes: 3 * evidence,
            tombstone_count: 3,
            tombstone_bytes: 3 * evidence,
        }
    );
    assert!(matches!(
        service.prepare(request("id-3", b"d")),
        Err(PreparedPublicationErrorV1::IdentityEvidenceCapacity { available_count: 0, .. })
    ));
}

#[test]
fn corrupt_reservation_rejects_retirement_without_moving_payload() {
    let service = service_with_limits(None, Some((1, u64::MAX)));
    let prepared = service.prepare(request("id", b"payload")).expect("prepare");
    let reserved = service.state.lock().expect("state").persistent_bytes;
    service.state.lock().expect("inject corrupt reservation").persistent_bytes = 0;
    let before = service.stats().expect("corrupt preimage");

    assert!(matches!(
        service.abort(prepared.token()),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(_)))
    ));
    assert_eq!(service.stats().expect("unchanged after rejection"), before);
    assert_eq!(service.resolve(prepared.token()).expect("payload remains"), b"payload".to_vec());

    service.state.lock().expect("restore test corruption").persistent_bytes = reserved;
    assert_identity_evidence_invariant(&service);
    assert_eq!(
        service.abort(prepared.token()).expect("retire after repair"),
        AbortDispositionV1::Aborted
    );
    assert_identity_evidence_invariant(&service);
}
