use super::*;
use crate::domain::ledger::UsageBreakdown;
use crate::ports::reclaim::{ReclaimOutcome, ReclaimScope, ReclaimUnit};

type Callback = Box<dyn FnOnce() -> MemoryResult<ReclaimOutcome> + Send>;

#[derive(Default)]
struct ReplacementReclaimer {
    requests: Mutex<Vec<ReclaimRequest>>,
    action: Mutex<Option<Callback>>,
}

impl ReplacementReclaimer {
    fn arm(&self, action: impl FnOnce() -> MemoryResult<ReclaimOutcome> + Send + 'static) {
        *self.action.lock().expect("callback slot") = Some(Box::new(action));
    }

    fn requests(&self) -> Vec<ReclaimRequest> {
        self.requests.lock().expect("requests").clone()
    }
}

impl MemoryReclaimer for ReplacementReclaimer {
    fn reclaim(&self, request: &ReclaimRequest) -> MemoryResult<ReclaimOutcome> {
        self.requests.lock().expect("requests").push(request.clone());
        // Release the fixture lock before executing hostile callback behavior.
        let action = self.action.lock().expect("callback slot").take();
        action.map_or_else(|| Ok(ReclaimOutcome::default()), |action| action())
    }
}

fn policy(soft: u64, hard: u64) -> ResidencyPolicy {
    ResidencyPolicy::new(1_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, soft, hard, false))
        .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 1_000, 1_000, false))
}

fn live(reclaimer: &Arc<ReplacementReclaimer>, policy: ResidencyPolicy) -> LiveProcessMemoryBroker {
    LiveProcessMemoryBroker::from_report_with_reclaimer(
        report_with(policy),
        Some(reclaimer.clone()),
    )
    .expect("valid replacement fixture")
}

fn charge(broker: &LiveProcessMemoryBroker, owner: &str, bytes: u64) -> ManagedCharge {
    broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            owner.into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            bytes,
        )
        .expect("seed charge below hard cap")
}

fn replace(old: &mut ManagedCharge, bytes: u64) -> Result<(), ManagedChargeRechargeError> {
    old.recharge(
        MemoryClass::ArtifactCache,
        "old".into(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        bytes,
    )
}

fn preserved(old: &ManagedCharge, id: u64, bytes: u64) {
    assert_eq!(old.id(), Some(id), "denial preserves the issuing ticket identity");
    assert_eq!(old.bytes(), Some(bytes));
    assert_eq!(old.class(), Some(&MemoryClass::ArtifactCache));
    assert_eq!(old.owner().map(String::as_str), Some("old"));
    assert_eq!(old.context(), Some(MemoryContextKind::Request));
    assert_eq!(old.kind(), Some(ChargeKind::RetainedPayload));
}

#[test]
fn replacement_remeasures_zero_partial_exact_and_excess_actual_release() {
    for freed in [0, 20, 40, 60] {
        let reclaimer = Arc::new(ReplacementReclaimer::default());
        let broker = live(&reclaimer, policy(100, 100));
        let mut old = charge(&broker, "old", 40);
        let old_id = old.id().expect("old id");
        let victims = Arc::new(Mutex::new(vec![
            charge(&broker, "victim", 20),
            charge(&broker, "victim", 40),
        ]));
        let callback_broker = broker.clone();
        let callback_victims = victims.clone();
        reclaimer.arm(move || {
            let before = callback_broker.observe().expect("callback runs outside broker lock");
            assert_eq!(before.ledger.owner_usage["old"].retained_payload_bytes, 40);
            assert_eq!(before.ledger.live_ticket_count, 3, "old is held until admission succeeds");
            let mut victims = callback_victims.lock().expect("victims");
            match freed {
                20 => victims[0].release().expect("partial release"),
                40 => victims[1].release().expect("exact release"),
                60 => {
                    victims[0].release().expect("first release");
                    victims[1].release().expect("second release");
                }
                _ => {}
            }
            // Both an under-report and an exaggerated report must lose to the ledger.
            Ok(ReclaimOutcome { reclaimed_bytes: if freed >= 40 { 0 } else { u64::MAX } })
        });
        let result = replace(&mut old, 80);
        assert_eq!(result.is_ok(), freed >= 40, "actual reduction {freed}");
        let requests = reclaimer.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].trigger, ReclaimTrigger::CapacityDenial);
        assert_eq!(requests[0].requested_bytes, 80, "absolute new charge, not delta");
        assert_eq!(requests[0].target.unit, ReclaimUnit::Bytes);
        assert_eq!(requests[0].target.scope, ReclaimScope::Class(MemoryClass::ArtifactCache));
        assert_eq!(requests[0].target.minimum_reduction, Some(40));
        let snap = broker.ledger_snapshot().expect("final live image");
        snap.validate_exact().expect("all axes balance");
        assert_eq!(snap.total_owner_bytes(), if freed >= 40 { 140 - freed } else { 100 - freed });
        assert_eq!(
            snap.owner_usage["old"].retained_payload_bytes,
            if freed >= 40 { 80 } else { 40 }
        );
        assert_eq!(
            snap.live_ticket_count,
            if freed == 60 {
                1
            } else if freed == 0 {
                3
            } else {
                2
            }
        );
        if freed >= 40 {
            assert_ne!(old.id(), Some(old_id));
        } else {
            assert_eq!(
                result.expect_err("insufficient release").recovery,
                ManagedChargeRechargeRecovery::ChargePreserved
            );
            preserved(&old, old_id, 40);
        }
        old.release().expect("old or replacement retains usable release authority");
        drop(victims);
        sep23_assert_fully_drained(&broker);
    }
}

#[test]
fn wrong_class_victim_release_cannot_admit_replacement() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(100, 100));
    let mut old = charge(&broker, "old", 40);
    let old_id = old.id().expect("id");
    let victim = charge(&broker, "victim", 60);
    let mut wrong = broker
        .reserve_managed_charge(
            MemoryClass::EvidenceCache,
            "wrong".into(),
            MemoryContextKind::Session,
            ChargeKind::RetainedMetadata,
            20,
        )
        .expect("wrong-scope victim");
    reclaimer.arm(move || {
        wrong.release().expect("real release in the wrong scope");
        Ok(ReclaimOutcome { reclaimed_bytes: u64::MAX })
    });
    assert!(matches!(
        replace(&mut old, 80),
        Err(ManagedChargeRechargeError { error: MemoryError::Capacity(_), .. })
    ));
    preserved(&old, old_id, 40);
    let snap = broker.ledger_snapshot().expect("failed replacement image");
    snap.validate_exact().expect("exact denied postimage");
    assert_eq!(snap.total_owner_bytes(), 100, "external victim release is not rolled back");
    assert!(!snap.owner_usage.contains_key("wrong"));
    assert_eq!(reclaimer.requests().len(), 1);
    old.release().expect("old usable");
    drop(victim);
    sep23_assert_fully_drained(&broker);
}

#[test]
fn callback_error_and_panic_keep_old_authority_and_completed_victim_release() {
    for panics in [false, true] {
        let reclaimer = Arc::new(ReplacementReclaimer::default());
        let broker = live(&reclaimer, policy(100, 100));
        let mut old = charge(&broker, "old", 40);
        let id = old.id().expect("id");
        let survivor = charge(&broker, "survivor", 40);
        let mut victim = charge(&broker, "victim", 20);
        reclaimer.arm(move || {
            victim.release().expect("completed release survives callback failure");
            if panics {
                panic!("replacement reclaimer panic after release");
            }
            Err(MemoryError::operation_in_progress("injected callback error"))
        });
        assert!(replace(&mut old, 80).is_err());
        preserved(&old, id, 40);
        assert_eq!(reclaimer.requests().len(), 1);
        let observed = broker.observe().expect("broker survives callback failure");
        observed.ledger.validate_exact().expect("all axes remain exact");
        assert_eq!(observed.semantic_bytes, 80);
        assert_eq!(observed.ledger.live_ticket_count, 2);
        assert_eq!(observed.reclaim.failures, 1);
        old.release().expect("old authority survives");
        drop(survivor);
        sep23_assert_fully_drained(&broker);
    }
}

#[test]
fn legal_same_axis_shrink_and_equal_replacement_do_not_reclaim() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(50, 100));
    let mut old = charge(&broker, "old", 80);
    reclaimer.requests.lock().expect("requests").clear();
    let attempts = broker.reclaim_runtime_stats().expect("baseline").attempts;
    replace(&mut old, 80).expect("equal replacement");
    replace(&mut old, 60).expect("shrink remains above soft cap");
    replace(&mut old, 40).expect("shrink below soft cap");
    assert!(reclaimer.requests().is_empty());
    assert_eq!(broker.reclaim_runtime_stats().expect("after shrink").attempts, attempts);
    old.release().expect("drain");
    sep23_assert_fully_drained(&broker);
}

#[test]
fn impossible_new_charge_uses_zero_probe_without_subtracting_old_twice() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(100, 100));
    let mut old = charge(&broker, "old", 40);
    let id = old.id().expect("id");
    let before = broker.ledger_snapshot().expect("preimage");
    assert!(matches!(
        replace(&mut old, 101),
        Err(ManagedChargeRechargeError { error: MemoryError::Capacity(_), .. })
    ));
    assert!(reclaimer.requests().is_empty(), "zero usage still cannot fit 101 in a 100 class");
    assert_eq!(broker.ledger_snapshot().expect("postimage"), before);
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").terminal_denials, 1);
    preserved(&old, id, 40);
    old.release().expect("original remains usable");
    sep23_assert_fully_drained(&broker);
}

#[test]
fn soft_reclaim_recomputes_net_target_and_does_not_add_hard_retry_after_policy_change() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(50, 100));
    let mut old = charge(&broker, "old", 40);
    let id = old.id().expect("id");
    let callback_broker = broker.clone();
    reclaimer.arm(move || {
        callback_broker
            .try_live_reconfigure(policy(50, 50))
            .expect("legal current usage, smaller future cap");
        Ok(ReclaimOutcome::default())
    });
    assert!(replace(&mut old, 60).is_err(), "final admission uses current policy");
    let requests = reclaimer.requests();
    assert_eq!(requests.len(), 1, "soft attempt must not fall into a second hard callback");
    assert_eq!(requests[0].trigger, ReclaimTrigger::SoftThreshold);
    assert_eq!(requests[0].requested_bytes, 60);
    assert_eq!(
        requests[0].target.minimum_reduction,
        Some(10),
        "60 net postimage minus 50 soft cap"
    );
    assert_eq!(requests[0].target.scope, ReclaimScope::Class(MemoryClass::ArtifactCache));
    preserved(&old, id, 40);
    let observation = broker.observe().expect("consistent reconfigured image");
    assert_eq!(observation.report.policy_revision, 2);
    assert_eq!(observation.semantic_bytes, 40);
    old.release().expect("drain");
    sep23_assert_fully_drained(&broker);
}

#[test]
fn callback_policy_raise_can_admit_from_current_state_without_releasing_old() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(100, 100));
    let mut old = charge(&broker, "old", 40);
    let victim = charge(&broker, "victim", 60);
    let callback_broker = broker.clone();
    reclaimer.arm(move || {
        callback_broker.try_live_reconfigure(policy(150, 150)).expect("raise cap during callback");
        Ok(ReclaimOutcome::default())
    });
    replace(&mut old, 80).expect("current policy admits net replacement");
    let observation = broker.observe().expect("postimage");
    assert_eq!(observation.report.policy_revision, 2);
    assert_eq!(observation.semantic_bytes, 140);
    assert_eq!(observation.ledger.live_ticket_count, 2);
    assert_eq!(reclaimer.requests().len(), 1);
    old.release().expect("release replacement");
    drop(victim);
    sep23_assert_fully_drained(&broker);
}

#[test]
fn callback_started_rebuild_denies_replacement_with_old_held_until_fence_finish() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(100, 100));
    let mut old = charge(&broker, "old", 40);
    let id = old.id().expect("id");
    let victim = charge(&broker, "victim", 60);
    let fence = Arc::new(Mutex::new(None::<RebuildFence>));
    let callback_fence = fence.clone();
    let callback_broker = broker.clone();
    reclaimer.arm(move || {
        *callback_fence.lock().expect("fence slot") =
            Some(callback_broker.begin_rebuild().expect("callback begins rebuild outside lock"));
        Ok(ReclaimOutcome::default())
    });
    let before = broker.ledger_snapshot().expect("preimage");
    assert!(matches!(
        replace(&mut old, 80),
        Err(ManagedChargeRechargeError { error: MemoryError::InvalidState(_), .. })
    ));
    preserved(&old, id, 40);
    let during = broker.ledger_snapshot().expect("fenced image");
    assert_eq!(during.owner_usage, before.owner_usage);
    assert_eq!(during.live_ticket_count, 2);
    assert_eq!(reclaimer.requests().len(), 1);
    assert!(fence
        .lock()
        .expect("fence")
        .take()
        .expect("live fence")
        .finish()
        .expect("finish")
        .drain_error
        .is_none());
    old.release().expect("old remains usable after fence");
    drop(victim);
    sep23_assert_fully_drained(&broker);
}

#[test]
fn callback_invalidated_old_ticket_is_not_resurrected() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(100, 100));
    let old = broker
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "old".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            40,
        )
        .expect("old committed");
    let victim = charge(&broker, "victim", 60);
    let callback_old = old.clone();
    let callback_broker = broker.clone();
    reclaimer.arm(move || {
        callback_broker.release(&callback_old).expect("external invalidation of old authority");
        Ok(ReclaimOutcome { reclaimed_bytes: 40 })
    });
    let result = broker.recharge(
        &old,
        MemoryClass::ArtifactCache,
        "old".into(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        80,
    );
    assert!(matches!(result, Err(RechargeError { error: MemoryError::InvalidState(_), .. })));
    let snap = broker.ledger_snapshot().expect("invalidated postimage");
    snap.validate_exact().expect("exact ledger");
    assert_eq!(snap.total_owner_bytes(), 60);
    assert_eq!(snap.live_ticket_count, 1);
    assert!(!snap.owner_usage.contains_key("old"));
    assert!(broker.release(&old).is_err(), "old remains invalid");
    assert_eq!(reclaimer.requests().len(), 1);
    drop(victim);
    sep23_assert_fully_drained(&broker);
}

#[test]
fn replacement_callback_reentry_is_bounded_and_keeps_nested_old_unchanged() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(100, 100));
    let mut old = charge(&broker, "old", 40);
    let nested = Arc::new(Mutex::new(charge(&broker, "nested", 60)));
    let nested_result = Arc::new(Mutex::new(None));
    let callback_result = nested_result.clone();
    let callback_nested = nested.clone();
    let callback_broker = broker.clone();
    reclaimer.arm(move || {
        let mut nested = callback_nested.lock().expect("nested lease");
        let id = nested.id();
        let result = nested.recharge(
            MemoryClass::ArtifactCache,
            "nested".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        );
        *callback_result.lock().expect("nested result") = Some((
            result,
            id,
            nested.id(),
            nested.bytes(),
            callback_broker.observe()?.semantic_bytes,
        ));
        Ok(ReclaimOutcome::default())
    });
    assert!(replace(&mut old, 80).is_err());
    let (result, before_id, after_id, bytes, semantic) =
        nested_result.lock().expect("result").take().expect("callback executed nested recharge");
    assert!(matches!(
        result,
        Err(ManagedChargeRechargeError { error: MemoryError::OperationInProgress(_), .. })
    ));
    assert_eq!(before_id, after_id);
    assert_eq!(bytes, Some(60));
    assert_eq!(semantic, 100);
    assert_eq!(reclaimer.requests().len(), 1);
    old.release().expect("outer old remains usable");
    drop(nested);
    sep23_assert_fully_drained(&broker);
}

#[test]
fn smaller_total_replacement_still_checks_each_new_attribution_axis() {
    for axis in ["class", "owner", "group", "scratch"] {
        let reclaimer = Arc::new(ReplacementReclaimer::default());
        let mut residency = policy(1_000, 1_000);
        if axis == "class" {
            residency = residency
                .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 80, 80, false));
        }
        if axis == "owner" {
            residency = residency
                .with_owner_quota("new", OwnerQuota { soft_cap_bytes: 80, hard_cap_bytes: 80 });
        }
        if axis == "group" {
            residency = residency
                .with_resource_group(
                    "new-group",
                    ResourceGroup {
                        reserved_bytes: 0,
                        soft_cap_bytes: 80,
                        hard_cap_bytes: 80,
                        burst_percent: 0,
                        reclaim_priority: ReclaimPriority::Normal,
                        admission_weight: 1,
                    },
                )
                .with_owner_group("new", "new-group");
        }
        let mut report = report_with(residency);
        if axis == "scratch" {
            report.execution.max_execution_scratch_bytes = Some(80);
        }
        let broker =
            LiveProcessMemoryBroker::from_report_with_reclaimer(report, Some(reclaimer.clone()))
                .expect("axis fixture");
        let mut old = charge(&broker, "old", 80);
        let kind =
            if axis == "scratch" { ChargeKind::LiveScratch } else { ChargeKind::RetainedMetadata };
        let mut victim = broker
            .reserve_managed_charge(
                MemoryClass::EvidenceCache,
                "new".into(),
                MemoryContextKind::Session,
                kind,
                40,
            )
            .expect("new-axis victim");
        reclaimer.arm(move || {
            victim.release().expect("release matching victim");
            Ok(ReclaimOutcome::default())
        });
        old.recharge(
            MemoryClass::EvidenceCache,
            "new".into(),
            MemoryContextKind::Session,
            kind,
            60,
        )
        .expect("reclaimed new axis admits smaller total");
        let requests = reclaimer.requests();
        assert_eq!(requests.len(), 1, "{axis}: 80 to 60 is not an all-axis shrink");
        assert_eq!(requests[0].target.minimum_reduction, Some(20), "{axis}");
        let expected_scope = match axis {
            "class" => ReclaimScope::Class(MemoryClass::EvidenceCache),
            "owner" => ReclaimScope::Owner("new".into()),
            "group" => ReclaimScope::ResourceGroup("new-group".into()),
            _ => ReclaimScope::ExecutionScratch,
        };
        assert_eq!(requests[0].target.scope, expected_scope, "{axis}");
        let observation = broker.observe().expect("atomic moved axes");
        observation.ledger.validate_exact().expect("all moved axes exact");
        let expected = if kind == ChargeKind::LiveScratch {
            UsageBreakdown { live_scratch_bytes: 60, ..UsageBreakdown::default() }
        } else {
            UsageBreakdown { retained_metadata_bytes: 60, ..UsageBreakdown::default() }
        };
        assert_eq!(
            observation.ledger.class_usage,
            BTreeMap::from([(MemoryClass::EvidenceCache, expected)])
        );
        assert_eq!(observation.ledger.owner_usage, BTreeMap::from([("new".into(), expected)]));
        assert_eq!(
            observation.ledger.context_usage,
            BTreeMap::from([(MemoryContextKind::Session, expected)])
        );
        assert_eq!(observation.ledger.live_ticket_count, 1);
        if axis == "group" {
            assert_eq!(observation.resource_group_usage["new-group"], 60);
        }
        old.release().expect("replacement is usable");
        sep23_assert_fully_drained(&broker);
    }
}

#[test]
fn unrepresentable_net_postimage_has_unknown_byte_reduction_and_can_reclaim() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(
        &reclaimer,
        ResidencyPolicy::new(u64::MAX).with_class_budget(
            MemoryClass::ArtifactCache,
            class_budget(0, u64::MAX, u64::MAX, false),
        ),
    );
    let mut old = charge(&broker, "old", 1);
    let mut victim = charge(&broker, "victim", u64::MAX - 1);
    reclaimer.arm(move || {
        victim.release().expect("refund huge semantic charge");
        Ok(ReclaimOutcome::default())
    });
    replace(&mut old, 2).expect("real release makes overflowed postimage representable");
    let requests = reclaimer.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].target.unit, ReclaimUnit::Bytes);
    assert_eq!(requests[0].target.minimum_reduction, None, "overflow is not an exact MAX deficit");
    assert_eq!(requests[0].requested_bytes, 2);
    assert_eq!(broker.ledger_snapshot().expect("postimage").total_owner_bytes(), 2);
    old.release().expect("release replacement");
    sep23_assert_fully_drained(&broker);
}

#[test]
fn replacement_and_reserve_share_the_same_in_flight_callback_permit() {
    use std::sync::mpsc;
    use std::time::Duration;
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(100, 100));
    let mut old = charge(&broker, "old", 40);
    let id = old.id().expect("id");
    let victim = charge(&broker, "victim", 60);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    reclaimer.arm(move || {
        entered_tx.send(()).expect("callback entered");
        release_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|error| MemoryError::config(format!("bounded callback wait: {error}")))?;
        Ok(ReclaimOutcome::default())
    });
    let competing_broker = broker.clone();
    let reserve = std::thread::spawn(move || {
        competing_broker.reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "competing".into(),
            MemoryContextKind::Runtime,
            ChargeKind::RetainedMetadata,
            1,
        )
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).expect("reserve holds callback permit");
    let before = broker.ledger_snapshot().expect("preimage");
    assert!(replace(&mut old, 80).is_err());
    preserved(&old, id, 40);
    assert_eq!(broker.ledger_snapshot().expect("denied image"), before);
    release_tx.send(()).expect("release first callback");
    assert!(reserve.join().expect("reserve thread").is_err());
    assert_eq!(
        reclaimer.requests().len(),
        1,
        "replacement must not get an independent callback lane"
    );
    assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 1);
    old.release().expect("old remains usable");
    drop(victim);
    sep23_assert_fully_drained(&broker);
}

#[test]
fn replacement_preflight_attempt_agreement_over_evolving_attribution_and_floors() {
    use crate::domain::admission::{AdmissionRequest, AdmissionVerdict, PreflightInputs};
    const BYTES: [u64; 13] = [0, 1, 24, 25, 26, 49, 50, 51, 79, 80, 81, 301, u64::MAX];
    let residency = ResidencyPolicy::new(300)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 150, 150, false))
        .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false))
        .with_owner_quota("new", OwnerQuota { soft_cap_bytes: 80, hard_cap_bytes: 80 })
        .with_resource_group(
            "g-old",
            ResourceGroup {
                reserved_bytes: 25,
                soft_cap_bytes: 150,
                hard_cap_bytes: 150,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_resource_group(
            "g-new",
            ResourceGroup {
                reserved_bytes: 25,
                soft_cap_bytes: 100,
                hard_cap_bytes: 100,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("old", "g-old")
        .with_owner_group("new", "g-new");
    let mut report = report_with(residency.clone());
    report.execution.max_execution_scratch_bytes = Some(50);
    let broker =
        LiveProcessMemoryBroker::from_report(report).expect("agreement fixture without callbacks");
    let mut tickets = Vec::new();
    for owner in ["old", "new", "plain"] {
        tickets.push(
            broker
                .reserve_committed(
                    MemoryClass::ArtifactCache,
                    owner.into(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    10,
                )
                .expect("seed authority"),
        );
    }
    let mut seed = 0xD3AD_B33F_ABC0_1234_u64;
    let mut accepted = 0;
    let mut denied = 0;
    for step in 0..500 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let index = (seed % 3) as usize;
        let old = &tickets[index];
        let class =
            if seed & 8 == 0 { MemoryClass::ArtifactCache } else { MemoryClass::EvidenceCache };
        let owner = ["old", "new", "plain"][((seed >> 4) % 3) as usize].to_string();
        let context =
            if seed & 128 == 0 { MemoryContextKind::Request } else { MemoryContextKind::Session };
        let kind =
            [ChargeKind::RetainedPayload, ChargeKind::RetainedMetadata, ChargeKind::LiveScratch]
                [((seed >> 8) % 3) as usize];
        let bytes = BYTES[((seed >> 12) % BYTES.len() as u64) as usize];
        let before = broker.ledger_snapshot().expect("preimage");
        let core = broker.inner.lock().expect("core");
        // Derive outer net axes from the independent held-ticket inventory,
        // not from the broker's replacement projection or scheduler clone.
        let remaining: Vec<_> = tickets
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, ticket)| ticket)
            .collect();
        let group_of = |ticket: &ReservationTicket| {
            residency.owner_groups.get(ticket.owner()).map(String::as_str)
        };
        let owner_group = residency.owner_groups.get(&owner).map(String::as_str);
        let group_bytes = |group: &str| {
            remaining
                .iter()
                .filter(|ticket| group_of(ticket) == Some(group))
                .map(|ticket| ticket.bytes())
                .sum::<u64>()
        };
        let verdict = crate::domain::admission::preflight(&PreflightInputs {
            policy: &residency,
            usage: core.ledger.usage_view().with_credit(old),
            non_headroom_bytes: remaining.iter().map(|ticket| ticket.bytes()).sum(),
            live_scratch_bytes: remaining
                .iter()
                .filter(|ticket| ticket.kind() == ChargeKind::LiveScratch)
                .map(|ticket| ticket.bytes())
                .sum(),
            execution_scratch_max: Some(50),
            fair_burst_shares: core.scheduler.fair_burst_shares(),
            group_current_usage: owner_group.map_or(0, group_bytes),
            group_current_non_headroom_usage: owner_group.map_or(0, group_bytes),
            unused_group_floors: residency
                .resource_groups
                .iter()
                .map(|(group, budget)| {
                    u128::from(budget.reserved_bytes.saturating_sub(group_bytes(group)))
                })
                .sum(),
            owner_group,
            request: AdmissionRequest { class: &class, owner: &owner, context, kind, bytes },
            rebuild_in_progress: false,
            ticket_id_space_exhausted: false,
            live_ticket_limit_reached: false,
            ticket_count_delta: 0,
        });
        drop(core);
        let result = broker.recharge(old, class.clone(), owner.clone(), context, kind, bytes);
        match verdict {
            AdmissionVerdict::Admit => {
                tickets[index] = result
                    .unwrap_or_else(|error| panic!("step {step}: Admit disagrees: {error:?}"));
                accepted += 1;
            }
            AdmissionVerdict::ReclaimCandidate { breach }
            | AdmissionVerdict::TerminalDenial { breach } => {
                assert_eq!(
                    result.expect_err("byte breach denied without callback").error,
                    breach.into_error(),
                    "step {step}"
                );
                denied += 1;
                assert_eq!(broker.ledger_snapshot().expect("denied image"), before);
            }
            AdmissionVerdict::Blocked { reason } => {
                assert_eq!(result.expect_err("blocked").error, reason.into_error());
                denied += 1;
                assert_eq!(broker.ledger_snapshot().expect("blocked image"), before);
            }
        }
        let mut classes = BTreeMap::<MemoryClass, UsageBreakdown>::new();
        let mut owners = BTreeMap::<String, UsageBreakdown>::new();
        let mut contexts = BTreeMap::<MemoryContextKind, UsageBreakdown>::new();
        for ticket in &tickets {
            if ticket.bytes() == 0 {
                continue;
            }
            for usage in [
                classes.entry(ticket.class().clone()).or_default(),
                owners.entry(ticket.owner().clone()).or_default(),
                contexts.entry(ticket.context()).or_default(),
            ] {
                match ticket.kind() {
                    ChargeKind::RetainedPayload => usage.retained_payload_bytes += ticket.bytes(),
                    ChargeKind::RetainedMetadata => usage.retained_metadata_bytes += ticket.bytes(),
                    ChargeKind::LiveScratch => usage.live_scratch_bytes += ticket.bytes(),
                }
            }
        }
        let image = broker.observe().expect("consistent postimage");
        image.ledger.validate_exact().expect("balanced axes");
        assert_eq!(image.ledger.class_usage, classes, "step {step}: class attribution");
        assert_eq!(image.ledger.owner_usage, owners, "step {step}: owner attribution");
        assert_eq!(image.ledger.context_usage, contexts, "step {step}: context attribution");
        assert_eq!(image.ledger.live_ticket_count, 3);
        for group in ["g-old", "g-new"] {
            let expected = tickets
                .iter()
                .filter(|ticket| group_of(ticket) == Some(group))
                .map(|ticket| ticket.bytes())
                .sum::<u64>();
            assert_eq!(
                image.resource_group_usage[group], expected,
                "step {step}: group accounting"
            );
        }
    }
    assert!(accepted > 0 && denied > 0, "both poles must execute: {accepted}/{denied}");
    for ticket in tickets {
        broker.release(&ticket).expect("release current authority");
    }
    sep23_assert_fully_drained(&broker);
}

#[test]
fn replacement_at_live_ticket_cap_needs_no_ticket_reclaim() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(100, 100));
    let mut old = charge(&broker, "old", 40);
    let mut zero = Vec::new();
    for _ in 1..crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER {
        zero.push(charge(&broker, "zero", 0));
    }
    let old_id = old.id();
    replace(&mut old, 80).expect("count delta zero, with a fresh ticket identity");
    assert_ne!(old.id(), old_id);
    assert!(reclaimer.requests().is_empty(), "a replacement cannot ask for ticket-cap reclaim");
    let image = broker.ledger_snapshot().expect("full ticket image");
    image.validate_exact().expect("all axes");
    assert_eq!(image.total_owner_bytes(), 80);
    assert_eq!(
        image.live_ticket_count,
        crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
    );
    old.release().expect("replacement release");
    drop(zero);
    sep23_assert_fully_drained(&broker);
}

#[test]
fn foreign_and_pending_old_authority_block_before_any_callback() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let broker = live(&reclaimer, policy(100, 100));
    let pending = broker
        .reserve(
            MemoryClass::ArtifactCache,
            "old".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            40,
        )
        .expect("pending authority");
    let victim = charge(&broker, "victim", 60);
    let foreign_broker = LiveProcessMemoryBroker::from_report(report_with(policy(100, 100)))
        .expect("foreign issuer");
    let foreign = foreign_broker
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "old".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            40,
        )
        .expect("foreign committed authority");
    let before = broker.ledger_snapshot().expect("preimage");
    for old in [&pending, &foreign] {
        let result = broker.recharge(
            old,
            MemoryClass::ArtifactCache,
            "old".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            80,
        );
        assert!(matches!(result, Err(RechargeError { error: MemoryError::InvalidState(_), .. })));
        assert_eq!(broker.ledger_snapshot().expect("unchanged"), before);
        assert!(reclaimer.requests().is_empty());
    }
    broker.commit(&pending).expect("pending identity still usable");
    broker.release(&pending).expect("release original");
    foreign_broker.release(&foreign).expect("foreign issuer still releases it");
    drop(victim);
    sep23_assert_fully_drained(&broker);
    sep23_assert_fully_drained(&foreign_broker);
}

#[test]
fn soft_target_order_and_pressure_use_the_net_replacement_postimage() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let residency = ResidencyPolicy::new(100)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 50, 100, false))
        .with_owner_quota("old", OwnerQuota { soft_cap_bytes: 55, hard_cap_bytes: 100 })
        .with_resource_group(
            "g",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 58,
                hard_cap_bytes: 100,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("old", "g");
    let broker = live(&reclaimer, residency);
    let mut old = charge(&broker, "old", 40);
    replace(&mut old, 60).expect("net replacement remains admissible");
    let requests = reclaimer.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].trigger, ReclaimTrigger::SoftThreshold);
    assert_eq!(
        requests[0].target.scope,
        ReclaimScope::Class(MemoryClass::ArtifactCache),
        "class crossing wins over simultaneous owner/group crossings"
    );
    assert_eq!(requests[0].target.minimum_reduction, Some(10));
    assert_eq!(
        requests[0].pressure,
        PressureState::Soft,
        "net 60% has the soft-trigger floor; transient 100% would be Fatal"
    );
    assert!(requests[0].candidates.iter().all(|victim| victim.class == MemoryClass::ArtifactCache));
    let image = broker.observe().expect("postimage");
    assert_eq!(image.semantic_bytes, 60);
    assert_eq!(image.resource_group_usage["g"], 60);
    old.release().expect("drain");
    sep23_assert_fully_drained(&broker);
}

#[test]
fn first_shared_pool_borrower_remains_an_eligible_replacement_victim_class() {
    let reclaimer = Arc::new(ReplacementReclaimer::default());
    let residency = ResidencyPolicy::new(1_000)
        .with_shared_burst_pool_bytes(20)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 30, 30, true));
    let broker = live(&reclaimer, residency);
    let mut old = charge(&broker, "old", 20);
    let mut victim = charge(&broker, "victim", 10);
    reclaimer.arm(move || {
        victim.release().expect("actual matching-class reduction");
        Ok(ReclaimOutcome::default())
    });
    replace(&mut old, 50).expect("released 10 leaves exactly 20 shared-pool borrowing");
    let requests = reclaimer.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].trigger, ReclaimTrigger::CapacityDenial);
    assert_eq!(requests[0].target.scope, ReclaimScope::SharedBorrowPool);
    assert_eq!(requests[0].target.minimum_reduction, Some(10));
    assert!(
        requests[0].candidates.iter().any(|victim| victim.class == MemoryClass::ArtifactCache),
        "requesting class was at its hard cap before replacement, so it is a first borrower"
    );
    let image = broker.ledger_snapshot().expect("postimage");
    image.validate_exact().expect("all axes");
    assert_eq!(image.total_owner_bytes(), 50);
    assert_eq!(image.live_ticket_count, 1);
    old.release().expect("replacement release");
    sep23_assert_fully_drained(&broker);
}
