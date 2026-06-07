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
}

impl EffectiveMemoryBudgetReport {
    #[must_use]
    pub fn from_manifest(manifest: MemoryBudgetManifest, override_sources: Vec<String>) -> Self {
        Self {
            preset: manifest.preset,
            execution: manifest.execution,
            residency: manifest.residency,
            override_sources,
            override_provenance: Vec::new(),
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
        Self {
            preset: resolved.preset,
            execution,
            residency: resolved.residency,
            override_sources,
            override_provenance: resolved.provenance,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllocatorSnapshot {
    pub allocated_bytes: Option<u64>,
    pub resident_bytes: Option<u64>,
    pub retained_bytes: Option<u64>,
    pub fragmentation_bytes: Option<u64>,
}
