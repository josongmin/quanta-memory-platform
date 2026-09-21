use crate::domain::config::{OverrideRecord, ResolvedBudget};
use crate::domain::policy::{ExecutionBudget, MemoryBudgetManifest, MemoryPreset, ResidencyPolicy};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PressureState {
    Normal,
    Soft,
    Hard,
    Fatal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveMemoryBudgetReport {
    pub preset: MemoryPreset,
    pub execution: ExecutionBudget,
    pub residency: ResidencyPolicy,
    /// Stable string labels of override sources, in precedence order. Retained
    /// as the human-readable provenance summary.
    pub override_sources: Vec<String>,
    /// Structured override provenance: which source last wrote which field.
    /// Empty when the report was built from a bare manifest with no resolver.
    pub override_provenance: Vec<OverrideRecord>,
    /// Monotonic identity of the active policy. Initial construction is 1;
    /// every accepted live reconfiguration increments it atomically.
    pub policy_revision: u64,
    /// Stable source label for the active policy postimage.
    pub active_policy_source: String,
}

impl EffectiveMemoryBudgetReport {
    #[must_use]
    pub fn from_manifest(manifest: MemoryBudgetManifest, override_sources: Vec<String>) -> Self {
        let active_policy_source =
            override_sources.last().cloned().unwrap_or_else(|| "manifest".to_string());
        Self {
            preset: manifest.preset,
            execution: manifest.execution,
            residency: manifest.residency,
            override_sources,
            override_provenance: Vec::new(),
            policy_revision: 1,
            active_policy_source,
        }
    }

    /// Build a report from a resolved budget, carrying full structured
    /// provenance plus a deduplicated ordered label summary.
    #[must_use]
    pub fn from_resolved(execution: ExecutionBudget, resolved: ResolvedBudget) -> Self {
        let mut override_sources = Vec::new();
        for record in &resolved.provenance {
            let label = record.source.label().to_string();
            if !override_sources.contains(&label) {
                override_sources.push(label);
            }
        }
        let active_policy_source =
            override_sources.last().cloned().unwrap_or_else(|| "preset".to_string());
        Self {
            preset: resolved.preset,
            execution,
            residency: resolved.residency,
            override_sources,
            override_provenance: resolved.provenance,
            policy_revision: 1,
            active_policy_source,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReclaimRuntimeStats {
    pub attempts: u128,
    pub successes: u128,
    pub failures: u128,
    pub reported_reclaimed_bytes: u128,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllocatorSnapshot {
    pub allocated_bytes: Option<u64>,
    pub resident_bytes: Option<u64>,
    pub retained_bytes: Option<u64>,
    pub fragmentation_bytes: Option<u64>,
}
