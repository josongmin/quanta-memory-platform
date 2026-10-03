use super::*;

#[test]
fn competing_rebuild_fence_preserves_deferred_authority_and_original_error() {
    let broker = broker();
    let ticket = broker
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:deferred".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            17,
        )
        .expect("reserve");
    broker.commit(&ticket).expect("commit");
    broker.inner.lock().expect("core").deferred_committed_releases.push_back(ticket.clone());
    let fence = broker.begin_rebuild().expect("competing fence");
    let (pending, committed, error) = broker.drain_deferred_queues();
    assert_eq!((pending, committed), (0, 1));
    assert!(
        matches!(error, Some(MemoryError::InvalidState(message)) if message.contains("re-entered"))
    );
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 17);
    assert_eq!(
        broker
            .inner
            .lock()
            .expect("core")
            .deferred_committed_releases
            .front()
            .expect("preserved")
            .id(),
        ticket.id()
    );
    let receipt = fence.finish().expect("finish competing fence");
    assert!(receipt.drain_error.is_none(), "authority drains when the fence opens");
    assert_eq!(broker.ledger_snapshot().expect("drained").live_ticket_count, 0);
}
#[test]
fn sep23_fup006_denied_reconfigure_inside_callback_preserves_receipt_and_admits() {
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

    // Inside the hard callback: attempt to drop group "g" while the grouped
    // baseline is still live (FUP-007 must deny), then release the baseline.
    // Admission proceeds under the OLD policy with the receipt untouched.
    let ungrouped = ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
    *reclaimer.reconfigure_inside.lock().expect("policy") = Some(ungrouped);
    *reclaimer.charge_to_release.lock().expect("charge") = Some(baseline);
    let admitted = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        )
        .expect("release still admits under the unchanged policy");

    let attempts = reclaimer.reconfigure_attempts.lock().expect("attempts");
    assert_eq!(attempts.len(), 1);
    assert!(
        matches!(attempts[0], Err(MemoryError::InvalidState(_))),
        "group removal under a live grouped ticket is denied, got {:?}",
        attempts[0]
    );
    drop(attempts);

    let after = broker.observe().expect("observe");
    assert_eq!(
        after.report.policy_revision, before.report.policy_revision,
        "denied reconfigure bumps no revision"
    );
    assert_eq!(
        after.report.override_provenance, before.report.override_provenance,
        "denied reconfigure preserves the provenance receipt byte-for-byte"
    );
    assert_eq!(after.report.active_policy_source, before.report.active_policy_source);
    assert_eq!(after.resource_group_usage.get("g"), Some(&30));
    assert_eq!(after.ledger.total_owner_bytes(), 30);
    drop(admitted);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn sep23_audit_group_denial_runs_single_hard_regardless_of_soft_crossing() {
    // The audit's soft-45 vs soft-30 probe: the first plan for a group
    // hard denial must be Hard in both cases — soft crossing only adds an
    // earlier proactive signal on the seed, never a soft-first plan.
    for group_soft in [45u64, 30u64] {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(sep23_audit_group_ladder_policy(group_soft)),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                40,
            )
            .expect("baseline");
        let seed_requests = reclaimer.requests.lock().expect("requests").len();
        *reclaimer.charge_to_release.lock().expect("charge") = Some(baseline);
        let admitted = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                20,
            )
            .expect("hard reclaim admits the group denial");
        let requests = reclaimer.requests.lock().expect("requests").clone();
        assert_eq!(
            requests.len(),
            seed_requests + 1,
            "soft {group_soft}: exactly one plan for the denial"
        );
        assert_eq!(
            requests.last().expect("hard request").trigger,
            ReclaimTrigger::CapacityDenial,
            "soft {group_soft}: the denial plan is Hard, never soft-first"
        );
        drop(admitted);
        sep23_assert_fully_drained(&broker);
    }
}
#[test]
fn sep23_audit_wrong_victim_release_does_not_admit() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let policy = ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false))
        .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 90, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80,
        )
        .expect("baseline");
    let victim = broker
        .reserve_managed_charge(
            MemoryClass::EvidenceCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            50,
        )
        .expect("victim");
    // The callback frees 50 bytes on the WRONG class: the Artifact breach
    // (80 + 30 > 100) survives, so the remeasure denies terminally.
    *reclaimer.charge_to_release.lock().expect("charge") = Some(victim);
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
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").terminal_denials, 1);
    let snapshot = broker.ledger_snapshot().expect("snapshot");
    assert_eq!(snapshot.total_owner_bytes(), 80, "only the victim left");
    drop(baseline);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn sep23_audit_forged_reclaimed_bytes_are_recorded_but_never_trusted() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let policy = ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80,
        )
        .expect("baseline");
    // The callback frees nothing but reports 30 reclaimed bytes.
    *reclaimer.forged_reported_bytes.lock().expect("forged") = Some(30);
    let denied = broker.reserve(
        MemoryClass::ArtifactCache,
        "owner:r".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        30,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
    let stats = broker.reclaim_runtime_stats().expect("stats");
    assert_eq!(stats.attempts, 1);
    assert_eq!(stats.reported_reclaimed_bytes, 30, "telemetry records the lie");
    assert_eq!(stats.terminal_denials, 1, "admission remeasured the ledger");
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 80);
    drop(baseline);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn sep23_audit_panicking_reclaimer_is_contained_and_broker_survives() {
    let reclaimer = Arc::new(PanickingReclaimer { calls: Mutex::new(0) });
    let policy = ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80,
        )
        .expect("baseline");
    // The hard plan panics inside the callback: the panic is contained,
    // the remeasure denies on the unmoved ledger, and the broker (and its
    // lock) stays fully usable.
    let denied = broker.reserve(
        MemoryClass::ArtifactCache,
        "owner:r".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        30,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
    assert_eq!(*reclaimer.calls.lock().expect("calls"), 1, "single attempt");
    let stats = broker.reclaim_runtime_stats().expect("stats");
    assert_eq!(stats.attempts, 1);
    assert_eq!(stats.failures, 1, "panic counts as a callback failure");
    assert_eq!(stats.terminal_denials, 1);
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 80);
    drop(baseline);
    sep23_assert_fully_drained(&broker);
    // A hostile callback is quarantined after the first panic: admission
    // still remeasures the unchanged ledger, without retrying host code.
    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80,
        )
        .expect("baseline again");
    let denied = broker.reserve(
        MemoryClass::ArtifactCache,
        "owner:r".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        30,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
    assert_eq!(*reclaimer.calls.lock().expect("calls"), 1);
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 1);
    drop(baseline);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn reclaimer_release_then_panic_uses_remeasured_ledger_not_callback_result() {
    struct ReleaseThenPanic {
        victim: Mutex<Option<ManagedCharge>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl MemoryReclaimer for ReleaseThenPanic {
        fn reclaim(
            &self,
            _request: &ReclaimRequest,
        ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            let mut victim = self.victim.lock().expect("victim slot").take().expect("victim");
            victim.release().expect("callback releases real ledger authority");
            panic!("callback fails after making progress");
        }
    }

    let reclaimer = Arc::new(ReleaseThenPanic {
        victim: Mutex::new(None),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let policy = ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    let victim = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "victim".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80,
        )
        .expect("victim charge");
    *reclaimer.victim.lock().expect("victim slot") = Some(victim);

    let admitted = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "request".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        )
        .expect("post-callback ledger, not callback success, determines admission");
    assert_eq!(reclaimer.calls.load(AtomicOrdering::SeqCst), 1);
    let stats = broker.reclaim_runtime_stats().expect("stats");
    assert_eq!(stats.attempts, 1);
    assert_eq!(stats.failures, 1, "panic remains observable as callback failure");
    assert_eq!(stats.terminal_denials, 0);
    let snapshot = broker.ledger_snapshot().expect("snapshot");
    snapshot.validate_exact().expect("post-callback axes balance");
    assert_eq!(snapshot.total_owner_bytes(), 30);
    assert_eq!(snapshot.live_ticket_count, 1);
    drop(admitted);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn sep23_audit_zero_byte_reserve_fires_no_callback_even_over_soft() {
    let reclaimer = Arc::new(RecordingReclaimer::default());
    let policy = ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
    let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("broker");
    // Seed past soft 90: the seed itself fires one proactive soft signal.
    let baseline = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            95,
        )
        .expect("baseline");
    assert_eq!(reclaimer.requests.lock().expect("requests").len(), 1);
    // A 0-byte reserve over soft adds nothing, crosses nothing, fires nothing.
    let zero = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            0,
        )
        .expect("0-byte admits");
    assert_eq!(
        reclaimer.requests.lock().expect("requests").len(),
        1,
        "no callback for a 0-byte edge that crosses no threshold"
    );
    drop(zero);
    drop(baseline);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn terminal_preflight_preserves_the_live_first_breach_when_zero_axis_differs() {
    let policy = ResidencyPolicy::new(100)
        .with_class_budget(MemoryClass::MemoResidency, class_budget(0, 10, 10, false))
        .with_resource_group(
            "g",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 10,
                hard_cap_bytes: 10,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("owner:x", "g");
    let mut report = report_with(policy);
    // The shared test fixture widens constructor-zero class caps; restore
    // this case's intentional zero cap to keep the oracle independent.
    report
        .residency
        .classes
        .insert(MemoryClass::ArtifactCache, MemoryClassBudget::zero(ReclaimPriority::Normal));
    let broker = LiveProcessMemoryBroker::from_report(report)
        .expect("zero ArtifactCache and ten-byte group form a valid policy");
    let seed = broker
        .reserve_managed_charge(
            MemoryClass::MemoResidency,
            "owner:x".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata,
            10,
        )
        .expect("fill group with a different class");
    let before = broker.ledger_snapshot().expect("before denial");
    let (preflight_error, attempt_error) = {
        let mut core = broker.lock().expect("broker lock");
        let verdict = LiveProcessMemoryBroker::admission_preflight(
            &core,
            &MemoryClass::ArtifactCache,
            &"owner:x".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        );
        let AdmissionVerdict::TerminalDenial { breach } = verdict else {
            panic!("a zero-cap class is terminal: {verdict:?}");
        };
        let attempt_error = LiveProcessMemoryBroker::reserve_on(
            &mut core,
            MemoryClass::ArtifactCache,
            "owner:x".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        )
        .expect_err("group-first attempt denies");
        (breach.into_error(), attempt_error)
    };
    let public_error = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:x".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        )
        .expect_err("public denial");
    assert_eq!(preflight_error, attempt_error);
    assert_eq!(public_error, attempt_error);
    assert!(
        matches!(public_error, MemoryError::Capacity(message) if message.contains("resource group g"))
    );
    assert_eq!(broker.ledger_snapshot().expect("denial is fail-closed"), before);
    drop(seed);
}
#[test]
fn sep23_fup003_preflight_attempt_agreement_randomized() {
    use crate::domain::admission::AdmissionVerdict;
    const EDGE_BYTES: [u64; 20] = [
        0,
        1,
        2,
        63,
        64,
        65,
        99,
        100,
        101,
        799,
        800,
        801,
        899,
        900,
        901,
        999,
        1_000,
        1_001,
        u64::MAX - 1,
        u64::MAX,
    ];
    const CLASSES: [MemoryClass; 5] = [
        MemoryClass::ArtifactCache,
        MemoryClass::EvidenceCache,
        MemoryClass::TraceIndex,
        MemoryClass::Headroom,
        MemoryClass::SpillFileCache,
    ];
    const OWNERS: [&str; 3] = ["owner:plain", "owner:q", "owner:r"];
    const KINDS: [ChargeKind; 3] =
        [ChargeKind::RetainedPayload, ChargeKind::RetainedMetadata, ChargeKind::LiveScratch];
    for shape in 0..4u64 {
        let policy = sep23_agreement_policy(shape);
        // No reclaimer: ReclaimCandidate denies with the live breach and
        // fires nothing, so verdict and attempt are directly comparable.
        let broker =
            LiveProcessMemoryBroker::from_report(report_with(policy.clone())).expect("broker");
        let mut rng = Sep23XorShift64(0x9E37_79B9_7F4A_7C15u64.wrapping_add(shape));
        let mut live: Vec<ManagedCharge> = Vec::new();
        let mut terminal_spot_checks = 0u32;
        for step in 0..500u32 {
            if step % 100 == 99 {
                // Rebuild-fence agreement inside the evolving state.
                let fence20 = broker.begin_rebuild().expect("begin rebuild");
                let class = CLASSES[rng.below(5) as usize].clone();
                let owner = OWNERS[rng.below(3) as usize].to_string();
                let core = broker.inner.lock().expect("core");
                let verdict = LiveProcessMemoryBroker::admission_preflight(
                    &core,
                    &class,
                    &owner,
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    10,
                );
                drop(core);
                assert!(
                    matches!(
                        verdict,
                        AdmissionVerdict::Blocked {
                            reason: crate::domain::admission::AdmissionBlock::RebuildInProgress
                        }
                    ),
                    "shape {shape} step {step}: fenced preflight blocks, got {verdict:?}"
                );
                let result = broker.reserve(
                    class,
                    owner,
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    10,
                );
                assert_eq!(
                    result,
                    Err(MemoryError::InvalidState(
                        "rebuild in progress: live reserve rejected".into()
                    )),
                    "shape {shape} step {step}: fenced attempt matches"
                );
                assert!(fence20.finish().expect("finish rebuild").drain_error.is_none());
                continue;
            }
            let class = CLASSES[rng.below(5) as usize].clone();
            let owner = OWNERS[rng.below(3) as usize].to_string();
            let kind = KINDS[rng.below(3) as usize];
            let bytes = if rng.below(2) == 0 {
                EDGE_BYTES[rng.below(20) as usize]
            } else {
                rng.below(2_000)
            };
            let core = broker.inner.lock().expect("core");
            let verdict = LiveProcessMemoryBroker::admission_preflight(
                &core,
                &class,
                &owner,
                MemoryContextKind::Request,
                kind,
                bytes,
            );
            drop(core);
            let result = broker.reserve_managed_charge(
                class.clone(),
                owner.clone(),
                MemoryContextKind::Request,
                kind,
                bytes,
            );
            match verdict {
                AdmissionVerdict::Admit => {
                    let charge = result.unwrap_or_else(|error| {
                        panic!(
                            "shape {shape} step {step}: preflight Admit but attempt \
                             denied ({class:?} {owner} {kind:?} {bytes}): {error:?}"
                        )
                    });
                    // Hold most charges to evolve live state; release some
                    // to exercise the release path mid-stream.
                    if live.len() < 24 && rng.below(4) != 0 {
                        live.push(charge);
                    }
                    if live.len() == 24 {
                        live.remove(0);
                    }
                    if rng.below(8) == 0 && !live.is_empty() {
                        live.remove(rng.below(live.len() as u64) as usize);
                    }
                }
                AdmissionVerdict::TerminalDenial { breach } => {
                    let expected = breach.into_error();
                    match &result {
                        Err(error) if error == &expected => {}
                        other => panic!(
                            "shape {shape} step {step}: terminal verdict but attempt \
                             diverged ({class:?} {owner} {kind:?} {bytes}): {other:?} \
                             vs {expected:?}"
                        ),
                    }
                    // Terminality proof: the same request denies on a
                    // fresh broker, so no callback on live state could
                    // have helped. Its first limiting axis may differ
                    // from the live first breach and need not have an
                    // identical diagnostic.
                    if terminal_spot_checks < 20 {
                        terminal_spot_checks += 1;
                        let fresh =
                            LiveProcessMemoryBroker::from_report(report_with(policy.clone()))
                                .expect("fresh broker");
                        let fresh_result = fresh
                            .reserve_managed_charge(
                                class.clone(),
                                owner.clone(),
                                MemoryContextKind::Request,
                                kind,
                                bytes,
                            )
                            .map(|_| ());
                        assert!(
                            matches!(fresh_result, Err(MemoryError::Capacity(_))),
                            "shape {shape} step {step}: terminal denial reproduces at zero usage: {fresh_result:?}"
                        );
                    }
                }
                AdmissionVerdict::ReclaimCandidate { breach } => {
                    let expected = breach.into_error();
                    match &result {
                        Err(error) if error == &expected => {}
                        other => panic!(
                            "shape {shape} step {step}: candidate verdict but attempt \
                             diverged ({class:?} {owner} {kind:?} {bytes}): {other:?} \
                             vs {expected:?}"
                        ),
                    }
                }
                AdmissionVerdict::Blocked { reason } => {
                    let expected = reason.into_error();
                    match &result {
                        Err(error) if error == &expected => {}
                        other => panic!(
                            "shape {shape} step {step}: blocked verdict but attempt \
                             diverged ({class:?} {owner} {kind:?} {bytes}): {other:?} \
                             vs {expected:?}"
                        ),
                    }
                }
            }
        }
        drop(live);
        sep23_assert_fully_drained(&broker);
    }
}
