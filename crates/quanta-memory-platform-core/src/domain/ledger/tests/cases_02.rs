use super::*;

#[test]
fn mixed_kind_release_preserves_charge_kind_partition() {
    // scenario A: payload and metadata charges live in different partition
    // slots (invariant 2). Releasing the metadata ticket must subtract ONLY
    // from the metadata slot, leaving the payload slot intact and the
    // partition exact.
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(10_000_000));
    let payload = reserve_kind(
        &mut ledger,
        MemoryClass::ArtifactCache,
        "o",
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        500,
    )
    .expect("payload reserve");
    let metadata = reserve_kind(
        &mut ledger,
        MemoryClass::ArtifactCache,
        "o",
        MemoryContextKind::Request,
        ChargeKind::RetainedMetadata,
        300,
    )
    .expect("metadata reserve");

    let snap = ledger.snapshot();
    assert_eq!(snap.total_retained_payload_bytes(), 500);
    assert_eq!(snap.total_retained_metadata_bytes(), 300);
    assert_axes_balanced(&snap);

    // Release metadata: payload slot must be untouched.
    ledger.release(&metadata).expect("release metadata ticket");
    let snap = ledger.snapshot();
    assert_eq!(snap.total_retained_metadata_bytes(), 0, "metadata slot drained");
    assert_eq!(
        snap.total_retained_payload_bytes(),
        500,
        "payload slot untouched by metadata release"
    );
    assert_axes_balanced(&snap);

    // Release payload: everything returns to zero, partition still exact.
    ledger.release(&payload).expect("release payload ticket");
    let snap = ledger.snapshot();
    assert_eq!(snap.total_owner_bytes(), 0);
    assert_axes_balanced(&snap);
}
#[test]
fn deterministic_fuzz_preserves_invariants_and_returns_to_zero() {
    // scenario A: a fixed, index-derived (no RNG crate) sequence of reserves
    // and releases across several owners/classes/contexts/kinds. Invariants
    // 1, 2, 5 (<= global) and 6 (within burst pool) must hold at EVERY step,
    // and releasing every live ticket must return all totals to exactly 0.
    let global = 10_000_000u64;
    let mut ledger = ledger_from_policy(
        ResidencyPolicy::new(global)
            .with_shared_burst_pool_bytes(50_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(40_000, 60_000, true))
            .with_class_budget(MemoryClass::EvidenceCache, class_budget(40_000, 60_000, true)),
    );

    let classes = [
        MemoryClass::ArtifactCache,
        MemoryClass::EvidenceCache,
        MemoryClass::TraceIndex, // no budget configured -> only global gates it
    ];
    let owners = ["o0", "o1", "o2"];
    let contexts =
        [MemoryContextKind::Request, MemoryContextKind::Session, MemoryContextKind::Background];
    let kinds =
        [ChargeKind::RetainedPayload, ChargeKind::RetainedMetadata, ChargeKind::LiveScratch];

    let mut live: Vec<ReservationTicket> = Vec::new();

    for i in 0u64..400 {
        // Deterministic pseudo-random decisions derived purely from `i`.
        let do_release = (i % 3 == 2) && !live.is_empty();
        if do_release {
            // Pick a live ticket deterministically and release it.
            let idx = (i.wrapping_mul(2_654_435_761) % (live.len() as u64)) as usize;
            let t = live.swap_remove(idx);
            ledger.release(&t).expect("releasing a live ticket always succeeds");
        } else {
            let class = classes[(i % classes.len() as u64) as usize].clone();
            let owner = owners[((i / 2) % owners.len() as u64) as usize];
            let ctx = contexts[((i / 3) % contexts.len() as u64) as usize];
            let kind = kinds[((i / 5) % kinds.len() as u64) as usize];
            // Modest byte sizes so we mostly stay admissible; some reserves
            // may be denied by a class/global cap — a denial is fine and
            // must not corrupt anything.
            let bytes = 1 + (i.wrapping_mul(97) % 2_000);
            if let Ok(t) = ledger.reserve(class, owner.to_string(), ctx, kind, bytes) {
                live.push(t);
            }
        }

        // Invariants hold at EVERY step, admitted or denied.
        let snap = ledger.snapshot();
        assert!(snap.class_owner_balanced(), "inv 1 at step {i}: {snap:?}");
        assert!(snap.owner_partition_balanced(), "inv 2 at step {i}: {snap:?}");
        assert!(snap.context_owner_balanced(), "3rd axis at step {i}: {snap:?}");
        assert!(
            snap.total_owner_bytes() <= global,
            "inv 5 at step {i}: total {} > global {global}",
            snap.total_owner_bytes()
        );
        // Inv 6: no borrowing class may exceed cap+its share of the pool; in
        // aggregate, over-cap borrow never exceeds the pool.
        let over_artifact =
            ledger.class_usage(&MemoryClass::ArtifactCache).total().saturating_sub(60_000);
        let over_evidence =
            ledger.class_usage(&MemoryClass::EvidenceCache).total().saturating_sub(60_000);
        assert!(
            over_artifact.saturating_add(over_evidence) <= 50_000,
            "inv 6 at step {i}: total borrow exceeds shared pool"
        );
    }

    // Drain every remaining live ticket: totals must return to exactly 0.
    for t in live.drain(..) {
        ledger.release(&t).expect("final drain release succeeds");
    }
    let snap = ledger.snapshot();
    assert_eq!(snap.total_owner_bytes(), 0, "all owner bytes returned");
    assert_eq!(snap.total_class_bytes(), 0, "all class bytes returned");
    assert_eq!(snap.total_context_bytes(), 0, "all context bytes returned");
    assert_axes_balanced(&snap);
}
#[test]
fn sep23_fup004_honest_mixed_kind_snapshot_validates() {
    let ledger = sep23_mixed_kind_ledger();
    let snap = ledger.snapshot();
    snap.validate_exact().expect("honest kind splits validate");
    assert_eq!(
        LedgerSnapshot::checked_kind_totals(&snap.class_usage).expect("kinds"),
        [1_000, 250, 500]
    );
    assert_eq!(
        LedgerSnapshot::checked_kind_totals(&snap.owner_usage).expect("kinds"),
        [1_000, 250, 500]
    );
    assert_eq!(
        LedgerSnapshot::checked_kind_totals(&snap.context_usage).expect("kinds"),
        [1_000, 250, 500]
    );
}
#[test]
fn sep23_audit_admitted_recharge_moves_kind_vectors_exactly() {
    let (mut ledger, tickets) = sep23_audit_mixed_kind_ledger_with_tickets();
    // Recharge the payload leg into a metadata leg on another class: kind
    // [1000,250,500] becomes [0,500,500] on every axis.
    let _replacement = ledger
        .replace_committed(
            &tickets[0],
            MemoryClass::EvidenceCache,
            "owner:a".to_string(),
            MemoryContextKind::Session,
            ChargeKind::RetainedMetadata,
            250,
        )
        .expect("admitted recharge");
    let snap = ledger.snapshot();
    snap.validate_exact().expect("recharged ledger validates");
    assert_eq!(
        sep23_audit_kind_vectors(&snap),
        ([0, 500, 500], [0, 500, 500], [0, 500, 500]),
        "recharge moves the kind split on every axis"
    );
    assert_eq!(snap.total_owner_bytes(), 1_000);
}
#[test]
fn sep23_audit_batch_release_moves_kind_vectors_exactly() {
    let (mut ledger, tickets) = sep23_audit_mixed_kind_ledger_with_tickets();
    ledger.release_batch(&tickets[0..2]).expect("batch release");
    let snap = ledger.snapshot();
    snap.validate_exact().expect("batch-released ledger validates");
    assert_eq!(
        sep23_audit_kind_vectors(&snap),
        ([0, 0, 500], [0, 0, 500], [0, 0, 500]),
        "only the scratch leg remains on every axis"
    );
    ledger.release_batch(&tickets[2..3]).expect("final release");
    let drained = ledger.snapshot();
    drained.validate_exact().expect("drained ledger validates");
    assert_eq!(sep23_audit_kind_vectors(&drained), ([0, 0, 0], [0, 0, 0], [0, 0, 0]));
}
#[test]
fn sep23_fup004_class_kind_swap_preserving_totals_is_rejected() {
    let ledger = sep23_mixed_kind_ledger();
    let mut snap = ledger.snapshot();
    // Move 250 payload<->metadata inside ONE class bucket; every scalar
    // total stays identical.
    let bucket = snap.class_usage.get_mut(&MemoryClass::ArtifactCache).expect("bucket");
    assert_eq!((bucket.retained_payload_bytes, bucket.retained_metadata_bytes), (1_000, 0));
    bucket.retained_payload_bytes = 750;
    bucket.retained_metadata_bytes = 250;
    assert_eq!(snap.total_class_bytes(), snap.total_owner_bytes(), "totals still balance");
    assert_eq!(snap.total_owner_bytes(), snap.total_context_bytes());
    let verdict = snap.validate_exact();
    assert!(
        matches!(verdict, Err(MemoryError::InvalidState(_))),
        "class-confined kind swap must be rejected, got {verdict:?}"
    );
}
#[test]
fn sep23_fup004_context_kind_swap_preserving_totals_is_rejected() {
    let ledger = sep23_mixed_kind_ledger();
    let mut snap = ledger.snapshot();
    let bucket = snap.context_usage.get_mut(&MemoryContextKind::Request).expect("bucket");
    assert_eq!((bucket.retained_payload_bytes, bucket.live_scratch_bytes), (1_000, 500));
    bucket.retained_payload_bytes = 500;
    bucket.live_scratch_bytes = 1_000;
    assert_eq!(snap.total_class_bytes(), snap.total_owner_bytes(), "totals still balance");
    assert_eq!(snap.total_owner_bytes(), snap.total_context_bytes());
    let verdict = snap.validate_exact();
    assert!(
        matches!(verdict, Err(MemoryError::InvalidState(_))),
        "context-confined kind swap must be rejected, got {verdict:?}"
    );
}
#[test]
fn sep23_fup004_owner_kind_swap_preserving_totals_is_rejected() {
    let ledger = sep23_mixed_kind_ledger();
    let mut snap = ledger.snapshot();
    // The old owner-side re-sum cannot see this either: the kinds still
    // add up to the same owner total.
    let bucket = snap.owner_usage.get_mut("owner:a").expect("bucket");
    assert_eq!((bucket.retained_payload_bytes, bucket.retained_metadata_bytes), (1_000, 250));
    bucket.retained_payload_bytes = 250;
    bucket.retained_metadata_bytes = 1_000;
    assert_eq!(snap.total_class_bytes(), snap.total_owner_bytes(), "totals still balance");
    let verdict = snap.validate_exact();
    assert!(
        matches!(verdict, Err(MemoryError::InvalidState(_))),
        "owner-confined kind swap must be rejected, got {verdict:?}"
    );
}
