#[cfg(feature = "live_control")]
use crate::api::control::LiveProcessMemoryBroker;
use crate::domain::config::EffectiveBudgetResolver;
use crate::domain::config::{BudgetOverrideLayer, OverrideSource};
use crate::domain::policy::MemoryBudgetManifest;
use crate::domain::stats::EffectiveMemoryBudgetReport;
#[cfg(feature = "live_control")]
use crate::ports::reclaim::MemoryReclaimer;
#[cfg(feature = "live_control")]
use crate::ports::telemetry::{MemoryEventSink, NoopMemoryEventSink};
use crate::{MemoryError, MemoryPreset, MemoryResult};
#[cfg(feature = "live_control")]
use std::sync::Arc;

#[derive(Clone)]
#[cfg_attr(not(feature = "live_control"), derive(Default))]
pub struct ProcessMemoryBrokerBuilder {
    manifest: Option<MemoryBudgetManifest>,
    layers: Vec<(OverrideSource, BudgetOverrideLayer)>,
    #[cfg(feature = "live_control")]
    reclaimer: Option<Arc<dyn MemoryReclaimer>>,
    #[cfg(feature = "live_control")]
    event_sink: Arc<dyn MemoryEventSink>,
}

#[cfg(feature = "live_control")]
impl Default for ProcessMemoryBrokerBuilder {
    fn default() -> Self {
        Self {
            manifest: None,
            layers: Vec::new(),
            #[cfg(feature = "live_control")]
            reclaimer: None,
            #[cfg(feature = "live_control")]
            event_sink: Arc::new(NoopMemoryEventSink),
        }
    }
}

impl std::fmt::Debug for ProcessMemoryBrokerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("ProcessMemoryBrokerBuilder");
        debug.field("manifest", &self.manifest).field("layers", &self.layers);
        #[cfg(feature = "live_control")]
        debug.field("has_reclaimer", &self.reclaimer.is_some());
        #[cfg(feature = "live_control")]
        debug.field("event_sink", &"bounded MemoryEventSink");
        debug.finish()
    }
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

    /// Install the bounded reclaim callback used by soft-threshold and hard
    /// denial slow paths. The callback is invoked without the broker mutex.
    #[cfg(feature = "live_control")]
    #[must_use]
    pub fn with_reclaimer(mut self, reclaimer: Arc<dyn MemoryReclaimer>) -> Self {
        self.reclaimer = Some(reclaimer);
        self
    }

    /// Install an advisory, bounded-cardinality operational event sink. The
    /// broker invokes it outside its mutex and contains callback panics.
    #[cfg(feature = "live_control")]
    #[must_use]
    pub fn with_event_sink(mut self, event_sink: Arc<dyn MemoryEventSink>) -> Self {
        self.event_sink = event_sink;
        self
    }

    /// Resolve config precedence and build the enforced accounting-backed broker.
    ///
    /// This runs the full effective-budget resolution (manifest + override
    /// layers), validates the residency policy, emits an
    /// [`EffectiveMemoryBudgetReport`] with override provenance, and only then
    /// constructs the broker. Fails closed with a typed error if the resolved
    /// policy is invalid.
    #[cfg(feature = "live_control")]
    pub fn build(self) -> MemoryResult<LiveProcessMemoryBroker> {
        let reclaimer = self.reclaimer.clone();
        let event_sink = Arc::clone(&self.event_sink);
        let report = self.resolve_report()?;
        LiveProcessMemoryBroker::from_report_with_reclaimer_and_events(
            report, reclaimer, event_sink,
        )
    }

    /// Compatibility alias for callers migrating to the enforced default.
    #[cfg(feature = "live_control")]
    pub fn build_live(self) -> MemoryResult<LiveProcessMemoryBroker> {
        self.build()
    }

    /// Resolve the same manifest and override chain as [`Self::build`] but return
    /// only the effective report. The report carries no runtime, scratch, store,
    /// or accounting authority.
    pub fn build_report_only(self) -> MemoryResult<EffectiveMemoryBudgetReport> {
        self.resolve_report()
    }

    fn resolve_report(self) -> MemoryResult<EffectiveMemoryBudgetReport> {
        let manifest = self
            .manifest
            .ok_or_else(|| MemoryError::build("memory budget manifest is required"))?;
        let execution = manifest.execution.clone();

        let mut resolver = EffectiveBudgetResolver::from_manifest(manifest);
        for (source, layer) in self.layers {
            resolver = resolver.with_layer(source, layer);
        }
        let resolved = resolver.resolve()?;
        Ok(EffectiveMemoryBudgetReport::from_resolved(execution, resolved))
    }

    pub fn with_preset(self, preset: MemoryPreset) -> Self {
        let manifest = MemoryBudgetManifest::for_preset(preset);
        self.with_manifest(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    // scenario C-builder: both products fail closed without a manifest.
    #[test]
    fn build_without_manifest_returns_build_error() {
        let report = ProcessMemoryBrokerBuilder::new().build_report_only();
        assert!(matches!(report, Err(MemoryError::Build(_))));
        #[cfg(feature = "live_control")]
        assert!(matches!(ProcessMemoryBrokerBuilder::new().build(), Err(MemoryError::Build(_))));
    }

    // scenario C-builder: a structurally INVALID residency (class hard cap above
    // the global hard limit) must be rejected by build() with a typed Config
    // error, mirroring build/from_report — report resolution can never
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
        let result = ProcessMemoryBrokerBuilder::new().with_manifest(manifest).build_report_only();
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
                ProcessMemoryBrokerBuilder::new().with_manifest(manifest).build_report_only(),
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
        let report = ProcessMemoryBrokerBuilder::new()
            .with_manifest(manifest)
            .build_report_only()
            .expect("a structurally valid manifest must build");
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
            ProcessMemoryBrokerBuilder::new().with_manifest(manifest).build_report_only(),
            Err(MemoryError::Config(_))
        ));
    }

    #[test]
    fn report_only_builder_applies_override_layers() {
        let report = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .with_override_layer(
                OverrideSource::TestFixture,
                BudgetOverrideLayer::empty().with_global_hard_limit_bytes(5_000_000_000),
            )
            .build_report_only()
            .expect("override remains structurally valid");

        assert_eq!(report.residency.global_hard_limit_bytes, 5_000_000_000);
        assert_eq!(
            report.override_provenance.last().map(|record| record.source),
            Some(OverrideSource::TestFixture)
        );
    }
}
