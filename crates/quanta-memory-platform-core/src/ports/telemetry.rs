//! Allocator telemetry advisory lane.
//!
//! Allocator telemetry reports what the *allocator* believes is resident. It is
//! **advisory only**: it is never the semantic accounting authority. The ledger
//! ([`crate::domain::ledger::AccountingLedger`]) is the SSOT. This lane lets a
//! host compare allocator truth against semantic truth and surface a mismatch
//! signal WITHOUT mutating any semantic counter (the advisory telemetry contract).

use crate::AllocatorSnapshot;

/// Fixed operation vocabulary for operational events. Variants, rather than
/// caller strings, are the cardinality boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryOperation {
    Reserve,
    Release,
    Close,
    Recharge,
    Reclaim,
    SpillInProcess,
    SpillDurable,
    RestoreDurable,
    TombstoneSaturation,
    Pressure,
    Reconfigure,
    ObserveAllocator,
    ObserveBroker,
}

impl MemoryOperation {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Reserve => "reserve",
            Self::Release => "release",
            Self::Close => "close",
            Self::Recharge => "recharge",
            Self::Reclaim => "reclaim",
            Self::SpillInProcess => "spill_in_process",
            Self::SpillDurable => "spill_durable",
            Self::RestoreDurable => "restore_durable",
            Self::TombstoneSaturation => "tombstone_saturation",
            Self::Pressure => "pressure",
            Self::Reconfigure => "reconfigure",
            Self::ObserveAllocator => "observe_allocator",
            Self::ObserveBroker => "observe_broker",
        }
    }
}

/// Fixed result vocabulary. No owner, runtime, key, session or free-form error
/// text can enter a metric label through this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryOutcome {
    Succeeded,
    CapacityDenied,
    Busy,
    Closed,
    Poisoned,
    InvalidState,
    ConfigRejected,
    Unsupported,
    AllocatorMismatch,
    ObservationFailed,
    CallbackFailed,
}

impl MemoryOutcome {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::CapacityDenied => "capacity_denied",
            Self::Busy => "busy",
            Self::Closed => "closed",
            Self::Poisoned => "poisoned",
            Self::InvalidState => "invalid_state",
            Self::ConfigRejected => "config_rejected",
            Self::Unsupported => "unsupported",
            Self::AllocatorMismatch => "allocator_mismatch",
            Self::ObservationFailed => "observation_failed",
            Self::CallbackFailed => "callback_failed",
        }
    }

    /// Map the public typed error taxonomy into a bounded observation label.
    /// No message text is parsed or exposed as a label.
    #[must_use]
    pub const fn from_error(error: &crate::MemoryError) -> Self {
        match error {
            crate::MemoryError::Capacity(_) => Self::CapacityDenied,
            crate::MemoryError::Closed(_) => Self::Closed,
            crate::MemoryError::OperationInProgress(_) => Self::Busy,
            crate::MemoryError::Config(_) | crate::MemoryError::Build(_) => Self::ConfigRejected,
            crate::MemoryError::Unsupported(_) => Self::Unsupported,
            crate::MemoryError::Poisoned(_) => Self::Poisoned,
            crate::MemoryError::InvalidState(_) => Self::InvalidState,
            crate::MemoryError::Report(_) => Self::ObservationFailed,
        }
    }
}

/// One bounded-cardinality operational observation. `bytes` is a numeric
/// measurement, never a label or accounting mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryEvent {
    pub operation: MemoryOperation,
    pub outcome: MemoryOutcome,
    pub bytes: Option<u64>,
}

impl MemoryEvent {
    #[must_use]
    pub const fn new(
        operation: MemoryOperation,
        outcome: MemoryOutcome,
        bytes: Option<u64>,
    ) -> Self {
        Self { operation, outcome, bytes }
    }
}

/// Advisory event sink. Implementations must not own or mutate semantic
/// accounting state. Broker implementations invoke this only after releasing
/// their internal mutex and contain callback panics.
pub trait MemoryEventSink: Send + Sync {
    fn emit(&self, event: MemoryEvent);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopMemoryEventSink;

impl MemoryEventSink for NoopMemoryEventSink {
    fn emit(&self, _event: MemoryEvent) {}
}

/// Reads an advisory allocator snapshot.
pub trait AllocatorTelemetry {
    fn snapshot(&self) -> AllocatorSnapshot;
}

/// A fixed-snapshot telemetry source, useful for wiring and tests. Holds an
/// allocator snapshot verbatim; it has no semantic authority.
#[derive(Clone, Debug, Default)]
pub struct StaticAllocatorTelemetry {
    snapshot: AllocatorSnapshot,
}

impl StaticAllocatorTelemetry {
    #[must_use]
    pub fn new(snapshot: AllocatorSnapshot) -> Self {
        Self { snapshot }
    }
}

impl AllocatorTelemetry for StaticAllocatorTelemetry {
    fn snapshot(&self) -> AllocatorSnapshot {
        self.snapshot.clone()
    }
}

/// The advisory result of comparing allocator-reported resident bytes against
/// semantic accounting bytes. This is a *report*, not a mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocatorAdvisory {
    /// Semantic accounting total (SSOT).
    pub semantic_bytes: u64,
    /// Allocator-reported resident bytes, if available.
    pub allocator_resident_bytes: Option<u64>,
    /// `true` when allocator and semantic accounting disagree beyond zero.
    pub mismatch: bool,
}

impl AllocatorAdvisory {
    /// Compute an advisory by comparing a snapshot against the semantic total.
    /// Never mutates anything; the caller decides what to do with `mismatch`.
    #[must_use]
    pub fn compare(snapshot: &AllocatorSnapshot, semantic_bytes: u64) -> Self {
        let allocator_resident_bytes = snapshot.resident_bytes;
        let mismatch = match allocator_resident_bytes {
            Some(resident) => resident != semantic_bytes,
            None => false,
        };
        Self { semantic_bytes, allocator_resident_bytes, mismatch }
    }

    /// The signed drift (allocator - semantic) when allocator data is present.
    #[must_use]
    pub fn drift_bytes(&self) -> Option<i128> {
        self.allocator_resident_bytes
            .map(|resident| i128::from(resident) - i128::from(self.semantic_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_labels_are_closed_bounded_sets() {
        let operations = [
            MemoryOperation::Reserve,
            MemoryOperation::Release,
            MemoryOperation::Close,
            MemoryOperation::Recharge,
            MemoryOperation::Reclaim,
            MemoryOperation::SpillInProcess,
            MemoryOperation::SpillDurable,
            MemoryOperation::RestoreDurable,
            MemoryOperation::TombstoneSaturation,
            MemoryOperation::Pressure,
            MemoryOperation::Reconfigure,
            MemoryOperation::ObserveAllocator,
            MemoryOperation::ObserveBroker,
        ];
        let outcomes = [
            MemoryOutcome::Succeeded,
            MemoryOutcome::CapacityDenied,
            MemoryOutcome::Busy,
            MemoryOutcome::Closed,
            MemoryOutcome::Poisoned,
            MemoryOutcome::InvalidState,
            MemoryOutcome::ConfigRejected,
            MemoryOutcome::Unsupported,
            MemoryOutcome::AllocatorMismatch,
            MemoryOutcome::ObservationFailed,
            MemoryOutcome::CallbackFailed,
        ];
        assert_eq!(operations.map(MemoryOperation::label).len(), 13);
        assert_eq!(outcomes.map(MemoryOutcome::label).len(), 11);
        assert!(operations.iter().all(|operation| !operation.label().is_empty()));
        assert!(outcomes.iter().all(|outcome| !outcome.label().is_empty()));
    }

    #[test]
    fn advisory_agreement_has_no_mismatch() {
        let snap =
            AllocatorSnapshot { resident_bytes: Some(1_000), ..AllocatorSnapshot::default() };
        let advisory = AllocatorAdvisory::compare(&snap, 1_000);
        assert!(!advisory.mismatch);
        assert_eq!(advisory.drift_bytes(), Some(0));
    }

    #[test]
    fn missing_allocator_data_is_not_treated_as_mismatch() {
        let snap = AllocatorSnapshot::default(); // resident_bytes = None
        let advisory = AllocatorAdvisory::compare(&snap, 1_000);
        assert!(!advisory.mismatch, "absent allocator data cannot contradict semantic SSOT");
        assert_eq!(advisory.drift_bytes(), None);
    }

    #[test]
    fn static_telemetry_returns_its_snapshot_verbatim() {
        let snap = AllocatorSnapshot { resident_bytes: Some(42), ..AllocatorSnapshot::default() };
        let t = StaticAllocatorTelemetry::new(snap.clone());
        assert_eq!(t.snapshot(), snap);
    }

    // scenario L: drift sign + magnitude in both directions, and the mismatch
    // flag transition exactly at equality. Table-driven so the equality boundary
    // is exercised from both sides plus the exact crossing point.
    #[test]
    fn compare_reports_signed_drift_and_mismatch_boundary() {
        // (allocator_resident, semantic, expected_drift, expected_mismatch)
        let cases: &[(u64, u64, i128, bool)] = &[
            // allocator > semantic: positive drift, mismatch set.
            (2_000, 1_000, 1_000, true),
            (1_001, 1_000, 1, true),
            // allocator == semantic: zero drift, mismatch CLEAR at the boundary.
            (1_000, 1_000, 0, false),
            (0, 0, 0, false),
            // allocator < semantic: negative drift, mismatch set.
            (999, 1_000, -1, true),
            (1, 1_000, -999, true),
            (0, 1_000, -1_000, true),
        ];
        for &(resident, semantic, expected_drift, expected_mismatch) in cases {
            let snap = AllocatorSnapshot {
                resident_bytes: Some(resident),
                ..AllocatorSnapshot::default()
            };
            let advisory = AllocatorAdvisory::compare(&snap, semantic);
            assert_eq!(
                advisory.mismatch, expected_mismatch,
                "mismatch flag for allocator={resident} vs semantic={semantic}"
            );
            assert_eq!(
                advisory.drift_bytes(),
                Some(expected_drift),
                "drift = allocator - semantic for {resident} vs {semantic}"
            );
            // The advisory must always carry the SEMANTIC value as authority,
            // independent of what the allocator reports.
            assert_eq!(
                advisory.semantic_bytes, semantic,
                "semantic_bytes must reflect the SSOT, never the allocator value"
            );
        }
    }

    // scenario L: even when the allocator reports a wildly larger resident value,
    // the advisory carries the semantic SSOT verbatim and never adopts the
    // allocator figure as the authoritative number.
    #[test]
    fn compare_semantic_bytes_is_ssot_not_allocator() {
        let snap = AllocatorSnapshot {
            resident_bytes: Some(u64::MAX),
            allocated_bytes: Some(u64::MAX),
            retained_bytes: Some(u64::MAX),
            fragmentation_bytes: Some(u64::MAX),
        };
        let advisory = AllocatorAdvisory::compare(&snap, 7);
        assert_eq!(advisory.semantic_bytes, 7, "carried authority is the semantic total only");
        assert_eq!(advisory.allocator_resident_bytes, Some(u64::MAX));
        assert!(advisory.mismatch);
        // Drift must not overflow: i128 headroom keeps allocator-semantic exact.
        assert_eq!(advisory.drift_bytes(), Some(i128::from(u64::MAX) - 7));
    }

    // scenario L: the advisory lane is OBSERVATION-ONLY. The trait surface only
    // hands back a snapshot; there is no method to write a counter. We prove the
    // generic trait contract by driving an arbitrary AllocatorTelemetry impl and
    // confirming compare() consumes the snapshot without any mutating seam.
    #[test]
    fn allocator_telemetry_trait_surface_is_observation_only() {
        fn observe_via_trait<T: AllocatorTelemetry>(t: &T, semantic: u64) -> AllocatorAdvisory {
            // Only API reachable through the trait is `snapshot()`. There is no
            // mutate/charge/release on the trait surface.
            AllocatorAdvisory::compare(&t.snapshot(), semantic)
        }
        let snap = AllocatorSnapshot { resident_bytes: Some(500), ..AllocatorSnapshot::default() };
        let telemetry = StaticAllocatorTelemetry::new(snap.clone());

        let advisory = observe_via_trait(&telemetry, 400);
        assert!(advisory.mismatch);
        assert_eq!(advisory.drift_bytes(), Some(100));

        // Repeated observation is idempotent: snapshot() must be a pure read that
        // does not consume or alter the telemetry source's state.
        let again = observe_via_trait(&telemetry, 400);
        assert_eq!(advisory, again, "observation must be a side-effect-free read");
        assert_eq!(telemetry.snapshot(), snap, "snapshot() leaves the source unchanged");
    }

    // scenario L: a custom telemetry impl that reports no allocator data cannot
    // manufacture a mismatch against the SSOT through the trait path either.
    #[test]
    fn trait_path_absent_allocator_data_never_contradicts_ssot() {
        struct BlindTelemetry;
        impl AllocatorTelemetry for BlindTelemetry {
            fn snapshot(&self) -> AllocatorSnapshot {
                AllocatorSnapshot::default()
            }
        }
        let advisory = AllocatorAdvisory::compare(&BlindTelemetry.snapshot(), 9_999);
        assert!(!advisory.mismatch, "no allocator reading cannot contradict the SSOT");
        assert_eq!(advisory.drift_bytes(), None);
        assert_eq!(advisory.semantic_bytes, 9_999);
    }
}
