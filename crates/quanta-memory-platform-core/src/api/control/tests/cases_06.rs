use super::*;

#[test]
fn owner_and_group_soft_caps_each_trigger_the_same_reclaim_lane() {
    let owner_reclaimer = Arc::new(RecordingReclaimer::default());
    let owner_policy = ResidencyPolicy::new(1_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false))
        .with_owner_quota("soft-owner", OwnerQuota { soft_cap_bytes: 10, hard_cap_bytes: 100 });
    let owner_broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(owner_policy),
        Some(owner_reclaimer.clone()),
    )
    .expect("owner broker");
    let owner_charge = owner_broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "soft-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            11,
        )
        .expect("owner soft crossing");
    assert_eq!(owner_reclaimer.requests.lock().expect("requests").len(), 1);

    let group_reclaimer = Arc::new(RecordingReclaimer::default());
    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 10,
        hard_cap_bytes: 100,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let group_policy = ResidencyPolicy::new(1_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false))
        .with_resource_group("soft-group", group)
        .with_owner_group("group-owner", "soft-group");
    let group_broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(group_policy),
        Some(group_reclaimer.clone()),
    )
    .expect("group broker");
    let group_charge = group_broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "group-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            11,
        )
        .expect("group soft crossing");
    let group_requests = group_reclaimer.requests.lock().expect("requests");
    assert_eq!(group_requests.len(), 1);
    assert_eq!(group_requests[0].resource_group.as_deref(), Some("soft-group"));
    drop(group_requests);
    drop(owner_charge);
    drop(group_charge);
}
#[test]
fn hard_cap_denial_reclaims_outside_lock_remeasures_and_retries_once() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let policy = ResidencyPolicy::new(1_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 50, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    *reclaimer.broker_for_reentry.lock().expect("broker") = Some(broker.clone());

    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "hard-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80,
        )
        .expect("baseline");
    *reclaimer.charge_to_release.lock().expect("charge") = Some(baseline);
    let replacement = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "hard-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        )
        .expect("retry sees callback release and admits");

    let requests = reclaimer.requests.lock().expect("requests");
    assert_eq!(requests.last().expect("hard request").trigger, ReclaimTrigger::CapacityDenial);
    assert_eq!(requests.last().expect("hard request").max_attempts, 1);
    drop(requests);
    let stats = broker.reclaim_runtime_stats().expect("stats");
    assert_eq!(stats.attempts, 2, "baseline soft signal plus one hard retry");
    assert_eq!(stats.reported_reclaimed_bytes, 80);
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 30);
    drop(replacement);
}
#[test]
fn group_reserved_floor_is_unavailable_to_peer_group() {
    let protected = ResourceGroup {
        reserved_bytes: 400,
        soft_cap_bytes: 500,
        hard_cap_bytes: 600,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::First,
        admission_weight: 1,
    };
    let peer = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 700,
        hard_cap_bytes: 1_000,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Last,
        admission_weight: 1,
    };
    let policy = ResidencyPolicy::new(1_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 1_000, 1_000, false))
        .with_resource_group("protected", protected)
        .with_resource_group("peer", peer)
        .with_owner_group("protected-owner", "protected")
        .with_owner_group("peer-owner", "peer");
    let broker = LiveProcessMemoryBroker::from_report(report_with(policy)).expect("broker");

    assert!(matches!(
        broker.reserve(
            MemoryClass::ArtifactCache,
            "peer-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            601,
        ),
        Err(MemoryError::Capacity(_))
    ));
    let peer_ticket = broker
        .reserve(
            MemoryClass::ArtifactCache,
            "peer-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            600,
        )
        .expect("peer may use unprotected band");
    let protected_ticket = broker
        .reserve(
            MemoryClass::ArtifactCache,
            "protected-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            400,
        )
        .expect("protected group consumes its floor");
    broker.release(&peer_ticket).expect("release peer");
    broker.release(&protected_ticket).expect("release protected");
}
#[test]
fn grouped_headroom_preserves_protected_floor_across_reconfigure_and_release() {
    let protected = ResourceGroup {
        reserved_bytes: 100,
        soft_cap_bytes: 200,
        hard_cap_bytes: 200,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let peer = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 800,
        hard_cap_bytes: 800,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let mut policy = ResidencyPolicy::new(1_000)
        .with_headroom_bytes(200)
        .with_class_budget(MemoryClass::Headroom, class_budget(0, 200, 200, false))
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800, 800, false))
        .with_resource_group("protected", protected)
        .with_resource_group("peer", peer)
        .with_owner_group("protected-owner", "protected")
        .with_owner_group("peer-owner", "peer");
    let report = report_with(policy.clone());
    policy = report.residency.clone();
    let broker = LiveProcessMemoryBroker::from_report(report).expect("broker");
    let headroom = broker
        .reserve(
            MemoryClass::Headroom,
            "protected-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            100,
        )
        .expect("headroom");
    assert!(matches!(
        broker.reserve(
            MemoryClass::ArtifactCache,
            "peer-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            800
        ),
        Err(MemoryError::Capacity(_))
    ));
    let peer = broker
        .reserve(
            MemoryClass::ArtifactCache,
            "peer-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            700,
        )
        .expect("unprotected band");
    broker.try_live_reconfigure(policy).expect("identity reconfigure");
    assert!(matches!(
        broker.reserve(
            MemoryClass::ArtifactCache,
            "peer-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1
        ),
        Err(MemoryError::Capacity(_))
    ));
    let floor = broker
        .reserve(
            MemoryClass::ArtifactCache,
            "protected-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100,
        )
        .expect("own protected floor");
    {
        let mut core = broker.inner.lock().expect("core");
        LiveProcessMemoryBroker::release_many_on(&mut core, &[headroom, floor])
            .expect("mixed-class batch release");
    }
    assert!(
        matches!(
            broker.reserve(
                MemoryClass::ArtifactCache,
                "peer-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1
            ),
            Err(MemoryError::Capacity(_))
        ),
        "mixed batch release must restore the protected group's floor"
    );
    broker.release(&peer).expect("peer release");
    let snapshot = broker.ledger_snapshot().expect("snapshot");
    snapshot.validate_exact().expect("all axes balance");
    assert_eq!(snapshot.live_ticket_count, 0);
}
#[test]
fn accepted_reconfigure_updates_revision_source_and_matches_headroom_predicate() {
    let mut policy = ResidencyPolicy::new(1_000).with_headroom_bytes(200);
    policy.classes.insert(MemoryClass::Headroom, class_budget(0, 200, 200, false));
    policy.classes.insert(MemoryClass::ArtifactCache, class_budget(0, 800, 800, false));
    let broker = LiveProcessMemoryBroker::from_report(report_with(policy.clone())).expect("broker");
    let headroom = broker
        .reserve(
            MemoryClass::Headroom,
            "headroom".into(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            200,
        )
        .expect("headroom");
    let work = broker
        .reserve(
            MemoryClass::ArtifactCache,
            "work".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            800,
        )
        .expect("separate non-headroom band");

    broker.try_live_reconfigure(policy).expect("identity policy remains legal");
    let report = broker.effective_report().expect("report");
    assert_eq!(report.policy_revision, 2);
    assert_eq!(report.active_policy_source, "runtime_reconfigure");
    assert_eq!(
        report.override_provenance.last(),
        Some(&OverrideRecord::new(OverrideSource::RuntimeReconfigure, "residency.active"))
    );
    assert_eq!(report.residency.headroom_bytes, 200);
    broker.release(&headroom).expect("release headroom");
    broker.release(&work).expect("release work");
}
#[test]
fn identity_reconfigure_succeeds_during_legal_class_borrow() {
    let policy = ResidencyPolicy::new(1_000)
        .with_shared_burst_pool_bytes(100)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 50, 100, true));
    let broker = LiveProcessMemoryBroker::from_report(report_with(policy.clone())).expect("broker");
    let ticket = broker
        .reserve(
            MemoryClass::ArtifactCache,
            "borrower".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            150,
        )
        .expect("legal 50-byte class borrow");
    broker.try_live_reconfigure(policy).expect("identity reconfigure preserves legal borrow");
    broker.release(&ticket).expect("release");
}
#[test]
fn identity_reconfigure_succeeds_during_legal_group_burst() {
    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 50,
        hard_cap_bytes: 100,
        burst_percent: 50,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let policy = ResidencyPolicy::new(1_000)
        .with_shared_burst_pool_bytes(100)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 300, 500, false))
        .with_resource_group("burstable", group)
        .with_owner_group("group-owner", "burstable");
    let broker = LiveProcessMemoryBroker::from_report(report_with(policy.clone())).expect("broker");
    let ticket = broker
        .reserve(
            MemoryClass::ArtifactCache,
            "group-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            150,
        )
        .expect("legal 50-byte group burst");
    broker.try_live_reconfigure(policy).expect("identity reconfigure preserves legal burst");
    broker.release(&ticket).expect("release");
}
#[test]
fn reclaim_priority_orders_victims_deterministically() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let mut last = class_budget(0, 10, 100, false);
    last.reclaim_priority = ReclaimPriority::Last;
    let mut normal = class_budget(0, 100, 100, false);
    normal.reclaim_priority = ReclaimPriority::Normal;
    let mut first = class_budget(0, 100, 100, false);
    first.reclaim_priority = ReclaimPriority::First;
    let group_with_priority = |priority| ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 50,
        hard_cap_bytes: 100,
        burst_percent: 0,
        reclaim_priority: priority,
        admission_weight: 1,
    };
    let policy = ResidencyPolicy::new(1_000)
        .with_class_budget(MemoryClass::ArtifactCache, last)
        .with_class_budget(MemoryClass::EvidenceCache, normal)
        .with_class_budget(MemoryClass::TraceIndex, first)
        .with_resource_group("last-group", group_with_priority(ReclaimPriority::Last))
        .with_resource_group("normal-group", group_with_priority(ReclaimPriority::Normal))
        .with_resource_group("first-group", group_with_priority(ReclaimPriority::First));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    let charge = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "priority-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            11,
        )
        .expect("soft crossing");
    let requests = reclaimer.requests.lock().expect("requests");
    let candidates = &requests[0].candidates;
    let last_pos = candidates
        .iter()
        .position(|candidate| candidate.class == MemoryClass::ArtifactCache)
        .expect("last candidate");
    let normal_pos = candidates
        .iter()
        .position(|candidate| candidate.class == MemoryClass::EvidenceCache)
        .expect("normal candidate");
    let first_pos = candidates
        .iter()
        .position(|candidate| candidate.class == MemoryClass::TraceIndex)
        .expect("first candidate");
    assert!(last_pos < normal_pos && normal_pos < first_pos);
    let groups = &requests[0].group_candidates;
    assert_eq!(groups[0].group, "last-group");
    assert_eq!(groups[1].group, "normal-group");
    assert_eq!(groups[2].group, "first-group");
    drop(requests);
    drop(charge);
}
#[test]
fn sep23_fup007_zero_byte_committed_blocks_group_removal_until_explicit_release() {
    let b = sep23_grouped_broker();
    let mut charge = sep23_zero_committed(&b, "owner:g");
    assert_eq!(b.ledger_snapshot().expect("snapshot").live_ticket_count, 1);

    // Removing the owner mapping AND the group while the ticket is live must
    // be atomically denied — zero usage is not the absence of authority.
    let denied = b.try_live_reconfigure(sep23_ungrouped_residency());
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "group removal under a live zero-byte ticket must be denied, got {denied:?}"
    );

    // Explicit release still succeeds against the preserved issuing scope.
    charge.release().expect("explicit release of zero-byte charge");
    sep23_assert_fully_drained(&b);

    // With no live ticket left, the identical reconfigure is legal.
    b.try_live_reconfigure(sep23_ungrouped_residency())
        .expect("group removal after final ticket release");
    sep23_assert_fully_drained(&b);
}
#[test]
fn sep23_fup007_zero_byte_committed_drop_after_denied_reconfigure_releases() {
    let b = sep23_grouped_broker();
    let charge = sep23_zero_committed(&b, "owner:g");
    assert!(matches!(
        b.try_live_reconfigure(sep23_ungrouped_residency()),
        Err(MemoryError::InvalidState(_))
    ));
    drop(charge);
    sep23_assert_fully_drained(&b);
    b.try_live_reconfigure(sep23_ungrouped_residency()).expect("removal after Drop");
}
#[test]
fn sep23_fup007_zero_byte_committed_rebuild_fence_drop_drains_on_finish() {
    let b = sep23_grouped_broker();
    let charge = sep23_zero_committed(&b, "owner:g");
    let fence16 = b.begin_rebuild().expect("begin rebuild");
    drop(charge);
    // Under the fence the exact ticket is deferred, still charged.
    assert_eq!(b.ledger_snapshot().expect("fenced").live_ticket_count, 1);
    assert!(fence16
        .finish()
        .expect("finish drains the deferred zero-byte release")
        .drain_error
        .is_none());
    sep23_assert_fully_drained(&b);
}
#[test]
fn sep23_fup007_zero_byte_pending_blocks_removal_explicit_rollback_unblocks() {
    let b = sep23_grouped_broker();
    let mut pending = b
        .reserve_owned(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            0,
        )
        .expect("zero-byte pending reservation");
    assert!(
        matches!(
            b.try_live_reconfigure(sep23_ungrouped_residency()),
            Err(MemoryError::InvalidState(_))
        ),
        "pending zero-byte authority must also block scope removal"
    );
    pending.rollback().expect("explicit pending rollback");
    sep23_assert_fully_drained(&b);
    b.try_live_reconfigure(sep23_ungrouped_residency()).expect("removal after rollback");
}
#[test]
fn sep23_fup007_zero_byte_pending_drop_rolls_back_without_stranding() {
    let b = sep23_grouped_broker();
    let pending = b
        .reserve_owned(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            0,
        )
        .expect("zero-byte pending reservation");
    assert!(matches!(
        b.try_live_reconfigure(sep23_ungrouped_residency()),
        Err(MemoryError::InvalidState(_))
    ));
    drop(pending);
    sep23_assert_fully_drained(&b);
}
#[test]
fn sep23_fup007_denied_reconfigure_preserves_policy_revision_scheduler_ledger() {
    let b = sep23_grouped_broker();
    let _charge = sep23_zero_committed(&b, "owner:g");
    let before_report = b.effective_report().expect("report");
    let before_snapshot = b.ledger_snapshot().expect("snapshot");
    let before_group = b.admits_group("g", 100_000).expect("group probe");

    let denied = b.try_live_reconfigure(sep23_ungrouped_residency());
    assert!(matches!(denied, Err(MemoryError::InvalidState(_))));

    let after_report = b.effective_report().expect("report");
    assert_eq!(after_report.policy_revision, before_report.policy_revision, "revision frozen");
    assert_eq!(after_report.residency, before_report.residency, "policy frozen");
    assert_eq!(after_report.override_provenance, before_report.override_provenance);
    assert_eq!(b.ledger_snapshot().expect("snapshot"), before_snapshot, "ledger frozen");
    assert_eq!(b.admits_group("g", 100_000).expect("group probe"), before_group);
    assert_eq!(b.observe().expect("observe").resource_group_usage.get("g"), Some(&0));
}
#[test]
fn sep23_fup007_mixed_zero_and_nonzero_partial_termination() {
    let b = sep23_grouped_broker();
    let mut zero = sep23_zero_committed(&b, "owner:g");
    let mut nonzero = b
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            10_000,
        )
        .expect("nonzero charge");
    assert!(matches!(
        b.try_live_reconfigure(sep23_ungrouped_residency()),
        Err(MemoryError::InvalidState(_))
    ));
    // Terminating only the nonzero ticket still leaves zero-byte authority.
    nonzero.release().expect("release nonzero");
    assert_eq!(b.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
    assert!(
        matches!(
            b.try_live_reconfigure(sep23_ungrouped_residency()),
            Err(MemoryError::InvalidState(_))
        ),
        "remaining zero-byte ticket must keep blocking"
    );
    zero.release().expect("release zero");
    sep23_assert_fully_drained(&b);
    b.try_live_reconfigure(sep23_ungrouped_residency()).expect("removal when fully drained");
}
#[test]
fn sep23_fup007_scope_matrix_remap_cap_ungrouped_terminated() {
    // Owner remap (g -> h) with a live zero-byte ticket: denied.
    let residency_h = ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
        .with_resource_group(
            "g",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 50_000,
                hard_cap_bytes: 100_000,
                burst_percent: 100,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_resource_group(
            "h",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 50_000,
                hard_cap_bytes: 100_000,
                burst_percent: 100,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("owner:g", "h");
    let b = sep23_grouped_broker();
    let mut charge = sep23_zero_committed(&b, "owner:g");
    assert!(
        matches!(b.try_live_reconfigure(residency_h), Err(MemoryError::InvalidState(_))),
        "owner remap under a live zero-byte ticket must be denied"
    );

    // Group cap-only change (mapping untouched): accepted despite the ticket.
    let raised_cap = ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
        .with_resource_group(
            "g",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 60_000,
                hard_cap_bytes: 120_000,
                burst_percent: 100,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("owner:g", "g");
    b.try_live_reconfigure(raised_cap).expect("cap-only change keeps the ticket scope");
    charge.release().expect("release after cap change");

    // Terminated tickets never block a later removal.
    b.try_live_reconfigure(sep23_ungrouped_residency()).expect("removal after termination");

    // An ungrouped zero-byte ticket does not pin group "g".
    let b2 = sep23_grouped_broker();
    let mut stray = sep23_zero_committed(&b2, "owner:stray");
    b2.try_live_reconfigure(sep23_ungrouped_residency())
        .expect("ungrouped ticket must not block unrelated group removal");
    stray.release().expect("release stray");
    sep23_assert_fully_drained(&b2);

    // A different owner's live ticket does not pin owner:g's mapping either,
    // but removing the group both owners share is still denied while any
    // grouped ticket is live.
    let b3 = sep23_grouped_broker();
    let mut other = b3
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:other".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            0,
        )
        .expect("other zero charge");
    // owner:other has no mapping; dropping only owner:g's mapping is denied
    // only if owner:g holds authority — it holds none here, so a policy that
    // keeps both owners unmapped-but-valid is accepted. Removing group "g"
    // while no ticket was issued under it is legal.
    b3.try_live_reconfigure(sep23_ungrouped_residency()).expect("no grouped ticket pins group g");
    other.release().expect("release other");
    sep23_assert_fully_drained(&b3);
}
#[test]
fn sep23_fup005_ten_thousand_reconfigures_keep_provenance_bounded() {
    let b = broker();
    let initial = b.effective_report().expect("report");
    let initial_len = initial.override_provenance.len();
    let initial_revision = initial.policy_revision;
    for i in 0..10_000u32 {
        let soft = 60_000 + u64::from(i % 2) * 1_000;
        let policy = ResidencyPolicy::new(1_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, soft, 200_000, false))
            .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 10_000, 20_000, false));
        b.try_live_reconfigure(policy).expect("cap flap accepted");
    }
    let after = b.effective_report().expect("report");
    assert_eq!(after.policy_revision, initial_revision + 10_000, "revision still counts");
    assert!(
        after.override_provenance.len() <= initial_len + 1,
        "10k reconfigures must add at most one record, got {} over {initial_len}",
        after.override_provenance.len()
    );
    assert_eq!(
        after.override_provenance[..initial_len],
        initial.override_provenance[..],
        "initial resolver provenance is never touched"
    );
    assert_eq!(
        after.override_provenance.last(),
        Some(&OverrideRecord::new(OverrideSource::RuntimeReconfigure, "residency.active")),
        "the single current record names the live source"
    );
    assert_eq!(after.active_policy_source, "runtime_reconfigure");
    // `observe` and `effective_report` share one report object: same
    // active policy, same revision, same bounded provenance.
    assert_eq!(b.observe().expect("observe").report, after);
    sep23_assert_fully_drained(&b);
}
#[test]
fn sep23_fup005_denied_reconfigure_preserves_provenance_byte_for_byte() {
    let b = sep23_grouped_broker();
    // Plant the active record with an accepted cap-only change first.
    let raised_cap = ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
        .with_resource_group(
            "g",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 60_000,
                hard_cap_bytes: 120_000,
                burst_percent: 100,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("owner:g", "g");
    b.try_live_reconfigure(raised_cap).expect("cap-only accepted");
    let _charge = sep23_zero_committed(&b, "owner:g");
    let before = b.effective_report().expect("report");
    assert!(before.override_provenance.iter().any(|r| r.field == "residency.active"));

    let denied = b.try_live_reconfigure(sep23_ungrouped_residency());
    assert!(matches!(denied, Err(MemoryError::InvalidState(_))));
    let after = b.effective_report().expect("report");
    assert_eq!(after.override_provenance, before.override_provenance, "denial preserves all");
    assert_eq!(after.policy_revision, before.policy_revision);
    assert_eq!(after.active_policy_source, before.active_policy_source);
}
