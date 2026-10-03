use super::*;

#[test]
fn sep23_fup003_terminal_matrix_never_fires_callback() {
    use crate::domain::admission::{AdmissionVerdict, LimitAxis};
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let b = sep23_fup003_matrix_broker(&reclaimer);
    let pristine = b.ledger_snapshot().expect("snapshot");
    let mut terminals = 0u128;

    // (label, class, owner, kind, bytes, expected axis, expected deficit)
    let cases = [
        (
            "class hard, no borrow",
            MemoryClass::EvidenceCache,
            "owner:plain",
            ChargeKind::RetainedPayload,
            101u64,
            "Class",
            1u64,
        ),
        (
            "global hard",
            MemoryClass::Headroom,
            "owner:plain",
            ChargeKind::RetainedPayload,
            1_001,
            "Global",
            1,
        ),
        (
            "owner hard",
            MemoryClass::ArtifactCache,
            "owner:quota",
            ChargeKind::RetainedPayload,
            901,
            "Owner",
            1,
        ),
        (
            "group hard+burst",
            MemoryClass::ArtifactCache,
            "owner:grouped",
            ChargeKind::RetainedPayload,
            901,
            "Group",
            1,
        ),
        (
            "execution scratch max",
            MemoryClass::ArtifactCache,
            "owner:plain",
            ChargeKind::LiveScratch,
            51,
            "ExecutionScratch",
            1,
        ),
        (
            "huge bytes",
            MemoryClass::Headroom,
            "owner:plain",
            ChargeKind::RetainedPayload,
            u64::MAX,
            "Global",
            u64::MAX - 1_000,
        ),
    ];
    for (label, class, owner, kind, bytes, axis, deficit) in cases {
        // Every case also crosses a soft threshold, which is exactly when
        // the old soft-first flow would have fired a useless callback.
        let denied =
            b.reserve(class.clone(), owner.to_string(), MemoryContextKind::Request, kind, bytes);
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "{label}: terminal request must fail capacity, got {denied:?}"
        );
        terminals += 1;
        assert_eq!(
            b.reclaim_runtime_stats().expect("stats").terminal_denials,
            terminals,
            "{label}: terminal counter tracks"
        );
        assert!(
            reclaimer.requests.lock().expect("requests").is_empty(),
            "{label}: no callback for terminal denials"
        );
        // The verdict carries the limiting axis + deficit for observability.
        let core = b.inner.lock().expect("broker lock");
        match LiveProcessMemoryBroker::admission_preflight(
            &core,
            &class,
            &owner.to_string(),
            MemoryContextKind::Request,
            kind,
            bytes,
        ) {
            AdmissionVerdict::TerminalDenial { breach } => {
                assert_eq!(
                    format!("{:?}", breach.axis).split([' ', '{']).next(),
                    Some(axis),
                    "{label}: axis"
                );
                assert_eq!(breach.deficit, deficit, "{label}: deficit");
                // And the verdict is deterministic: same state, same verdict.
                let again = LiveProcessMemoryBroker::admission_preflight(
                    &core,
                    &class,
                    &owner.to_string(),
                    MemoryContextKind::Request,
                    kind,
                    bytes,
                );
                assert_eq!(again, AdmissionVerdict::TerminalDenial { breach: breach.clone() });
                let _ = LimitAxis::Global;
            }
            other => panic!("{label}: expected TerminalDenial, got {other:?}"),
        }
        drop(core);
    }
    assert_eq!(b.ledger_snapshot().expect("snapshot"), pristine, "denials mutate nothing");
    assert_eq!(b.reclaim_runtime_stats().expect("stats").attempts, 0);
}
#[test]
fn sep23_fup003_borrowing_class_hard_is_not_terminal_until_pool_exhausted() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let policy = ResidencyPolicy::new(10_000)
        .with_shared_burst_pool_bytes(50)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, true));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("borrow broker");
    // Borrow within the pool admits without any callback.
    let held = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:b".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            120,
        )
        .expect("borrow within pool admits");
    // Crossing soft 90 fires a proactive soft request only — never a hard plan.
    let soft_requests = reclaimer.requests.lock().expect("requests").len();
    assert!(
        reclaimer
            .requests
            .lock()
            .expect("requests")
            .iter()
            .all(|r| r.trigger == ReclaimTrigger::SoftThreshold),
        "borrow admission may only fire proactive soft requests"
    );
    // Beyond hard+pool even at zero usage: terminal, no further callback.
    let denied = broker.reserve(
        MemoryClass::ArtifactCache,
        "owner:b".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        200,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
    assert_eq!(
        reclaimer.requests.lock().expect("requests").len(),
        soft_requests,
        "terminal denial adds no callback"
    );
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").terminal_denials, 1);
    drop(held);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn sep23_fup003_blocked_matrix_never_fires_callback() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let b = sep23_fup003_matrix_broker(&reclaimer);
    // Optional class with no configured budget: the legacy capacity message
    // is preserved byte-for-byte, but no callback fires for a request that
    // was never viable.
    let denied = b.reserve(
        MemoryClass::SpillFileCache,
        "owner:plain".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        10,
    );
    assert_eq!(
        denied,
        Err(MemoryError::capacity(
            "class SpillFileCache is not configured in the active residency policy"
        ))
    );
    assert_eq!(b.reclaim_runtime_stats().expect("stats").blocked_denials, 1);
    assert!(reclaimer.requests.lock().expect("requests").is_empty());

    let fence17 = b.begin_rebuild().expect("begin rebuild");
    let fenced = b.reserve(
        MemoryClass::ArtifactCache,
        "owner:plain".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        10,
    );
    assert_eq!(
        fenced,
        Err(MemoryError::InvalidState("rebuild in progress: live reserve rejected".into()))
    );
    assert_eq!(b.reclaim_runtime_stats().expect("stats").blocked_denials, 2);
    assert!(reclaimer.requests.lock().expect("requests").is_empty());
    assert!(fence17.finish().expect("finish rebuild").drain_error.is_none());
    sep23_assert_fully_drained(&b);
}
#[test]
fn sep23_fup003_ticket_exhaustion_blocks_without_callback() {
    use crate::domain::admission::{
        AdmissionBlock, AdmissionRequest, AdmissionVerdict, PreflightInputs, UsageView,
    };
    // A live ledger cannot exhaust u64 ticket ids in a test, so drive the
    // pure preflight directly: exhaustion blocks with the legacy message.
    let policy = ResidencyPolicy::new(1_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 500_000, 1_000_000, false));
    let empty_classes = BTreeMap::new();
    let empty_owners = BTreeMap::new();
    let empty_contexts = BTreeMap::new();
    let empty_total = crate::domain::ledger::UsageBreakdown::default();
    let empty_shares = BTreeMap::new();
    let class = MemoryClass::ArtifactCache;
    let owner = "owner:x".to_string();
    let inputs = PreflightInputs {
        policy: &policy,
        usage: UsageView {
            class_usage: &empty_classes,
            owner_usage: (&empty_owners).into(),
            context_usage: &empty_contexts,
            total_usage: &empty_total,
        },
        non_headroom_bytes: 0,
        live_scratch_bytes: 0,
        execution_scratch_max: None,
        fair_burst_shares: &empty_shares,
        group_current_usage: 0,
        group_current_non_headroom_usage: 0,
        unused_group_floors: 0,
        owner_group: None,
        request: AdmissionRequest {
            class: &class,
            owner: &owner,
            context: MemoryContextKind::Request,
            kind: ChargeKind::RetainedPayload,
            bytes: 100,
        },
        rebuild_in_progress: false,
        ticket_id_space_exhausted: true,
        live_ticket_limit_reached: false,
    };
    assert_eq!(
        crate::domain::admission::preflight(&inputs),
        AdmissionVerdict::Blocked { reason: AdmissionBlock::TicketIdSpaceExhausted }
    );
    let impossible = PreflightInputs {
        request: AdmissionRequest { bytes: 1_000_001, ..inputs.request },
        ..inputs
    };
    assert_eq!(
        crate::domain::admission::preflight(&impossible),
        AdmissionVerdict::Blocked { reason: AdmissionBlock::TicketIdSpaceExhausted },
        "permanent identifier exhaustion must win over a reclaimable byte breach"
    );
    assert_eq!(
        AdmissionBlock::TicketIdSpaceExhausted.into_error(),
        MemoryError::InvalidState("reservation ticket id space exhausted".into())
    );
}
#[test]
fn live_ticket_count_cap_runs_one_reclaim_then_remeasures() {
    for with_group in [false, true] {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let mut policy = ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
        if with_group {
            policy = policy
                .with_resource_group(
                    "g",
                    ResourceGroup {
                        reserved_bytes: 0,
                        soft_cap_bytes: 90,
                        hard_cap_bytes: 100,
                        burst_percent: 0,
                        reclaim_priority: ReclaimPriority::Normal,
                        admission_weight: 1,
                    },
                )
                .with_owner_group("owner:cap", "g");
        }
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:cap".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline");
        let mut zero_tickets =
            Vec::with_capacity(crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER - 1);
        for _ in 1..crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER {
            zero_tickets.push(
                broker
                    .reserve_managed_charge(
                        MemoryClass::ArtifactCache,
                        "owner:cap".into(),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        0,
                    )
                    .expect("zero-byte ticket below cap"),
            );
        }
        let before = broker.ledger_snapshot().expect("before");
        assert_eq!(
            before.live_ticket_count,
            crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
        );
        assert!(reclaimer.requests.lock().expect("requests").is_empty());

        // No-op reclaim may run once, but the unchanged ledger must still
        // deny. The group case also exercises mixed admission limits.
        let denied = broker.reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:cap".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "{denied:?}");
        if !with_group {
            assert_eq!(
                denied.err(),
                Some(MemoryError::capacity(format!(
                    "live reservation ticket limit {} reached",
                    crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
                )))
            );
        }
        assert_eq!(
            reclaimer.requests.lock().expect("requests").len(),
            1,
            "group={with_group}: one bounded reclaim attempt"
        );
        assert_eq!(broker.ledger_snapshot().expect("after"), before);

        drop(zero_tickets);
        drop(baseline);
        sep23_assert_fully_drained(&broker);
    }
}
#[test]
fn live_ticket_count_cap_can_recover_when_host_releases_zero_byte_charge() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let policy = ResidencyPolicy::new(1_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 0, 0, false));
    let mut report = report_with(policy);
    *report.residency.classes.get_mut(&MemoryClass::ArtifactCache).expect("configured class") =
        class_budget(0, 0, 0, false);
    let broker =
        LiveProcessMemoryBroker::from_report_with_reclaimer(report, Some(reclaimer.clone()))
            .expect("broker");
    *reclaimer.charge_to_release.lock().expect("reclaimable charge") = Some(
        broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:reclaimable".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                0,
            )
            .expect("host-owned zero-byte charge"),
    );
    let mut retained =
        Vec::with_capacity(crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER - 1);
    for _ in 1..crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER {
        retained.push(
            broker
                .reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    "owner:retained".into(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    0,
                )
                .expect("fill authority cap"),
        );
    }
    let admitted = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:new".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            0,
        )
        .expect("callback released one live ticket");
    let requests = reclaimer.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .candidates
            .iter()
            .any(|candidate| candidate.class == MemoryClass::ArtifactCache),
        "zero-byte ticket owners must be eligible victims at the ticket-count cap"
    );
    drop(requests);
    let after = broker.ledger_snapshot().expect("after");
    assert_eq!(
        after.live_ticket_count,
        crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
    );
    assert_eq!(after.total_owner_bytes(), 0);
    drop(admitted);
    drop(retained);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn sep23_fup003_predicted_failure_skips_soft_and_runs_single_hard() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let policy = ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("reclaim broker");
    // 80 does not cross soft 90: no proactive request yet.
    let mut baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80,
        )
        .expect("baseline");
    assert!(reclaimer.requests.lock().expect("requests").is_empty());
    // 30 crosses soft (80 -> 110) AND exceeds hard 100, but 30 alone fits:
    // the plan must be a single hard request, never soft-first.
    *reclaimer.charge_to_release.lock().expect("charge") = Some(std::mem::replace(
        &mut baseline,
        broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                0,
            )
            .expect("placeholder"),
    ));
    let admitted = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        )
        .expect("hard reclaim admits the predicted failure");
    let requests = reclaimer.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1, "exactly one external attempt");
    assert_eq!(requests[0].trigger, ReclaimTrigger::CapacityDenial, "hard plan first");
    drop(requests);
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 1);
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").terminal_denials, 0);
    drop(admitted);
    drop(baseline);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn sep23_fup003_hard_remeasure_still_denied_is_terminal_without_second_callback() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let policy = ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("reclaim broker");
    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80,
        )
        .expect("baseline");
    // The callback frees nothing: the remeasure still denies, terminally,
    // with no second external attempt.
    let denied = broker.reserve(
        MemoryClass::ArtifactCache,
        "owner:r".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        30,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
    assert_eq!(reclaimer.requests.lock().expect("requests").len(), 1);
    assert_eq!(
        reclaimer.requests.lock().expect("requests")[0].trigger,
        ReclaimTrigger::CapacityDenial
    );
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 1);
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").terminal_denials, 1);
    drop(baseline);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn sep23_fup006_accepted_reconfigure_inside_callback_remeasures_under_new_policy() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(sep23_fup006_grouped_policy(100)),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    *reclaimer.broker_for_reentry.lock().expect("broker") = Some(broker.clone());
    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80,
        )
        .expect("baseline");
    let before = broker.observe().expect("observe");
    assert_eq!(before.resource_group_usage.get("g"), Some(&80));
    assert_eq!(before.ledger.live_ticket_count, 1);

    // Inside the hard callback: raise the class cap 100 -> 200 (the group
    // mapping is kept, so the live grouped ticket does not pin it) and
    // release the 80-byte baseline. The remeasure must admit the 30-byte
    // request under the NEW policy generation only.
    *reclaimer.reconfigure_inside.lock().expect("policy") = Some(sep23_fup006_grouped_policy(200));
    *reclaimer.charge_to_release.lock().expect("charge") = Some(baseline);
    let admitted = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        )
        .expect("remeasure under the raised cap admits");

    let attempts = reclaimer.reconfigure_attempts.lock().expect("attempts");
    assert_eq!(attempts.len(), 1, "exactly one in-callback reconfigure");
    assert!(attempts[0].is_ok(), "cap raise is accepted, got {:?}", attempts[0]);
    drop(attempts);
    assert_eq!(reclaimer.requests.lock().expect("requests").len(), 1, "single hard plan");

    let after = broker.observe().expect("observe");
    assert_eq!(
        after.report.policy_revision,
        before.report.policy_revision + 1,
        "initial/final revisions bracket exactly one accepted reconfigure"
    );
    assert_eq!(after.ledger.live_ticket_count, 1, "baseline released, request admitted");
    assert_eq!(after.ledger.total_owner_bytes(), 30);
    assert_eq!(after.ledger.total_class_bytes(), 30);
    assert_eq!(
        after.resource_group_usage.get("g"),
        Some(&30),
        "scheduler group usage tracks the post-callback ledger"
    );
    assert_eq!(after.reclaim.attempts, 1);
    drop(admitted);
    let drained = broker.observe().expect("observe");
    assert_eq!(drained.resource_group_usage.get("g"), Some(&0));
    assert_eq!(drained.report.policy_revision, after.report.policy_revision);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn sep23_audit_max_scale_plus_one_byte_denies_clean_without_callback() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let policy = ResidencyPolicy::new(u64::MAX)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, u64::MAX, u64::MAX, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("max-scale policy validates");
    // Exact-fill one kind slot to u64::MAX in a single reserve.
    let maxed = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:max".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            u64::MAX,
        )
        .expect("exact u64::MAX fill admits");
    assert!(reclaimer.requests.lock().expect("requests").is_empty());
    // One more byte: the preflight sees a reclaimable breach, but the
    // hard plan's prospective aggregate overflows u64, so construction
    // fails before any callback can run.
    let denied = broker.reserve(
        MemoryClass::ArtifactCache,
        "owner:max".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedMetadata,
        1,
    );
    assert_eq!(denied, Err(MemoryError::capacity("reclaim prospective usage exceeds u64")));
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 0);
    assert!(reclaimer.requests.lock().expect("requests").is_empty());
    // The broker is fully usable afterwards: observe, release, drain.
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), u64::MAX);
    drop(maxed);
    sep23_assert_fully_drained(&broker);
    let fresh = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:max".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata,
            7,
        )
        .expect("broker admits after the blocked attempt");
    drop(fresh);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn sep23_audit_failing_pending_drain_preserves_ticket_for_retry() {
    let b = broker();
    let pristine = b.ledger_snapshot().expect("snapshot");
    let foreign = sep23_audit_seed_deferred_foreign_ticket();
    let foreign_id = foreign.id();
    // A Drop-time rollback failure seeds the deferred queue (never swallowed).
    b.rollback_pending_ticket_on_drop(&foreign);
    {
        let core = b.inner.lock().expect("core");
        assert_eq!(core.deferred_pending_rollbacks.len(), 1, "Drop seeds the queue");
        assert_eq!(core.deferred_pending_rollbacks.front().unwrap().id(), foreign_id);
        assert_eq!(core.deferred_pending_rollbacks.front().unwrap().bytes(), 64);
    }
    // A still-failing drain retry keeps the exact ticket queued, and
    // finish_rebuild surfaces the lack of progress without losing authority.
    {
        let mut core = b.inner.lock().expect("core");
        LiveProcessMemoryBroker::drain_deferred_pending_rollbacks_batch(&mut core);
        assert_eq!(core.deferred_pending_rollbacks.len(), 1, "drain preserves failures");
        assert_eq!(core.deferred_pending_rollbacks.front().unwrap().id(), foreign_id);
    }
    let fence18 = b.begin_rebuild().expect("begin rebuild");
    let receipt = fence18.finish().expect("the fence opens even when cleanup is stuck");
    assert!(receipt.drain_error.is_some(), "stuck cleanup is surfaced as a drain error");
    assert_eq!(
        b.inner.lock().expect("core").deferred_pending_rollbacks.len(),
        1,
        "the finish receipt keeps the failing ticket queued"
    );
    assert_eq!(b.ledger_snapshot().expect("snapshot"), pristine, "drain mutates nothing");
    assert_eq!(b.ledger_snapshot().expect("snapshot").live_ticket_count, 0);
    // The broker is unusable-state-free: fresh admission works on top.
    let fresh = b
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            8,
        )
        .expect("broker usable after failed drain");
    drop(fresh);
    // Ledger counters drain to zero; the foreign ticket intentionally
    // stays queued (sep23_assert_fully_drained would reject it).
    let drained = b.ledger_snapshot().expect("snapshot");
    drained.validate_exact().expect("axes balance after drain");
    assert_eq!(drained.total_owner_bytes(), 0);
    assert_eq!(drained.total_class_bytes(), 0);
    assert_eq!(
        b.inner.lock().expect("core").deferred_pending_rollbacks.len(),
        1,
        "foreign authority still preserved at the end"
    );
}
#[test]
fn sep23_audit_failing_committed_drain_restores_whole_batch() {
    let b = broker();
    let pristine = b.ledger_snapshot().expect("snapshot");
    let foreign = sep23_audit_seed_deferred_foreign_ticket();
    let foreign_id = foreign.id();
    b.inner.lock().expect("core").deferred_committed_releases.push_back(foreign);
    let fence19 = b.begin_rebuild().expect("begin rebuild");
    let drained = fence19.finish().expect("the fence opens before the failing batch");
    assert!(drained.drain_error.is_some(), "a still-failing batch drain surfaces its error");
    let core = b.inner.lock().expect("core");
    assert_eq!(core.deferred_committed_releases.len(), 1, "whole batch restored");
    assert_eq!(core.deferred_committed_releases.front().unwrap().id(), foreign_id);
    drop(core);
    assert_eq!(b.ledger_snapshot().expect("snapshot"), pristine, "drain mutates nothing");
    let drained = b.ledger_snapshot().expect("snapshot");
    drained.validate_exact().expect("axes balance after drain");
    assert_eq!(drained.total_owner_bytes(), 0);
    assert_eq!(
        b.inner.lock().expect("core").deferred_committed_releases.len(),
        1,
        "foreign authority still preserved at the end"
    );
}
