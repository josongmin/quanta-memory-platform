use super::*;

#[test]
fn rebuild_fence_rejects_foreign_clone_finish_and_second_live_begin() {
    let b = broker();
    let other = b.clone();
    let fence = b.begin_rebuild().expect("first coordinator opens the fence");

    assert!(
        matches!(other.begin_rebuild(), Err(MemoryError::InvalidState(_))),
        "a second live coordinator must not open over the first fence"
    );
    assert_eq!(
        other.rebuild_fence_status().expect("status"),
        crate::api::control::RebuildFenceStatus::Active,
        "the rejected foreign begin leaves the fence closed and owned"
    );
    assert!(
        b.ledger_snapshot().expect("snap").rebuild_in_progress,
        "the rejected foreign calls must leave the fence closed"
    );
    assert!(fence.finish().expect("owning coordinator finishes").drain_error.is_none());
    assert!(!b.ledger_snapshot().expect("snap").rebuild_in_progress);
}
#[test]
fn rebuild_fence_gates_reclaim_slow_path_before_capacity_breaches() {
    use crate::ports::reclaim::ReclaimOutcome;

    struct CountingReclaimer {
        requests: std::sync::Mutex<Vec<ReclaimRequest>>,
    }

    impl MemoryReclaimer for CountingReclaimer {
        fn reclaim(&self, request: &ReclaimRequest) -> MemoryResult<ReclaimOutcome> {
            self.requests.lock().expect("requests").push(request.clone());
            Ok(ReclaimOutcome { reclaimed_bytes: 0 })
        }
    }

    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 900,
        hard_cap_bytes: 900,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let residency = ResidencyPolicy::new(1_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 1_000_000, 1_000_000, false))
        .with_resource_group("g", group)
        .with_owner_group("owner:g", "g");
    let reclaimer = Arc::new(CountingReclaimer { requests: std::sync::Mutex::new(Vec::new()) });
    let b = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(residency),
        Some(reclaimer.clone() as Arc<dyn MemoryReclaimer>),
    )
    .expect("broker");

    let filling = managed_charge_owner(&b, "owner:g", 900).expect("fill the group exactly");
    let fence12 = b.begin_rebuild().expect("begin rebuild");

    // The group is full AND the fence is closed: the fence must win, so
    // no host callback fires and the denial is a typed rebuild refusal.
    let denied = b.reserve_managed_charge(
        MemoryClass::ArtifactCache,
        "owner:g".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        10,
    );
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "a closed rebuild fence must surface as InvalidState, got {denied:?}"
    );
    assert!(
        reclaimer.requests.lock().expect("requests").is_empty(),
        "no reclaim callback may run while the fence is closed"
    );
    let stats = b.reclaim_runtime_stats().expect("stats");
    assert_eq!(stats.attempts, 0, "no reclaim attempt during the fence");
    assert!(stats.blocked_denials >= 1, "the fence denial is blocked (recoverable), not terminal");

    assert!(fence12.finish().expect("finish rebuild").drain_error.is_none());
    // After the fence opens, the same group breach takes the normal slow
    // path with exactly one bounded callback.
    let slow = b.reserve_managed_charge(
        MemoryClass::ArtifactCache,
        "owner:g".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        10,
    );
    assert!(
        matches!(slow, Err(MemoryError::Capacity(_))),
        "post-fence group breach is an ordinary Capacity denial, got {slow:?}"
    );
    assert_eq!(reclaimer.requests.lock().expect("requests").len(), 1);
    drop(filling);
    sep23_assert_fully_drained(&b);
}
#[test]
fn live_reconfig_denies_removing_a_quota_while_the_owner_holds_authority() {
    let quota_policy = ResidencyPolicy::new(1_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 1_000_000, 1_000_000, false))
        .with_owner_quota(
            "owner:q",
            crate::domain::policy::OwnerQuota { soft_cap_bytes: 5_000, hard_cap_bytes: 5_000 },
        );
    let b =
        LiveProcessMemoryBroker::from_report(report_with(quota_policy.clone())).expect("broker");
    let live =
        managed_charge_owner(&b, "owner:q", 800).expect("owner holds live bytes under quota");

    // Removing the quota would silently uncap a live owner.
    let mut uncap = quota_policy.clone();
    uncap.owner_quotas.remove("owner:q");
    let denied = b.try_live_reconfigure(uncap);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "quota removal under live authority must be denied, got {denied:?}"
    );
    assert_eq!(
        b.ledger_snapshot().expect("unchanged").total_owner_bytes(),
        800,
        "the denied reconfigure leaves the ledger untouched"
    );

    // Retaining (even lowering within live usage) or raising the quota
    // stays a legal reconfiguration.
    let mut narrowed = quota_policy;
    narrowed.owner_quotas.insert(
        "owner:q".to_string(),
        crate::domain::policy::OwnerQuota { soft_cap_bytes: 800, hard_cap_bytes: 1_000 },
    );
    b.try_live_reconfigure(narrowed).expect("retained quota reconfigures legally");

    drop(live);
    sep23_assert_fully_drained(&b);
}
#[test]
fn reentrant_recharge_from_reclaimer_callback_is_bounded_and_typed() {
    use crate::ports::reclaim::ReclaimOutcome;
    use std::sync::OnceLock;

    type ChargeHolder = Arc<std::sync::Mutex<Option<ManagedCharge>>>;

    struct RechargingReclaimer {
        charge: OnceLock<ChargeHolder>,
        outcomes: std::sync::Mutex<Vec<Result<(), ManagedChargeRechargeError>>>,
    }

    impl MemoryReclaimer for RechargingReclaimer {
        fn reclaim(&self, _request: &ReclaimRequest) -> MemoryResult<ReclaimOutcome> {
            let holder = self.charge.get().expect("charge registered before use");
            if let Some(charge) = holder.lock().expect("charge").as_mut() {
                // Same-size swap: admissible on its own, so only the
                // reentry bound can reject it.
                let bytes = charge.bytes().expect("live charge reports bytes");
                let outcome = charge.recharge(
                    MemoryClass::ArtifactCache,
                    "owner:g".to_string(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    bytes,
                );
                self.outcomes.lock().expect("outcomes").push(outcome);
            }
            Ok(ReclaimOutcome { reclaimed_bytes: 0 })
        }
    }

    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 900,
        hard_cap_bytes: 900,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let residency = ResidencyPolicy::new(1_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 1_000_000, 1_000_000, false))
        .with_resource_group("g", group)
        .with_owner_group("owner:g", "g");
    let reclaimer = Arc::new(RechargingReclaimer {
        charge: OnceLock::new(),
        outcomes: std::sync::Mutex::new(Vec::new()),
    });
    let b = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(residency),
        Some(reclaimer.clone() as Arc<dyn MemoryReclaimer>),
    )
    .expect("broker");

    let holder: ChargeHolder = Arc::new(std::sync::Mutex::new(Some(
        managed_charge_owner(&b, "owner:g", 900).expect("fill the group"),
    )));
    assert!(reclaimer.charge.set(Arc::clone(&holder)).is_ok());

    // The full group triggers the callback; the nested recharge must be
    // rejected by the reentry bound with the charge byte-identical.
    let denied = managed_charge_owner(&b, "owner:g", 10);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "outer admission stays an ordinary Capacity denial, got {denied:?}"
    );
    let outcomes = reclaimer.outcomes.lock().expect("outcomes");
    assert_eq!(outcomes.len(), 1, "exactly one callback with one nested recharge");
    assert!(
        matches!(
            &outcomes[0],
            Err(ManagedChargeRechargeError { error: MemoryError::OperationInProgress(_), .. })
        ),
        "reentrant recharge must be typed OperationInProgress, got {:?}",
        outcomes[0]
    );
    drop(outcomes);
    {
        let mut guard = holder.lock().expect("charge");
        let charge = guard.as_mut().expect("charge preserved");
        assert_eq!(charge.bytes(), Some(900), "the bounded recharge never mutated the lease");
    }
    drop(holder.lock().expect("charge").take());
    sep23_assert_fully_drained(&b);
}
#[test]
fn trait_effective_report_matches_inherent_and_is_not_fabricated() {
    use crate::api::broker::ProcessMemoryBroker;
    let b = broker();
    let inherent = b.effective_report().expect("inherent report");
    let via_trait = <LiveProcessMemoryBroker as ProcessMemoryBroker>::effective_report(&b);
    assert_eq!(
        inherent, via_trait,
        "trait impl must mirror the inherent report exactly on the healthy path"
    );
    // The real report carries the 1M limit we built from, NOT a fabricated
    // 4 GiB Production budget.
    assert_eq!(via_trait.residency.global_hard_limit_bytes, 1_000_000);
    assert_ne!(
        via_trait.residency.global_hard_limit_bytes,
        4 * 1024 * 1024 * 1024,
        "must not return the old fabricated Production fallback"
    );
    assert!(
        !via_trait.override_sources.iter().any(|s| s == "poisoned-fallback"),
        "healthy report must not carry the fabricated poisoned-fallback marker"
    );
}
#[test]
fn reserve_gated_by_resource_group_admission_fail_closed() {
    use crate::domain::policy::ResourceGroup;
    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 50_000,
        hard_cap_bytes: 100_000,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    // Roomy global so ONLY the group admission can deny; owner mapped to "g".
    let residency = ResidencyPolicy::new(10_000_000)
        .with_resource_group("g", group)
        .with_owner_group("owner:g", "g");
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");

    // Within the group hard cap: admitted.
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80_000,
        )
        .expect("within group cap");
    b.commit(&t).expect("commit");

    // Over the group cap (80k+40k=120k > 100k, no burst pool) -> denied.
    let denied = b.reserve(
        MemoryClass::ArtifactCache,
        "owner:g".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        40_000,
    );
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "group admission denial must be a typed capacity error, got {denied:?}"
    );
    // Fail-closed: the denied reserve did not move the ledger.
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 80_000);

    // Releasing returns the group's burst capacity so a later reserve fits.
    b.release(&t).expect("release");
    assert!(
        b.reserve(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            90_000,
        )
        .is_ok(),
        "released group capacity is re-admissible"
    );
}
#[test]
fn owner_without_group_mapping_is_not_gated_by_admission() {
    use crate::domain::policy::ResourceGroup;
    let tiny = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 1,
        hard_cap_bytes: 1,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    // The group exists but "owner:free" is NOT mapped to it.
    let residency = ResidencyPolicy::new(10_000_000).with_resource_group("g", tiny);
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
    assert!(
        b.reserve(
            MemoryClass::ArtifactCache,
            "owner:free".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            5_000_000,
        )
        .is_ok(),
        "an unmapped owner is gated only by class/owner/global caps, not by any group"
    );
}
#[test]
fn execution_scratch_budget_caps_live_scratch_and_spares_other_kinds() {
    let report = EffectiveMemoryBudgetReport {
        execution: crate::domain::policy::ExecutionBudget {
            max_execution_scratch_bytes: Some(100_000),
            ..crate::domain::policy::ExecutionBudget::unlimited()
        },
        ..report_with(ResidencyPolicy::new(10_000_000))
    };
    let b = LiveProcessMemoryBroker::from_report(report).expect("valid");

    let t = b
        .reserve(
            MemoryClass::ExecutionScratch,
            "o".to_string(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            80_000,
        )
        .expect("within scratch budget");
    b.commit(&t).expect("commit");

    let denied = b.reserve(
        MemoryClass::ExecutionScratch,
        "o".to_string(),
        MemoryContextKind::Request,
        ChargeKind::LiveScratch,
        40_000,
    );
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "live-scratch over the execution scratch budget must be denied, got {denied:?}"
    );
    assert_eq!(
        b.ledger_snapshot().expect("snap").total_live_scratch_bytes(),
        80_000,
        "denied scratch reserve is fail-closed"
    );

    // A non-scratch (payload) charge is NOT gated by the scratch budget.
    assert!(
        b.reserve(
            MemoryClass::ArtifactCache,
            "o".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            5_000_000,
        )
        .is_ok(),
        "the execution scratch budget bounds only live scratch, not retained payload"
    );
}
#[test]
fn execution_scratch_budget_accepts_exact_cap_and_denies_one_more() {
    let report = EffectiveMemoryBudgetReport {
        execution: crate::domain::policy::ExecutionBudget {
            max_execution_scratch_bytes: Some(100_000),
            ..crate::domain::policy::ExecutionBudget::unlimited()
        },
        ..report_with(ResidencyPolicy::new(10_000_000))
    };
    let broker = LiveProcessMemoryBroker::from_report(report).expect("valid");
    let at_cap = broker
        .reserve(
            MemoryClass::ExecutionScratch,
            "boundary-owner".to_string(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            100_000,
        )
        .expect("the exact configured cap is admissible");
    broker.commit(&at_cap).expect("commit at cap");

    let denied = broker.reserve(
        MemoryClass::ExecutionScratch,
        "boundary-owner".to_string(),
        MemoryContextKind::Request,
        ChargeKind::LiveScratch,
        1,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(
        broker.ledger_snapshot().expect("snapshot").total_live_scratch_bytes(),
        100_000,
        "the over-cap attempt leaves semantic accounting unchanged"
    );
}
#[test]
fn live_runtime_scratch_charge_appears_in_ledger_snapshot() {
    let b = runtime_scratch_broker(100);
    let runtime =
        b.attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime).expect("attach runtime");
    let context = runtime.context(MemoryContextKind::Request, "owner:scratch".to_string());
    let mut reservation = runtime
        .scratch()
        .reserve_scratch_scope(&context, ScratchRequest::new("working-set", 60))
        .expect("scratch reserve");

    let snapshot = b.ledger_snapshot().expect("snapshot");
    assert_eq!(snapshot.total_live_scratch_bytes(), 60);
    assert_eq!(snapshot.total_owner_bytes(), 60);
    assert_eq!(
        snapshot.class_usage.get(&MemoryClass::ExecutionScratch).map(|usage| usage.total()),
        Some(60)
    );
    assert_eq!(snapshot.owner_usage.get("owner:scratch").map(|usage| usage.total()), Some(60));
    assert_eq!(
        snapshot.context_usage.get(&MemoryContextKind::Request).map(|usage| usage.total()),
        Some(60)
    );
    assert!(reservation.is_enforced());

    reservation.release().expect("explicit release");
    assert_eq!(b.ledger_snapshot().expect("after release").total_owner_bytes(), 0);
}
#[test]
fn scratch_peak_denial_release_and_drop_keep_authority_exact() {
    // Explicit-release scope: peak observations never lower the recorded
    // high-water mark, an over-ceiling observation is denied fail-closed,
    // and the broker-backed scratch authority drains exactly on release.
    {
        let b = runtime_scratch_broker(1000);
        let runtime = b
            .attach_runtime("runtime-peak-release".to_string(), RuntimeKind::Runtime)
            .expect("attach runtime");
        let context = runtime.context(MemoryContextKind::Request, "owner:scratch-peak".to_string());
        let mut reservation = runtime
            .scratch()
            .reserve_scratch_scope(&context, ScratchRequest::new("peak-scope", 1000))
            .expect("reserve the exact scratch ceiling");

        assert_eq!(reservation.observed_bytes(), 0);
        reservation.grow_to_observed(800).expect("observe 800");
        assert_eq!(reservation.observed_bytes(), 800);
        reservation.grow_to_observed(200).expect("smaller observation accepted");
        assert_eq!(reservation.observed_bytes(), 800, "the runtime scratch peak is monotonic");
        assert!(
            matches!(reservation.grow_to_observed(1001), Err(MemoryError::Capacity(_))),
            "an observation over the reserved ceiling is a typed Capacity error"
        );
        assert_eq!(reservation.observed_bytes(), 800, "denied observation keeps the peak");

        let live = b.ledger_snapshot().expect("live snapshot");
        assert_eq!(live.total_live_scratch_bytes(), 1000);
        assert_eq!(live.live_ticket_count, 1);

        reservation.release().expect("explicit release");
        let drained = b.ledger_snapshot().expect("drained snapshot");
        assert_eq!(drained.total_live_scratch_bytes(), 0);
        assert_eq!(drained.live_ticket_count, 0);
    }

    // Drop scope: the same peak discipline with Drop-driven cleanup.
    {
        let b = runtime_scratch_broker(1000);
        let runtime = b
            .attach_runtime("runtime-peak-drop".to_string(), RuntimeKind::Runtime)
            .expect("attach runtime");
        let context = runtime.context(MemoryContextKind::Request, "owner:scratch-peak".to_string());
        let mut reservation = runtime
            .scratch()
            .reserve_scratch_scope(&context, ScratchRequest::new("peak-scope", 1000))
            .expect("reserve the exact scratch ceiling");
        reservation.grow_to_observed(800).expect("observe 800");
        reservation.grow_to_observed(200).expect("smaller observation accepted");
        assert_eq!(reservation.observed_bytes(), 800, "peak survives a smaller observation");

        drop(reservation);
        let drained = b.ledger_snapshot().expect("drop-drained snapshot");
        assert_eq!(drained.total_live_scratch_bytes(), 0);
        assert_eq!(drained.live_ticket_count, 0);
    }
}
#[test]
fn live_runtime_scratch_over_cap_is_denied_without_mutation() {
    let b = runtime_scratch_broker(100);
    let runtime =
        b.attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime).expect("attach runtime");
    let context = runtime.context(MemoryContextKind::Request, "owner:scratch".to_string());
    let held = runtime
        .scratch()
        .reserve_scratch_scope(&context, ScratchRequest::new("held", 80))
        .expect("initial reserve");
    let before = b.ledger_snapshot().expect("before denial");

    let denied =
        runtime.scratch().reserve_scratch_scope(&context, ScratchRequest::new("denied", 21));
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(b.ledger_snapshot().expect("after denial"), before);

    drop(held);
    assert_eq!(b.ledger_snapshot().expect("after drop").total_owner_bytes(), 0);
}
#[test]
fn runtime_scratch_and_direct_broker_charge_share_one_limit() {
    let b = runtime_scratch_broker(100);
    let direct = b
        .reserve_committed(
            MemoryClass::ExecutionScratch,
            "owner:direct".to_string(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            60,
        )
        .expect("direct scratch charge");
    let runtime =
        b.attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime).expect("attach runtime");
    let context = runtime.context(MemoryContextKind::Request, "owner:runtime".to_string());

    let denied =
        runtime.scratch().reserve_scratch_scope(&context, ScratchRequest::new("shared-limit", 41));
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(b.ledger_snapshot().expect("snapshot").total_live_scratch_bytes(), 60);
    b.release(&direct).expect("release direct charge");
}
#[test]
fn dropping_runtime_scratch_during_rebuild_returns_capacity() {
    let b = runtime_scratch_broker(100);
    let runtime =
        b.attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime).expect("attach runtime");
    let context = runtime.context(MemoryContextKind::Request, "owner:scratch".to_string());
    let reservation = runtime
        .scratch()
        .reserve_scratch_scope(&context, ScratchRequest::new("temporary", 100))
        .expect("fill capacity");
    let fence13 = b.begin_rebuild().expect("begin rebuild");

    drop(reservation);

    let snapshot = b.ledger_snapshot().expect("snapshot");
    assert!(snapshot.rebuild_in_progress);
    assert_eq!(snapshot.total_owner_bytes(), 0);
    assert!(fence13.finish().expect("finish rebuild").drain_error.is_none());
}
#[test]
fn runtime_scratch_rejects_foreign_runtime_context_without_mutation() {
    let b = runtime_scratch_broker(100);
    let runtime =
        b.attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime).expect("attach runtime");
    let foreign = MemoryContextHandle::new(
        "runtime-b".to_string(),
        MemoryContextKind::Request,
        "owner:scratch".to_string(),
    );
    let before = b.ledger_snapshot().expect("before");

    let denied =
        runtime.scratch().reserve_scratch_scope(&foreign, ScratchRequest::new("foreign", 1));
    assert!(matches!(denied, Err(MemoryError::InvalidState(_))));
    assert_eq!(b.ledger_snapshot().expect("after"), before);
}
#[test]
fn runtime_scratch_release_restores_resource_group_capacity() {
    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 100,
        hard_cap_bytes: 100,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let residency = ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ExecutionScratch, class_budget(0, 10_000, 10_000, false))
        .with_resource_group("runtime-group", group)
        .with_owner_group("owner:scratch", "runtime-group");
    let report = EffectiveMemoryBudgetReport {
        execution: crate::domain::policy::ExecutionBudget {
            max_execution_scratch_bytes: Some(10_000),
            ..crate::domain::policy::ExecutionBudget::unlimited()
        },
        ..report_with(residency)
    };
    let b = LiveProcessMemoryBroker::from_report(report).expect("broker");
    let runtime =
        b.attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime).expect("attach runtime");
    let context = runtime.context(MemoryContextKind::Request, "owner:scratch".to_string());
    let mut reservation = runtime
        .scratch()
        .reserve_scratch_scope(&context, ScratchRequest::new("fills-group", 100))
        .expect("fill group capacity");
    assert!(!b.admits_group("runtime-group", 1).expect("group is full"));

    reservation.release().expect("explicit scratch release");

    assert!(b.admits_group("runtime-group", 100).expect("group capacity restored"));
    assert_eq!(b.ledger_snapshot().expect("after release").total_owner_bytes(), 0);
}
#[test]
fn recharge_denied_by_scratch_budget_preserves_old_scratch_charge() {
    let report = EffectiveMemoryBudgetReport {
        execution: crate::domain::policy::ExecutionBudget {
            max_execution_scratch_bytes: Some(100_000),
            ..crate::domain::policy::ExecutionBudget::unlimited()
        },
        ..report_with(ResidencyPolicy::new(10_000_000))
    };
    let b = LiveProcessMemoryBroker::from_report(report).expect("valid");
    let old = b
        .reserve_committed(
            MemoryClass::ExecutionScratch,
            "o".to_string(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            80_000,
        )
        .expect("seed 80k scratch (within 100k budget)");

    // Recharge to 120k > 100k scratch budget -> denied without consuming old.
    let err = b
        .recharge(
            &old,
            MemoryClass::ExecutionScratch,
            "o".to_string(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            120_000,
        )
        .expect_err("over scratch budget must be denied");
    assert!(matches!(err.error, MemoryError::Capacity(_)));
    assert_eq!(err.recovery, RechargeRecoveryV1::OldChargePreserved);
    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(snap.total_live_scratch_bytes(), 80_000, "old scratch charge restored exactly");
    assert!(snap.class_owner_balanced() && snap.owner_partition_balanced(), "axes balanced");
    b.release(&old).expect("release preserved old ticket");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
}
#[test]
fn concurrent_reserves_never_exceed_global_limit_no_toctou() {
    let b = LiveProcessMemoryBroker::from_report(report_with(ResidencyPolicy::new(1_000_000)))
        .expect("valid");
    let mut handles = Vec::new();
    for i in 0..16 {
        let b = b.clone();
        handles.push(std::thread::spawn(move || {
            let mut admitted = 0u64;
            for _ in 0..100 {
                if let Ok(t) = b.reserve(
                    MemoryClass::ArtifactCache,
                    format!("o{i}"),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    1_000,
                ) {
                    let _ = b.commit(&t);
                    admitted = admitted.saturating_add(1_000);
                }
            }
            admitted
        }));
    }
    let total_admitted: u64 = handles.into_iter().map(|h| h.join().expect("thread panicked")).sum();
    let snap = b.ledger_snapshot().expect("snap");
    assert!(
        snap.total_owner_bytes() <= 1_000_000,
        "global limit must hold under concurrent reserves (no TOCTOU): {}",
        snap.total_owner_bytes()
    );
    assert_eq!(
        snap.total_owner_bytes(),
        total_admitted,
        "no torn updates: the ledger equals exactly the sum of admitted reserves"
    );
    assert!(snap.class_owner_balanced());
}
