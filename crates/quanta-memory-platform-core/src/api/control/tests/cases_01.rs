use super::*;

#[test]
fn overlong_owner_is_denied_before_reclaim_or_accounting() {
    let b = broker();
    let before = b.ledger_snapshot().expect("before snapshot");
    let owner = "o".repeat(crate::domain::management_limits::MAX_OWNER_KEY_BYTES + 1);
    let result = b.reserve_owned(
        MemoryClass::ArtifactCache,
        owner,
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        0,
    );
    assert!(matches!(result, Err(MemoryError::Capacity(_))));
    assert_eq!(b.ledger_snapshot().expect("after snapshot"), before);
    assert_eq!(b.reclaim_runtime_stats().expect("reclaim stats").attempts, 0);
}
#[test]
fn scratch_reserve_cap_overflow_is_denied_without_mutation() {
    let mut report = report_with(ResidencyPolicy::new(u64::MAX));
    report.execution.max_execution_scratch_bytes = Some(usize::MAX);
    let b = LiveProcessMemoryBroker::from_report(report).expect("validated policy");
    let first = b
        .reserve(
            MemoryClass::ArtifactCache,
            "o".into(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            u64::MAX - 10,
        )
        .expect("first reserve");
    b.commit(&first).expect("commit first");
    let before = b.ledger_snapshot().expect("before");
    let second = b.reserve(
        MemoryClass::EvidenceCache,
        "o".into(),
        MemoryContextKind::Request,
        ChargeKind::LiveScratch,
        100,
    );
    assert!(
        matches!(second, Err(MemoryError::Capacity(_))),
        "scratch cap overflow must deny: {second:?}"
    );
    assert_eq!(b.ledger_snapshot().expect("after denial"), before);
    b.release(&first).expect("first ticket remains live");
    assert_eq!(b.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn scratch_recharge_cap_overflow_preserves_original_ticket() {
    let mut report = report_with(ResidencyPolicy::new(u64::MAX));
    report.execution.max_execution_scratch_bytes = Some(usize::MAX);
    let b = LiveProcessMemoryBroker::from_report(report).expect("validated policy");
    let first = b
        .reserve(
            MemoryClass::ArtifactCache,
            "o".into(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            u64::MAX - 11,
        )
        .expect("first reserve");
    b.commit(&first).expect("commit first");
    let old = b
        .reserve(
            MemoryClass::EvidenceCache,
            "o".into(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            1,
        )
        .expect("old reserve");
    b.commit(&old).expect("commit old");
    let before = b.ledger_snapshot().expect("before");
    let replacement = b.recharge(
        &old,
        MemoryClass::EvidenceCache,
        "o".into(),
        MemoryContextKind::Request,
        ChargeKind::LiveScratch,
        100,
    );
    assert!(
        matches!(replacement, Err(RechargeError { error: MemoryError::Capacity(_), .. })),
        "scratch recharge overflow must deny: {replacement:?}"
    );
    assert_eq!(b.ledger_snapshot().expect("after denial"), before);
    b.release(&old).expect("old ticket remains live");
    b.release(&first).expect("first ticket remains live");
    assert_eq!(b.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn dropping_pending_reservation_rolls_back_all_axes() {
    let b = broker();
    let before = b.ledger_snapshot().expect("before");
    {
        let pending = b
            .reserve_owned(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("reserve pending");
        assert_eq!(pending.bytes(), 100_000);
        assert_eq!(b.ledger_snapshot().expect("reserved").total_owner_bytes(), 100_000);
    }

    assert_eq!(b.ledger_snapshot().expect("after drop"), before);
}
#[test]
fn dropping_pending_reservation_during_rebuild_does_not_strand_capacity() {
    let b = broker();
    let pending = b
        .reserve_owned(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("reserve pending");
    let fence1 = b.begin_rebuild().expect("begin rebuild");

    drop(pending);

    let snapshot = b.ledger_snapshot().expect("snapshot");
    assert!(snapshot.rebuild_in_progress);
    assert_eq!(snapshot.total_owner_bytes(), 0);
    assert!(fence1.finish().expect("finish rebuild").drain_error.is_none());
}
#[test]
fn failed_pending_commit_during_rebuild_rolls_back_capacity() {
    let b = broker();
    let pending = b
        .reserve_owned(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("reserve pending");
    let fence2 = b.begin_rebuild().expect("begin rebuild");

    assert!(matches!(pending.commit(), Err(MemoryError::InvalidState(_))));
    assert_eq!(b.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
    assert!(fence2.finish().expect("finish rebuild").drain_error.is_none());
}
#[test]
fn pending_drop_restores_resource_group_capacity() {
    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 100,
        hard_cap_bytes: 100,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    let residency = ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 10_000, 10_000, false))
        .with_resource_group("g", group)
        .with_owner_group("owner:g", "g");
    let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("broker");
    let pending = b
        .reserve_owned(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100,
        )
        .expect("fill group capacity");
    assert!(!b.admits_group("g", 1).expect("probe full group"));

    drop(pending);

    assert!(b.admits_group("g", 100).expect("capacity restored"));
    assert_eq!(b.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
}
#[test]
fn deferred_committed_cleanup_is_batched_and_drains_large_backlog() {
    let b = broker();
    let count = crate::domain::management_limits::MAX_DEFERRED_TICKET_DRAIN_BATCH * 2 + 17;
    let charges: Vec<_> = (0..count).map(|_| managed_charge(&b, 0)).collect();
    let fence3 = b.begin_rebuild().expect("begin rebuild");
    drop(charges);
    assert_eq!(b.inner.lock().expect("core").deferred_committed_releases.len(), count);

    // Exercise the critical-section unit directly: the first pass may
    // consume only one configured batch, leaving the exact remainder queued.
    {
        let mut core = b.inner.lock().expect("core");
        core.ledger.finish_rebuild();
        assert_eq!(
            LiveProcessMemoryBroker::drain_deferred_committed_releases_batch(&mut core)
                .expect("first bounded batch"),
            crate::domain::management_limits::MAX_DEFERRED_TICKET_DRAIN_BATCH
        );
        assert_eq!(
            core.deferred_committed_releases.len(),
            count - crate::domain::management_limits::MAX_DEFERRED_TICKET_DRAIN_BATCH
        );
        // Restore the closed fence so the owning permit can finish it.
        core.ledger.begin_rebuild();
    }
    let receipt = fence3.finish().expect("finish remaining batches");
    assert!(receipt.fence_open);
    assert!(receipt.drain_error.is_none());
    let snapshot = b.ledger_snapshot().expect("snapshot");
    assert_eq!(snapshot.live_ticket_count, 0);
    assert_eq!(snapshot.total_owner_bytes(), 0);
    assert!(b.inner.lock().expect("core").deferred_committed_releases.is_empty());
}
#[test]
fn managed_charge_drop_on_healthy_broker_releases_exact_capacity() {
    let b = broker();
    let charge = managed_charge(&b, 100_000);
    assert_eq!(charge.bytes(), Some(100_000));
    assert_eq!(charge.owner().map(String::as_str), Some("owner:a"));
    assert_eq!(charge.context(), Some(MemoryContextKind::Request));
    assert_eq!(charge.kind(), Some(ChargeKind::RetainedPayload));
    assert_eq!(b.ledger_snapshot().expect("charged").total_owner_bytes(), 100_000);

    drop(charge);

    assert_eq!(b.ledger_snapshot().expect("released").total_owner_bytes(), 0);
}
#[test]
fn managed_charge_drop_during_rebuild_defers_until_finish() {
    let b = broker();
    let charge = managed_charge(&b, 100_000);
    let fence4 = b.begin_rebuild().expect("begin rebuild");

    drop(charge);

    let fenced = b.ledger_snapshot().expect("fenced snapshot");
    assert!(fenced.rebuild_in_progress);
    assert_eq!(fenced.total_owner_bytes(), 100_000, "deferred release stays conservative");

    assert!(fence4.finish().expect("finish drains deferred charge").drain_error.is_none());
    let drained = b.ledger_snapshot().expect("drained snapshot");
    assert!(!drained.rebuild_in_progress);
    assert_eq!(drained.total_owner_bytes(), 0);
}
#[test]
fn managed_charge_explicit_release_failure_retains_authority() {
    let b = broker();
    let mut charge = managed_charge(&b, 100_000);
    let id = charge.id();
    let fence5 = b.begin_rebuild().expect("begin rebuild");

    let error = charge.release().expect_err("release is fenced during rebuild");
    assert!(matches!(error, MemoryError::InvalidState(_)));
    assert!(charge.is_active());
    assert_eq!(charge.id(), id);
    assert_eq!(b.ledger_snapshot().expect("still charged").total_owner_bytes(), 100_000);

    assert!(fence5.finish().expect("finish rebuild").drain_error.is_none());
    charge.release().expect("retry exact release");
    assert!(!charge.is_active());
    assert_eq!(b.ledger_snapshot().expect("released").total_owner_bytes(), 0);
}
#[test]
fn managed_charge_batch_failure_retains_every_lease() {
    let b = broker();
    let mut charges = [managed_charge(&b, 100_000), managed_charge(&b, 120_000)];
    let ids = [charges[0].id(), charges[1].id()];
    let fence6 = b.begin_rebuild().expect("begin rebuild");

    b.release_managed_many(&mut charges).expect_err("batch release is fenced");
    assert!(charges.iter().all(ManagedCharge::is_active));
    assert_eq!([charges[0].id(), charges[1].id()], ids);
    assert_eq!(b.ledger_snapshot().expect("unchanged").total_owner_bytes(), 220_000);

    assert!(fence6.finish().expect("finish rebuild").drain_error.is_none());
    b.release_managed_many(&mut charges).expect("atomic retry");
    assert!(charges.iter().all(|charge| !charge.is_active()));
    assert_eq!(b.ledger_snapshot().expect("released").total_owner_bytes(), 0);
}
#[test]
fn managed_charge_batch_rejects_foreign_broker_before_mutation() {
    let a = broker();
    let b = broker();
    let mut charges = [managed_charge(&a, 100_000), managed_charge(&b, 120_000)];

    let error = a
        .release_managed_many(&mut charges)
        .expect_err("foreign broker lease must reject the complete batch");
    assert!(matches!(error, MemoryError::InvalidState(_)));
    assert!(charges.iter().all(ManagedCharge::is_active));
    assert_eq!(a.ledger_snapshot().expect("a unchanged").total_owner_bytes(), 100_000);
    assert_eq!(b.ledger_snapshot().expect("b unchanged").total_owner_bytes(), 120_000);

    drop(charges);
    assert_eq!(a.ledger_snapshot().expect("a drop cleanup").total_owner_bytes(), 0);
    assert_eq!(b.ledger_snapshot().expect("b drop cleanup").total_owner_bytes(), 0);
}
#[test]
fn managed_batch_invalid_lease_rejects_before_ticket_collection() {
    use crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER;

    fn metadata_charge(b: &LiveProcessMemoryBroker, bytes: u64) -> ManagedCharge {
        b.reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata,
            bytes,
        )
        .expect("reserve metadata charge")
    }

    // An inactive lease at the FIRST position rejects before any ticket
    // collection; the live tail keeps exact ids, flags, and authority.
    {
        let b = broker();
        let mut batch = [managed_charge(&b, 10), managed_charge(&b, 20), managed_charge(&b, 30)];
        batch[0].release().expect("retire the head lease");
        let before = b.ledger_snapshot().expect("before");
        assert_eq!(before.total_owner_bytes(), 50);
        assert_eq!(before.live_ticket_count, 2);
        let ids = [batch[1].id(), batch[2].id()];

        assert!(matches!(b.release_managed_many(&mut batch), Err(MemoryError::InvalidState(_))));
        assert_eq!(
            [batch[0].is_active(), batch[1].is_active(), batch[2].is_active()],
            [false, true, true]
        );
        assert_eq!([batch[1].id(), batch[2].id()], ids);
        let after = b.ledger_snapshot().expect("after");
        assert_eq!(after, before);

        // A valid retry over the active tail drains the broker exactly.
        b.release_managed_many(&mut batch[1..]).expect("valid tail retry");
        assert!(batch[1..].iter().all(|charge| !charge.is_active()));
        let drained = b.ledger_snapshot().expect("drained");
        assert_eq!(drained.total_owner_bytes(), 0);
        assert_eq!(drained.live_ticket_count, 0);
    }

    // An inactive lease at the LAST position rejects identically.
    {
        let b = broker();
        let mut batch = [managed_charge(&b, 10), managed_charge(&b, 20), managed_charge(&b, 30)];
        batch[2].release().expect("retire the tail lease");
        let before = b.ledger_snapshot().expect("before");
        assert_eq!(before.total_owner_bytes(), 30);
        assert_eq!(before.live_ticket_count, 2);
        let ids = [batch[0].id(), batch[1].id()];

        assert!(matches!(b.release_managed_many(&mut batch), Err(MemoryError::InvalidState(_))));
        assert_eq!(
            [batch[0].is_active(), batch[1].is_active(), batch[2].is_active()],
            [true, true, false]
        );
        assert_eq!([batch[0].id(), batch[1].id()], ids);
        assert_eq!(b.ledger_snapshot().expect("unchanged"), before);
    }

    // One foreign broker lease inside an otherwise valid batch.
    {
        let a = broker();
        let b = broker();
        let mut batch = [managed_charge(&b, 10), managed_charge(&a, 20), managed_charge(&b, 30)];
        let b_before = b.ledger_snapshot().expect("b before");
        let a_before = a.ledger_snapshot().expect("a before");
        assert!(matches!(b.release_managed_many(&mut batch), Err(MemoryError::InvalidState(_))));
        assert!(batch.iter().all(ManagedCharge::is_active));
        assert_eq!(b.ledger_snapshot().expect("b unchanged"), b_before);
        assert_eq!(a.ledger_snapshot().expect("a unchanged"), a_before);
    }

    // A valid mixed zero/nonzero batch with mixed kinds drains atomically
    // on every axis.
    {
        let b = broker();
        let mut batch = [
            managed_charge(&b, 0),
            metadata_charge(&b, 0),
            managed_charge(&b, 40),
            metadata_charge(&b, 60),
        ];
        let before = b.ledger_snapshot().expect("before");
        assert_eq!(
            (
                before.total_retained_payload_bytes(),
                before.total_retained_metadata_bytes(),
                before.total_live_scratch_bytes(),
                before.total_owner_bytes(),
                before.live_ticket_count
            ),
            (40, 60, 0, 100, 4)
        );
        let usage = before.owner_usage.values().next().expect("single owner");
        assert_eq!((usage.retained_payload_bytes, usage.retained_metadata_bytes), (40, 60));

        b.release_managed_many(&mut batch).expect("mixed valid batch");
        assert!(batch.iter().all(|charge| !charge.is_active()));
        let drained = b.ledger_snapshot().expect("drained");
        assert_eq!(
            (
                drained.total_retained_payload_bytes(),
                drained.total_retained_metadata_bytes(),
                drained.total_live_scratch_bytes(),
                drained.total_owner_bytes(),
                drained.live_ticket_count
            ),
            (0, 0, 0, 0, 0)
        );
    }

    // A rebuild-fenced valid batch keeps every lease for an exact retry.
    {
        let b = broker();
        let mut batch = [managed_charge(&b, 40), metadata_charge(&b, 60)];
        let ids = [batch[0].id(), batch[1].id()];
        let fence7 = b.begin_rebuild().expect("begin rebuild");
        assert!(matches!(b.release_managed_many(&mut batch), Err(MemoryError::InvalidState(_))));
        assert!(batch.iter().all(ManagedCharge::is_active));
        assert_eq!([batch[0].id(), batch[1].id()], ids);
        let fenced = b.ledger_snapshot().expect("fenced");
        assert_eq!(fenced.total_owner_bytes(), 100);
        assert_eq!(fenced.live_ticket_count, 2);

        assert!(fence7.finish().expect("finish rebuild").drain_error.is_none());
        b.release_managed_many(&mut batch).expect("retry after fence");
        let drained = b.ledger_snapshot().expect("drained");
        assert_eq!(drained.total_owner_bytes(), 0);
        assert_eq!(drained.live_ticket_count, 0);
    }

    // An impossibly large slice (above the live-ticket cap) is rejected by
    // length alone, before any per-lease work or ticket vector.
    {
        let b = broker();
        let mut oversized: Vec<ManagedCharge> = (0..=MAX_LIVE_TICKETS_PER_BROKER)
            .map(|_| {
                let mut charge = managed_charge(&b, 0);
                charge.release().expect("retire immediately");
                charge
            })
            .collect();
        let before = b.ledger_snapshot().expect("before");
        assert_eq!(before.live_ticket_count, 0);
        assert!(matches!(
            b.release_managed_many(&mut oversized),
            Err(MemoryError::InvalidState(_))
        ));
        assert_eq!(b.ledger_snapshot().expect("unchanged"), before);
    }
}
#[test]
fn managed_charge_recharge_denial_restores_old_authority() {
    let b = broker();
    let mut charge = managed_charge(&b, 100_000);
    let original_id = charge.id();

    let denied = charge
        .recharge(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            700_000,
        )
        .expect_err("replacement over class cap is denied");
    assert_eq!(denied.recovery, ManagedChargeRechargeRecovery::ChargePreserved);
    assert_eq!(charge.id(), original_id);
    assert_eq!(charge.bytes(), Some(100_000));
    assert_eq!(b.ledger_snapshot().expect("old preserved").total_owner_bytes(), 100_000);

    charge
        .recharge(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            200_000,
        )
        .expect("legal replacement");
    assert_ne!(charge.id(), original_id);
    assert_eq!(charge.bytes(), Some(200_000));
    drop(charge);
    assert_eq!(b.ledger_snapshot().expect("drop replacement").total_owner_bytes(), 0);
}
#[test]
fn managed_recharge_at_live_ticket_cap_keeps_broker_authority_maps_exact() {
    let broker = LiveProcessMemoryBroker::from_report(report_with(ResidencyPolicy::new(1_000)))
        .expect("broker");
    let mut charges =
        Vec::with_capacity(crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER);
    for _ in 0..crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER {
        charges.push(
            broker
                .reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    "owner:cap".to_string(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    0,
                )
                .expect("zero-byte authority below cap"),
        );
    }
    let source = charges.last_mut().expect("source charge");
    let old_id = source.id();
    source
        .recharge(
            MemoryClass::ArtifactCache,
            "owner:cap".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        )
        .expect("managed replacement needs no extra ticket slot");
    assert_ne!(source.id(), old_id);
    let snapshot = broker.ledger_snapshot().expect("snapshot");
    snapshot.validate_exact().expect("axes after managed swap");
    assert_eq!(
        snapshot.live_ticket_count,
        crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
    );
    assert_eq!(snapshot.total_owner_bytes(), 1);
    drop(charges);
    sep23_assert_fully_drained(&broker);
}
#[test]
fn replace_denial_returns_original_live_charge() {
    let b = broker();
    let charge = b
        .reserve_committed_owned(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("seed committed charge");

    let denied = b
        .recharge_owned(
            charge,
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            700_000,
        )
        .expect_err("replacement over class cap is denied");
    let ChargeRechargeRecovery::ChargePreserved(charge) = denied.recovery;
    assert_eq!(charge.bytes(), 100_000);
    assert_eq!(b.ledger_snapshot().expect("snapshot").total_owner_bytes(), 100_000);
    b.release_owned(charge).expect("release preserved charge");
}
#[test]
fn owned_charge_sequence_matches_exact_reference_model() {
    let b = broker();
    let mut live = Vec::<ManagedCharge>::new();
    let mut model_bytes = 0u64;

    for step in 0..100u64 {
        match step % 4 {
            0 => {
                let pending = b
                    .reserve_owned(
                        MemoryClass::ArtifactCache,
                        "owner:a".to_string(),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        100,
                    )
                    .expect("pending reserve");
                drop(pending);
            }
            1 => {
                let charge = b
                    .reserve_owned(
                        MemoryClass::ArtifactCache,
                        "owner:a".to_string(),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        100,
                    )
                    .expect("reserve")
                    .commit()
                    .expect("commit");
                live.push(charge);
                model_bytes += 100;
            }
            2 if !live.is_empty() => {
                let mut charge = live.pop().expect("non-empty");
                model_bytes -= charge.bytes().expect("active charge");
                charge.release().expect("release");
            }
            _ if !live.is_empty() => {
                let mut replacement = live.pop().expect("non-empty");
                model_bytes -= replacement.bytes().expect("active charge");
                replacement
                    .recharge(
                        MemoryClass::ArtifactCache,
                        "owner:a".to_string(),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        150,
                    )
                    .expect("replacement fits");
                model_bytes += replacement.bytes().expect("replacement active");
                live.push(replacement);
            }
            _ => {}
        }

        let snapshot = b.ledger_snapshot().expect("snapshot");
        assert_eq!(snapshot.total_owner_bytes(), model_bytes, "step {step}");
        assert!(snapshot.class_owner_balanced(), "step {step}");
        assert!(snapshot.owner_partition_balanced(), "step {step}");
        assert!(snapshot.context_owner_balanced(), "step {step}");
    }

    while let Some(mut charge) = live.pop() {
        model_bytes -= charge.bytes().expect("active charge");
        charge.release().expect("drain");
    }
    assert_eq!(model_bytes, 0);
    assert_eq!(b.ledger_snapshot().expect("final").total_owner_bytes(), 0);
}
#[test]
fn reserve_commit_keeps_class_owner_and_partition_balanced() {
    let b = broker();
    let t1 = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        )
        .expect("reserve within caps must succeed");
    b.commit(&t1).expect("commit must succeed");

    let t2 = b
        .reserve(
            MemoryClass::EvidenceCache,
            "owner:a".to_string(),
            MemoryContextKind::Session,
            ChargeKind::RetainedMetadata,
            50_000,
        )
        .expect("second reserve within caps must succeed");
    b.commit(&t2).expect("commit must succeed");

    let snap = b.ledger_snapshot().expect("snapshot");
    // Invariant 1
    assert_eq!(
        snap.total_class_bytes(),
        snap.total_owner_bytes(),
        "class total must equal owner total"
    );
    assert!(snap.class_owner_balanced());
    // Invariant 2
    assert!(snap.owner_partition_balanced(), "owner usage must partition into kinds");
    assert_eq!(snap.total_owner_bytes(), 150_000);
    assert_eq!(snap.total_retained_payload_bytes(), 100_000);
    assert_eq!(snap.total_retained_metadata_bytes(), 50_000);
    assert_eq!(snap.total_live_scratch_bytes(), 0);
}
#[test]
fn reserve_denied_owner_hard_cap_leaves_ledger_untouched() {
    let b = broker();
    // Charge owner:a up to 400k (under 500k hard cap).
    let t = b
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            400_000,
        )
        .expect("first reserve fits");
    b.commit(&t).expect("commit");
    let before = b.ledger_snapshot().expect("snapshot before");

    // Now request 200k more -> 600k > 500k owner hard cap. Must deny.
    let denied = b.reserve(
        MemoryClass::ArtifactCache,
        "owner:a".to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        200_000,
    );
    assert!(matches!(denied, Err(MemoryError::Capacity(_))), "must be typed capacity error");

    let after = b.ledger_snapshot().expect("snapshot after");
    assert_eq!(
        before.total_owner_bytes(),
        after.total_owner_bytes(),
        "denied reserve must NOT mutate accounting"
    );
    assert_eq!(after.total_owner_bytes(), 400_000);
}
