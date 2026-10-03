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
    /// global-hard-vs-reserved-plus-headroom floor. A layer claiming the
    /// [`OverrideSource::RuntimeReconfigure`] provenance is rejected: that
    /// source is minted only by an accepted live policy replacement and must
    /// never be supplied at build time, where its derived ordering would
    /// silently outrank every static layer and mislabel the initial report.
    pub fn resolve(self) -> MemoryResult<ResolvedBudget> {
        let preset = self.manifest.preset.clone();
        let mut residency = self.manifest.residency.clone();
        let mut provenance = Vec::new();

        // The base manifest is always the lowest-authority provenance source.
        provenance.push(OverrideRecord::new(OverrideSource::Preset, "residency.base"));

        // Apply layers in canonical precedence, not call order.
        let mut ordered = self.layers;
        if ordered.iter().any(|(source, _)| matches!(source, OverrideSource::RuntimeReconfigure)) {
            return Err(MemoryError::config(
                "runtime-reconfigure override layers cannot be supplied at build time: that \
                 provenance is minted only by an accepted live policy replacement",
            ));
        }
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

    // Policy maps are bounded trusted configuration: the scheduler and
    // reclaim planning do map-scale work under the broker lock, so an
    // over-capacity dynamic policy is rejected fail-closed before any broker
    // is constructed from it.
    if residency.classes.len() > crate::domain::management_limits::MAX_POLICY_CLASSES_PER_POLICY {
        return Err(MemoryError::config(format!(
            "residency policy declares {} classes; maximum is {}",
            residency.classes.len(),
            crate::domain::management_limits::MAX_POLICY_CLASSES_PER_POLICY
        )));
    }
    if residency.owner_quotas.len() > crate::domain::management_limits::MAX_OWNER_QUOTAS_PER_POLICY
    {
        return Err(MemoryError::config(format!(
            "residency policy declares {} owner quotas; maximum is {}",
            residency.owner_quotas.len(),
            crate::domain::management_limits::MAX_OWNER_QUOTAS_PER_POLICY
        )));
    }
    if residency.resource_groups.len()
        > crate::domain::management_limits::MAX_RESOURCE_GROUPS_PER_POLICY
    {
        return Err(MemoryError::config(format!(
            "residency policy declares {} resource groups; maximum is {}",
            residency.resource_groups.len(),
            crate::domain::management_limits::MAX_RESOURCE_GROUPS_PER_POLICY
        )));
    }
    if residency.owner_groups.len()
        > crate::domain::management_limits::MAX_OWNER_GROUP_MAPPINGS_PER_POLICY
    {
        return Err(MemoryError::config(format!(
            "residency policy declares {} owner-to-group mappings; maximum is {}",
            residency.owner_groups.len(),
            crate::domain::management_limits::MAX_OWNER_GROUP_MAPPINGS_PER_POLICY
        )));
    }

    // The shared burst pool is advertised as a real oversubscription bound
    // (invariant 6): a pool above the global hard limit can never bind and
    // would make the report's bound vacuous.
    if residency.shared_burst_pool_bytes > residency.global_hard_limit_bytes {
        return Err(MemoryError::config(format!(
            "shared burst pool {} exceeds the global hard limit {}",
            residency.shared_burst_pool_bytes, residency.global_hard_limit_bytes
        )));
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
        // A zero-weight group's weighted burst share is provably zero, so a
        // positive burst demand on it would advertise capacity the split can
        // never deliver. Zero weight with zero burst stays a legal no-burst
        // group.
        if policy.admission_weight == 0 && policy.burst_percent > 0 {
            return Err(MemoryError::config(format!(
                "resource group {group} demands burst_percent {} with admission_weight 0: its \
                 weighted burst share is always zero",
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
mod tests;
