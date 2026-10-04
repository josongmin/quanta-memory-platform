use super::*;

#[test]
fn full_identity_reservation_denies_before_broker_charge_and_still_aborts() {
    let broker = broker(1_000_000);
    let service = PreparedPublicationService::new_budgeted(
        binding(),
        None,
        Some((1, evidence_cost("id-0"))),
        broker.clone(),
    );
    let first = service.prepare(request("id-0", b"payload")).expect("prepare");
    let before_stats = service.stats().expect("before stats");
    let before_ledger = broker.ledger_snapshot().expect("before ledger");
    assert_eq!(before_stats.identity_evidence_count, 1);
    assert_eq!(before_stats.identity_evidence_bytes, evidence_cost("id-0"));
    assert_eq!(before_ledger.total_owner_bytes(), live_cost("id-0", b"payload"));

    assert!(matches!(
        service.prepare(request("id-1", b"other")),
        Err(PreparedPublicationErrorV1::IdentityEvidenceCapacity {
            requested_count: 1,
            available_count: 0,
            requested_bytes: 132,
            available_bytes: 0,
        })
    ));
    assert_eq!(service.stats().expect("denial preserves state"), before_stats);
    assert_eq!(broker.ledger_snapshot().expect("denial preserves ledger"), before_ledger);
    assert_eq!(service.resolve(first.token()).expect("live payload"), b"payload".to_vec());

    assert_eq!(
        service.abort(first.token()).expect("abort at capacity"),
        AbortDispositionV1::Aborted
    );
    assert_eq!(
        broker.ledger_snapshot().expect("retained metadata").total_owner_bytes(),
        evidence_cost("id-0")
    );
    let aborted_stats = service.stats().expect("after abort");
    assert_eq!(aborted_stats.identity_evidence_count, 1);
    assert_eq!(aborted_stats.identity_evidence_bytes, evidence_cost("id-0"));
    assert_eq!(aborted_stats.tombstone_bytes, evidence_cost("id-0"));
    service.close().expect("close releases metadata");
    assert_eq!(broker.ledger_snapshot().expect("closed ledger").total_owner_bytes(), 0);
}
