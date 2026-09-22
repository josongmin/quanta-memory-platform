#![forbid(unsafe_code)]

//! `quanta-memory-platform-testkit`: reusable conformance, chaos, and allocation
//! tooling for the memory platform.
//!
//! Feature-sliced, host-independent test infrastructure:
//!
//! - `fake/*` — the fake broker / runtime substrate. Obeys the SAME accounting
//!   invariants as the real core broker, and can be deliberately broken to prove
//!   the conformance suite bites.
//! - `conformance/*` — a reusable suite that exercises the core + facade
//!   contracts generically. Failures localize to a [`ContractFamily`], never to a
//!   host package.
//! - `faults/*` — deterministic chaos fixtures for reclaim / pin / rebuild /
//!   spill failure (no timing heuristics).
//! - `alloc_tracking/*` — the ONLY integration of `tracking-allocator`, gated
//!   behind the `alloc_tracking` feature so it can never enter a production
//!   dependency graph.
//! - `fixtures/*` — deterministic fixtures and the [`MemoryTestKit`] entry point.
//! - `doctor/*` — a minimal diagnostic report data model for host tooling.
//!
//! This crate is NEVER a mandatory production dependency.

#[cfg(feature = "conformance")]
pub mod conformance;
#[cfg(feature = "doctor")]
pub mod doctor;
pub mod fake;
#[cfg(feature = "faults")]
pub mod faults;
#[cfg(feature = "fixtures")]
pub mod fixtures;
#[cfg(all(test, feature = "loom"))]
mod loom_models;

#[cfg(feature = "alloc_tracking")]
pub mod alloc_tracking;

#[cfg(feature = "conformance")]
pub use conformance::{
    AccountingView, BrokerConformanceTarget, ChargeAxis, ConformanceSuite, ConformanceVerdict,
    ContractFamily, InvariantAssertion,
};
#[cfg(feature = "doctor")]
pub use doctor::MemoryDoctorReport;
pub use fake::{
    AccountingFidelity, FakeBrokerFactory, FakeProcessBroker, FakeReservation,
    FakeRuntimeViewFactory,
};
#[cfg(feature = "faults")]
pub use faults::{FaultInjectionHarness, FaultOutcome, FaultReport, FaultScenario};
#[cfg(feature = "fixtures")]
pub use fixtures::{MemoryFixture, MemoryFixtureBuilder, MemoryTestKit};

#[cfg(feature = "alloc_tracking")]
pub use alloc_tracking::{AllocationProbe, AllocationSignals};

#[cfg(all(test, feature = "conformance"))]
mod self_tests {
    //! CRITICAL META-REQUIREMENT: the conformance suite is itself test
    //! infrastructure. It must demonstrably FAIL when a contract is violated. A
    //! suite that passes everything is worthless. These tests feed deliberately
    //! broken fakes into the public suite and assert it REJECTS them, alongside
    //! the honest-fake and real-broker positive cases proven in the suite module.

    use crate::conformance::{ConformanceSuite, ContractFamily};
    use crate::fake::{AccountingFidelity, FakeProcessBroker};

    #[test]
    fn conformance_suite_rejects_a_fake_that_under_reports_usage() {
        let mut liar = FakeProcessBroker::broken(AccountingFidelity::UnderReportOwnerBytes(128));
        let verdict = ConformanceSuite::new().run_broker_accounting(&mut liar);
        assert!(
            !verdict.passed(),
            "a fake that under-reports usage MUST be rejected; a suite that passes it is worthless"
        );
        assert!(
            verdict.violated_families().contains(&ContractFamily::ClassOwnerBalance),
            "the lie must localize to the class/owner balance contract family, got {:?}",
            verdict.violated_families()
        );
    }

    #[test]
    fn conformance_suite_rejects_a_fake_with_divergent_class_totals() {
        let mut liar = FakeProcessBroker::broken(AccountingFidelity::DivergeClassFromOwner(256));
        let verdict = ConformanceSuite::new().run_broker_accounting(&mut liar);
        assert!(!verdict.passed(), "divergent class totals must be rejected");
    }

    #[test]
    fn conformance_suite_accepts_an_honest_fake() {
        // The positive pole of the meta-requirement: the same suite must PASS an
        // honest fake, or it would be rejecting everything (also worthless).
        let mut honest = FakeProcessBroker::honest();
        let verdict = ConformanceSuite::new().run_broker_accounting(&mut honest);
        assert!(verdict.passed(), "honest fake must pass: {verdict:?}");
    }
}
