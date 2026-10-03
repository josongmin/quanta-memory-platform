use super::*;

#[test]
fn reserve_denied_class_hard_cap_no_borrow() {
    // ArtifactCache hard cap 600k, borrow=false. Use a roomy owner with no
    // quota so the *class* cap is what trips, not the owner cap.
    let residency = ResidencyPolicy::new(1_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400_000, 600_000, false));
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");

    // owner:unbounded has no quota entry -> only class cap applies.
    let denied = b.reserve(
        MemoryClass::ArtifactCache,
        "owner:unbounded".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        650_000,
    );
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "class hard cap breach (no borrow) must be a typed capacity error"
    );
    // negative pair: a borrow-enabled class with a funded burst pool accepts
    // the same overflow (the 50k over-cap fits the 100k shared pool).
    let borrow_residency = ResidencyPolicy::new(1_000_000)
        .with_shared_burst_pool_bytes(100_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400_000, 600_000, true));
    let b2 = LiveProcessMemoryBroker::from_report(report_with(borrow_residency)).expect("valid");
    assert!(
        b2.reserve(
            MemoryClass::ArtifactCache,
            "owner:unbounded".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            650_000,
        )
        .is_ok(),
        "borrow-enabled class admits overflow within the shared burst pool"
    );
}
#[test]
fn reserve_borrow_beyond_shared_pool_denied() {
    let residency = ResidencyPolicy::new(10_000_000)
        .with_shared_burst_pool_bytes(40_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400_000, 600_000, true));
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
    // 650k over a 600k cap = 50k borrow > 40k pool -> denied.
    let denied = b.reserve(
        MemoryClass::ArtifactCache,
        "owner:unbounded".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        650_000,
    );
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "borrow exceeding the shared burst pool must be a typed capacity error"
    );
}
#[test]
fn release_returns_bytes_and_preserves_invariants() {
    let b = broker();
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            120_000,
        )
        .expect("reserve");
    b.commit(&t).expect("commit");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 120_000);

    b.release(&t).expect("release");
    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(snap.total_owner_bytes(), 0, "release must subtract owner usage");
    assert!(snap.class_owner_balanced());
    assert!(snap.owner_partition_balanced());
}
#[test]
fn rebuild_in_progress_rejects_reserve_commit_release() {
    let b = broker();
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            10_000,
        )
        .expect("reserve before rebuild");
    let fence8 = b.begin_rebuild().expect("begin rebuild");

    let r = b.reserve(
        MemoryClass::ArtifactCache,
        "owner:a".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        10_000,
    );
    assert!(matches!(r, Err(MemoryError::InvalidState(_))), "reserve rejected during rebuild");
    assert!(
        matches!(b.commit(&t), Err(MemoryError::InvalidState(_))),
        "commit rejected during rebuild"
    );
    assert!(
        matches!(b.release(&t), Err(MemoryError::InvalidState(_))),
        "release rejected during rebuild"
    );

    // positive pair: after finishing rebuild, writes resume.
    assert!(fence8.finish().expect("finish rebuild").drain_error.is_none());
    assert!(b.commit(&t).is_ok(), "commit resumes after rebuild");
}
#[test]
fn live_reconfig_shrink_below_usage_denied_and_noop() {
    let b = broker();
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            300_000,
        )
        .expect("reserve");
    b.commit(&t).expect("commit");

    let before = b.effective_report().expect("report").residency.global_hard_limit_bytes;

    // Attempt to shrink global hard limit below current usage (300k).
    let shrink = ResidencyPolicy::new(100_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 80_000, 100_000, false));
    let denied = b.try_live_reconfigure(shrink);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "shrink below usage must be denied"
    );

    let after = b.effective_report().expect("report").residency.global_hard_limit_bytes;
    assert_eq!(before, after, "denied reconfig must leave active policy untouched");
    // The ledger must still be intact too.
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 300_000);
}
#[test]
fn live_reconfig_class_hard_cap_below_class_usage_denied() {
    let b = broker();
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            300_000,
        )
        .expect("reserve");
    b.commit(&t).expect("commit");

    // New policy shrinks ArtifactCache hard cap to 200k < 300k used.
    let bad = base_residency()
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 150_000, 200_000, false));
    assert!(
        matches!(b.try_live_reconfigure(bad), Err(MemoryError::InvalidState(_))),
        "class cap below class usage must be denied"
    );
}
#[test]
fn live_reconfig_dropping_a_class_with_live_usage_is_denied() {
    let b = broker();
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            300_000,
        )
        .expect("reserve");
    b.commit(&t).expect("commit");

    // New policy raises the global limit but OMITS ArtifactCache entirely,
    // which previously left that class uncapped (fail-open). Must be denied.
    let dropped = ResidencyPolicy::new(2_000_000)
        .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 400_000, 600_000, false));
    let denied = b.try_live_reconfigure(dropped);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "dropping a class that holds live bytes must be denied, got {denied:?}"
    );
    // untouched
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 300_000);
    assert_eq!(
        b.effective_report().expect("report").residency.global_hard_limit_bytes,
        1_000_000,
        "denied reconfig must leave the active policy untouched"
    );
}
#[test]
fn live_reconfig_owner_quota_below_owner_usage_is_denied() {
    let b = broker();
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            300_000,
        )
        .expect("reserve");
    b.commit(&t).expect("commit");

    // owner:a holds 300k; install a quota with a 200k hard cap -> contradiction.
    let bad = base_residency().with_owner_quota(
        "owner:a",
        OwnerQuota { soft_cap_bytes: 150_000, hard_cap_bytes: 200_000 },
    );
    assert!(
        matches!(b.try_live_reconfigure(bad), Err(MemoryError::InvalidState(_))),
        "owner quota below current owner usage must be denied"
    );
}
#[test]
fn live_reconfig_raising_headroom_over_non_headroom_usage_is_denied() {
    // Roomy class + no owner quota so ONLY the headroom band can bite.
    let residency = ResidencyPolicy::new(1_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800_000, 1_000_000, false));
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:free".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            800_000,
        )
        .expect("reserve 800k non-Headroom");
    b.commit(&t).expect("commit");

    // Raise headroom to 300k: effective non-Headroom ceiling 700k < 800k live.
    let raised = ResidencyPolicy::new(1_000_000)
        .with_headroom_bytes(300_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800_000, 1_000_000, false));
    let denied = b.try_live_reconfigure(raised);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "raising headroom above the free band must be denied, got {denied:?}"
    );
    // Untouched: still 0 headroom in the active policy.
    assert_eq!(b.effective_report().expect("report").residency.headroom_bytes, 0);

    // A headroom of 100k (ceiling 900k >= 800k live) is legal and accepted.
    let legal = ResidencyPolicy::new(1_000_000)
        .with_headroom_bytes(100_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800_000, 1_000_000, false));
    b.try_live_reconfigure(legal).expect("headroom within the free band is legal");
    assert_eq!(b.effective_report().expect("report").residency.headroom_bytes, 100_000);
}
#[test]
fn live_reconfig_lowering_group_cap_below_live_group_usage_is_denied() {
    use crate::domain::policy::ResourceGroup;
    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 50_000,
        hard_cap_bytes: 100_000,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let residency = ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
        .with_resource_group("g", group.clone())
        .with_owner_group("owner:g", "g");
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
    let t = b
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            90_000,
        )
        .expect("group usage 90k");

    // Lower group g's cap to 50k (< 90k live), mapping unchanged -> denied.
    let lowered = ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
        .with_resource_group("g", ResourceGroup { hard_cap_bytes: 50_000, ..group })
        .with_owner_group("owner:g", "g");
    let denied = b.try_live_reconfigure(lowered);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "lowering a group cap below live group usage must be denied, got {denied:?}"
    );
    let observation = b.observe().expect("lock-consistent group observation");
    assert_eq!(observation.resource_group_usage.get("g"), Some(&90_000));
    assert_eq!(observation.ledger.live_ticket_count, 1);

    // Raising the cap to 120k (>= 90k live) is legal and accepted.
    let raised = ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
        .with_resource_group("g", ResourceGroup { hard_cap_bytes: 120_000, ..group })
        .with_owner_group("owner:g", "g");
    b.try_live_reconfigure(raised).expect("raising the group cap over live usage is legal");
    b.release(&t).expect("release");
    let drained = b.observe().expect("drained group observation");
    assert_eq!(drained.resource_group_usage.get("g"), Some(&0));
    assert_eq!(drained.ledger.live_ticket_count, 0);
}
#[test]
fn live_reconfig_raise_caps_accepted_and_preserves_usage() {
    let b = broker();
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            300_000,
        )
        .expect("reserve");
    b.commit(&t).expect("commit");

    let bigger = ResidencyPolicy::new(2_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800_000, 1_200_000, false))
        .with_owner_quota(
            "owner:a",
            OwnerQuota { soft_cap_bytes: 600_000, hard_cap_bytes: 900_000 },
        );
    b.try_live_reconfigure(bigger).expect("raising caps is legal");

    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(snap.total_owner_bytes(), 300_000, "usage preserved across legal reconfig");
    assert!(snap.class_owner_balanced());
    assert_eq!(b.effective_report().expect("report").residency.global_hard_limit_bytes, 2_000_000);
}
#[test]
fn allocator_advisory_mismatch_does_not_mutate_accounting() {
    let b = broker();
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("reserve");
    b.commit(&t).expect("commit");
    let before = b.ledger_snapshot().expect("snap").total_owner_bytes();

    // Allocator claims a wildly different resident value.
    let snapshot =
        AllocatorSnapshot { resident_bytes: Some(999_999), ..AllocatorSnapshot::default() };
    let advisory = b.observe_allocator(&snapshot).expect("observe");
    assert!(advisory.mismatch, "advisory must flag the disagreement");
    assert_eq!(advisory.semantic_bytes, before, "advisory reports semantic SSOT, not allocator");
    assert_eq!(advisory.drift_bytes(), Some(999_999i128 - before as i128));

    let after = b.ledger_snapshot().expect("snap").total_owner_bytes();
    assert_eq!(before, after, "advisory observation must NOT change semantic accounting");
}
#[test]
fn pressure_classifies_from_semantic_usage() {
    let b = broker();
    // Global hard limit is 1_000_000. Default thresholds soft=75%.
    assert_eq!(b.classify_pressure().expect("classify"), PressureState::Normal);

    // Charge owner:a to 450k via two owners to cross 75% globally.
    let t1 = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            400_000,
        )
        .expect("reserve a");
    b.commit(&t1).expect("commit");
    let t2 = b
        .reserve(
            MemoryClass::EvidenceCache,
            "owner:c".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            400_000,
        )
        .expect("reserve c");
    b.commit(&t2).expect("commit");
    // 800k / 1_000_000 = 80% -> Soft (>=75) but < hard (90).
    assert_eq!(b.classify_pressure().expect("classify"), PressureState::Soft);
    let before = b.ledger_snapshot().expect("semantic snapshot before telemetry");
    for resident_bytes in [0, 999_999] {
        let advisory = b
            .observe_allocator(&AllocatorSnapshot {
                resident_bytes: Some(resident_bytes),
                ..AllocatorSnapshot::default()
            })
            .expect("observe contradictory allocator usage");
        assert!(advisory.mismatch);
        assert_eq!(advisory.semantic_bytes, 800_000);
        assert_eq!(
            crate::PressureController::current_pressure(
                b.lock().expect("pressure after advisory observation").controller.as_ref()
            ),
            PressureState::Soft,
            "advisory observation must not change cached pressure before reclassification"
        );
        assert_eq!(
            b.classify_pressure().expect("classify after telemetry"),
            PressureState::Soft,
            "allocator telemetry must neither downgrade nor escalate semantic pressure"
        );
        assert_eq!(b.ledger_snapshot().expect("snapshot after telemetry"), before);
    }
}
#[test]
fn denied_reconfigure_leaves_scheduler_controller_and_report_unchanged() {
    let residency = base_residency().with_resource_group(
        "group:a",
        ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 100_000,
            hard_cap_bytes: 200_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        },
    );
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
    // Drive usage to 800k so classification is Soft and so a shrink is illegal.
    let t1 = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            400_000,
        )
        .expect("reserve a");
    b.commit(&t1).expect("commit");
    let t2 = b
        .reserve(
            MemoryClass::EvidenceCache,
            "owner:c".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            400_000,
        )
        .expect("reserve c");
    b.commit(&t2).expect("commit");

    // Capture the entire observable surface before the denied op.
    let report_before = b.effective_report().expect("report");
    let snap_before = b.ledger_snapshot().expect("snap");
    let pressure_before = b.classify_pressure().expect("classify");
    let admits_before = b.admits_group("group:a", 150_000).expect("admits");
    assert_eq!(pressure_before, PressureState::Soft, "precondition: 80% is Soft");

    // Illegal narrowing: global limit 500k < current usage 800k -> denied.
    let shrink = ResidencyPolicy::new(500_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400_000, 450_000, false))
        .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 400_000, 450_000, false));
    let denied = b.try_live_reconfigure(shrink);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "shrink below live usage must be denied, got {denied:?}"
    );

    // Every facet must be byte-for-byte identical after the denied op.
    let report_after = b.effective_report().expect("report");
    let snap_after = b.ledger_snapshot().expect("snap");
    let pressure_after = b.classify_pressure().expect("classify");
    let admits_after = b.admits_group("group:a", 150_000).expect("admits");
    assert_eq!(report_before.residency, report_after.residency, "report residency unchanged");
    assert_eq!(
        snap_before.total_owner_bytes(),
        snap_after.total_owner_bytes(),
        "ledger snapshot unchanged"
    );
    assert_eq!(snap_before.class_usage, snap_after.class_usage, "class usage unchanged");
    assert_eq!(
        pressure_before, pressure_after,
        "controller classification unchanged (still Soft against the old 1M limit)"
    );
    assert_eq!(admits_before, admits_after, "scheduler admission unchanged");
}
#[test]
fn accepted_reconfigure_rebuilds_controller_to_new_global_limit() {
    let b = broker();
    let t1 = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            400_000,
        )
        .expect("reserve a");
    b.commit(&t1).expect("commit");
    let t2 = b
        .reserve(
            MemoryClass::EvidenceCache,
            "owner:c".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            400_000,
        )
        .expect("reserve c");
    b.commit(&t2).expect("commit");
    // 800k / 1M = 80% -> Soft under the old limit.
    assert_eq!(b.classify_pressure().expect("classify"), PressureState::Soft);

    // Raise the global limit to 4M (and class caps to fit). Now 800k / 4M =
    // 20% -> Normal. Caps only rise so the reconfig is legal; owner:a's
    // quota is carried forward because a live owner must stay capped.
    let bigger = ResidencyPolicy::new(4_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 1_000_000, 2_000_000, false))
        .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 1_000_000, 2_000_000, false))
        .with_owner_quota(
            "owner:a",
            OwnerQuota { soft_cap_bytes: 300_000, hard_cap_bytes: 500_000 },
        );
    b.try_live_reconfigure(bigger).expect("raising the global limit is legal");

    assert_eq!(
        b.classify_pressure().expect("classify"),
        PressureState::Normal,
        "controller rebuilt: 800k is now Normal against the new 4M limit"
    );
    // Usage is preserved across the rebuild.
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 800_000);
    assert_eq!(
        b.effective_report().expect("report").residency.global_hard_limit_bytes,
        4_000_000,
        "report residency reflects the accepted new global limit"
    );
}
#[test]
fn reconfigure_rejects_policy_missing_drained_mandatory_class() {
    let b = broker();
    let before = b.effective_report().expect("report");
    let mut incomplete = ResidencyPolicy::new(2_000_000);
    incomplete.classes.remove(&MemoryClass::GenerationMetadata);

    let result = b.try_live_reconfigure(incomplete);

    assert!(matches!(result, Err(MemoryError::Config(_))));
    assert_eq!(b.effective_report().expect("report"), before);
    assert_eq!(b.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
}
#[test]
fn reconfigure_missing_class_denial_preserves_policy_scheduler_and_usage() {
    let b = broker();
    let ticket = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("seed live usage");
    b.commit(&ticket).expect("commit");
    let report_before = b.effective_report().expect("report");
    let snapshot_before = b.ledger_snapshot().expect("snapshot");
    let admission_before = b.admits_group("absent", 1).expect("admission probe");

    let mut incomplete = ResidencyPolicy::new(2_000_000);
    incomplete.classes.remove(&MemoryClass::ArtifactCache);
    let result = b.try_live_reconfigure(incomplete);

    assert!(matches!(result, Err(MemoryError::Config(_))));
    assert_eq!(b.effective_report().expect("report"), report_before);
    assert_eq!(b.ledger_snapshot().expect("snapshot"), snapshot_before);
    assert_eq!(b.admits_group("absent", 1).expect("admission probe"), admission_before);
}
#[test]
fn pressure_thresholds_are_inclusive_at_exact_boundary_fractions() {
    // (used_bytes, expected) over the 1_000_000 limit. `>=` semantics:
    // the boundary value itself crosses up into the higher band.
    let cases: &[(u64, PressureState)] = &[
        (0, PressureState::Normal),
        (749_999, PressureState::Normal), // just below 75% stays Normal
        (750_000, PressureState::Soft),   // exactly 75% -> Soft (>= boundary)
        (899_999, PressureState::Soft),   // just below 90% stays Soft
        (900_000, PressureState::Hard),   // exactly 90% -> Hard
        (979_999, PressureState::Hard),   // just below 98% stays Hard
        (980_000, PressureState::Fatal),  // exactly 98% -> Fatal
    ];
    for &(used, expected) in cases {
        // Fresh broker per case so usage is deterministic and isolated. The
        // single class/owner caps are set at the global limit so the *only*
        // thing under test is the controller's threshold classification, not
        // an incidental class/owner cap denial.
        let roomy = ResidencyPolicy::new(1_000_000).with_class_budget(
            MemoryClass::ArtifactCache,
            class_budget(0, 1_000_000, 1_000_000, false),
        );
        let b = LiveProcessMemoryBroker::from_report(report_with(roomy)).expect("valid");
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:boundary".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                used,
            )
            .expect("reserve at boundary fits within the at-limit class cap");
        b.commit(&t).expect("commit");
        assert_eq!(
            b.classify_pressure().expect("classify"),
            expected,
            "usage {used} / 1_000_000 must classify as {expected:?} (>= boundary)"
        );
    }
}
#[test]
fn rebuild_fence_is_single_owner_finishes_once_and_recovers_after_abandon() {
    let b = broker();
    let other = b.clone();
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            10_000,
        )
        .expect("reserve before rebuild");

    let fence = b.begin_rebuild().expect("first coordinator opens the fence");
    assert_eq!(
        other.rebuild_fence_status().expect("status"),
        crate::api::control::RebuildFenceStatus::Active
    );
    assert!(
        matches!(other.begin_rebuild(), Err(MemoryError::InvalidState(_))),
        "a second live coordinator must not open over the first fence"
    );
    assert!(
        matches!(b.commit(&t), Err(MemoryError::InvalidState(_))),
        "writes rejected while the fence is closed"
    );

    let receipt = fence.finish().expect("owning coordinator finishes");
    assert!(receipt.fence_open);
    assert_eq!(receipt.pending_rollbacks_remaining, 0);
    assert_eq!(receipt.committed_releases_remaining, 0);
    assert!(receipt.drain_error.is_none());
    assert!(
        !b.ledger_snapshot().expect("snap").rebuild_in_progress,
        "the owning finish reopens writes"
    );
    assert!(b.commit(&t).is_ok(), "writes resume after the finish");

    // Abandonment: a dropped unfinished permit leaves the fence closed
    // and observably abandoned; writes stay rejected; an explicit
    // re-begin recovers it and finishes cleanly.
    let abandoned = b.begin_rebuild().expect("open a fence to abandon");
    drop(abandoned);
    assert_eq!(
        b.rebuild_fence_status().expect("status"),
        crate::api::control::RebuildFenceStatus::Abandoned
    );
    assert!(
        matches!(
            b.reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1,
            ),
            Err(MemoryError::InvalidState(_))
        ),
        "an abandoned fence still rejects writes"
    );
    let recovered = b.begin_rebuild().expect("explicit re-adoption recovers the fence");
    let receipt = recovered.finish().expect("recovered fence finishes");
    assert!(receipt.fence_open);
    assert!(!b.ledger_snapshot().expect("snap").rebuild_in_progress);
}
#[test]
fn begin_rebuild_epoch_exhaustion_leaves_fence_state_untouched() {
    let b = broker();
    {
        let mut core = b.inner.lock().expect("core");
        core.ledger.begin_rebuild();
        core.rebuild_epoch = u64::MAX;
        core.rebuild_abandoned = true;
    }
    assert_eq!(
        b.rebuild_fence_status().expect("status"),
        crate::api::control::RebuildFenceStatus::Abandoned
    );

    let denied = b.begin_rebuild();
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "the exhausted epoch must be a typed rejection, got {denied:?}"
    );
    // The Err path must not have re-adopted the fence: it stays
    // abandoned (recoverable-by-inspection), never active-with-no-permit.
    assert_eq!(
        b.rebuild_fence_status().expect("status"),
        crate::api::control::RebuildFenceStatus::Abandoned,
        "a rejected begin must leave the abandoned fence observable"
    );
}
