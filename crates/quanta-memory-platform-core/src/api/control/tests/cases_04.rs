use super::*;

#[test]
fn broker_clones_share_one_ledger() {
    let b1 = broker();
    let b2 = b1.clone();
    let t = b1
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("reserve via clone 1");
    b1.commit(&t).expect("commit");
    assert_eq!(
        b2.ledger_snapshot().expect("snap").total_owner_bytes(),
        100_000,
        "a clone observes the other clone's charge (shared ledger)"
    );
    b2.release(&t).expect("release via clone 2");
    assert_eq!(
        b1.ledger_snapshot().expect("snap").total_owner_bytes(),
        0,
        "release via one clone is seen by the other"
    );
}
#[test]
fn broker_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LiveProcessMemoryBroker>();
}
#[test]
fn accepted_reconfigure_preserves_scheduler_group_usage() {
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
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 150_000, 300_000, false))
        .with_resource_group("g", group.clone())
        .with_owner_group("owner:g", "g");
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            90_000,
        )
        .expect("within group cap");
    b.commit(&t).expect("commit");

    // Legal raise that KEEPS the class, the group, and the owner mapping.
    let raised = ResidencyPolicy::new(20_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 400_000, false))
        .with_resource_group("g", group)
        .with_owner_group("owner:g", "g");
    b.try_live_reconfigure(raised).expect("raising caps is legal");

    // The group still holds 90k of live usage post-reconfigure, so a further
    // 20k (-> 110k > 100k cap) must still be denied. If the scheduler had been
    // reset to zero usage, this would wrongly be admitted.
    let denied = b.reserve(
        MemoryClass::ArtifactCache,
        "owner:g".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        20_000,
    );
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "scheduler group usage must survive an accepted reconfigure, got {denied:?}"
    );
}
#[test]
fn remapping_live_owner_group_is_denied_fail_closed() {
    use crate::domain::policy::ResourceGroup;

    let g1 = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 50_000,
        hard_cap_bytes: 100_000,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let g2 = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 50_000,
        hard_cap_bytes: 100_000,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let residency = ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 150_000, 300_000, false))
        .with_resource_group("g1", g1.clone())
        .with_resource_group("g2", g2.clone())
        .with_owner_group("owner:g", "g1");
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
    let ticket = b
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            90_000,
        )
        .expect("baseline charge");

    let remapped = ResidencyPolicy::new(20_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 400_000, false))
        .with_resource_group("g1", g1)
        .with_resource_group("g2", g2)
        .with_owner_group("owner:g", "g2");
    let denied = b.try_live_reconfigure(remapped);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "live owner-group remap must fail closed while bytes are still live, got {denied:?}"
    );

    let still_denied = b.reserve(
        MemoryClass::ArtifactCache,
        "owner:g".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        20_000,
    );
    assert!(
        matches!(still_denied, Err(MemoryError::Capacity(_))),
        "failed remap must leave original scheduler truth intact, got {still_denied:?}"
    );

    b.release(&ticket).expect("release baseline");
    let remapped_after_drain = ResidencyPolicy::new(20_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 400_000, false))
        .with_resource_group(
            "g1",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 50_000,
                hard_cap_bytes: 100_000,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_resource_group(
            "g2",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 50_000,
                hard_cap_bytes: 100_000,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("owner:g", "g2");
    b.try_live_reconfigure(remapped_after_drain).expect("remap is legal after live bytes drain");
}
#[test]
fn reserve_committed_during_rebuild_is_fail_closed_no_strand() {
    let b = broker();
    let _fence14 = b.begin_rebuild().expect("begin rebuild");
    let before = b.ledger_snapshot().expect("snap").total_owner_bytes();
    let denied = b.reserve_committed(
        MemoryClass::ArtifactCache,
        "owner:a".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        100_000,
    );
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "reserve_committed during rebuild must be rejected, got {denied:?}"
    );
    assert_eq!(
        b.ledger_snapshot().expect("snap").total_owner_bytes(),
        before,
        "a rejected atomic charge must strand nothing (ledger unchanged)"
    );
}
#[test]
fn reserve_committed_charges_exactly_once() {
    let b = broker();
    let t = b
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            120_000,
        )
        .expect("atomic charge within caps");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 120_000);
    // The returned ticket is a normal committed charge: releasing it nets zero.
    b.release(&t).expect("release");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
}
#[test]
fn recharge_swaps_committed_charge_atomically() {
    let b = broker();
    let old = b
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("charge old");
    let new = b
        .recharge(
            &old,
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            50_000,
        )
        .expect("recharge to smaller value");
    // Exactly the new charge remains; the old is consumed (released).
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 50_000);
    assert!(
        matches!(b.release(&old), Err(MemoryError::InvalidState(_))),
        "the old ticket is consumed by a successful recharge"
    );
    b.release(&new).expect("release new");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
}
#[test]
fn recharge_denied_preserves_original_old_ticket() {
    let b = broker(); // owner:a hard cap 500k
    let old = b
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            400_000,
        )
        .expect("charge old");
    // Recharge to 600k > owner cap 500k -> denied with no old-state mutation.
    let err = b
        .recharge(
            &old,
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            600_000,
        )
        .expect_err("over owner cap must be denied");
    assert!(matches!(err.error, MemoryError::Capacity(_)));
    assert_eq!(err.recovery, RechargeRecoveryV1::OldChargePreserved);
    // The old bytes are still charged (no loss) AND every accounting axis is
    // balanced — a wrong-axis restore (e.g. restoring a different ChargeKind)
    // would pass the owner-total check but break the partition/class balance.
    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(snap.total_owner_bytes(), 400_000);
    assert!(snap.class_owner_balanced(), "class==owner after restore");
    assert!(snap.owner_partition_balanced(), "partition intact after restore");
    assert_eq!(snap.total_retained_payload_bytes(), 400_000, "restored to the SAME charge kind");
    b.release(&old).expect("preserved ticket releases the old bytes");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
}
#[test]
fn denied_recharge_preserves_the_old_axis_not_the_new_one() {
    let b = broker(); // owner:a hard cap 500k
    let old = b
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload, // OLD kind = payload
            400_000,
        )
        .expect("charge old as payload");

    // Recharge to RetainedMetadata 600k > owner cap 500k -> denied; restore must
    // land back on RetainedPayload (the OLD kind), leaving metadata at zero.
    let err = b
        .recharge(
            &old,
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata, // NEW kind differs
            600_000,
        )
        .expect_err("over owner cap denied");
    assert!(matches!(err.error, MemoryError::Capacity(_)));
    assert_eq!(err.recovery, RechargeRecoveryV1::OldChargePreserved);
    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(
        snap.total_retained_payload_bytes(),
        400_000,
        "restore landed on the OLD payload axis"
    );
    assert_eq!(
        snap.total_retained_metadata_bytes(),
        0,
        "restore must NOT move bytes to the new (metadata) axis"
    );
    assert!(snap.owner_partition_balanced(), "partition balanced after wrong-axis-safe restore");
    b.release(&old).expect("release preserved ticket");
}
#[test]
fn recharge_during_rebuild_leaves_old_intact() {
    let b = broker();
    let old = b
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("charge old");
    let fence15 = b.begin_rebuild().expect("begin rebuild");
    let err = b
        .recharge(
            &old,
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            50_000,
        )
        .expect_err("recharge during rebuild is rejected");
    assert!(matches!(err.error, MemoryError::InvalidState(_)));
    assert_eq!(err.recovery, RechargeRecoveryV1::OldChargePreserved);
    assert_eq!(
        b.ledger_snapshot().expect("snap").total_owner_bytes(),
        100_000,
        "rebuild recharge strands nothing and leaves the old charge intact"
    );
    // After rebuild the ORIGINAL ticket is still valid.
    assert!(fence15.finish().expect("finish").drain_error.is_none());
    b.release(&old).expect("original ticket still valid after rebuild");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
}
#[test]
fn recharge_does_not_transiently_double_count_group_admission() {
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
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 150_000, 300_000, false))
        .with_resource_group("g", group)
        .with_owner_group("owner:g", "g");
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");

    let old = b
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            90_000,
        )
        .expect("group usage 90k");

    // Overwrite to exactly the group cap (100k). A naive charge-new-then-refund
    // would transiently see 90k+100k=190k > 100k group cap and be DENIED; the
    // atomic recharge (release-then-charge under one lock) admits it.
    let new = b
        .recharge(
            &old,
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("atomic recharge avoids transient double-count against the group");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 100_000);
    // The group is now exactly at cap: a further 1-byte reserve is denied.
    assert!(matches!(
        b.reserve(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        ),
        Err(MemoryError::Capacity(_))
    ));
    b.release(&new).expect("release");
}
#[test]
fn denied_recharge_on_grouped_owner_preserves_group_usage() {
    use crate::domain::policy::ResourceGroup;
    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 50_000,
        hard_cap_bytes: 100_000,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    // owner:g mapped to group g (cap 100k); class cap 80k so the NEW charge is
    // denied by the class while the group has room.
    let residency = ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 60_000, 80_000, false))
        .with_resource_group("g", group)
        .with_owner_group("owner:g", "g");
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");

    let old = b
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            70_000,
        )
        .expect("seed 70k (group usage 70k)");

    // Recharge to 90k > class cap 80k -> denied; old 70k remains live.
    let err = b
        .recharge(
            &old,
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            90_000,
        )
        .expect_err("over class cap denied");
    assert!(matches!(err.error, MemoryError::Capacity(_)));
    assert_eq!(err.recovery, RechargeRecoveryV1::OldChargePreserved);
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 70_000);

    // Group usage is back at exactly 70k: it admits 30k more (-> 100k cap) but
    // not 31k. Symmetric — no phantom group drift from the release/restore.
    assert!(
        b.admits_group("g", 30_000).expect("admits"),
        "group should be at 70k, admitting 30k up to the 100k cap"
    );
    assert!(
        !b.admits_group("g", 31_000).expect("admits"),
        "group must be exactly at 70k usage (no phantom drift), so 31k overflows"
    );
    b.release(&old).expect("release preserved ticket");
}
#[test]
fn recharge_cannot_consume_peer_group_reserved_floor() {
    use crate::domain::policy::ResourceGroup;
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
    let old = broker
        .reserve_committed(
            MemoryClass::ArtifactCache,
            "peer-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            500,
        )
        .expect("seed peer usage inside unprotected band");

    let denied = broker
        .recharge(
            &old,
            MemoryClass::ArtifactCache,
            "peer-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            601,
        )
        .expect_err("recharge must preserve the other group's 400-byte floor");
    assert!(matches!(denied.error, MemoryError::Capacity(_)));
    assert_eq!(denied.recovery, RechargeRecoveryV1::OldChargePreserved);
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 500);

    let replacement = broker
        .recharge(
            &old,
            MemoryClass::ArtifactCache,
            "peer-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            600,
        )
        .expect("replacement exactly at the unprotected boundary is legal");
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 600);
    broker.release(&replacement).expect("release replacement");
}
#[test]
fn managed_charge_drop_recovers_poisoned_lock_for_exact_release() {
    let b = broker();
    let charge = managed_charge(&b, 100_000);
    poison_broker_lock(&b);

    drop(charge);

    let core = b.inner.lock().unwrap_or_else(PoisonError::into_inner);
    let snapshot = core.ledger.snapshot();
    assert_eq!(snapshot.total_owner_bytes(), 0);
    assert!(snapshot.class_owner_balanced() && snapshot.owner_partition_balanced());
    assert!(core.deferred_committed_releases.is_empty());
}
#[test]
fn poisoned_runtime_scratch_explicit_release_preserves_then_drop_recovers_authority() {
    let b = runtime_scratch_broker(100);
    let runtime =
        b.attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime).expect("attach runtime");
    let context = runtime.context(MemoryContextKind::Request, "owner:scratch".to_string());
    let mut reservation = runtime
        .scratch()
        .reserve_scratch_scope(&context, ScratchRequest::new("poison-cleanup", 100))
        .expect("reserve scratch");
    poison_broker_lock(&b);

    let denied = reservation.release();
    assert!(matches!(denied, Err(MemoryError::Poisoned(_))));
    assert!(reservation.is_enforced(), "failed explicit release retains retry authority");

    drop(reservation);

    let core = b.inner.lock().unwrap_or_else(PoisonError::into_inner);
    let snapshot = core.ledger.snapshot();
    assert_eq!(snapshot.total_owner_bytes(), 0);
    assert!(snapshot.class_owner_balanced() && snapshot.owner_partition_balanced());
}
#[test]
fn poisoned_pending_commit_retains_ticket_until_drop_rollback() {
    let b = broker();
    let pending = b
        .reserve_owned(
            MemoryClass::ArtifactCache,
            "owner:pending-poison".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("pending reservation");
    poison_broker_lock(&b);

    let denied = pending.commit().expect_err("poisoned commit must fail closed");
    assert!(matches!(denied, MemoryError::Poisoned(_)));

    // `commit` consumes `pending`; its Drop path must already have recovered
    // the poisoned lock and rolled the exact pending ticket back.
    let core = b.inner.lock().unwrap_or_else(PoisonError::into_inner);
    let snapshot = core.ledger.snapshot();
    assert_eq!(snapshot.total_owner_bytes(), 0);
    assert_eq!(snapshot.total_context_bytes(), 0);
    assert!(snapshot.class_owner_balanced());
    assert!(snapshot.owner_partition_balanced());
    assert!(snapshot.context_owner_balanced());
    assert!(core.ticket_attribution.is_empty());
    assert!(core.deferred_pending_rollbacks.is_empty());
}
#[test]
fn poisoned_broker_fails_closed_and_trait_report_stays_truthful() {
    use crate::api::broker::ProcessMemoryBroker;

    let b = broker();
    let ticket = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            10_000,
        )
        .expect("reserve before poison");
    let report_before = <LiveProcessMemoryBroker as ProcessMemoryBroker>::effective_report(&b);
    // Acquire the fence BEFORE poisoning so its finish exercises the
    // poisoned-lock path (a fence cannot be minted after the poison).
    let pre_fence = b.begin_rebuild().expect("fence acquired before poison");
    poison_broker_lock(&b);

    assert!(matches!(
        b.reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        ),
        Err(MemoryError::Poisoned(_))
    ));
    assert!(matches!(b.commit(&ticket), Err(MemoryError::Poisoned(_))));
    assert!(matches!(b.release(&ticket), Err(MemoryError::Poisoned(_))));
    assert!(matches!(b.begin_rebuild(), Err(MemoryError::Poisoned(_))));
    assert!(
        matches!(pre_fence.finish(), Err(MemoryError::Poisoned(_))),
        "finishing a fence over a poisoned broker lock fails closed"
    );
    assert!(matches!(
        b.try_live_reconfigure(ResidencyPolicy::new(2_000_000)),
        Err(MemoryError::Poisoned(_))
    ));
    assert!(matches!(b.ledger_snapshot(), Err(MemoryError::Poisoned(_))));
    assert!(matches!(
        b.observe_allocator(&AllocatorSnapshot {
            resident_bytes: Some(123),
            ..AllocatorSnapshot::default()
        }),
        Err(MemoryError::Poisoned(_))
    ));
    assert!(matches!(b.effective_report(), Err(MemoryError::Poisoned(_))));
    assert!(matches!(b.observe(), Err(MemoryError::Poisoned(_))));

    let report_after = <LiveProcessMemoryBroker as ProcessMemoryBroker>::effective_report(&b);
    assert_eq!(report_after, report_before, "trait report must stay truthful under poison");
    assert_eq!(report_after.residency.global_hard_limit_bytes, 1_000_000);
    assert!(
        !report_after.override_sources.iter().any(|s| s == "poisoned-fallback"),
        "trait report must not fabricate a poison fallback marker"
    );
}
#[test]
fn bounded_events_execute_outside_lock_and_cover_major_broker_outcomes() {
    let sink = Arc::new(RecordingEventSink::default());
    let broker = crate::ProcessMemoryBrokerBuilder::new()
        .with_preset(crate::MemoryPreset::Development)
        .with_event_sink(sink.clone())
        .build()
        .expect("broker");
    *sink.broker.lock().expect("install broker") = Some(broker.clone());

    let mut charge = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "bounded-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            64,
        )
        .expect("reserve");
    charge.release().expect("release");
    let denied = broker.reserve_managed_charge(
        MemoryClass::ArtifactCache,
        "bounded-owner".into(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        u64::MAX,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    let observation = broker.observe().expect("observation");
    assert_eq!(observation.semantic_bytes, 0);
    assert_eq!(observation.report.policy_revision, 1);

    let events = sink.events.lock().expect("events").clone();
    assert!(events.contains(&MemoryEvent::new(
        MemoryOperation::Reserve,
        MemoryOutcome::Succeeded,
        Some(64),
    )));
    assert!(events.contains(&MemoryEvent::new(
        MemoryOperation::Release,
        MemoryOutcome::Succeeded,
        Some(64),
    )));
    assert!(events.contains(&MemoryEvent::new(
        MemoryOperation::Reserve,
        MemoryOutcome::CapacityDenied,
        Some(u64::MAX),
    )));
    assert!(events.contains(&MemoryEvent::new(
        MemoryOperation::ObserveBroker,
        MemoryOutcome::Succeeded,
        Some(0),
    )));
}
#[test]
fn telemetry_panic_cannot_change_or_abort_accounting() {
    let broker = crate::ProcessMemoryBrokerBuilder::new()
        .with_preset(crate::MemoryPreset::Development)
        .with_event_sink(Arc::new(PanickingEventSink))
        .build()
        .expect("broker");
    let charge = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "panic-sink-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            32,
        )
        .expect("telemetry panic is contained after accounting commit");
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 32);
    drop(charge);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
