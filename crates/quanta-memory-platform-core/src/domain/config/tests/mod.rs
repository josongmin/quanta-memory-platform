mod cases_01;

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

// --- negative pair: removing the higher layer changes the effective value

// --- validation fail-closed: reserved <= soft <= hard

// ---- validation hardening: a per-axis hard cap may never exceed global ----

// ---- policy cardinality bounds (ENG-05 validation half): the policy
// maps are bounded trusted configuration, so scheduler clones and
// reclaim planning under the broker lock stay O(bounded). Latency
// claims remain with the scaling rail; these are logical bookkeeping
// bounds like the other MAX_* management limits.

// ---- provenance authority: RuntimeReconfigure is a runtime-minted
// provenance only. A build-time layer claiming it would silently outrank
// every static layer (derived Ord) and mislabel the initial report as an
// accepted live replacement, so resolution rejects it fail-closed.

// ---- invariant-6 coherence: the shared burst pool is advertised as a
// real oversubscription bound, so it may not exceed the global hard limit.

// ---- burst-share coherence: a zero-weight group demands no burst, so a
// positive burst_percent on it would silently advertise capacity the
// weighted split can never deliver.

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

// scenario B: reserved > soft inside a resource group is rejected as Config.

// scenario B: soft > hard inside a resource group is rejected as Config.

// scenario B: a group hard cap above the global limit is rejected as Config.

// scenario B: burst_percent outside 0..=100 is rejected as Config.

// scenario B: the boundary values 0 and 100 are both inside the band.

// Class and resource-group reservations are independent overlapping axes:
// each axis must fit the global band, but they are not summed together.

// scenario D: an owner->group mapping to an undeclared group is rejected.
