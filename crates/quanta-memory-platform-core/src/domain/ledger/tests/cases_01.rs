use super::*;

#[test]
fn zero_byte_tickets_obey_independent_live_ticket_cap_fail_closed() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
    let mut tickets =
        Vec::with_capacity(crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER);
    for _ in 0..crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER {
        tickets.push(
            reserve(&mut ledger, MemoryClass::ArtifactCache, "zero", 0)
                .expect("ticket count below the independent cap"),
        );
    }
    let before = ledger.snapshot();
    assert_eq!(
        before.live_ticket_count,
        crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
    );
    let denied = reserve(&mut ledger, MemoryClass::ArtifactCache, "zero", 0);
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(ledger.snapshot(), before, "denied zero-byte ticket must be atomic");

    ledger.rollback_pending(&tickets.pop().expect("one ticket exists")).expect("free slot");
    reserve(&mut ledger, MemoryClass::ArtifactCache, "zero", 0)
        .expect("released ticket returns one bounded slot");
    assert_eq!(
        ledger.snapshot().live_ticket_count,
        crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
    );
}
#[test]
fn committed_replacement_at_live_ticket_cap_is_a_net_zero_authority_swap() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
    let mut tickets =
        Vec::with_capacity(crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER);
    for _ in 0..crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER {
        let ticket = reserve(&mut ledger, MemoryClass::ArtifactCache, "zero", 0)
            .expect("fill ticket cap with zero-byte authority");
        ledger.commit(&ticket).expect("commit ticket");
        tickets.push(ticket);
    }

    let old = tickets.pop().expect("replacement source");
    let before_denial = ledger.snapshot();
    let denied = ledger.replace_committed(
        &old,
        MemoryClass::ArtifactCache,
        "replacement".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        1_001,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(ledger.snapshot(), before_denial, "denied swap restores the old ticket");
    let replacement = ledger
        .replace_committed(
            &old,
            MemoryClass::ArtifactCache,
            "replacement".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        )
        .expect("replacement does not transiently require another ticket slot");
    let snap = ledger.snapshot();
    snap.validate_exact().expect("replacement preserves all ledger axes");
    assert_eq!(
        snap.live_ticket_count,
        crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
    );
    assert_eq!(snap.total_owner_bytes(), 1);
    assert!(matches!(ledger.release(&old), Err(MemoryError::InvalidState(_))));

    ledger.release(&replacement).expect("release replacement");
    for ticket in tickets {
        ledger.release(&ticket).expect("release peer");
    }
    let drained = ledger.snapshot();
    drained.validate_exact().expect("drained ledger validates");
    assert_eq!(drained.live_ticket_count, 0);
    assert_eq!(drained.total_owner_bytes(), 0);
}
#[test]
fn owner_key_byte_limit_is_exact_and_denial_is_fail_closed() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
    let max = "x".repeat(crate::domain::management_limits::MAX_OWNER_KEY_BYTES);
    reserve(&mut ledger, MemoryClass::ArtifactCache, &max, 0)
        .expect("exact byte limit is admitted");
    let before = ledger.snapshot();
    let too_long = format!("{max}x");
    let denied = reserve(&mut ledger, MemoryClass::ArtifactCache, &too_long, 0);
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(ledger.snapshot(), before, "invalid owner key must not mutate");

    let mut utf8_ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
    let multibyte_exact = "é".repeat(crate::domain::management_limits::MAX_OWNER_KEY_BYTES / 2);
    reserve(&mut utf8_ledger, MemoryClass::ArtifactCache, &multibyte_exact, 0)
        .expect("limit counts UTF-8 storage bytes");
    let overlong_utf8 = format!("{multibyte_exact}é");
    assert!(matches!(
        reserve(&mut utf8_ledger, MemoryClass::ArtifactCache, &overlong_utf8, 0),
        Err(MemoryError::Capacity(_))
    ));
}
#[test]
fn reserve_denied_when_global_hard_limit_would_be_exceeded() {
    // Permissive class budgets and no owner quota: only global can deny.
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
    reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 600).expect("fits under global");
    let before = ledger.snapshot();

    let denied = reserve(&mut ledger, MemoryClass::EvidenceCache, "o2", 600);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "total 1200 > global 1000 must be a typed capacity error, got {denied:?}"
    );
    // fail-closed: denied reserve leaves the ledger byte-for-byte unchanged.
    assert_eq!(ledger.snapshot(), before, "denied global reserve must not mutate");
    assert_eq!(ledger.snapshot().total_owner_bytes(), 600);
}
#[test]
fn reserve_for_unconfigured_class_is_denied_without_mutation() {
    let validated = ValidatedResidencyPolicy::try_from(ResidencyPolicy::new(1_000))
        .expect("mandatory classes are total");
    let mut ledger = AccountingLedger::new(validated);
    let before = ledger.snapshot();

    let denied = reserve(&mut ledger, MemoryClass::SpillFileCache, "future", 1);

    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(ledger.snapshot(), before, "unconfigured-class denial must not mutate");
}
#[test]
fn global_ceiling_reserves_headroom_for_non_headroom_classes() {
    // global 1000, headroom 200 -> non-headroom classes share at most 800,
    // but the Headroom class itself may draw into the reserved band.
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000).with_headroom_bytes(200));

    let denied = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 900);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "900 > (1000 - 200 headroom) must be denied for a non-headroom class"
    );
    assert_eq!(ledger.snapshot().total_owner_bytes(), 0, "denial must not mutate");

    // The Headroom class is allowed to consume the reserved band up to global.
    reserve(&mut ledger, MemoryClass::Headroom, "o", 900)
        .expect("Headroom class may use the reserved band up to the global ceiling");
    assert_eq!(ledger.snapshot().total_owner_bytes(), 900);
}
#[test]
fn headroom_usage_does_not_consume_protected_non_headroom_band_twice() {
    let mut policy = ResidencyPolicy::new(1_000).with_headroom_bytes(200);
    policy.classes.insert(
        MemoryClass::Headroom,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: 200,
            hard_cap_bytes: 200,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Last,
        },
    );
    let mut ledger = ledger_from_policy(policy);

    reserve(&mut ledger, MemoryClass::Headroom, "headroom", 200).expect("headroom band");
    reserve(&mut ledger, MemoryClass::ArtifactCache, "work", 800)
        .expect("non-headroom owns its separate 800-byte band");
    assert_eq!(ledger.snapshot().total_owner_bytes(), 1_000);
}
#[test]
fn reserved_floor_protects_class_from_peer_consumption() {
    let mut policy = ResidencyPolicy::new(1_000);
    policy.classes.insert(
        MemoryClass::ArtifactCache,
        MemoryClassBudget {
            reserved_bytes: 400,
            soft_cap_bytes: 700,
            hard_cap_bytes: 1_000,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        },
    );
    policy.classes.insert(
        MemoryClass::EvidenceCache,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: 1_000,
            hard_cap_bytes: 1_000,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        },
    );
    let mut ledger = ledger_from_policy(policy);

    assert!(matches!(
        reserve(&mut ledger, MemoryClass::EvidenceCache, "peer", 601),
        Err(MemoryError::Capacity(_))
    ));
    assert_eq!(ledger.snapshot().total_owner_bytes(), 0, "denial is fail-closed");
    reserve(&mut ledger, MemoryClass::EvidenceCache, "peer", 600).expect("unprotected band");
    reserve(&mut ledger, MemoryClass::ArtifactCache, "protected", 400)
        .expect("reserved class can consume its floor");
    assert_eq!(ledger.snapshot().total_owner_bytes(), 1_000);
}
#[test]
fn double_release_of_a_cloned_ticket_is_rejected_and_does_not_corrupt() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
    let ticket = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 100_000).expect("reserve");
    ledger.commit(&ticket).expect("commit");

    // First release returns the bytes.
    ledger.release(&ticket).expect("first release succeeds");
    let after_first = ledger.snapshot();
    assert_eq!(after_first.total_owner_bytes(), 0, "release returns the bytes once");

    // A cloned ticket released a second time MUST be rejected, and must not
    // drive any counter below the truth (invariants 1 & 2 stay intact).
    let twin = ticket.clone();
    let second = ledger.release(&twin);
    assert!(
        matches!(second, Err(MemoryError::InvalidState(_))),
        "double release must be a typed invalid-state error, got {second:?}"
    );
    assert_eq!(ledger.snapshot(), after_first, "rejected double release must not mutate");
    assert!(ledger.snapshot().class_owner_balanced());
    assert!(ledger.snapshot().owner_partition_balanced());
}
#[test]
fn commit_after_release_is_rejected() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
    let ticket = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 10_000).expect("reserve");
    ledger.release(&ticket).expect("release uncommitted reservation");

    assert!(
        matches!(ledger.commit(&ticket), Err(MemoryError::InvalidState(_))),
        "committing a released ticket must be rejected"
    );
}
#[test]
fn second_commit_of_same_reservation_is_rejected() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
    let ticket = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 10_000).expect("reserve");
    let replay = ticket.clone();
    ledger.commit(&ticket).expect("first commit");
    let before = ledger.snapshot();

    let second = ledger.commit(&replay);

    assert!(matches!(second, Err(MemoryError::InvalidState(_))));
    assert_eq!(ledger.snapshot(), before, "duplicate commit must not mutate counters");
}
#[test]
fn ticket_id_exhaustion_fails_without_reuse() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
    ledger.next_ticket_id = Some(u64::MAX);
    let last = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 1)
        .expect("last representable id is issued once");
    assert_eq!(last.id(), u64::MAX);
    let before = ledger.snapshot();

    let exhausted = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 1);

    assert!(matches!(exhausted, Err(MemoryError::InvalidState(_))));
    assert_eq!(
        reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 1_000_001),
        Err(MemoryError::InvalidState("reservation ticket id space exhausted".into())),
        "an impossible capacity request cannot mask permanent ID exhaustion"
    );
    assert_eq!(ledger.snapshot(), before, "exhaustion must not mutate or reuse an id");
}
#[test]
fn committed_replacement_cannot_recycle_old_id_after_identifier_exhaustion() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(100));
    let old = reserve(&mut ledger, MemoryClass::ArtifactCache, "old", 40).expect("old ticket");
    ledger.commit(&old).expect("old committed");
    ledger.next_ticket_id = Some(u64::MAX);
    let last = reserve(&mut ledger, MemoryClass::ArtifactCache, "last", 0).expect("last id");
    ledger.commit(&last).expect("zero-byte last authority");
    let before = ledger.snapshot();
    for bytes in [0, 20, 101] {
        assert_eq!(
            ledger.replace_committed(
                &old,
                MemoryClass::ArtifactCache,
                "new".into(),
                MemoryContextKind::Session,
                ChargeKind::RetainedMetadata,
                bytes
            ),
            Err(MemoryError::InvalidState("reservation ticket id space exhausted".into())),
            "replacement {bytes} requires a fresh monotonic identity even with ticket delta zero"
        );
        assert_eq!(ledger.snapshot(), before, "permanent exhaustion preserves every old axis");
    }
    ledger.release(&old).expect("exhaustion does not strand old release authority");
    ledger.release(&last).expect("drain last ticket");
    assert_eq!(ledger.snapshot().live_ticket_count, 0);
    assert_eq!(ledger.snapshot().total_owner_bytes(), 0);
}
#[test]
fn borrow_over_class_cap_is_bounded_by_shared_burst_pool() {
    // global is roomy so only the class cap / burst pool gate the reserve.
    let mut ledger = ledger_from_policy(
        ResidencyPolicy::new(10_000_000)
            .with_shared_burst_pool_bytes(100)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(400, 600, true)),
    );

    // Fill to the hard cap (no borrow yet).
    reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 600).expect("up to hard cap");
    // Borrow 50 of the 100-byte pool: admitted.
    reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 50).expect("borrow within pool");
    // A further 60 would push total borrow to 110 > 100 pool: denied.
    let denied = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 60);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "borrow beyond the shared burst pool must be denied, got {denied:?}"
    );
    assert_eq!(ledger.class_usage(&MemoryClass::ArtifactCache).total(), 650);
}
#[test]
fn shared_burst_pool_is_shared_across_borrowing_classes() {
    let mut ledger = ledger_from_policy(
        ResidencyPolicy::new(10_000_000)
            .with_shared_burst_pool_bytes(100)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(400, 600, true))
            .with_class_budget(MemoryClass::EvidenceCache, class_budget(400, 600, true)),
    );
    reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 660).expect("borrow 60 of 100");
    // Evidence wants to borrow 60 too -> total borrow 120 > 100 pool: denied.
    let denied = reserve(&mut ledger, MemoryClass::EvidenceCache, "o", 660);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "second class cannot exceed the pool already drawn by the first"
    );
    // But borrowing only 40 more (total 100) is exactly admissible.
    reserve(&mut ledger, MemoryClass::EvidenceCache, "o", 640)
        .expect("borrow that exactly fills the remaining pool is admitted");
}
#[test]
fn borrow_false_still_denies_over_cap_regardless_of_pool() {
    let mut ledger = ledger_from_policy(
        ResidencyPolicy::new(10_000_000)
            .with_shared_burst_pool_bytes(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(400, 600, false)),
    );
    let denied = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 700);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "a non-borrowing class cannot use the pool"
    );
}
#[test]
fn owner_hard_cap_still_enforced() {
    let mut ledger = ledger_from_policy(
        ResidencyPolicy::new(10_000_000)
            .with_owner_quota("o", OwnerQuota { soft_cap_bytes: 300, hard_cap_bytes: 500 }),
    );
    reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 400).expect("under owner cap");
    assert!(matches!(
        reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 200),
        Err(MemoryError::Capacity(_))
    ));
}
#[test]
fn empty_release_batch_is_a_noop_even_during_rebuild() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
    let t = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 1_000).expect("reserve");
    ledger.commit(&t).expect("commit");
    ledger.begin_rebuild();

    // Empty batch: no-op, succeeds despite the rebuild window.
    ledger.release_batch(&[]).expect("empty batch is a no-op even during rebuild");
    assert_eq!(ledger.snapshot().total_owner_bytes(), 1_000, "no-op batch changed nothing");

    // Non-empty batch: still fenced by the rebuild gate.
    assert!(
        matches!(ledger.release_batch(std::slice::from_ref(&t)), Err(MemoryError::InvalidState(_))),
        "a non-empty batch release during rebuild must still be rejected"
    );
    ledger.finish_rebuild();
    ledger.release_batch(&[t]).expect("batch release resumes after rebuild");
    assert_eq!(ledger.snapshot().total_owner_bytes(), 0);
}
#[test]
fn zero_byte_reserve_commits_and_releases_coherently() {
    // scenario A: a 0-byte ticket is a real, live, single-use ticket. It
    // must commit (return 0), release once, and stay invariant-balanced;
    // its second release is rejected (invariant 7 idempotency).
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
    let before = ledger.snapshot();

    let ticket = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 0)
        .expect("a zero-byte reserve is admitted");
    assert_eq!(ticket.bytes(), 0, "ticket carries 0 bytes");
    assert_eq!(ledger.snapshot().live_ticket_count, 1);
    // A 0-byte reserve must not move any counter.
    assert_eq!(ledger.snapshot().total_owner_bytes(), 0, "0 bytes charged nothing");
    assert_axes_balanced(&ledger.snapshot());

    // Commit reports the (zero) charged bytes for an honest, live ticket.
    assert_eq!(ledger.commit(&ticket).expect("commit live zero-byte ticket"), 0);
    assert_eq!(ledger.snapshot().live_ticket_count, 1);

    // First release succeeds and the ledger returns to its pre-reserve state.
    ledger.release(&ticket).expect("first release of zero-byte ticket");
    assert_eq!(ledger.snapshot().live_ticket_count, 0);
    assert_eq!(ledger.snapshot(), before, "release of a 0-byte ticket is a no-op on counters");

    // Invariant 7 still bites for a zero-byte ticket: the second release of
    // the same id is rejected and nothing changes.
    let after_release = ledger.snapshot();
    let second = ledger.release(&ticket);
    assert!(
        matches!(second, Err(MemoryError::InvalidState(_))),
        "second release of a zero-byte ticket must be rejected, got {second:?}"
    );
    assert_eq!(ledger.snapshot(), after_release, "rejected release must not mutate");
}
#[test]
fn overflow_denial_preserves_exact_live_ticket_accounting() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(u64::MAX));
    let first = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", u64::MAX - 10)
        .expect("first fits within validated global policy");
    ledger.commit(&first).expect("first commits");
    let before = ledger.snapshot();
    let second = reserve(&mut ledger, MemoryClass::EvidenceCache, "o", 100);
    assert!(matches!(second, Err(MemoryError::Capacity(_))), "overflow must deny, got {second:?}");
    assert_eq!(ledger.snapshot(), before, "denial must leave all axes unchanged");
}
#[test]
fn exact_boundary_ticket_survives_first_release() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(u64::MAX));
    let first = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", u64::MAX - 10)
        .expect("first reserve");
    ledger.commit(&first).expect("first commit");
    let second = reserve(&mut ledger, MemoryClass::EvidenceCache, "o", 10)
        .expect("sum equals u64::MAX exactly");
    ledger.commit(&second).expect("second commit");
    ledger.release(&first).expect("release first");
    assert_eq!(
        ledger.snapshot().total_owner_bytes(),
        10,
        "second live ticket must remain exactly charged"
    );
    ledger.release(&second).expect("release second");
    assert_eq!(ledger.snapshot().total_owner_bytes(), 0);
}
#[test]
fn unrepresentable_sum_is_denied_without_mutating_any_axis() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(u64::MAX));
    let first = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", u64::MAX - 10)
        .expect("first huge reserve fits under u64::MAX global");
    let before = ledger.snapshot();
    assert!(matches!(
        reserve(&mut ledger, MemoryClass::EvidenceCache, "other", 100),
        Err(MemoryError::Capacity(_))
    ));
    assert_eq!(ledger.snapshot(), before);
    ledger.release(&first).expect("original ticket remains live and releasable");
    assert_eq!(ledger.snapshot().total_owner_bytes(), 0);
}
#[test]
fn batch_release_preflights_every_charge_kind_before_first_ticket_mutates() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
    let first =
        reserve(&mut ledger, MemoryClass::ArtifactCache, "first", 100).expect("first reserve");
    let second =
        reserve(&mut ledger, MemoryClass::EvidenceCache, "second", 100).expect("second reserve");
    // Keep the class total unchanged while corrupting the second ticket's
    // exact kind slot. A totals-only preflight would miss this mismatch.
    let usage = ledger.class_usage.get_mut(&MemoryClass::EvidenceCache).expect("second class");
    usage.retained_payload_bytes = 0;
    usage.retained_metadata_bytes = 100;
    let before = ledger.snapshot();
    assert!(matches!(
        ledger.release_batch(&[first.clone(), second.clone()]),
        Err(MemoryError::InvalidState(_))
    ));
    assert_eq!(ledger.snapshot(), before, "preflight failure must not release first ticket");
    assert!(ledger.ticket_states.contains_key(&first.id()));
    assert!(ledger.ticket_states.contains_key(&second.id()));
}
#[test]
fn owner_probe_meter_observes_keyed_reads_and_every_scanned_key() {
    fn require_send<T: Send>() {}
    require_send::<MeteredOwnerUsage>();
    let mut owners = MeteredOwnerUsage::default();
    for owner in ["a", "b", "c"] {
        owners.entry(owner.to_string()).or_default();
    }
    let guard = owners.start_probes();
    assert!(owners.get(&"c".to_string()).is_some());
    assert_eq!(guard.key_visits(), 1);
    assert!(owners.iter().any(|(owner, _)| owner.as_str() == "c"));
    assert_eq!(guard.key_visits(), 4, "nested find visits every inspected owner");
    assert_eq!(owners.values().count(), 3);
    assert_eq!(guard.key_visits(), 7, "values iteration is also metered");
    for _ in &owners {}
    assert_eq!(guard.key_visits(), 10, "borrowed-map iteration is also metered");
    let read = OwnerUsageRead(OwnerUsageReadData::Metered(&owners));
    assert!(read.get(&"c".to_string()).is_some());
    assert_eq!(guard.key_visits(), 11, "admission reads preserve metering");
    drop(guard);
    assert!(owners.get(&"c".to_string()).is_some());
    assert_eq!(owners.key_visits.load(Ordering::Relaxed), 11, "inactive reads are unmetered");
}
#[test]
fn single_release_preflight_is_bounded_independent_of_owner_cardinality() {
    assert_single_release_probe_bound(32);
    assert_single_release_probe_bound(128);
}
#[test]
fn aggregate_usage_tracks_charge_kinds_and_headroom() {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
    let payload = reserve_kind(
        &mut ledger,
        MemoryClass::ArtifactCache,
        "payload",
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        100,
    )
    .expect("payload");
    let scratch = reserve_kind(
        &mut ledger,
        MemoryClass::ExecutionScratch,
        "scratch",
        MemoryContextKind::Generation,
        ChargeKind::LiveScratch,
        200,
    )
    .expect("scratch");
    let headroom = reserve_kind(
        &mut ledger,
        MemoryClass::Headroom,
        "headroom",
        MemoryContextKind::Runtime,
        ChargeKind::RetainedMetadata,
        50,
    )
    .expect("headroom");

    assert_eq!(ledger.total_semantic_bytes(), 350);
    assert_eq!(ledger.total_live_scratch_bytes(), 200);
    assert_eq!(ledger.non_headroom_bytes(), 300);
    ledger.release(&scratch).expect("scratch release");
    assert_eq!(ledger.total_semantic_bytes(), 150);
    assert_eq!(ledger.total_live_scratch_bytes(), 0);
    assert_eq!(ledger.non_headroom_bytes(), 100);
    ledger.release_batch(&[payload, headroom]).expect("final release");
    assert_eq!(ledger.total_semantic_bytes(), 0);
    assert_eq!(ledger.non_headroom_bytes(), 0);
    assert_axes_balanced(&ledger.snapshot());
}
#[test]
fn snapshot_validation_rejects_unrepresentable_axis_instead_of_saturating_it() {
    let mut snapshot = LedgerSnapshot::default();
    snapshot.owner_usage.insert(
        "owner".into(),
        UsageBreakdown {
            retained_payload_bytes: u64::MAX,
            retained_metadata_bytes: 1,
            live_scratch_bytes: 0,
        },
    );
    assert!(matches!(snapshot.validate_exact(), Err(MemoryError::InvalidState(_))));
    assert!(std::panic::catch_unwind(|| snapshot.total_owner_bytes()).is_err());
}
#[test]
#[should_panic(expected = "ledger owner total exceeds u64")]
fn forged_snapshot_total_owner_bytes_panics_as_documented() {
    let mut snapshot = LedgerSnapshot::default();
    for owner in ["owner:one", "owner:two"] {
        snapshot.owner_usage.insert(
            owner.into(),
            UsageBreakdown {
                retained_payload_bytes: u64::MAX,
                retained_metadata_bytes: 0,
                live_scratch_bytes: 0,
            },
        );
    }
    assert!(snapshot.checked_total_owner_bytes().is_err());
    let _ = snapshot.total_owner_bytes();
}
#[test]
fn reserve_above_i64_max_does_not_wrap_to_negative_delta() {
    // scenario A: regression guard for the old `bytes as i64` cast. A charge
    // of bytes > i64::MAX, when routed through i64, wraps to a NEGATIVE delta
    // and SUBTRACTS — silently corrupting the counters. The fixed path adds
    // the full u64 magnitude.
    let big: u64 = (i64::MAX as u64) + 12_345; // > i64::MAX, fits in u64
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(u64::MAX));

    let ticket = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", big)
        .expect("reserve of bytes>i64::MAX is admitted under a u64::MAX global");

    let snap = ledger.snapshot();
    // If the cast had wrapped, this would be a small/garbage value, not `big`.
    assert_eq!(
        snap.total_owner_bytes(),
        big,
        "bytes>i64::MAX must be charged at full magnitude, not a wrapped delta"
    );
    assert_eq!(
        snap.class_usage[&MemoryClass::ArtifactCache].retained_payload_bytes,
        big,
        "class axis charged the full magnitude"
    );
    assert_axes_balanced(&snap);

    // And release must subtract the same full magnitude back to zero.
    ledger.release(&ticket).expect("release the huge ticket");
    let after = ledger.snapshot();
    assert_eq!(after.total_owner_bytes(), 0, "release subtracts the full magnitude");
    assert_axes_balanced(&after);
}
#[test]
fn ticket_from_one_ledger_cannot_release_on_another() {
    // scenario A: ledgers A and B have overlapping id spaces (both start at
    // 1). A ticket issued by A is not live in B, so releasing it on B must be
    // rejected (invariant 7) and must leave B byte-for-byte unchanged.
    let mut ledger_a = ledger_from_policy(ResidencyPolicy::new(1_000_000));
    let mut ledger_b = ledger_from_policy(ResidencyPolicy::new(1_000_000));

    let a_ticket = reserve(&mut ledger_a, MemoryClass::ArtifactCache, "o", 500).expect("A");
    // Give B its own state with the SAME id (1), so this is not merely an
    // unknown id but a genuine id-space collision.
    let b_ticket = reserve(&mut ledger_b, MemoryClass::EvidenceCache, "o2", 700).expect("B");
    assert_eq!(a_ticket.id(), b_ticket.id(), "overlapping id space (both start at 1)");

    let before_b = ledger_b.snapshot();
    let misuse = ledger_b.release(&a_ticket);
    assert!(
        matches!(misuse, Err(MemoryError::InvalidState(_))),
        "releasing A's ticket on B must be a typed invalid-state error, got {misuse:?}"
    );
    assert_eq!(ledger_b.snapshot(), before_b, "B must be unchanged by the foreign release");
    // B's own ticket still releases honestly afterward.
    ledger_b.release(&b_ticket).expect("B's own ticket still releases");
    assert_eq!(ledger_b.snapshot().total_owner_bytes(), 0);
}
#[test]
fn ticket_ids_are_monotonic_and_unique_across_many_reserves() {
    // scenario A: every reserve must mint a fresh, strictly increasing id so
    // the idempotency set can never confuse two distinct reservations.
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(10_000_000));
    let mut ids = Vec::new();
    let mut prev: Option<u64> = None;
    for i in 0..64u64 {
        let t = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", i).expect("reserve");
        if let Some(p) = prev {
            assert!(t.id() > p, "ids must be strictly increasing: {} !> {p}", t.id());
        }
        prev = Some(t.id());
        ids.push(t.id());
    }
    let unique: BTreeSet<u64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "every reserve mints a unique id");
}
#[test]
fn context_axis_balances_with_owner_after_mixed_sequence() {
    // scenario A: the third axis. After a mix of reserves across different
    // contexts/owners/classes (and one release), total context usage must
    // equal total owner usage at every step.
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(10_000_000));

    let steps: &[(MemoryClass, &str, MemoryContextKind, ChargeKind, u64)] = &[
        (
            MemoryClass::ArtifactCache,
            "a",
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100,
        ),
        (
            MemoryClass::EvidenceCache,
            "b",
            MemoryContextKind::Session,
            ChargeKind::RetainedMetadata,
            200,
        ),
        (MemoryClass::TraceIndex, "a", MemoryContextKind::Background, ChargeKind::LiveScratch, 300),
        (
            MemoryClass::ArtifactCache,
            "c",
            MemoryContextKind::Generation,
            ChargeKind::RetainedPayload,
            400,
        ),
    ];
    let mut tickets = Vec::new();
    for (class, owner, ctx, kind, bytes) in steps {
        let t = reserve_kind(&mut ledger, class.clone(), owner, *ctx, *kind, *bytes)
            .expect("mixed reserve");
        tickets.push(t);
        let snap = ledger.snapshot();
        assert_eq!(
            snap.total_context_bytes(),
            snap.total_owner_bytes(),
            "context axis must track owner axis after each reserve"
        );
        assert_axes_balanced(&snap);
    }
    // Release one ticket and re-check the third axis still balances.
    ledger.release(&tickets[1]).expect("release the metadata ticket");
    let snap = ledger.snapshot();
    assert_eq!(snap.total_context_bytes(), snap.total_owner_bytes(), "balances after release");
    assert_axes_balanced(&snap);
}
