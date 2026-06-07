use quanta_memory_platform_core::{
    ExecutionBudget, MemoryClass, MemoryClassBudget, MemoryError, MemoryPreset, OwnerQuota,
    PressureState, ProcessMemoryBroker, ProcessMemoryBrokerBuilder, ReclaimPriority,
    ResidencyPolicy, ResourceGroup,
};

#[test]
fn execution_budget_default_matches_runtime_budget_truth() {
    let budget = ExecutionBudget::default();

    assert_eq!(budget.max_wall_time_ms, Some(30_000));
    assert_eq!(budget.max_subqueries, Some(1_000));
    assert_eq!(budget.max_trace_edges, Some(10_000));
    assert_eq!(budget.max_output_bytes, Some(100 * 1024 * 1024));
    assert_eq!(budget.max_execution_scratch_bytes, Some(100 * 1024 * 1024));
}

#[test]
fn memory_class_taxonomy_keeps_mandatory_and_future_classes_split() {
    let mandatory = MemoryClass::mandatory();
    let future = MemoryClass::future();

    assert_eq!(mandatory.len(), 14);
    assert!(mandatory.contains(&MemoryClass::ArtifactCache));
    assert!(mandatory.contains(&MemoryClass::PtaPortableBundle));
    assert!(mandatory.contains(&MemoryClass::Headroom));

    assert_eq!(future.len(), 4);
    assert!(future.contains(&MemoryClass::SpillFileCache));
    assert!(future.contains(&MemoryClass::ParserResidency));
    assert!(future.contains(&MemoryClass::IdeWorkingSet));
    assert!(!mandatory.iter().any(|class| future.contains(class)));
}

#[test]
fn residency_policy_tracks_class_quota_and_resource_group_vocabulary() {
    let policy = ResidencyPolicy::new(512 * 1024 * 1024)
        .with_headroom_bytes(64 * 1024 * 1024)
        .with_shared_burst_pool_bytes(32 * 1024 * 1024)
        .with_class_budget(
            MemoryClass::ArtifactCache,
            MemoryClassBudget {
                reserved_bytes: 8 * 1024 * 1024,
                soft_cap_bytes: 32 * 1024 * 1024,
                hard_cap_bytes: 64 * 1024 * 1024,
                borrow_from_shared_pool: true,
                reclaim_priority: ReclaimPriority::Normal,
            },
        )
        .with_owner_quota(
            "session:alpha",
            OwnerQuota { soft_cap_bytes: 16 * 1024 * 1024, hard_cap_bytes: 32 * 1024 * 1024 },
        )
        .with_resource_group(
            "foreground",
            ResourceGroup {
                reserved_bytes: 64 * 1024 * 1024,
                soft_cap_bytes: 128 * 1024 * 1024,
                hard_cap_bytes: 192 * 1024 * 1024,
                burst_percent: 25,
                reclaim_priority: ReclaimPriority::First,
                admission_weight: 100,
            },
        )
        .with_initial_pressure(PressureState::Soft);

    assert_eq!(policy.global_hard_limit_bytes, 512 * 1024 * 1024);
    assert_eq!(policy.headroom_bytes, 64 * 1024 * 1024);
    assert_eq!(policy.shared_burst_pool_bytes, 32 * 1024 * 1024);
    assert_eq!(policy.initial_pressure, PressureState::Soft);
    assert!(policy.classes.contains_key(&MemoryClass::ArtifactCache));
    assert!(policy.owner_quotas.contains_key("session:alpha"));
    assert!(policy.resource_groups.contains_key("foreground"));
}

#[test]
fn builder_emits_effective_budget_report_with_override_provenance() {
    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Ci)
        .build()
        .expect("builder with preset should succeed");

    let report = broker.effective_report();

    assert_eq!(report.preset, MemoryPreset::Ci);
    assert_eq!(report.execution.max_trace_edges, Some(10_000));
    assert_eq!(report.override_sources, vec!["builder.with_manifest".to_string()]);
    assert_eq!(report.residency.global_hard_limit_bytes, 512 * 1024 * 1024);
}

#[test]
fn memory_error_has_typed_config_and_report_variants() {
    assert_eq!(MemoryError::config("bad preset").to_string(), "config error: bad preset");
    assert_eq!(MemoryError::report("missing source").to_string(), "report error: missing source");
}

#[cfg(feature = "live_control")]
mod live_control_contracts {
    use super::*;
    use quanta_memory_platform_core::{
        ChargeKind, EffectiveMemoryBudgetReport, LiveProcessMemoryBroker, MemoryBudgetManifest,
        MemoryContextKind,
    };

    // ============================================================================
    // Scenario group R: cross-cutting invariants promoted to the PUBLIC API
    // boundary. These tests drive the new global invariants (5, 6, 7) and preset
    // class-cap isolation through the public `LiveProcessMemoryBroker` reserve /
    // commit / release surface, using ONLY re-exports from
    // `quanta_memory_platform_core`. They are the integration-level counterpart of
    // the module-internal ledger tests: a regression that weakens these invariants
    // must be visible to an external consumer, not just to white-box unit tests.
    // ============================================================================

    fn class_budget(reserved: u64, soft: u64, hard: u64, borrow: bool) -> MemoryClassBudget {
        MemoryClassBudget {
            reserved_bytes: reserved,
            soft_cap_bytes: soft,
            hard_cap_bytes: hard,
            borrow_from_shared_pool: borrow,
            reclaim_priority: ReclaimPriority::Normal,
        }
    }

    fn broker_from_residency(residency: ResidencyPolicy) -> LiveProcessMemoryBroker {
        let report = EffectiveMemoryBudgetReport::from_manifest(
            MemoryBudgetManifest::new(MemoryPreset::Ci, ExecutionBudget::default(), residency),
            vec!["wave1.contract".to_string()],
        );
        LiveProcessMemoryBroker::from_report(report)
            .expect("residency policy is structurally valid")
    }

    fn broker_for_preset(preset: MemoryPreset) -> LiveProcessMemoryBroker {
        let report = EffectiveMemoryBudgetReport::from_manifest(
            MemoryBudgetManifest::for_preset(preset),
            vec!["builder.with_manifest".to_string()],
        );
        LiveProcessMemoryBroker::from_report(report).expect("preset residency must be valid")
    }

    #[test]
    fn global_hard_limit_denies_reserve_through_public_broker_and_is_fail_closed() {
        let b = broker_from_residency(ResidencyPolicy::new(1_000));
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                600,
            )
            .expect("first reserve fits under the global ceiling");
        b.commit(&t).expect("commit");
        let before = b.ledger_snapshot().expect("snapshot before denied reserve");

        let denied = b.reserve(
            MemoryClass::EvidenceCache,
            "owner:b".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            600,
        );
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "exceeding the global hard limit must be a typed capacity error, got {denied:?}"
        );

        let after = b.ledger_snapshot().expect("snapshot after denied reserve");
        assert_eq!(
            before, after,
            "denied global reserve must leave the ledger byte-for-byte unchanged"
        );
        assert_eq!(after.total_owner_bytes(), 600);
        assert!(after.class_owner_balanced(), "invariant 1 holds after a denied reserve");
        assert!(after.owner_partition_balanced(), "invariant 2 holds after a denied reserve");
    }

    #[test]
    fn headroom_band_is_reserved_for_headroom_class_at_public_surface() {
        let b = broker_from_residency(ResidencyPolicy::new(1_000).with_headroom_bytes(200));

        let denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            900,
        );
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "900 > (1_000 - 200 headroom) must be denied for a non-headroom class, got {denied:?}"
        );
        assert_eq!(
            b.ledger_snapshot().expect("snap").total_owner_bytes(),
            0,
            "denial into the headroom band must not mutate"
        );

        let t = b
            .reserve(
                MemoryClass::Headroom,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                900,
            )
            .expect("Headroom class may use the reserved band up to the global ceiling");
        b.commit(&t).expect("commit");
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 900);
    }

    #[test]
    fn shared_burst_pool_bounds_borrowing_through_public_broker() {
        let b = broker_from_residency(
            ResidencyPolicy::new(10_000_000)
                .with_shared_burst_pool_bytes(100)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400, 600, true)),
        );

        let t0 = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                600,
            )
            .expect("up to the class hard cap");
        b.commit(&t0).expect("commit");
        let t1 = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                50,
            )
            .expect("borrow within the shared burst pool");
        b.commit(&t1).expect("commit");
        let before = b.ledger_snapshot().expect("snap before overdraw");

        let denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            60,
        );
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "borrow beyond the shared burst pool must be a typed capacity error, got {denied:?}"
        );
        let after = b.ledger_snapshot().expect("snap after overdraw");
        assert_eq!(before, after, "denied over-pool borrow must be fail-closed");
        assert_eq!(after.total_owner_bytes(), 650, "usage stays at cap + admitted borrow");
    }

    #[test]
    fn non_borrowing_class_cannot_use_burst_pool_at_public_surface() {
        let b = broker_from_residency(
            ResidencyPolicy::new(10_000_000)
                .with_shared_burst_pool_bytes(10_000)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400, 600, false)),
        );
        let denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            700,
        );
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "a non-borrowing class cannot draw from the shared pool, got {denied:?}"
        );
        assert_eq!(
            b.ledger_snapshot().expect("snap").total_owner_bytes(),
            0,
            "denial must leave the ledger untouched"
        );
    }

    #[test]
    fn double_release_of_cloned_ticket_rejected_through_public_broker() {
        let b = broker_from_residency(ResidencyPolicy::new(1_000_000));
        let ticket = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("reserve");
        b.commit(&ticket).expect("commit");

        b.release(&ticket).expect("first release succeeds");
        let after_first = b.ledger_snapshot().expect("snapshot after first release");
        assert_eq!(after_first.total_owner_bytes(), 0, "release returns the bytes exactly once");

        let twin = ticket.clone();
        let second = b.release(&twin);
        assert!(
            matches!(second, Err(MemoryError::InvalidState(_))),
            "double release must be a typed invalid-state error, got {second:?}"
        );
        assert_eq!(
            b.ledger_snapshot().expect("snapshot after rejected double release"),
            after_first,
            "rejected double release must not drive any counter below the truth"
        );
        assert!(
            after_first.class_owner_balanced(),
            "invariant 1 survives a rejected double release"
        );
        assert!(
            after_first.owner_partition_balanced(),
            "invariant 2 survives a rejected double release"
        );
    }

    #[test]
    fn commit_after_release_rejected_through_public_broker() {
        let b = broker_from_residency(ResidencyPolicy::new(1_000_000));
        let ticket = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                10_000,
            )
            .expect("reserve");
        b.release(&ticket).expect("release the uncommitted reservation");

        assert!(
            matches!(b.commit(&ticket), Err(MemoryError::InvalidState(_))),
            "committing a released ticket must be rejected at the public surface"
        );
        assert_eq!(
            b.ledger_snapshot().expect("snap").total_owner_bytes(),
            0,
            "the rejected commit must not resurrect the released charge"
        );
    }

    #[test]
    fn preset_class_caps_bite_at_public_reserve_surface() {
        for preset in [
            MemoryPreset::Development,
            MemoryPreset::Ci,
            MemoryPreset::Production,
            MemoryPreset::Interactive,
        ] {
            let report = EffectiveMemoryBudgetReport::from_manifest(
                MemoryBudgetManifest::for_preset(preset.clone()),
                vec!["builder.with_manifest".to_string()],
            );
            let cap = report.residency.classes[&MemoryClass::ManifestResidency].hard_cap_bytes;
            assert!(cap > 0, "{preset:?} must grant ManifestResidency a positive cap");
            let b = LiveProcessMemoryBroker::from_report(report).expect("preset broker builds");

            let denied = b.reserve(
                MemoryClass::ManifestResidency,
                "owner:probe".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                cap + 1,
            );
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "{preset:?}: reserving over the ManifestResidency cap must be denied, got {denied:?}"
            );
            assert_eq!(
                b.ledger_snapshot().expect("snap").total_owner_bytes(),
                0,
                "{preset:?}: the denied over-cap reserve must be fail-closed"
            );

            let t = b
                .reserve(
                    MemoryClass::ManifestResidency,
                    "owner:probe".to_string(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    cap,
                )
                .unwrap_or_else(|e| panic!("{preset:?}: reserving exactly the cap must fit: {e}"));
            b.commit(&t).expect("commit at the cap");
            assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), cap);
        }
    }

    #[test]
    fn preset_broker_global_limit_is_consistent_with_enforcement() {
        let b = broker_for_preset(MemoryPreset::Ci);
        let report = b.effective_report().expect("report");
        assert_eq!(
            report.residency.global_hard_limit_bytes,
            512 * 1024 * 1024,
            "Ci preset global hard limit is 512 MiB at the public report surface"
        );

        let over_global = report.residency.global_hard_limit_bytes + 1;
        for class in MemoryClass::mandatory() {
            let denied = b.reserve(
                class.clone(),
                "owner:probe".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                over_global,
            );
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "{class:?}: a reserve above the reported global limit must be denied, got {denied:?}"
            );
        }
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
    }
}
