//! Deterministic fault scenario drivers.
//!
//! Each driver reproduces one negative path (playbook section 10) against a real
//! core/facade contract or the fake broker, with NO timing heuristics: races are
//! modeled structurally (open a rebuild window, attempt the racing write) rather
//! than with sleeps. Every driver returns a typed [`FaultOutcome`] the harness
//! folds into an assertion.

use quanta_memory_platform_core::{
    ChargeKind, MemoryClass, MemoryPreset, RetainedEnvelope, StoreBindingHandle,
};
use quanta_memory_platform_facade::{
    InMemorySpillTarget, MemoryPlatformBuilder, PayloadStore, RoutedSpillController,
    SpillController,
};

use crate::fake::FakeProcessBroker;

/// Stable scenario identifiers (kept for the public test-entry surface).
pub const RECLAIM_RETURNS_ZERO: &str = "reclaim_returns_zero";
pub const PIN_INVALIDATION_FAILURE: &str = "pin_invalidation_failure";
pub const REBUILD_WINDOW_REJECTS_WRITES: &str = "rebuild_window_rejects_writes";
pub const SPILL_UNAVAILABLE: &str = "spill_unavailable";

/// The observed outcome of running a fault scenario. `Contained` means the
/// system failed closed as the contract requires; `Escaped` means the fault was
/// silently swallowed (a real bug), with evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FaultOutcome {
    /// The fault was contained / failed closed as the contract demands.
    Contained { evidence: String },
    /// The fault escaped containment (a contract violation), with evidence.
    Escaped { evidence: String },
}

impl FaultOutcome {
    #[must_use]
    pub fn contained(&self) -> bool {
        matches!(self, Self::Contained { .. })
    }

    #[must_use]
    pub fn evidence(&self) -> &str {
        match self {
            Self::Contained { evidence } | Self::Escaped { evidence } => evidence,
        }
    }
}

fn payload_binding() -> StoreBindingHandle {
    StoreBindingHandle::new(MemoryClass::ArtifactCache, "fault:owner".to_string(), None)
}

/// NEGATIVE PATH 1: reclaim callback returns zero under starvation. Contained
/// means usage is unchanged (no fake "reclaimed" bytes) and the caller can see
/// that nothing was freed.
#[must_use]
pub fn drive_reclaim_returns_zero() -> FaultOutcome {
    let mut broker = FakeProcessBroker::honest().with_reclaim_yield_bytes(0);
    let t = match broker.reserve(
        MemoryClass::ArtifactCache,
        "fault:owner".to_string(),
        ChargeKind::RetainedPayload,
        4_096,
    ) {
        Ok(t) => t,
        Err(e) => return FaultOutcome::Escaped { evidence: format!("setup reserve failed: {e}") },
    };
    let _ = broker.commit(&t);
    let before = broker.snapshot().total_owner_bytes();
    let freed = broker.reclaim_once();
    let after = broker.snapshot().total_owner_bytes();
    if freed == 0 && before == after {
        FaultOutcome::Contained {
            evidence: format!("reclaim freed {freed} bytes; usage stayed at {after}"),
        }
    } else {
        FaultOutcome::Escaped {
            evidence: format!("reclaim claimed {freed} bytes (before={before} after={after})"),
        }
    }
}

/// NEGATIVE PATH 2: pin invalidation race — a delete attempted while the payload
/// is pinned must be rejected (the pinned payload must survive). Contained means
/// the delete failed closed AND the bytes/entry survived.
#[must_use]
pub fn drive_pin_invalidation_failure() -> FaultOutcome {
    let platform = match MemoryPlatformBuilder::new().with_preset(MemoryPreset::Production).build()
    {
        Ok(platform) => platform,
        Err(error) => {
            return FaultOutcome::Escaped { evidence: format!("platform failed: {error}") }
        }
    };
    let store =
        match platform.payload_store_builder::<String, Vec<u8>>().bind(payload_binding()).build() {
            Ok(store) => store,
            Err(error) => {
                return FaultOutcome::Escaped { evidence: format!("store failed: {error}") }
            }
        };
    let published = match store.put_canonical(RetainedEnvelope::new(b"pinned-payload".to_vec())) {
        Ok(p) => p,
        Err(e) => return FaultOutcome::Escaped { evidence: format!("put failed: {e}") },
    };
    if store.pin(published.payload_ref()).is_err() {
        return FaultOutcome::Escaped { evidence: "pin of fresh payload failed".to_string() };
    }
    let charged_before = store.charged_bytes().unwrap_or(0);
    let delete = store.delete_if_unpinned(published.payload_ref());
    let charged_after = store.charged_bytes().unwrap_or(0);
    match delete {
        Err(e) if charged_after == charged_before && charged_after > 0 => FaultOutcome::Contained {
            evidence: format!("pinned delete rejected ({e}); bytes survived at {charged_after}"),
        },
        Err(e) => FaultOutcome::Escaped {
            evidence: format!("delete rejected ({e}) but bytes drifted to {charged_after}"),
        },
        Ok(()) => FaultOutcome::Escaped {
            evidence: "pinned payload was deleted (pin discipline violated)".to_string(),
        },
    }
}

/// NEGATIVE PATH 3: rebuild race — a write that arrives while the rebuild window
/// is open must be rejected (it must not be accepted "too early"). Contained
/// means the racing reserve failed AND accounting did not move.
#[must_use]
pub fn drive_rebuild_race_accepts_write_too_early() -> FaultOutcome {
    let mut broker = FakeProcessBroker::honest();
    broker.begin_rebuild();
    let before = broker.snapshot().total_owner_bytes();
    let racing = broker.reserve(
        MemoryClass::ArtifactCache,
        "fault:owner".to_string(),
        ChargeKind::RetainedPayload,
        1_024,
    );
    let after = broker.snapshot().total_owner_bytes();
    match racing {
        Err(_) if before == after => FaultOutcome::Contained {
            evidence: format!("write during rebuild rejected; usage stayed at {after}"),
        },
        Err(_) => FaultOutcome::Escaped {
            evidence: format!("write rejected but usage drifted {before}->{after}"),
        },
        Ok(_) => FaultOutcome::Escaped {
            evidence: "write was accepted during rebuild window (race violation)".to_string(),
        },
    }
}

/// NEGATIVE PATH 4: spill controller fails after the target is unavailable.
/// Contained means the spill fails closed and the success count does not move.
#[must_use]
pub fn drive_spill_failure_after_reservation() -> FaultOutcome {
    let controller = RoutedSpillController::new(InMemorySpillTarget::unavailable());
    let before = controller.spilled_count();
    let result = controller.spill("spillable-handle".to_string());
    let after = controller.spilled_count();
    match (before, result, after) {
        (Ok(before), Err(e), Ok(after)) if before == after => FaultOutcome::Contained {
            evidence: format!("spill failed closed ({e}); spilled_count stayed at {after}"),
        },
        (before, Err(e), after) => FaultOutcome::Escaped {
            evidence: format!("spill failed ({e}) but inventory proof was {before:?}->{after:?}"),
        },
        (_, Ok(_), _) => FaultOutcome::Escaped {
            evidence: "spill to an unavailable target reported success".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reclaim_returns_zero_is_contained() {
        let outcome = drive_reclaim_returns_zero();
        assert!(outcome.contained(), "reclaim-zero must be contained: {}", outcome.evidence());
        assert!(outcome.evidence().contains("freed 0"));
    }

    #[test]
    fn pinned_delete_failure_is_contained_and_bytes_survive() {
        let outcome = drive_pin_invalidation_failure();
        assert!(outcome.contained(), "pinned delete must be contained: {}", outcome.evidence());
        // Evidence proves the bytes survived the rejected delete.
        assert!(outcome.evidence().contains("bytes survived"));
    }

    #[test]
    fn rebuild_race_write_is_rejected() {
        let outcome = drive_rebuild_race_accepts_write_too_early();
        assert!(outcome.contained(), "rebuild race must be contained: {}", outcome.evidence());
    }

    #[test]
    fn spill_failure_is_contained_without_counting() {
        let outcome = drive_spill_failure_after_reservation();
        assert!(outcome.contained(), "spill failure must be contained: {}", outcome.evidence());
        assert!(outcome.evidence().contains("stayed at 0"));
    }

    // NEGATIVE POLE: the fault drivers' `Escaped` detection must actually fire for a
    // broken component — otherwise a driver that can only ever report `Contained` is
    // a rubber stamp. Classify an unavailable-spill against a LYING controller (one
    // that reports success when it must fail closed) and prove it is `Escaped`, with
    // the honest fail-closed controller as the `Contained` positive pole.
    #[test]
    fn spill_fault_classification_reports_escaped_for_a_lying_controller() {
        // Shared classification shape: Err => Contained, Ok => Escaped.
        fn classify<C: SpillController<String>>(controller: &C) -> FaultOutcome {
            match controller.spill("spillable-handle".to_string()) {
                Err(e) => {
                    FaultOutcome::Contained { evidence: format!("spill failed closed ({e})") }
                }
                Ok(_) => FaultOutcome::Escaped {
                    evidence: "spill to an unavailable target reported success".to_string(),
                },
            }
        }

        // A controller that LIES (reports Ok when it should fail closed).
        #[derive(Debug, Default)]
        struct LyingSpillController;
        impl SpillController<String> for LyingSpillController {
            fn spill(
                &self,
                _handle: String,
            ) -> Result<
                quanta_memory_platform_facade::SpillReceipt,
                quanta_memory_platform_facade::SpillFailure<String>,
            > {
                Ok(quanta_memory_platform_facade::SpillReceipt::InProcess { inventory_after: 0 })
            }
        }

        let escaped = classify(&LyingSpillController);
        assert!(!escaped.contained(), "a lying controller must classify as Escaped");
        assert!(matches!(escaped, FaultOutcome::Escaped { .. }));

        // Positive pole: the real fail-closed controller is Contained.
        let honest = RoutedSpillController::new(InMemorySpillTarget::unavailable());
        assert!(classify(&honest).contained(), "the honest controller fails closed (Contained)");
    }
}
