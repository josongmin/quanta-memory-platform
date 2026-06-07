//! Fault injection / chaos harness.
//!
//! `faults/*` simulates the library-level failure surfaces (playbook 10):
//! reclaim failure, pin race, rebuild race, spill failure. Every scenario is
//! deterministic and structural — no sleeps, no timing heuristics — and runs
//! against real core/facade contracts (or the fake broker), reporting a typed
//! [`FaultOutcome`].

pub mod scenarios;

pub use scenarios::{
    drive_pin_invalidation_failure, drive_rebuild_race_accepts_write_too_early,
    drive_reclaim_returns_zero, drive_spill_failure_after_reservation, FaultOutcome,
    PIN_INVALIDATION_FAILURE, REBUILD_WINDOW_REJECTS_WRITES, RECLAIM_RETURNS_ZERO,
    SPILL_UNAVAILABLE,
};

/// The fault scenarios this harness can inject.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultScenario {
    /// Reclaim callback returns zero under starvation.
    ReclaimReturnsZero,
    /// A delete is attempted while the payload is pinned.
    PinInvalidationFailure,
    /// A write arrives while the rebuild window is open.
    RebuildWindowRejectsWrites,
    /// The spill target is unavailable.
    SpillUnavailable,
}

impl FaultScenario {
    /// All scenarios, in stable order.
    #[must_use]
    pub fn all() -> [FaultScenario; 4] {
        [
            FaultScenario::ReclaimReturnsZero,
            FaultScenario::PinInvalidationFailure,
            FaultScenario::RebuildWindowRejectsWrites,
            FaultScenario::SpillUnavailable,
        ]
    }

    /// Run this scenario and return its outcome.
    #[must_use]
    pub fn run(self) -> FaultOutcome {
        match self {
            FaultScenario::ReclaimReturnsZero => drive_reclaim_returns_zero(),
            FaultScenario::PinInvalidationFailure => drive_pin_invalidation_failure(),
            FaultScenario::RebuildWindowRejectsWrites => {
                drive_rebuild_race_accepts_write_too_early()
            }
            FaultScenario::SpillUnavailable => drive_spill_failure_after_reservation(),
        }
    }
}

/// A report for one injected scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultReport {
    pub scenario: FaultScenario,
    pub outcome: FaultOutcome,
}

impl FaultReport {
    #[must_use]
    pub fn contained(&self) -> bool {
        self.outcome.contained()
    }
}

/// Runs fault scenarios and collects their outcomes.
#[derive(Clone, Debug, Default)]
pub struct FaultInjectionHarness {
    active: Vec<FaultScenario>,
}

impl FaultInjectionHarness {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A harness pre-loaded with every scenario.
    #[must_use]
    pub fn with_all_scenarios() -> Self {
        Self { active: FaultScenario::all().to_vec() }
    }

    #[must_use]
    pub fn inject(mut self, scenario: FaultScenario) -> Self {
        self.active.push(scenario);
        self
    }

    #[must_use]
    pub fn scenarios(&self) -> &[FaultScenario] {
        &self.active
    }

    /// Run every injected scenario and return ordered reports.
    #[must_use]
    pub fn run(&self) -> Vec<FaultReport> {
        self.active
            .iter()
            .map(|&scenario| FaultReport { scenario, outcome: scenario.run() })
            .collect()
    }

    /// Whether every injected scenario was contained (failed closed).
    #[must_use]
    pub fn all_contained(&self) -> bool {
        self.run().iter().all(FaultReport::contained)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_covers_reclaim_pin_rebuild_and_spill() {
        let harness = FaultInjectionHarness::with_all_scenarios();
        let reports = harness.run();
        // Every required negative path must be present (Not Done If #3).
        let covered: Vec<FaultScenario> = reports.iter().map(|r| r.scenario).collect();
        assert!(covered.contains(&FaultScenario::ReclaimReturnsZero));
        assert!(covered.contains(&FaultScenario::PinInvalidationFailure));
        assert!(covered.contains(&FaultScenario::RebuildWindowRejectsWrites));
        assert!(covered.contains(&FaultScenario::SpillUnavailable));
        // All must be contained / failed closed.
        assert!(
            reports.iter().all(FaultReport::contained),
            "uncontained faults: {:?}",
            reports.iter().filter(|r| !r.contained()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_harness_runs_nothing() {
        let harness = FaultInjectionHarness::new();
        assert!(harness.run().is_empty());
        // A vacuous harness is trivially "all contained"; assert it does not claim
        // coverage it does not have.
        assert!(harness.scenarios().is_empty());
    }
}
