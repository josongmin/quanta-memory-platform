use crate::api::broker::BasicProcessMemoryBroker;
#[cfg(feature = "live_control")]
use crate::api::control::LiveProcessMemoryBroker;
#[cfg(feature = "live_control")]
use crate::domain::config::EffectiveBudgetResolver;
use crate::domain::config::{validate_residency, BudgetOverrideLayer, OverrideSource};
use crate::domain::policy::MemoryBudgetManifest;
use crate::domain::stats::EffectiveMemoryBudgetReport;
use crate::{MemoryError, MemoryPreset, MemoryResult};

#[derive(Clone, Debug, Default)]
pub struct ProcessMemoryBrokerBuilder {
    manifest: Option<MemoryBudgetManifest>,
    layers: Vec<(OverrideSource, BudgetOverrideLayer)>,
}

impl ProcessMemoryBrokerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_manifest(mut self, manifest: MemoryBudgetManifest) -> Self {
        self.manifest = Some(manifest);
        self
    }

    /// Register a precedence override layer. Layers are reordered to canonical
    /// precedence (preset < manifest < env < cli < test_fixture) at build time.
    #[must_use]
    pub fn with_override_layer(
        mut self,
        source: OverrideSource,
        layer: BudgetOverrideLayer,
    ) -> Self {
        self.layers.push((source, layer));
        self
    }

    pub fn build(self) -> MemoryResult<BasicProcessMemoryBroker> {
        let manifest = self
            .manifest
            .clone()
            .ok_or_else(|| MemoryError::build("memory budget manifest is required"))?;
        // Mirror `build_live`/`from_report`: a non-live broker must never be able
        // to come up advertising a structurally impossible policy (e.g. a class
        // hard cap above the global limit, or soft > hard). Validate the
        // residency BEFORE we emit any report so the build fails closed with a
        // typed `Config` error instead of handing out a misleading report.
        validate_residency(&manifest.residency)?;
        let report = EffectiveMemoryBudgetReport::from_manifest(
            manifest,
            vec!["builder.with_manifest".to_string()],
        );
        Ok(BasicProcessMemoryBroker::new(report))
    }

    /// Resolve config precedence and build a live accounting-backed broker.
    ///
    /// This runs the full effective-budget resolution (manifest + override
    /// layers), validates the residency policy, emits an
    /// [`EffectiveMemoryBudgetReport`] with override provenance, and only then
    /// constructs the broker. Fails closed with a typed error if the resolved
    /// policy is invalid.
    #[cfg(feature = "live_control")]
    pub fn build_live(self) -> MemoryResult<LiveProcessMemoryBroker> {
        let manifest = self
            .manifest
            .ok_or_else(|| MemoryError::build("memory budget manifest is required"))?;
        let execution = manifest.execution.clone();

        let mut resolver = EffectiveBudgetResolver::from_manifest(manifest);
        for (source, layer) in self.layers {
            resolver = resolver.with_layer(source, layer);
        }
        let resolved = resolver.resolve()?;
        let report = EffectiveMemoryBudgetReport::from_resolved(execution, resolved);
        LiveProcessMemoryBroker::from_report(report)
    }

    pub fn with_preset(self, preset: MemoryPreset) -> Self {
        let manifest = MemoryBudgetManifest::for_preset(preset);
        self.with_manifest(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::broker::ProcessMemoryBroker;
    use crate::domain::policy::{
        ExecutionBudget, MemoryClass, MemoryClassBudget, ReclaimPriority, ResidencyPolicy,
    };

    fn class_budget(reserved: u64, soft: u64, hard: u64) -> MemoryClassBudget {
        MemoryClassBudget {
            reserved_bytes: reserved,
            soft_cap_bytes: soft,
            hard_cap_bytes: hard,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        }
    }

    // scenario C-builder: build() with NO manifest fails closed (typed Build).
    #[test]
    fn build_without_manifest_returns_build_error() {
        let err = ProcessMemoryBrokerBuilder::new().build();
        assert!(matches!(err, Err(MemoryError::Build(_))), "missing manifest is a Build error");
    }

    // scenario C-builder: a structurally INVALID residency (class hard cap above
    // the global hard limit) must be rejected by build() with a typed Config
    // error, mirroring build_live/from_report — the non-live broker can never
    // come up advertising an impossible policy.
    #[test]
    fn build_with_invalid_residency_returns_config_error() {
        // ArtifactCache hard cap 2_000_000 exceeds the global hard limit
        // 1_000_000: validate_residency rejects this as structurally incoherent.
        let bad_residency = ResidencyPolicy::new(1_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 1_500_000, 2_000_000));
        let manifest = MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            bad_residency,
        );
        let result = ProcessMemoryBrokerBuilder::new().with_manifest(manifest).build();
        assert!(
            matches!(result, Err(MemoryError::Config(_))),
            "invalid residency must fail build() with a typed Config error, got {result:?}"
        );
    }

    // scenario C-builder: soft > hard is the other structural-invalid axis; it
    // too must be rejected by build() rather than producing a report.
    #[test]
    fn build_with_soft_above_hard_returns_config_error() {
        let bad_residency = ResidencyPolicy::new(1_000_000)
            // soft 700k > hard 600k -> structurally incoherent.
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 700_000, 600_000));
        let manifest = MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            bad_residency,
        );
        assert!(
            matches!(
                ProcessMemoryBrokerBuilder::new().with_manifest(manifest).build(),
                Err(MemoryError::Config(_))
            ),
            "soft>hard must fail build() with a typed Config error"
        );
    }

    // scenario C-builder: a VALID manifest still builds and the resulting broker
    // advertises exactly the residency it was built from (positive pair proving
    // the new validation does not reject honest policies).
    #[test]
    fn build_with_valid_residency_succeeds_and_reports_that_residency() {
        let residency = ResidencyPolicy::new(1_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400_000, 600_000));
        let manifest = MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            residency.clone(),
        );
        let broker = ProcessMemoryBrokerBuilder::new()
            .with_manifest(manifest)
            .build()
            .expect("a structurally valid manifest must build");
        let report = broker.effective_report();
        assert_eq!(
            report.residency.global_hard_limit_bytes, 1_000_000,
            "broker must advertise the residency it was built from"
        );
        assert_eq!(
            report.residency.classes.get(&MemoryClass::ArtifactCache).map(|b| b.hard_cap_bytes),
            Some(600_000),
            "class caps survive the build verbatim"
        );
    }

    #[test]
    fn builder_rejects_manifest_missing_mandatory_class() {
        let mut residency = ResidencyPolicy::new(1_000_000);
        residency.classes.remove(&MemoryClass::ArtifactCache);
        let manifest = MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            residency,
        );

        assert!(matches!(
            ProcessMemoryBrokerBuilder::new().with_manifest(manifest).build(),
            Err(MemoryError::Config(_))
        ));
    }
}
