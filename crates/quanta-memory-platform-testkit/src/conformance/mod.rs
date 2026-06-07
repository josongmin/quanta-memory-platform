//! Reusable conformance suite.
//!
//! `conformance/*` exercises the core + facade contracts generically: it never
//! references a host package. A failure localizes to a [`ContractFamily`], not to
//! a Semantica crate. The suite is itself test infrastructure and is proven to
//! reject a deliberately broken fake (see `suites::tests` and the crate-level
//! self-test).

#[cfg(feature = "fixtures")]
use crate::fixtures::MemoryTestKit;

pub mod invariants;
pub mod suites;
pub mod target;

pub use invariants::{AssertionVerdict, ContractFamily, InvariantAssertion};
pub use suites::{run_broker_accounting_suite, CORE_ACCOUNTING_SUITE};
#[cfg(feature = "fixtures")]
pub use suites::{run_facade_store_suite, FACADE_STORE_SUITE};
pub use target::{
    AccountingView, AllocatorAdvisoryView, AllocatorObservation, BrokerConformanceTarget,
    ChargeAxis,
};

/// A pass/fail conformance verdict with the failing families localized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConformanceVerdict {
    /// Every invariant held.
    Passed,
    /// At least one invariant was violated. Carries the per-assertion evidence
    /// and the set of contract families that failed.
    Failed { violated_families: Vec<ContractFamily>, evidence: Vec<String> },
}

impl ConformanceVerdict {
    /// Fold an ordered list of assertion verdicts into a pass/fail verdict.
    #[must_use]
    pub fn from_assertions(verdicts: &[AssertionVerdict]) -> Self {
        if verdicts.is_empty() {
            return Self::Failed {
                violated_families: Vec::new(),
                evidence: vec!["no conformance checks ran".to_string()],
            };
        }
        let mut violated_families = Vec::new();
        let mut evidence = Vec::new();
        for v in verdicts {
            if !v.held() {
                if !violated_families.contains(&v.family) {
                    violated_families.push(v.family);
                }
                evidence.push(v.to_string());
            }
        }
        if violated_families.is_empty() {
            Self::Passed
        } else {
            Self::Failed { violated_families, evidence }
        }
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        matches!(self, Self::Passed)
    }

    /// The contract families that failed (empty when passed).
    #[must_use]
    pub fn violated_families(&self) -> &[ContractFamily] {
        match self {
            Self::Passed => &[],
            Self::Failed { violated_families, .. } => violated_families,
        }
    }
}

/// The public conformance suite entry point.
///
/// Holds no host state. Its single responsibility is to run the reusable
/// accounting suite against a [`BrokerConformanceTarget`] and fold the result.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConformanceSuite;

impl ConformanceSuite {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Run the broker accounting conformance suite against `target` and return a
    /// folded verdict. A broker that lies about its accounting yields `Failed`
    /// with the violated families localized.
    pub fn run_broker_accounting<T>(&self, target: &mut T) -> ConformanceVerdict
    where
        T: BrokerConformanceTarget,
    {
        let verdicts = run_broker_accounting_suite(target);
        ConformanceVerdict::from_assertions(&verdicts)
    }

    /// Run the portable facade-store suite against a live broker-backed
    /// [`MemoryTestKit`] harness and return the folded verdict.
    #[cfg(feature = "fixtures")]
    pub fn run_facade_store_suite(&self, kit: &MemoryTestKit) -> ConformanceVerdict {
        let verdicts = run_facade_store_suite(kit);
        ConformanceVerdict::from_assertions(&verdicts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{AccountingFidelity, FakeProcessBroker};
    use quanta_memory_platform_core::{
        EffectiveMemoryBudgetReport, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClass,
        MemoryClassBudget, MemoryPreset, ReclaimPriority, ResidencyPolicy,
    };

    #[test]
    fn honest_fake_yields_passed_verdict() {
        let mut fake = FakeProcessBroker::honest();
        let verdict = ConformanceSuite::new().run_broker_accounting(&mut fake);
        assert!(verdict.passed(), "honest fake must pass: {verdict:?}");
        assert!(verdict.violated_families().is_empty());
    }

    #[test]
    fn broken_fake_yields_failed_verdict_localized_to_balance_family() {
        // META-REQUIREMENT: the public suite must reject a lying broker.
        let mut broken = FakeProcessBroker::broken(AccountingFidelity::UnderReportOwnerBytes(100));
        let verdict = ConformanceSuite::new().run_broker_accounting(&mut broken);
        assert!(!verdict.passed(), "under-reporting fake must FAIL the suite");
        assert!(
            verdict.violated_families().contains(&ContractFamily::ClassOwnerBalance),
            "failure must localize to ClassOwnerBalance, got {:?}",
            verdict.violated_families()
        );
    }

    #[test]
    fn empty_assertion_list_is_failed_not_silently_passed() {
        // A suite that ran nothing must never report Passed.
        let verdict = ConformanceVerdict::from_assertions(&[]);
        assert!(!verdict.passed(), "no checks must not be a pass");
    }

    fn live_broker() -> LiveProcessMemoryBroker {
        let residency = ResidencyPolicy::new(10_000_000)
            .with_class_budget(
                MemoryClass::ArtifactCache,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: 4_000_000,
                    hard_cap_bytes: 6_000_000,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            )
            .with_class_budget(
                MemoryClass::EvidenceCache,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: 4_000_000,
                    hard_cap_bytes: 6_000_000,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            );
        let report = EffectiveMemoryBudgetReport::from_manifest(
            MemoryBudgetManifest::new(
                MemoryPreset::Ci,
                quanta_memory_platform_core::ExecutionBudget::default(),
                residency,
            ),
            vec!["conformance-test".to_string()],
        );
        LiveProcessMemoryBroker::from_report(report).expect("valid residency")
    }

    #[test]
    fn public_suite_rejects_every_break_variant_and_localizes_it() {
        // META-REQUIREMENT at the PUBLIC surface: the folded verdict (not just the
        // raw assertion list) must FAIL for every way the fake can lie, and the
        // violated-families set must name the broken contract. This is the
        // enumeration the whole conformance crate exists to guarantee.
        // Compile-time tie to the enum: this exhaustive match (no wildcard) fails to
        // compile when a new AccountingFidelity variant is added, forcing it to be
        // added to `cases` below so it cannot silently escape the public meta-test.
        fn _exhaustiveness_guard(v: AccountingFidelity) {
            match v {
                AccountingFidelity::Honest
                | AccountingFidelity::UnderReportOwnerBytes(_)
                | AccountingFidelity::DivergeClassFromOwner(_)
                | AccountingFidelity::DivergeOwnerPartition(_) => {}
            }
        }
        // Each break variant + the SPECIFIC family it must localize to at the public
        // folded surface. DivergeOwnerPartition breaks invariant 2 (OwnerPartition),
        // distinct from the invariant-1 (ClassOwnerBalance) variants.
        let cases = [
            (AccountingFidelity::UnderReportOwnerBytes(100), ContractFamily::ClassOwnerBalance),
            (AccountingFidelity::DivergeClassFromOwner(64), ContractFamily::ClassOwnerBalance),
            (AccountingFidelity::DivergeOwnerPartition(48), ContractFamily::OwnerPartition),
        ];
        for (variant, expected) in cases {
            let mut broken = FakeProcessBroker::broken(variant);
            let verdict = ConformanceSuite::new().run_broker_accounting(&mut broken);
            assert!(!verdict.passed(), "{variant:?} must fail the public suite");
            assert!(
                verdict.violated_families().contains(&expected),
                "{variant:?} must localize to {expected:?}, got {:?}",
                verdict.violated_families()
            );
        }

        // Positive pole: the honest fidelity is the only one accepted.
        let mut honest = FakeProcessBroker::broken(AccountingFidelity::Honest);
        assert!(
            ConformanceSuite::new().run_broker_accounting(&mut honest).passed(),
            "honest fidelity must pass the public suite"
        );
    }

    #[test]
    fn real_core_broker_passes_the_same_portable_suite() {
        // The exact same reusable suite, with zero host code, must hold against
        // the real `LiveProcessMemoryBroker`. This proves the suite encodes the
        // real contract and not a fake-only shape.
        let mut broker = live_broker();
        let verdict = ConformanceSuite::new().run_broker_accounting(&mut broker);
        assert!(verdict.passed(), "real core broker must pass conformance: {verdict:?}");
    }
}
