use super::*;

#[test]
fn higher_precedence_layer_wins_and_is_recorded_in_provenance() {
    let resolved = EffectiveBudgetResolver::from_manifest(manifest())
        // Intentionally register out of precedence order to prove the
        // resolver reorders by authority, not by call order.
        .with_layer(
            OverrideSource::TestFixture,
            BudgetOverrideLayer::empty().with_global_hard_limit_bytes(900),
        )
        .with_layer(
            OverrideSource::Env,
            BudgetOverrideLayer::empty().with_global_hard_limit_bytes(700),
        )
        .with_layer(
            OverrideSource::Manifest,
            BudgetOverrideLayer::empty().with_global_hard_limit_bytes(500),
        )
        .resolve()
        .expect("resolve");

    // Highest authority (TestFixture=900) must be the final value.
    assert_eq!(resolved.residency.global_hard_limit_bytes, 900);

    // Provenance must record the last writer for the field as TestFixture,
    // and the ordered records prove env and manifest wrote earlier.
    let writers: Vec<OverrideSource> = resolved
        .provenance
        .iter()
        .filter(|r| r.field == "residency.global_hard_limit_bytes")
        .map(|r| r.source)
        .collect();
    assert_eq!(
        writers,
        vec![OverrideSource::Manifest, OverrideSource::Env, OverrideSource::TestFixture],
        "provenance must reflect canonical precedence order"
    );
    // The last record for the field is the effective authority.
    assert_eq!(writers.last().copied(), Some(OverrideSource::TestFixture));
}
#[test]
fn removing_top_layer_changes_effective_value() {
    let with_top = EffectiveBudgetResolver::from_manifest(manifest())
        .with_layer(
            OverrideSource::Env,
            BudgetOverrideLayer::empty().with_global_hard_limit_bytes(700),
        )
        .with_layer(
            OverrideSource::TestFixture,
            BudgetOverrideLayer::empty().with_global_hard_limit_bytes(900),
        )
        .resolve()
        .expect("resolve");
    assert_eq!(with_top.residency.global_hard_limit_bytes, 900);

    let without_top = EffectiveBudgetResolver::from_manifest(manifest())
        .with_layer(
            OverrideSource::Env,
            BudgetOverrideLayer::empty().with_global_hard_limit_bytes(700),
        )
        .resolve()
        .expect("resolve");
    assert_eq!(without_top.residency.global_hard_limit_bytes, 700);
    assert_ne!(
        with_top.residency.global_hard_limit_bytes, without_top.residency.global_hard_limit_bytes,
        "dropping the highest-authority override must change the result"
    );
}
#[test]
fn empty_layer_is_skipped_in_provenance() {
    let resolved = EffectiveBudgetResolver::from_manifest(manifest())
        .with_layer(OverrideSource::Cli, BudgetOverrideLayer::empty())
        .resolve()
        .expect("resolve");
    assert!(
        !resolved.provenance.iter().any(|r| r.source == OverrideSource::Cli),
        "an empty override layer must not appear in provenance"
    );
}
#[test]
fn validate_rejects_reserved_above_soft() {
    let residency = crate::domain::policy::ResidencyPolicy::new(10_000).with_class_budget(
        MemoryClass::ArtifactCache,
        MemoryClassBudget {
            reserved_bytes: 500,
            soft_cap_bytes: 300, // reserved > soft -> invalid
            hard_cap_bytes: 1_000,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        },
    );
    assert!(matches!(validate_residency(&residency), Err(MemoryError::Config(_))));
}
#[test]
fn validate_rejects_global_below_reserved_plus_headroom_floor() {
    let residency = crate::domain::policy::ResidencyPolicy::new(1_000)
        .with_headroom_bytes(2_000)
        .with_class_budget(
            MemoryClass::ArtifactCache,
            MemoryClassBudget {
                reserved_bytes: 500,
                soft_cap_bytes: 600,
                hard_cap_bytes: 700,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        );
    // floor = 500 + 2000 = 2500 > global 1000 -> invalid
    assert!(matches!(validate_residency(&residency), Err(MemoryError::Config(_))));
}
#[test]
fn validate_rejects_class_hard_cap_above_global() {
    let residency = crate::domain::policy::ResidencyPolicy::new(1_000).with_class_budget(
        MemoryClass::ArtifactCache,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: 900,
            hard_cap_bytes: 2_000, // > global 1_000
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        },
    );
    assert!(matches!(validate_residency(&residency), Err(MemoryError::Config(_))));
}
#[test]
fn validate_rejects_owner_hard_cap_above_global() {
    let residency = crate::domain::policy::ResidencyPolicy::new(1_000).with_owner_quota(
        "owner:big",
        crate::domain::policy::OwnerQuota { soft_cap_bytes: 500, hard_cap_bytes: 5_000 },
    );
    assert!(matches!(validate_residency(&residency), Err(MemoryError::Config(_))));
}
#[test]
fn validate_accepts_well_formed_policy() {
    let residency = crate::domain::policy::ResidencyPolicy::new(10_000)
        .with_headroom_bytes(1_000)
        .with_class_budget(
            MemoryClass::ArtifactCache,
            MemoryClassBudget {
                reserved_bytes: 500,
                soft_cap_bytes: 600,
                hard_cap_bytes: 700,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        );
    assert!(validate_residency(&residency).is_ok());
}
#[test]
fn validate_rejects_policy_map_cardinality_above_the_caps() {
    use crate::domain::management_limits::{
        MAX_OWNER_GROUP_MAPPINGS_PER_POLICY, MAX_OWNER_QUOTAS_PER_POLICY,
        MAX_RESOURCE_GROUPS_PER_POLICY,
    };
    use crate::domain::policy::{OwnerQuota, ResourceGroup};

    let group = || ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 1_000,
        hard_cap_bytes: 1_000,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };

    // Resource groups: exactly at the cap is a valid trusted policy; one
    // above is rejected fail-closed.
    let groups_at_cap = (0..MAX_RESOURCE_GROUPS_PER_POLICY)
        .fold(ResidencyPolicy::new(100_000_000), |policy, index| {
            policy.with_resource_group(format!("g{index}"), group())
        });
    assert!(
        validate_residency(&groups_at_cap).is_ok(),
        "at-cap resource-group count must remain a valid policy"
    );
    let groups_over = groups_at_cap.clone().with_resource_group("g-over", group());
    assert!(
        matches!(validate_residency(&groups_over), Err(MemoryError::Config(_))),
        "a resource-group cardinality above the cap must be rejected"
    );

    // Owner->group mappings: at cap accepted, one above rejected.
    let mappings_at_cap = (0..MAX_OWNER_GROUP_MAPPINGS_PER_POLICY)
        .fold(groups_at_cap, |policy, index| {
            policy.with_owner_group(format!("o{index}"), "g0".to_string())
        });
    assert!(
        validate_residency(&mappings_at_cap).is_ok(),
        "at-cap owner mapping count must remain a valid policy"
    );
    let mappings_over = mappings_at_cap.with_owner_group("o-over".to_string(), "g0".to_string());
    assert!(
        matches!(validate_residency(&mappings_over), Err(MemoryError::Config(_))),
        "an owner-mapping cardinality above the cap must be rejected"
    );

    // Per-owner quotas: at cap accepted, one above rejected.
    let quotas_at_cap = (0..MAX_OWNER_QUOTAS_PER_POLICY).fold(
        ResidencyPolicy::new(100_000_000),
        |policy, index| {
            policy.with_owner_quota(
                format!("q{index}"),
                OwnerQuota { soft_cap_bytes: 1_000, hard_cap_bytes: 2_000 },
            )
        },
    );
    assert!(
        validate_residency(&quotas_at_cap).is_ok(),
        "at-cap owner quota count must remain a valid policy"
    );
    let quotas_over = quotas_at_cap.with_owner_quota(
        "q-over".to_string(),
        OwnerQuota { soft_cap_bytes: 1_000, hard_cap_bytes: 2_000 },
    );
    assert!(
        matches!(validate_residency(&quotas_over), Err(MemoryError::Config(_))),
        "an owner-quota cardinality above the cap must be rejected"
    );
    // The class axis needs no cardinality cap: it is keyed by the closed
    // MemoryClass enum, so its size is structurally bounded by the
    // taxonomy itself.
}
#[test]
fn runtime_reconfigure_layers_are_rejected_at_build_time() {
    let resolver = EffectiveBudgetResolver::from_manifest(manifest()).with_layer(
        OverrideSource::RuntimeReconfigure,
        BudgetOverrideLayer::empty().with_global_hard_limit_bytes(700),
    );
    let denied = resolver.resolve();
    assert!(
        matches!(denied, Err(MemoryError::Config(_))),
        "build-time runtime-reconfigure layer must be rejected, got {denied:?}"
    );

    // Static sources keep their documented precedence unchanged.
    let static_stack = EffectiveBudgetResolver::from_manifest(manifest())
        .with_layer(
            OverrideSource::TestFixture,
            BudgetOverrideLayer::empty().with_global_hard_limit_bytes(900),
        )
        .with_layer(
            OverrideSource::Env,
            BudgetOverrideLayer::empty().with_global_hard_limit_bytes(700),
        )
        .resolve()
        .expect("static layers resolve");
    assert_eq!(static_stack.residency.global_hard_limit_bytes, 900);
}
#[test]
fn validate_rejects_shared_burst_pool_above_global_hard_limit() {
    let oversized = ResidencyPolicy::new(10_000).with_shared_burst_pool_bytes(20_000);
    assert!(
        matches!(validate_residency(&oversized), Err(MemoryError::Config(_))),
        "a burst pool above the global hard limit is incoherent"
    );
    let bounded = ResidencyPolicy::new(10_000).with_shared_burst_pool_bytes(10_000);
    assert!(validate_residency(&bounded).is_ok(), "a pool at the global limit is legal");
}
#[test]
fn validate_rejects_zero_admission_weight_with_burst_demand() {
    use crate::domain::policy::ResourceGroup;
    let group = |weight: u16, burst: u8| ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 100,
        hard_cap_bytes: 100,
        burst_percent: burst,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: weight,
    };
    let demanding = ResidencyPolicy::new(10_000).with_resource_group("g", group(0, 50));
    assert!(
        matches!(validate_residency(&demanding), Err(MemoryError::Config(_))),
        "zero admission weight with positive burst demand is silently vacuous"
    );
    let silent = ResidencyPolicy::new(10_000).with_resource_group("g", group(0, 0));
    assert!(
        validate_residency(&silent).is_ok(),
        "zero weight with zero burst demand stays a legal no-burst group"
    );
}
#[test]
fn validate_rejects_missing_each_mandatory_class() {
    for class in MemoryClass::mandatory() {
        let mut residency = ResidencyPolicy::new(10_000);
        assert!(residency.classes.remove(&class).is_some());

        let result = validate_residency(&residency);
        assert!(
            matches!(result, Err(MemoryError::Config(_))),
            "missing mandatory class {class:?} must be rejected, got {result:?}"
        );
    }
}
#[test]
fn validated_policy_materializes_no_implicit_class_holes() {
    let validated = ValidatedResidencyPolicy::try_from(ResidencyPolicy::new(10_000))
        .expect("constructor materializes every mandatory class");

    for class in MemoryClass::mandatory() {
        assert!(
            validated.as_policy().classes.contains_key(&class),
            "validated policy must explicitly contain {class:?}"
        );
    }
}
#[test]
fn reserved_floor_checked_add_overflow_is_config_error() {
    let mut residency = ResidencyPolicy::new(u64::MAX);
    for class in [MemoryClass::ArtifactCache, MemoryClass::EvidenceCache] {
        residency.classes.insert(
            class,
            MemoryClassBudget {
                reserved_bytes: u64::MAX,
                soft_cap_bytes: u64::MAX,
                hard_cap_bytes: u64::MAX,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        );
    }

    assert!(matches!(validate_residency(&residency), Err(MemoryError::Config(_))));
}
#[test]
fn validate_accepts_well_formed_resource_group() {
    let residency = residency_with_group(good_group());
    assert!(
        validate_residency(&residency).is_ok(),
        "a coherent resource group (reserved<=soft<=hard, hard<=global, burst<=100) must pass"
    );
}
#[test]
fn validate_accepts_resource_group_reserved_equal_to_soft_cap() {
    let mut group = good_group();
    group.reserved_bytes = group.soft_cap_bytes;
    let result = validate_residency(&residency_with_group(group));
    assert!(
        result.is_ok(),
        "reserved == soft is valid under the inclusive reserved<=soft contract, got {result:?}"
    );
}
#[test]
fn validate_rejects_resource_group_reserved_above_soft() {
    let mut g = good_group();
    g.reserved_bytes = 250; // > soft 200
    assert!(matches!(validate_residency(&residency_with_group(g)), Err(MemoryError::Config(_))));
}
#[test]
fn validate_rejects_resource_group_soft_above_hard() {
    let mut g = good_group();
    g.soft_cap_bytes = 400; // > hard 300
    assert!(matches!(validate_residency(&residency_with_group(g)), Err(MemoryError::Config(_))));
}
#[test]
fn validate_rejects_resource_group_hard_above_global() {
    let mut g = good_group();
    g.soft_cap_bytes = 9_000;
    g.hard_cap_bytes = 20_000; // > global 10_000
    assert!(matches!(validate_residency(&residency_with_group(g)), Err(MemoryError::Config(_))));
}
#[test]
fn validate_rejects_resource_group_burst_percent_over_100() {
    let mut g = good_group();
    g.burst_percent = 101; // out of the documented 0..=100 band
    assert!(matches!(validate_residency(&residency_with_group(g)), Err(MemoryError::Config(_))));
}
#[test]
fn validate_accepts_resource_group_burst_percent_boundaries() {
    for burst in [0u8, 100u8] {
        let mut g = good_group();
        g.burst_percent = burst;
        assert!(
            validate_residency(&residency_with_group(g)).is_ok(),
            "burst_percent {burst} is on the inclusive boundary and must pass"
        );
    }
}
#[test]
fn resource_group_reserved_has_an_independent_global_floor() {
    let residency = ResidencyPolicy::new(10_000)
        .with_headroom_bytes(1_000)
        .with_resource_group(
            "group:a",
            ResourceGroup {
                reserved_bytes: 5_000,
                soft_cap_bytes: 6_000,
                hard_cap_bytes: 7_000,
                burst_percent: 10,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_class_budget(
            MemoryClass::ArtifactCache,
            MemoryClassBudget {
                reserved_bytes: 5_000,
                soft_cap_bytes: 6_000,
                hard_cap_bytes: 7_000,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        );
    assert!(validate_residency(&residency).is_ok());
}
#[test]
fn validate_rejects_group_reserved_plus_headroom_above_global() {
    let residency = ResidencyPolicy::new(10_000)
        .with_headroom_bytes(1_000)
        .with_resource_group(
            "group:a",
            ResourceGroup {
                reserved_bytes: 5_000,
                soft_cap_bytes: 5_000,
                hard_cap_bytes: 5_000,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_resource_group(
            "group:b",
            ResourceGroup {
                reserved_bytes: 5_000,
                soft_cap_bytes: 5_000,
                hard_cap_bytes: 5_000,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        );
    assert!(matches!(validate_residency(&residency), Err(MemoryError::Config(_))));
}
#[test]
fn validate_rejects_owner_group_mapping_to_unknown_group() {
    let residency =
        crate::domain::policy::ResidencyPolicy::new(1_000_000).with_owner_group("o", "ghost");
    assert!(matches!(validate_residency(&residency), Err(MemoryError::Config(_))));
}
#[test]
fn validate_accepts_owner_group_mapping_to_declared_group() {
    let residency = crate::domain::policy::ResidencyPolicy::new(1_000_000)
        .with_resource_group(
            "g",
            crate::domain::policy::ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 100,
                hard_cap_bytes: 200,
                burst_percent: 10,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("o", "g");
    assert!(validate_residency(&residency).is_ok());
}
#[test]
fn validate_rejects_overlong_owner_and_resource_group_identifiers() {
    use crate::domain::management_limits::{MAX_OWNER_KEY_BYTES, MAX_RESOURCE_GROUP_KEY_BYTES};
    let owner = "o".repeat(MAX_OWNER_KEY_BYTES + 1);
    let owner_policy = ResidencyPolicy::new(10_000).with_owner_quota(
        owner,
        crate::domain::policy::OwnerQuota { soft_cap_bytes: 1, hard_cap_bytes: 2 },
    );
    assert!(matches!(validate_residency(&owner_policy), Err(MemoryError::Config(_))));

    let owner_group_policy = ResidencyPolicy::new(10_000)
        .with_resource_group(
            "valid-group",
            crate::domain::policy::ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 1,
                hard_cap_bytes: 2,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("o".repeat(MAX_OWNER_KEY_BYTES + 1), "valid-group");
    assert!(matches!(validate_residency(&owner_group_policy), Err(MemoryError::Config(_))));

    let group = "g".repeat(MAX_RESOURCE_GROUP_KEY_BYTES + 1);
    let group_policy = ResidencyPolicy::new(10_000).with_resource_group(
        group,
        crate::domain::policy::ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 1,
            hard_cap_bytes: 2,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        },
    );
    assert!(matches!(validate_residency(&group_policy), Err(MemoryError::Config(_))));
}
