//! Reusable test fixtures and the top-level [`MemoryTestKit`] entry point.
//!
//! `fixtures/*` owns deterministic budget/fixture builders and the
//! [`MemoryTestKit`] facade that ties the testkit surfaces together for a
//! consumer: it can hand back a fake broker and run the conformance suite without
//! the consumer reaching into `conformance/*` or `fake/*` directly.

use quanta_memory_platform_core::{
    LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryPreset, ProcessMemoryBrokerBuilder,
};

use crate::conformance::{ConformanceSuite, ConformanceVerdict};
use crate::fake::FakeProcessBroker;

/// A deterministic test fixture: a budget manifest plus a fake-pressure toggle.
#[derive(Clone, Debug)]
pub struct MemoryFixture {
    pub manifest: MemoryBudgetManifest,
    pub fake_pressure: bool,
}

/// The top-level testkit entry point.
///
/// Holds a fixture and exposes the reusable surfaces (fake broker + conformance)
/// so a host test obtains everything from one handle.
#[derive(Clone, Debug)]
pub struct MemoryTestKit {
    pub fixture: MemoryFixture,
}

impl MemoryTestKit {
    /// A testkit seeded with the CI preset.
    #[must_use]
    pub fn ci() -> Self {
        MemoryFixtureBuilder::new().build_testkit()
    }

    /// A fresh honest fake broker for this testkit.
    #[must_use]
    pub fn fake_broker(&self) -> FakeProcessBroker {
        let broker = FakeProcessBroker::honest();
        // A scoped test fault enters at construction, so the negative pole
        // exercises the same public kit dispatcher as an honest consumer.
        #[cfg(test)]
        let broker = tests::faulted_broker().unwrap_or(broker);
        if self.fixture.fake_pressure {
            broker.with_pressure(quanta_memory_platform_core::PressureState::Soft)
        } else {
            broker
        }
    }

    /// A real live broker materialized from this fixture's manifest.
    pub fn live_broker(
        &self,
    ) -> quanta_memory_platform_core::MemoryResult<LiveProcessMemoryBroker> {
        ProcessMemoryBrokerBuilder::new().with_manifest(self.fixture.manifest.clone()).build()
    }

    /// Run the reusable broker accounting conformance suite against this kit's
    /// fake broker and return the folded verdict.
    #[must_use]
    pub fn run_conformance(&self) -> ConformanceVerdict {
        let mut broker = self.fake_broker();
        ConformanceSuite::new().run_broker_accounting(&mut broker)
    }

    /// Run the facade-store conformance suite against live broker-backed
    /// portable facade slices derived from this fixture.
    #[must_use]
    pub fn run_facade_store_conformance(&self) -> ConformanceVerdict {
        ConformanceSuite::new().run_facade_store_suite(self)
    }
}

/// Builder for [`MemoryFixture`] / [`MemoryTestKit`].
#[derive(Clone, Debug)]
pub struct MemoryFixtureBuilder {
    manifest: MemoryBudgetManifest,
    fake_pressure: bool,
}

impl Default for MemoryFixtureBuilder {
    fn default() -> Self {
        Self { manifest: MemoryBudgetManifest::for_preset(MemoryPreset::Ci), fake_pressure: false }
    }
}

impl MemoryFixtureBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_manifest(mut self, manifest: MemoryBudgetManifest) -> Self {
        self.manifest = manifest;
        self
    }

    #[must_use]
    pub fn with_fake_pressure(mut self) -> Self {
        self.fake_pressure = true;
        self
    }

    #[must_use]
    pub fn build(self) -> MemoryFixture {
        MemoryFixture { manifest: self.manifest, fake_pressure: self.fake_pressure }
    }

    #[must_use]
    pub fn build_testkit(self) -> MemoryTestKit {
        MemoryTestKit { fixture: self.build() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::ContractFamily;
    use crate::fake::AccountingFidelity;
    use std::cell::Cell;

    thread_local! {
        static FIDELITY: Cell<Option<AccountingFidelity>> = const { Cell::new(None) };
    }

    pub(super) fn faulted_broker() -> Option<FakeProcessBroker> {
        FIDELITY.with(|fidelity| fidelity.get().map(FakeProcessBroker::broken))
    }

    fn with_broken_accounting<T>(run: impl FnOnce() -> T) -> T {
        struct RestoreFidelity(Option<AccountingFidelity>);
        impl Drop for RestoreFidelity {
            fn drop(&mut self) {
                FIDELITY.with(|fidelity| fidelity.set(self.0));
            }
        }
        let previous = FIDELITY.with(|fidelity| {
            fidelity.replace(Some(AccountingFidelity::UnderReportOwnerBytes(100)))
        });
        let _restore = RestoreFidelity(previous);
        run()
    }

    #[test]
    fn testkit_runs_conformance_against_its_fake_and_passes() {
        let kit = MemoryTestKit::ci();
        let verdict = kit.run_conformance();
        assert!(verdict.passed(), "honest fake testkit must pass conformance: {verdict:?}");
        assert!(verdict.violated_families().is_empty());
        with_broken_accounting(|| {
            let rejected = kit.run_conformance();
            assert!(
                !rejected.passed(),
                "the kit must dispatch rejection, not return a constant pass"
            );
            assert!(rejected.violated_families().contains(&ContractFamily::ClassOwnerBalance));
        });
        assert!(kit.run_conformance().passed(), "scoped fault must not escape its negative pole");
    }

    #[test]
    fn fake_pressure_fixture_propagates_pressure_into_the_fake_broker() {
        let kit = MemoryFixtureBuilder::new().with_fake_pressure().build_testkit();
        let broker = kit.fake_broker();
        assert_eq!(broker.pressure(), quanta_memory_platform_core::PressureState::Soft);
        // Negative pair: the default fixture leaves pressure Normal.
        let calm = MemoryTestKit::ci().fake_broker();
        assert_eq!(calm.pressure(), quanta_memory_platform_core::PressureState::Normal);
    }

    #[test]
    fn conformance_verdict_localizes_to_contract_family_not_a_host_package() {
        let verdict = with_broken_accounting(|| MemoryTestKit::ci().run_conformance());
        assert!(!verdict.passed());
        assert!(verdict.violated_families().contains(&ContractFamily::ClassOwnerBalance));
        assert!(!verdict.violated_families().is_empty());
        for family in verdict.violated_families() {
            assert_eq!(family.to_string(), family.label());
            assert!(!family.label().is_empty());
        }
    }

    #[test]
    fn live_broker_materializes_from_fixture_manifest() {
        let kit = MemoryTestKit::ci();
        let broker = kit.live_broker().expect("fixture live broker");
        let report = broker.effective_report().expect("effective report");
        assert_eq!(report.preset, kit.fixture.manifest.preset);
        assert_eq!(report.execution, kit.fixture.manifest.execution);
        assert_eq!(report.residency, kit.fixture.manifest.residency);
    }

    #[test]
    fn testkit_runs_facade_store_suite_against_live_broker_and_passes() {
        let kit = MemoryTestKit::ci();
        let verdict = kit.run_facade_store_conformance();
        assert!(verdict.passed(), "live facade suite must pass: {verdict:?}");
        // Independent inventory of the composition contract. Keeping only
        // lifecycle checks must not silently erase the four facade slices.
        let assertions = crate::conformance::run_facade_store_suite(&kit);
        let mut expected = std::collections::BTreeMap::new();
        for name in [
            "payload_dedup_refund",
            "pin_safe_delete",
            "retained_map_refunds",
            "retained_atomic_recharge_overwrite",
            "transcript_truncate_refund",
            "transcript_over_cap_fail_closed",
            "spill_round_trip",
            "spill_unavailable_fail_closed",
        ] {
            expected.insert(name.to_string(), 1usize);
        }
        for store in ["payload", "retained", "revocable", "transcript", "prepared"] {
            for suffix in [
                "close_exactly_once",
                "rebuild_close_fail_closed",
                "last_owner_drop_releases",
                "rebuild_drop_defers_until_finish",
            ] {
                expected.insert(format!("{store}_{suffix}"), 1);
            }
        }
        let mut observed = std::collections::BTreeMap::new();
        for assertion in &assertions {
            *observed.entry(assertion.name.clone()).or_insert(0usize) += 1;
        }
        assert_eq!(observed, expected, "each required facade assertion must run exactly once");
        assert!(assertions.iter().all(|assertion| assertion.held()), "{assertions:?}");
    }
}
