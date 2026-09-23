//! Effective budget resolution: preset/manifest/override precedence.
//!
//! Precedence order (lowest authority first, highest authority last):
//!
//! 1. library preset + workload profile (base default class budgets)
//! 2. checked-in manifest values
//! 3. environment overrides
//! 4. CLI / process overrides
//! 5. test fixture overrides
//!
//! Each applied layer records an [`OverrideRecord`] so the emitted
//! [`crate::domain::stats::EffectiveMemoryBudgetReport`] can explain exactly
//! which source last wrote a given field. Resolution is deterministic: the same
//! ordered layer set produces the same provenance and the same residency policy.

use crate::domain::errors::{MemoryError, MemoryResult};
use crate::domain::policy::{MemoryBudgetManifest, MemoryClass, MemoryPreset, ResidencyPolicy};

/// The authority layer that produced or last mutated a budget value.
///
/// Ordered from lowest to highest precedence; a later layer overrides an
/// earlier one. The ordering is a stable contract, not an implementation
/// detail, because provenance reporting depends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OverrideSource {
    /// Library preset + workload profile base.
    Preset,
    /// Checked-in budget manifest file.
    Manifest,
    /// Environment-variable overrides.
    Env,
    /// CLI / process-launch overrides.
    Cli,
    /// Test fixture override used during initial resolution.
    TestFixture,
    /// Accepted live policy replacement after broker construction.
    RuntimeReconfigure,
}

impl OverrideSource {
    /// Stable, machine-and-human-readable provenance label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Preset => "preset",
            Self::Manifest => "manifest",
            Self::Env => "env",
            Self::Cli => "cli",
            Self::TestFixture => "test_fixture",
            Self::RuntimeReconfigure => "runtime_reconfigure",
        }
    }
}

/// One provenance entry: which field a layer touched, and which source it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverrideRecord {
    pub source: OverrideSource,
    pub field: String,
}

impl OverrideRecord {
    pub fn new(source: OverrideSource, field: impl Into<String>) -> Self {
        Self { source, field: field.into() }
    }
}

/// A single precedence layer: a partial residency override plus its source.
///
/// `None` fields mean "this layer does not touch that value"; only `Some`
/// fields overlay the resolved policy and produce provenance records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BudgetOverrideLayer {
    pub global_hard_limit_bytes: Option<u64>,
    pub headroom_bytes: Option<u64>,
    pub shared_burst_pool_bytes: Option<u64>,
}

impl BudgetOverrideLayer {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_global_hard_limit_bytes(mut self, bytes: u64) -> Self {
        self.global_hard_limit_bytes = Some(bytes);
        self
    }

    #[must_use]
    pub fn with_headroom_bytes(mut self, bytes: u64) -> Self {
        self.headroom_bytes = Some(bytes);
        self
    }

    #[must_use]
    pub fn with_shared_burst_pool_bytes(mut self, bytes: u64) -> Self {
        self.shared_burst_pool_bytes = Some(bytes);
        self
    }

    fn is_empty(&self) -> bool {
        self.global_hard_limit_bytes.is_none()
            && self.headroom_bytes.is_none()
            && self.shared_burst_pool_bytes.is_none()
    }
}

/// Deterministic config resolver. Layers are applied in precedence order; the
/// resolver validates the resulting policy and records provenance.
#[derive(Clone, Debug)]
pub struct EffectiveBudgetResolver {
    manifest: MemoryBudgetManifest,
    layers: Vec<(OverrideSource, BudgetOverrideLayer)>,
}

/// The output of [`EffectiveBudgetResolver::resolve`]: a validated residency
/// policy and the ordered provenance records that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedBudget {
    pub preset: MemoryPreset,
    pub residency: ResidencyPolicy,
    pub provenance: Vec<OverrideRecord>,
}

/// A residency policy that has crossed the canonical structural validation
/// boundary. The inner policy is intentionally private so accounting state
/// cannot be constructed or reconfigured from an unchecked value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedResidencyPolicy(ResidencyPolicy);

impl ValidatedResidencyPolicy {
    #[must_use]
    pub fn as_policy(&self) -> &ResidencyPolicy {
        &self.0
    }

    #[must_use]
    pub fn into_inner(self) -> ResidencyPolicy {
        self.0
    }
}

impl TryFrom<ResidencyPolicy> for ValidatedResidencyPolicy {
    type Error = MemoryError;

    fn try_from(residency: ResidencyPolicy) -> Result<Self, Self::Error> {
        validate_residency_inner(&residency)?;
        Ok(Self(residency))
    }
}

impl EffectiveBudgetResolver {
    /// Start resolution from a base manifest (preset + workload profile already
    /// materialized into default class budgets by the caller).
    #[must_use]
    pub fn from_manifest(manifest: MemoryBudgetManifest) -> Self {
        Self { manifest, layers: Vec::new() }
    }

    /// Queue a precedence layer. Layers are *not* sorted by call order; they are
    /// always reordered to canonical precedence at `resolve` time so callers
    /// cannot accidentally invert authority by registering out of order.
    #[must_use]
    pub fn with_layer(mut self, source: OverrideSource, layer: BudgetOverrideLayer) -> Self {
        self.layers.push((source, layer));
        self
    }

    /// Resolve to a validated [`ResolvedBudget`].
    ///
    /// Fails closed with a typed [`MemoryError::Config`] when the resolved
    /// residency policy violates the `reserved <= soft <= hard` ordering or the
    /// global-hard-vs-reserved-plus-headroom floor.
    pub fn resolve(self) -> MemoryResult<ResolvedBudget> {
        let preset = self.manifest.preset.clone();
        let mut residency = self.manifest.residency.clone();
        let mut provenance = Vec::new();

        // The base manifest is always the lowest-authority provenance source.
        provenance.push(OverrideRecord::new(OverrideSource::Preset, "residency.base"));

        // Apply layers in canonical precedence, not call order.
        let mut ordered = self.layers;
        ordered.sort_by_key(|entry| entry.0);

        for (source, layer) in ordered {
            if layer.is_empty() {
                continue;
            }
            if let Some(bytes) = layer.global_hard_limit_bytes {
                residency.global_hard_limit_bytes = bytes;
                provenance.push(OverrideRecord::new(source, "residency.global_hard_limit_bytes"));
            }
            if let Some(bytes) = layer.headroom_bytes {
                residency.headroom_bytes = bytes;
                provenance.push(OverrideRecord::new(source, "residency.headroom_bytes"));
            }
            if let Some(bytes) = layer.shared_burst_pool_bytes {
                residency.shared_burst_pool_bytes = bytes;
                provenance.push(OverrideRecord::new(source, "residency.shared_burst_pool_bytes"));
            }
        }

        let residency = ValidatedResidencyPolicy::try_from(residency)?.into_inner();

        Ok(ResolvedBudget { preset, residency, provenance })
    }
}

/// Validate the structural invariants of a residency policy.
///
/// Enforces `reserved <= soft <= hard` per class, per owner quota
/// (`soft <= hard`), and per resource group (`reserved <= soft <= hard`,
/// `hard <= global`, `burst_percent in 0..=100`); each per-axis hard cap must
/// stay at or below the global limit; and the global floor
/// `global_hard >= sum(class reserved) + headroom` must hold. This is the gate
/// the playbook (section 2.2) requires before
/// `ProcessMemoryBrokerBuilder::build()`.
pub fn validate_residency(residency: &ResidencyPolicy) -> MemoryResult<()> {
    validate_residency_inner(residency)
}

fn validate_residency_inner(residency: &ResidencyPolicy) -> MemoryResult<()> {
    for class in MemoryClass::mandatory() {
        if !residency.classes.contains_key(&class) {
            return Err(MemoryError::config(format!(
                "mandatory memory class {class:?} is absent from residency policy"
            )));
        }
    }

    let mut total_reserved: u64 = 0;
    for (class, budget) in &residency.classes {
        if budget.reserved_bytes > budget.soft_cap_bytes {
            return Err(MemoryError::config(format!(
                "class {class:?} reserved {} exceeds soft cap {}",
                budget.reserved_bytes, budget.soft_cap_bytes
            )));
        }
        if budget.soft_cap_bytes > budget.hard_cap_bytes {
            return Err(MemoryError::config(format!(
                "class {class:?} soft cap {} exceeds hard cap {}",
                budget.soft_cap_bytes, budget.hard_cap_bytes
            )));
        }
        // A per-class hard cap above the global ceiling is structurally
        // incoherent: it advertises a cap that the global limit can never honor.
        if budget.hard_cap_bytes > residency.global_hard_limit_bytes {
            return Err(MemoryError::config(format!(
                "class {class:?} hard cap {} exceeds global hard limit {}",
                budget.hard_cap_bytes, residency.global_hard_limit_bytes
            )));
        }
        total_reserved = total_reserved
            .checked_add(budget.reserved_bytes)
            .ok_or_else(|| MemoryError::config("sum of class reserved bytes overflowed u64"))?;
    }

    for (owner, quota) in &residency.owner_quotas {
        if owner.len() > crate::domain::management_limits::MAX_OWNER_KEY_BYTES {
            return Err(MemoryError::config(format!(
                "owner identifier is {} bytes; maximum is {}",
                owner.len(),
                crate::domain::management_limits::MAX_OWNER_KEY_BYTES
            )));
        }
        if quota.soft_cap_bytes > quota.hard_cap_bytes {
            return Err(MemoryError::config(format!(
                "owner {owner} soft cap {} exceeds hard cap {}",
                quota.soft_cap_bytes, quota.hard_cap_bytes
            )));
        }
        if quota.hard_cap_bytes > residency.global_hard_limit_bytes {
            return Err(MemoryError::config(format!(
                "owner {owner} hard cap {} exceeds global hard limit {}",
                quota.hard_cap_bytes, residency.global_hard_limit_bytes
            )));
        }
    }

    // Resource groups carry the same structural ordering as classes
    // (`reserved <= soft <= hard`, `hard <= global`) plus a bounded
    // `burst_percent` in `0..=100`: a group is a named, weighted admission band,
    // so an incoherent band would silently corrupt the scheduler's split.
    let mut total_group_reserved = 0u64;
    for (group, policy) in &residency.resource_groups {
        if group.len() > crate::domain::management_limits::MAX_RESOURCE_GROUP_KEY_BYTES {
            return Err(MemoryError::config(format!(
                "resource-group identifier is {} bytes; maximum is {}",
                group.len(),
                crate::domain::management_limits::MAX_RESOURCE_GROUP_KEY_BYTES
            )));
        }
        if policy.reserved_bytes > policy.soft_cap_bytes {
            return Err(MemoryError::config(format!(
                "resource group {group} reserved {} exceeds soft cap {}",
                policy.reserved_bytes, policy.soft_cap_bytes
            )));
        }
        if policy.soft_cap_bytes > policy.hard_cap_bytes {
            return Err(MemoryError::config(format!(
                "resource group {group} soft cap {} exceeds hard cap {}",
                policy.soft_cap_bytes, policy.hard_cap_bytes
            )));
        }
        if policy.hard_cap_bytes > residency.global_hard_limit_bytes {
            return Err(MemoryError::config(format!(
                "resource group {group} hard cap {} exceeds global hard limit {}",
                policy.hard_cap_bytes, residency.global_hard_limit_bytes
            )));
        }
        if policy.burst_percent > 100 {
            return Err(MemoryError::config(format!(
                "resource group {group} burst_percent {} exceeds 100",
                policy.burst_percent
            )));
        }
        total_group_reserved =
            total_group_reserved.checked_add(policy.reserved_bytes).ok_or_else(|| {
                MemoryError::config("sum of resource-group reserved bytes overflowed u64")
            })?;
    }

    // Every owner->group mapping must reference a declared resource group, or
    // the scheduler would gate (or fail to gate) reservations against a group
    // that does not exist.
    for (owner, group) in &residency.owner_groups {
        if owner.len() > crate::domain::management_limits::MAX_OWNER_KEY_BYTES {
            return Err(MemoryError::config(format!(
                "owner identifier is {} bytes; maximum is {}",
                owner.len(),
                crate::domain::management_limits::MAX_OWNER_KEY_BYTES
            )));
        }
        if group.len() > crate::domain::management_limits::MAX_RESOURCE_GROUP_KEY_BYTES {
            return Err(MemoryError::config(format!(
                "resource-group identifier is {} bytes; maximum is {}",
                group.len(),
                crate::domain::management_limits::MAX_RESOURCE_GROUP_KEY_BYTES
            )));
        }
        if !residency.resource_groups.contains_key(group) {
            return Err(MemoryError::config(format!(
                "owner {owner} mapped to unknown resource group {group}"
            )));
        }
    }

    let floor = total_reserved
        .checked_add(residency.headroom_bytes)
        .ok_or_else(|| MemoryError::config("reserved+headroom floor overflowed u64"))?;
    if residency.global_hard_limit_bytes < floor {
        return Err(MemoryError::config(format!(
            "global hard limit {} below reserved+headroom floor {floor}",
            residency.global_hard_limit_bytes
        )));
    }
    let group_floor =
        total_group_reserved.checked_add(residency.headroom_bytes).ok_or_else(|| {
            MemoryError::config("resource-group reserved+headroom floor overflowed u64")
        })?;
    if residency.global_hard_limit_bytes < group_floor {
        return Err(MemoryError::config(format!(
            "global hard limit {} below resource-group reserved+headroom floor {group_floor}",
            residency.global_hard_limit_bytes
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::policy::{
        MemoryBudgetManifest, MemoryClass, MemoryClassBudget, MemoryPreset, ReclaimPriority,
    };

    fn manifest() -> MemoryBudgetManifest {
        // A bare manifest (no class budgets) keeps these resolver/provenance unit
        // tests focused on precedence; they intentionally drive the global limit
        // down to tiny byte values, which a fully populated preset would reject.
        use crate::domain::policy::{ExecutionBudget, ResidencyPolicy};
        MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            ResidencyPolicy::new(4 * 1024 * 1024 * 1024),
        )
    }

    // --- Invariant 4: effective report records override provenance, last writer wins
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

    // --- negative pair: removing the higher layer changes the effective value
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
            with_top.residency.global_hard_limit_bytes,
            without_top.residency.global_hard_limit_bytes,
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

    // --- validation fail-closed: reserved <= soft <= hard
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

    // ---- validation hardening: a per-axis hard cap may never exceed global ----
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

    // ---- scenario B: resource_group structural validation ----

    use crate::domain::policy::{ResidencyPolicy, ResourceGroup};

    /// A coherent resource group used as the baseline that the rejection cases
    /// each mutate by exactly one field, so a rejection is attributable.
    fn good_group() -> ResourceGroup {
        ResourceGroup {
            reserved_bytes: 100,
            soft_cap_bytes: 200,
            hard_cap_bytes: 300,
            burst_percent: 50,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 4,
        }
    }

    fn residency_with_group(group: ResourceGroup) -> ResidencyPolicy {
        ResidencyPolicy::new(10_000).with_resource_group("group:a", group)
    }

    // scenario B: a single coherent resource group passes validation.
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

    // scenario B: reserved > soft inside a resource group is rejected as Config.
    #[test]
    fn validate_rejects_resource_group_reserved_above_soft() {
        let mut g = good_group();
        g.reserved_bytes = 250; // > soft 200
        assert!(matches!(
            validate_residency(&residency_with_group(g)),
            Err(MemoryError::Config(_))
        ));
    }

    // scenario B: soft > hard inside a resource group is rejected as Config.
    #[test]
    fn validate_rejects_resource_group_soft_above_hard() {
        let mut g = good_group();
        g.soft_cap_bytes = 400; // > hard 300
        assert!(matches!(
            validate_residency(&residency_with_group(g)),
            Err(MemoryError::Config(_))
        ));
    }

    // scenario B: a group hard cap above the global limit is rejected as Config.
    #[test]
    fn validate_rejects_resource_group_hard_above_global() {
        let mut g = good_group();
        g.soft_cap_bytes = 9_000;
        g.hard_cap_bytes = 20_000; // > global 10_000
        assert!(matches!(
            validate_residency(&residency_with_group(g)),
            Err(MemoryError::Config(_))
        ));
    }

    // scenario B: burst_percent outside 0..=100 is rejected as Config.
    #[test]
    fn validate_rejects_resource_group_burst_percent_over_100() {
        let mut g = good_group();
        g.burst_percent = 101; // out of the documented 0..=100 band
        assert!(matches!(
            validate_residency(&residency_with_group(g)),
            Err(MemoryError::Config(_))
        ));
    }

    // scenario B: the boundary values 0 and 100 are both inside the band.
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

    // Class and resource-group reservations are independent overlapping axes:
    // each axis must fit the global band, but they are not summed together.
    #[test]
    fn resource_group_reserved_has_an_independent_global_floor() {
        let residency =
            ResidencyPolicy::new(10_000).with_headroom_bytes(1_000).with_resource_group(
                "group:a",
                ResourceGroup {
                    reserved_bytes: 5_000,
                    soft_cap_bytes: 6_000,
                    hard_cap_bytes: 7_000,
                    burst_percent: 10,
                    reclaim_priority: ReclaimPriority::Normal,
                    admission_weight: 1,
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

    // scenario D: an owner->group mapping to an undeclared group is rejected.
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
}
