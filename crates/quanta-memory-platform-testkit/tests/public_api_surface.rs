//! Public API surface snapshot for `quanta-memory-platform-testkit`.
//!
//! This pins the feature-gated public surface:
//! - `conformance` alone exposes the broker-accounting suite only.
//! - `fixtures` upgrades the public surface with `MemoryTestKit`,
//!   `run_facade_store_suite`, and `FACADE_STORE_SUITE`.

#[cfg(feature = "conformance")]
use quanta_memory_platform_testkit::conformance::suites::{
    TranscriptAppendConformanceTarget, TranscriptRow,
};
#[cfg(feature = "conformance")]
use quanta_memory_platform_testkit::conformance::{
    run_broker_accounting_suite, AccountingView, AllocatorAdvisoryView, AllocatorObservation,
    BrokerConformanceTarget, ChargeAxis, ConformanceSuite, ConformanceVerdict, ContractFamily,
    InvariantAssertion, CORE_ACCOUNTING_SUITE,
};

#[cfg(feature = "fixtures")]
use quanta_memory_platform_testkit::{
    conformance::{run_facade_store_suite, FACADE_STORE_SUITE},
    MemoryFixture, MemoryFixtureBuilder, MemoryTestKit,
};

#[cfg(feature = "conformance")]
use quanta_memory_platform_testkit::conformance::AssertionVerdict;

#[cfg(feature = "conformance")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin only.
fn names_conformance_surface() {
    type _Conformance = (
        AccountingView,
        AllocatorAdvisoryView,
        AllocatorObservation,
        ChargeAxis,
        ConformanceSuite,
        ConformanceVerdict,
        ContractFamily,
        InvariantAssertion,
        TranscriptRow,
    );
    fn transcript_target_bound<T: TranscriptAppendConformanceTarget>() {}
    type _BrokerRunner<T> = fn(&mut T) -> Vec<AssertionVerdict>;
    let _runner: _BrokerRunner<DummyTarget> = run_broker_accounting_suite::<DummyTarget>;
    let _suite_name: &str = CORE_ACCOUNTING_SUITE;
}

#[cfg(feature = "fixtures")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin only.
fn names_fixture_surface() {
    type _Fixtures = (MemoryFixture, MemoryFixtureBuilder, MemoryTestKit);
    type _FacadeRunner = fn(&MemoryTestKit) -> Vec<AssertionVerdict>;
    let _runner: _FacadeRunner = run_facade_store_suite;
    let _method: fn(&ConformanceSuite, &MemoryTestKit) -> ConformanceVerdict =
        ConformanceSuite::run_facade_store_suite;
    let _suite_name: &str = FACADE_STORE_SUITE;
}

#[cfg(feature = "conformance")]
struct DummyTarget;

#[cfg(feature = "conformance")]
impl BrokerConformanceTarget for DummyTarget {
    type Ticket = ();
    type RebuildFence = ();

    fn do_reserve(
        &mut self,
        _class_axis: usize,
        _owner: &str,
        _kind: ChargeAxis,
        _bytes: u64,
    ) -> Result<Self::Ticket, String> {
        Ok(())
    }

    fn do_commit(&mut self, _ticket: &Self::Ticket) -> Result<(), String> {
        Ok(())
    }

    fn do_release(&mut self, _ticket: &Self::Ticket) -> Result<(), String> {
        Ok(())
    }

    fn do_begin_rebuild(&mut self) -> Result<Self::RebuildFence, String> {
        Ok(())
    }

    fn do_finish_rebuild(&mut self, _fence: Self::RebuildFence) -> Result<(), String> {
        Ok(())
    }

    fn class_axes(&self) -> &'static [&'static str] {
        &["a", "b"]
    }

    fn view(&self) -> Result<AccountingView, String> {
        Ok(AccountingView::default())
    }

    fn observe_allocator(
        &self,
        _observation: &AllocatorObservation,
    ) -> Result<AllocatorAdvisoryView, String> {
        Ok(AllocatorAdvisoryView::default())
    }
}

#[cfg(feature = "fixtures")]
#[test]
fn fixture_mode_exposes_the_facade_suite_end_to_end() {
    let verdict = ConformanceSuite::new().run_facade_store_suite(&MemoryTestKit::ci());
    assert!(verdict.passed(), "fixture facade suite must stay runnable: {verdict:?}");
}
