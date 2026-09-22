use std::collections::BTreeMap;

use crate::domain::errors::{MemoryError, MemoryResult};
use crate::domain::stats::PressureState;

/// Per-execution resource caps carried by the platform.
///
/// This workspace is a memory-budgeting platform, not an execution engine: there
/// is no query/eval loop here to meter wall-time or subqueries. The platform's
/// job is to *carry and resolve* these caps and to provide the canonical
/// enforcement helpers (`check_*`) the host's execution engine calls at the
/// relevant points. `None` means "unlimited" for that axis. Enforcement is
/// therefore host-driven through these helpers — never silently unenforced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionBudget {
    pub max_wall_time_ms: Option<u64>,
    pub max_subqueries: Option<usize>,
    pub max_trace_edges: Option<usize>,
    pub max_output_bytes: Option<usize>,
    pub max_execution_scratch_bytes: Option<usize>,
}

impl Default for ExecutionBudget {
    fn default() -> Self {
        Self {
            max_wall_time_ms: Some(30_000),
            max_subqueries: Some(1_000),
            max_trace_edges: Some(10_000),
            max_output_bytes: Some(100 * 1024 * 1024),
            max_execution_scratch_bytes: Some(100 * 1024 * 1024),
        }
    }
}

impl ExecutionBudget {
    #[must_use]
    pub fn unlimited() -> Self {
        Self {
            max_wall_time_ms: None,
            max_subqueries: None,
            max_trace_edges: None,
            max_output_bytes: None,
            max_execution_scratch_bytes: None,
        }
    }

    /// The canonical cap check: `used <= cap` for a `Some(cap)`, always OK for an
    /// unlimited (`None`) axis. The host's execution engine calls the `check_*`
    /// wrappers below at the metering points; a breach is a typed
    /// [`MemoryError::Capacity`], never a silent overrun.
    fn check(axis: &str, used: u64, cap: Option<u64>) -> MemoryResult<()> {
        match cap {
            Some(limit) if used > limit => Err(MemoryError::capacity(format!(
                "execution budget exceeded: {axis} used {used} over cap {limit}"
            ))),
            _ => Ok(()),
        }
    }

    /// Check observed wall-clock time (ms) against `max_wall_time_ms`.
    pub fn check_wall_time_ms(&self, used_ms: u64) -> MemoryResult<()> {
        Self::check("wall_time_ms", used_ms, self.max_wall_time_ms)
    }

    /// Check the observed subquery count against `max_subqueries`.
    pub fn check_subqueries(&self, used: usize) -> MemoryResult<()> {
        Self::check("subqueries", used as u64, self.max_subqueries.map(|c| c as u64))
    }

    /// Check the observed trace-edge count against `max_trace_edges`.
    pub fn check_trace_edges(&self, used: usize) -> MemoryResult<()> {
        Self::check("trace_edges", used as u64, self.max_trace_edges.map(|c| c as u64))
    }

    /// Check observed output bytes against `max_output_bytes`.
    pub fn check_output_bytes(&self, used: usize) -> MemoryResult<()> {
        Self::check("output_bytes", used as u64, self.max_output_bytes.map(|c| c as u64))
    }

    /// Check observed execution scratch bytes against `max_execution_scratch_bytes`.
    pub fn check_execution_scratch_bytes(&self, used: usize) -> MemoryResult<()> {
        Self::check(
            "execution_scratch_bytes",
            used as u64,
            self.max_execution_scratch_bytes.map(|c| c as u64),
        )
    }
}

#[cfg(test)]
mod execution_budget_tests {
    use super::*;

    #[test]
    fn check_helpers_admit_within_cap_and_reject_over_cap() {
        let budget = ExecutionBudget::default(); // output/scratch caps = 100 MiB, subqueries 1_000
                                                 // within cap: ok
        budget.check_output_bytes(50 * 1024 * 1024).expect("under output cap");
        budget.check_subqueries(1_000).expect("exactly at subquery cap is allowed");
        // over cap: typed capacity error (never a silent overrun)
        assert!(matches!(
            budget.check_output_bytes(200 * 1024 * 1024),
            Err(MemoryError::Capacity(_))
        ));
        assert!(matches!(budget.check_subqueries(1_001), Err(MemoryError::Capacity(_))));
        assert!(matches!(budget.check_wall_time_ms(30_001), Err(MemoryError::Capacity(_))));
        assert!(matches!(budget.check_trace_edges(10_001), Err(MemoryError::Capacity(_))));
        assert!(matches!(
            budget.check_execution_scratch_bytes(200 * 1024 * 1024),
            Err(MemoryError::Capacity(_))
        ));
    }

    #[test]
    fn unlimited_budget_admits_everything() {
        let budget = ExecutionBudget::unlimited();
        budget.check_output_bytes(usize::MAX).expect("unlimited output");
        budget.check_subqueries(usize::MAX).expect("unlimited subqueries");
        budget.check_wall_time_ms(u64::MAX).expect("unlimited wall time");
        budget.check_trace_edges(usize::MAX).expect("unlimited trace edges");
        budget.check_execution_scratch_bytes(usize::MAX).expect("unlimited scratch");
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryClass {
    ExecutionScratch,
    MemoResidency,
    TraceIndex,
    ArtifactCache,
    EvidenceCache,
    ManifestResidency,
    ReasoningGraphResidency,
    PtaResumeState,
    PtaAuthorityState,
    PtaPortableBundle,
    ReopenStore,
    SessionTranscript,
    GenerationMetadata,
    Headroom,
    SpillFileCache,
    ExecutionScratchRevocable,
    ParserResidency,
    IdeWorkingSet,
}

impl MemoryClass {
    /// Classes every valid residency policy must contain explicitly.
    /// `ResidencyPolicy::new` materializes these with zero capacity; presets or
    /// callers then opt individual classes into non-zero budgets.
    #[must_use]
    pub fn mandatory() -> Vec<Self> {
        vec![
            Self::ExecutionScratch,
            Self::MemoResidency,
            Self::TraceIndex,
            Self::ArtifactCache,
            Self::EvidenceCache,
            Self::ManifestResidency,
            Self::ReasoningGraphResidency,
            Self::PtaResumeState,
            Self::PtaAuthorityState,
            Self::PtaPortableBundle,
            Self::ReopenStore,
            Self::SessionTranscript,
            Self::GenerationMetadata,
            Self::Headroom,
        ]
    }

    /// Optional taxonomy reserved for opt-in capabilities. These classes are
    /// not materialized by `ResidencyPolicy::new`; reserving against one before
    /// an explicit budget is configured is denied by the ledger.
    #[must_use]
    pub fn future() -> Vec<Self> {
        vec![
            Self::SpillFileCache,
            Self::ExecutionScratchRevocable,
            Self::ParserResidency,
            Self::IdeWorkingSet,
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryClassBudget {
    pub reserved_bytes: u64,
    pub soft_cap_bytes: u64,
    pub hard_cap_bytes: u64,
    pub borrow_from_shared_pool: bool,
    pub reclaim_priority: ReclaimPriority,
}

impl MemoryClassBudget {
    #[must_use]
    pub fn zero(reclaim_priority: ReclaimPriority) -> Self {
        Self {
            reserved_bytes: 0,
            soft_cap_bytes: 0,
            hard_cap_bytes: 0,
            borrow_from_shared_pool: false,
            reclaim_priority,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerQuota {
    pub soft_cap_bytes: u64,
    pub hard_cap_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceGroup {
    pub reserved_bytes: u64,
    pub soft_cap_bytes: u64,
    pub hard_cap_bytes: u64,
    pub burst_percent: u8,
    pub reclaim_priority: ReclaimPriority,
    pub admission_weight: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReclaimPriority {
    First,
    Normal,
    Last,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidencyPolicy {
    pub global_hard_limit_bytes: u64,
    pub headroom_bytes: u64,
    pub shared_burst_pool_bytes: u64,
    pub classes: BTreeMap<MemoryClass, MemoryClassBudget>,
    pub owner_quotas: BTreeMap<String, OwnerQuota>,
    pub resource_groups: BTreeMap<String, ResourceGroup>,
    /// Maps an owner to the resource group whose weighted-fair admission gates
    /// that owner's reservations. An owner with no mapping is admitted purely on
    /// its own quota + the class/global caps (no group admission).
    pub owner_groups: BTreeMap<String, String>,
    pub initial_pressure: PressureState,
}

impl ResidencyPolicy {
    /// Construct a total, fail-closed policy.
    ///
    /// Every mandatory class is present with an explicit zero budget. Future
    /// classes remain absent and unusable until configured with
    /// [`Self::with_class_budget`].
    #[must_use]
    pub fn new(global_hard_limit_bytes: u64) -> Self {
        // A raw policy is structurally total over every currently mandatory
        // class. Zero is deliberately fail-closed: callers must opt classes
        // into capacity explicitly, while future classes remain absent and are
        // denied by the ledger until they are configured.
        let classes = MemoryClass::mandatory()
            .into_iter()
            .map(|class| (class, MemoryClassBudget::zero(ReclaimPriority::Normal)))
            .collect();
        Self {
            global_hard_limit_bytes,
            headroom_bytes: 0,
            shared_burst_pool_bytes: 0,
            classes,
            owner_quotas: BTreeMap::new(),
            resource_groups: BTreeMap::new(),
            owner_groups: BTreeMap::new(),
            initial_pressure: PressureState::Normal,
        }
    }

    #[must_use]
    pub fn with_headroom_bytes(mut self, headroom_bytes: u64) -> Self {
        self.headroom_bytes = headroom_bytes;
        self
    }

    #[must_use]
    pub fn with_shared_burst_pool_bytes(mut self, shared_burst_pool_bytes: u64) -> Self {
        self.shared_burst_pool_bytes = shared_burst_pool_bytes;
        self
    }

    #[must_use]
    pub fn with_class_budget(mut self, class: MemoryClass, budget: MemoryClassBudget) -> Self {
        self.classes.insert(class, budget);
        self
    }

    #[must_use]
    pub fn with_owner_quota(mut self, owner: impl Into<String>, quota: OwnerQuota) -> Self {
        self.owner_quotas.insert(owner.into(), quota);
        self
    }

    #[must_use]
    pub fn with_resource_group(mut self, group: impl Into<String>, policy: ResourceGroup) -> Self {
        self.resource_groups.insert(group.into(), policy);
        self
    }

    /// Map an owner to the resource group that gates its reservations via the
    /// weighted-fair scheduler. The group must also be declared via
    /// [`Self::with_resource_group`]; `validate_residency` rejects a mapping to
    /// an unknown group.
    #[must_use]
    pub fn with_owner_group(mut self, owner: impl Into<String>, group: impl Into<String>) -> Self {
        self.owner_groups.insert(owner.into(), group.into());
        self
    }

    #[must_use]
    pub fn with_initial_pressure(mut self, pressure: PressureState) -> Self {
        self.initial_pressure = pressure;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryPreset {
    Development,
    Ci,
    Production,
    Interactive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryBudgetManifest {
    pub preset: MemoryPreset,
    pub execution: ExecutionBudget,
    pub residency: ResidencyPolicy,
}

impl MemoryBudgetManifest {
    #[must_use]
    pub fn new(
        preset: MemoryPreset,
        execution: ExecutionBudget,
        residency: ResidencyPolicy,
    ) -> Self {
        Self { preset, execution, residency }
    }

    #[must_use]
    pub fn for_preset(preset: MemoryPreset) -> Self {
        let global_hard_limit_bytes = match preset {
            MemoryPreset::Development => 256 * 1024 * 1024,
            MemoryPreset::Ci => 512 * 1024 * 1024,
            MemoryPreset::Production => 4 * 1024 * 1024 * 1024,
            MemoryPreset::Interactive => 1024 * 1024 * 1024,
        };
        Self {
            preset,
            execution: ExecutionBudget::default(),
            residency: default_residency(global_hard_limit_bytes),
        }
    }

    #[must_use]
    pub fn with_execution(mut self, execution: ExecutionBudget) -> Self {
        self.execution = execution;
        self
    }

    #[must_use]
    pub fn with_residency(mut self, residency: ResidencyPolicy) -> Self {
        self.residency = residency;
        self
    }
}

/// Build the default residency policy for a preset's global hard limit.
///
/// Class-budget model (the default split — deterministic in the global limit;
/// tune per deployment):
///
/// - `headroom_bytes = global / 16`: a reserved emergency band that only the
///   [`MemoryClass::Headroom`] class may draw into (see ledger invariant 5).
/// - `shared_burst_pool_bytes = global / 8`: the bounded oversubscription band
///   that elastic classes (`borrow_from_shared_pool = true`) borrow from once
///   they exceed their own hard cap (ledger invariant 6).
/// - each non-`Headroom` mandatory class receives a weighted slice of the
///   distributable band (`global - headroom`) as its hard cap, with
///   `soft_cap = 3/4 * hard_cap` and `reserved = 0`. The `Headroom` class owns
///   the reserved band exactly.
///
/// Every hard cap is therefore strictly below the global limit, so class
/// isolation is real (a class is denied at its own cap and may only oversubscribe
/// through the finite shared pool), and the policy passes
/// [`crate::domain::config::validate_residency`] for any positive global limit.
fn default_residency(global_hard_limit_bytes: u64) -> ResidencyPolicy {
    // (class, relative weight, elastic?, reclaim priority)
    const WEIGHTS: &[(MemoryClass, u64, bool, ReclaimPriority)] = &[
        (MemoryClass::ExecutionScratch, 16, true, ReclaimPriority::First),
        (MemoryClass::MemoResidency, 12, false, ReclaimPriority::Normal),
        (MemoryClass::TraceIndex, 10, true, ReclaimPriority::First),
        (MemoryClass::ArtifactCache, 12, true, ReclaimPriority::First),
        (MemoryClass::EvidenceCache, 10, true, ReclaimPriority::First),
        (MemoryClass::ManifestResidency, 4, false, ReclaimPriority::Last),
        (MemoryClass::ReasoningGraphResidency, 12, true, ReclaimPriority::Normal),
        (MemoryClass::PtaResumeState, 4, false, ReclaimPriority::Last),
        (MemoryClass::PtaAuthorityState, 4, false, ReclaimPriority::Last),
        (MemoryClass::PtaPortableBundle, 4, false, ReclaimPriority::Last),
        (MemoryClass::ReopenStore, 4, false, ReclaimPriority::Last),
        (MemoryClass::SessionTranscript, 6, false, ReclaimPriority::Last),
        (MemoryClass::GenerationMetadata, 4, false, ReclaimPriority::Last),
    ];

    let headroom_bytes = global_hard_limit_bytes / 16;
    let shared_burst_pool_bytes = global_hard_limit_bytes / 8;
    let distributable = global_hard_limit_bytes.saturating_sub(headroom_bytes);
    let total_weight: u64 = WEIGHTS.iter().map(|entry| entry.1).sum();

    let mut residency = ResidencyPolicy::new(global_hard_limit_bytes)
        .with_headroom_bytes(headroom_bytes)
        .with_shared_burst_pool_bytes(shared_burst_pool_bytes);

    for (class, weight, elastic, priority) in WEIGHTS {
        // Compute the weighted share in u128 so `distributable * weight` cannot
        // saturate and UNDER-report the cap (saturation here rounds the share DOWN,
        // the unsafe direction). u128 cannot overflow for u64 * u64.
        let hard_cap_bytes = u64::try_from(
            u128::from(distributable) * u128::from(*weight) / u128::from(total_weight.max(1)),
        )
        .unwrap_or(u64::MAX);
        let soft_cap_bytes = hard_cap_bytes / 4 * 3;
        residency = residency.with_class_budget(
            class.clone(),
            MemoryClassBudget {
                reserved_bytes: 0,
                soft_cap_bytes,
                hard_cap_bytes,
                borrow_from_shared_pool: *elastic,
                reclaim_priority: *priority,
            },
        );
    }

    // The Headroom class owns the reserved band and never borrows.
    residency.with_class_budget(
        MemoryClass::Headroom,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: headroom_bytes,
            hard_cap_bytes: headroom_bytes,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Last,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::config::validate_residency;

    fn presets() -> [MemoryPreset; 4] {
        [
            MemoryPreset::Development,
            MemoryPreset::Ci,
            MemoryPreset::Production,
            MemoryPreset::Interactive,
        ]
    }

    // ---- #2: presets must materialize real, isolating class budgets ----

    #[test]
    fn for_preset_populates_every_mandatory_class() {
        for preset in presets() {
            let manifest = MemoryBudgetManifest::for_preset(preset.clone());
            for class in MemoryClass::mandatory() {
                assert!(
                    manifest.residency.classes.contains_key(&class),
                    "{preset:?} must define a budget for mandatory class {class:?}"
                );
            }
            assert_eq!(
                manifest.residency.classes.len(),
                MemoryClass::mandatory().len(),
                "{preset:?} defines exactly the mandatory class set"
            );
        }
    }

    #[test]
    fn for_preset_is_structurally_valid() {
        for preset in presets() {
            let manifest = MemoryBudgetManifest::for_preset(preset.clone());
            validate_residency(&manifest.residency)
                .unwrap_or_else(|e| panic!("{preset:?} preset must be a valid residency: {e}"));
        }
    }

    #[test]
    fn for_preset_class_caps_isolate_below_global() {
        // The point of #2: a class hard cap must actually be reachable (i.e.
        // strictly below the global limit), otherwise class isolation is nominal.
        for preset in presets() {
            let r = MemoryBudgetManifest::for_preset(preset.clone()).residency;
            for (class, budget) in &r.classes {
                assert!(
                    budget.hard_cap_bytes <= r.global_hard_limit_bytes,
                    "{preset:?} {class:?} hard cap exceeds global"
                );
                assert!(
                    budget.reserved_bytes <= budget.soft_cap_bytes
                        && budget.soft_cap_bytes <= budget.hard_cap_bytes,
                    "{preset:?} {class:?} must keep reserved<=soft<=hard"
                );
            }
            // At least one elastic class can borrow, and a shared pool exists.
            assert!(r.shared_burst_pool_bytes > 0, "{preset:?} must provision a burst pool");
            assert!(
                r.classes.values().any(|b| b.borrow_from_shared_pool),
                "{preset:?} must mark at least one class elastic"
            );
            assert!(r.headroom_bytes > 0, "{preset:?} must reserve headroom");
        }
    }

    #[test]
    fn for_preset_scales_caps_with_global_limit() {
        // A larger preset must grant a larger (or equal) cap to the same class —
        // the split is a deterministic function of the global limit.
        let dev = MemoryBudgetManifest::for_preset(MemoryPreset::Development).residency;
        let prod = MemoryBudgetManifest::for_preset(MemoryPreset::Production).residency;
        let dev_cap = dev.classes[&MemoryClass::ArtifactCache].hard_cap_bytes;
        let prod_cap = prod.classes[&MemoryClass::ArtifactCache].hard_cap_bytes;
        assert!(prod_cap > dev_cap, "production must grant a larger ArtifactCache cap than dev");
    }

    // ---- scenario B: weighted split never over-allocates the distributable ----

    #[test]
    fn for_preset_weighted_split_never_over_allocates_distributable() {
        // The non-Headroom class hard caps are weighted slices of
        // `global - headroom`; their sum must stay within that distributable
        // band (slices floor-divide, so the sum can only undershoot, never
        // exceed). This is the real coherence property of the split: it cannot
        // promise more reachable bytes than the global limit leaves available.
        for preset in presets() {
            let r = MemoryBudgetManifest::for_preset(preset.clone()).residency;
            let distributable = r.global_hard_limit_bytes - r.headroom_bytes;
            let non_headroom_sum: u64 = r
                .classes
                .iter()
                .filter(|(class, _)| **class != MemoryClass::Headroom)
                .map(|(_, b)| b.hard_cap_bytes)
                .sum();
            assert!(
                non_headroom_sum <= distributable,
                "{preset:?} weighted class caps {non_headroom_sum} over-allocate distributable {distributable}"
            );
        }
    }

    #[test]
    fn for_preset_headroom_class_owns_exactly_the_reserved_band() {
        // The Headroom class hard cap must equal headroom_bytes exactly, and the
        // reserved band (headroom) plus the distributable band must reconstruct
        // the global limit — no byte is double-counted across the two bands.
        for preset in presets() {
            let r = MemoryBudgetManifest::for_preset(preset.clone()).residency;
            let headroom_cap = r.classes[&MemoryClass::Headroom].hard_cap_bytes;
            assert_eq!(
                headroom_cap, r.headroom_bytes,
                "{preset:?} Headroom class must own exactly the reserved band"
            );
            let distributable = r.global_hard_limit_bytes - r.headroom_bytes;
            assert_eq!(
                headroom_cap + distributable,
                r.global_hard_limit_bytes,
                "{preset:?} reserved band + distributable must reconstruct global"
            );
        }
    }

    #[test]
    fn for_preset_no_single_class_cap_plus_headroom_exceeds_global() {
        // resource_group + headroom interplay at the class level: even the
        // largest single class cap, taken together with the reserved headroom
        // band, never exceeds the global limit. So a class saturating its cap
        // can never crowd out the emergency headroom band.
        for preset in presets() {
            let r = MemoryBudgetManifest::for_preset(preset.clone()).residency;
            let max_non_headroom = r
                .classes
                .iter()
                .filter(|(class, _)| **class != MemoryClass::Headroom)
                .map(|(_, b)| b.hard_cap_bytes)
                .max()
                .unwrap_or(0);
            assert!(
                max_non_headroom + r.headroom_bytes <= r.global_hard_limit_bytes,
                "{preset:?} a saturated class cap plus headroom must fit under global"
            );
        }
    }

    // ---- scenario B: degenerate-global edges must not panic and must validate ----

    #[test]
    fn default_residency_global_one_does_not_underflow_and_validates() {
        // global = 1: headroom = 1/16 = 0, burst = 1/8 = 0, distributable = 1.
        // No division-by-zero, no underflow, and the policy still validates.
        let r = default_residency(1);
        validate_residency(&r).expect("global=1 residency must still validate");
        // distributable - headroom reconstruction holds even at the floor.
        assert_eq!(r.headroom_bytes, 0, "global=1 yields a zero headroom band");
        for (class, budget) in &r.classes {
            assert!(
                budget.reserved_bytes <= budget.soft_cap_bytes
                    && budget.soft_cap_bytes <= budget.hard_cap_bytes,
                "{class:?} must keep reserved<=soft<=hard even at global=1"
            );
            assert!(
                budget.hard_cap_bytes <= r.global_hard_limit_bytes,
                "{class:?} cap must stay within global=1"
            );
        }
    }

    #[test]
    fn default_residency_small_globals_all_validate() {
        // Sweep small globals where floor division dominates; each must build a
        // structurally valid policy with no panic, underflow, or div-by-zero.
        for global in [1u64, 2, 7, 15, 16, 17, 31, 64, 255] {
            let r = default_residency(global);
            validate_residency(&r)
                .unwrap_or_else(|e| panic!("global={global} residency must validate: {e}"));
            assert_eq!(
                r.global_hard_limit_bytes, global,
                "global={global} must be preserved verbatim"
            );
        }
    }

    // ---- scenario B: constructed-policy floor and burst-pool coherence ----

    #[test]
    fn validate_rejects_constructed_headroom_above_global() {
        // A hand-built policy whose headroom alone exceeds the global limit must
        // be rejected by the reserved+headroom floor check (no classes needed).
        let r = ResidencyPolicy::new(1_000).with_headroom_bytes(2_000);
        // floor = sum(reserved=0) + headroom 2_000 > global 1_000 -> invalid.
        assert!(matches!(
            validate_residency(&r),
            Err(crate::domain::errors::MemoryError::Config(_))
        ));
    }

    #[test]
    fn validate_accepts_headroom_equal_to_global() {
        // Boundary: headroom == global is the exact floor (reserved 0), which is
        // permitted; only strictly-below-floor is rejected.
        let r = ResidencyPolicy::new(1_000).with_headroom_bytes(1_000);
        assert!(validate_residency(&r).is_ok());
    }

    #[test]
    fn borrow_flag_with_zero_shared_pool_is_valid_and_preserved() {
        // Validation-only: borrow_from_shared_pool = true with an empty pool is
        // a coherent policy. Admission behavior is covered by the public-broker
        // contract in tests/policy_contracts.rs, not here.
        let elastic = MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: 600,
            hard_cap_bytes: 800,
            borrow_from_shared_pool: true,
            reclaim_priority: ReclaimPriority::First,
        };
        let r = ResidencyPolicy::new(10_000)
            .with_shared_burst_pool_bytes(0)
            .with_class_budget(MemoryClass::ExecutionScratch, elastic);

        validate_residency(&r).expect("borrow=true with empty pool is a valid policy");
        assert_eq!(r.shared_burst_pool_bytes, 0, "the pool stays empty");
        assert!(
            r.classes[&MemoryClass::ExecutionScratch].borrow_from_shared_pool,
            "the borrow flag is preserved"
        );
    }
}
