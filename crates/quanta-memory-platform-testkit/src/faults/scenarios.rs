//! Deterministic fault scenario drivers.
//!
//! Each driver reproduces one negative path (playbook section 10) against a real
//! core/facade contract or the fake broker, with NO timing heuristics: races are
//! modeled structurally (open a rebuild window, attempt the racing write) rather
//! than with sleeps. Every driver returns a typed [`FaultOutcome`] the harness
//! folds into an assertion.

use quanta_memory_platform_core::{
    ChargeKind, LedgerSnapshot, LiveProcessMemoryBroker, MemoryClass, MemoryPreset, MemoryResult,
    RetainedEnvelope, StoreBindingHandle, StoreStats, UsageBreakdown,
};
use quanta_memory_platform_facade::{
    Budgeted, InMemorySpillTarget, MemoryPlatformBuilder, PayloadStore, RoutedSpillController,
    SimplePayloadStore, SpillController,
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

/// The reference lookup (`pin_count`) proves the actual pinned entry is present;
/// store statistics and the full ledger snapshot separately prove its accounting.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PinnedPayloadObservation {
    pin_count: u64,
    stats: StoreStats,
    charged_bytes: u64,
    ledger: LedgerSnapshot,
}

fn kind_totals<'a>(mut usage: impl Iterator<Item = &'a UsageBreakdown>) -> Option<(u64, u64, u64)> {
    usage.try_fold((0u64, 0u64, 0u64), |(payload, metadata, scratch), usage| {
        Some((
            payload.checked_add(usage.retained_payload_bytes)?,
            metadata.checked_add(usage.retained_metadata_bytes)?,
            scratch.checked_add(usage.live_scratch_bytes)?,
        ))
    })
}

impl PinnedPayloadObservation {
    fn proves_live_accounting(&self) -> bool {
        let class = kind_totals(self.ledger.class_usage.values());
        let owner = kind_totals(self.ledger.owner_usage.values());
        let context = kind_totals(self.ledger.context_usage.values());
        self.ledger.live_ticket_count > 0
            && !self.ledger.rebuild_in_progress
            && self.ledger.checked_total_class_bytes() == Ok(self.charged_bytes)
            && self.ledger.checked_total_owner_bytes() == Ok(self.charged_bytes)
            && self.ledger.checked_total_context_bytes() == Ok(self.charged_bytes)
            && class.is_some()
            && class == owner
            && owner == context
    }
}

trait PinnedPayloadProbe {
    type Reference;

    fn observe(&self, payload_ref: &Self::Reference) -> MemoryResult<PinnedPayloadObservation>;
    fn delete_if_unpinned(&self, payload_ref: &Self::Reference) -> MemoryResult<()>;
}

struct LivePinnedPayloadProbe<'a> {
    store: &'a SimplePayloadStore<String, Vec<u8>, Budgeted>,
    broker: &'a LiveProcessMemoryBroker,
}

impl PinnedPayloadProbe for LivePinnedPayloadProbe<'_> {
    type Reference = String;
    fn observe(&self, payload_ref: &String) -> MemoryResult<PinnedPayloadObservation> {
        Ok(PinnedPayloadObservation {
            pin_count: self.store.pin_count(payload_ref)?,
            stats: self.store.stats()?,
            charged_bytes: self.store.charged_bytes()?,
            ledger: self.broker.ledger_snapshot()?,
        })
    }

    fn delete_if_unpinned(&self, payload_ref: &String) -> MemoryResult<()> {
        self.store.delete_if_unpinned(payload_ref)
    }
}

/// Both the live driver and the broken targets execute this one classifier.
/// An unreadable observation or a missing reference cannot count as containment.
fn classify_pinned_delete<P: PinnedPayloadProbe>(
    probe: &P,
    payload_ref: &P::Reference,
) -> FaultOutcome {
    let before = probe.observe(payload_ref);
    let delete = probe.delete_if_unpinned(payload_ref);
    let after = probe.observe(payload_ref);
    match (before, delete, after) {
        (Ok(before), Err(error), Ok(after))
            if before.pin_count > 0
                && before.stats.item_count > 0
                && before.charged_bytes > 0
                && before.stats.estimated_bytes == before.charged_bytes
                && before.proves_live_accounting()
                && before == after => FaultOutcome::Contained {
                    evidence: format!(
                        "pinned delete rejected ({error}); reference and bytes survived at {} (pins={} items={})",
                        after.charged_bytes, after.pin_count, after.stats.item_count,
                    ),
                },
        (before, Err(error), after) => FaultOutcome::Escaped {
            evidence: format!("delete rejected ({error}) but pinned payload proof was {before:?}->{after:?}"),
        },
        (_, Ok(()), _) => FaultOutcome::Escaped {
            evidence: "pinned payload was deleted (pin discipline violated)".to_string(),
        },
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
    classify_pinned_delete(
        &LivePinnedPayloadProbe { store: &store, broker: platform.broker() },
        published.payload_ref(),
    )
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

/// Private inventory probe behind a spill controller: the authoritative count the
/// [`classify_spill_failure`] classifier reads before and after the spill. The
/// facade exposes counts only on the concrete routed controller, so this local
/// trait (never exported, never touching facade APIs) lets the single
/// production classifier run unchanged over both the honest controller and
/// test-only fault doubles.
trait SpillCountProbe {
    fn spill_count(&self) -> MemoryResult<u64>;
}

impl SpillCountProbe for RoutedSpillController<InMemorySpillTarget<String>> {
    fn spill_count(&self) -> MemoryResult<u64> {
        self.spilled_count()
    }
}

/// Production spill-failure classifier: reads `spilled_count before → spill →
/// spilled_count after` and owns every `Contained`/`Escaped` branch. Both the
/// public driver and the lying-controller negative pole call this one path, so
/// a broken driver that always reports `Contained` is caught by the tests.
fn classify_spill_failure<C>(controller: &C) -> FaultOutcome
where
    C: SpillController<String> + SpillCountProbe,
{
    let before = controller.spill_count();
    let result = controller.spill("spillable-handle".to_string());
    let after = controller.spill_count();
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

/// NEGATIVE PATH 4: spill controller fails after the target is unavailable.
/// Contained means the spill fails closed and the success count does not move.
#[must_use]
pub fn drive_spill_failure_after_reservation() -> FaultOutcome {
    let controller = RoutedSpillController::new(InMemorySpillTarget::unavailable());
    classify_spill_failure(&controller)
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
    fn pinned_delete_classifier_rejects_reference_loss_and_accounting_lies() {
        use quanta_memory_platform_core::{MemoryContextKind, MemoryError, UsageBreakdown};
        use std::cell::Cell;

        #[derive(Clone, Copy, Debug)]
        enum BrokenPin {
            MissingReference,
            LostEntry,
            StoreDebitDrift,
            ClassKindDrift,
            TicketAuthorityDrift,
            UnreadableProof,
            ReportsSuccess,
            InvalidInitialProof,
        }
        struct BrokenPinProbe {
            fault: BrokenPin,
            deleted: Cell<bool>,
            before: PinnedPayloadObservation,
        }
        impl PinnedPayloadProbe for BrokenPinProbe {
            type Reference = String;

            fn observe(&self, _payload_ref: &String) -> MemoryResult<PinnedPayloadObservation> {
                let mut proof = self.before.clone();
                if matches!(self.fault, BrokenPin::InvalidInitialProof) {
                    let usage = proof
                        .ledger
                        .class_usage
                        .get_mut(&MemoryClass::ArtifactCache)
                        .expect("fixture class");
                    usage.retained_payload_bytes = 0;
                    usage.retained_metadata_bytes = 103;
                }
                if self.deleted.get() {
                    match self.fault {
                        // The debit and item count deliberately still claim survival.
                        BrokenPin::MissingReference => proof.pin_count = 0,
                        BrokenPin::LostEntry => {
                            proof.pin_count = 0;
                            proof.stats.item_count = 0;
                        }
                        BrokenPin::StoreDebitDrift => proof.charged_bytes += 1,
                        BrokenPin::ClassKindDrift => {
                            let usage = proof
                                .ledger
                                .class_usage
                                .get_mut(&MemoryClass::ArtifactCache)
                                .expect("fixture class");
                            usage.retained_payload_bytes = 0;
                            usage.retained_metadata_bytes = 103;
                        }
                        BrokenPin::TicketAuthorityDrift => proof.ledger.live_ticket_count = 0,
                        BrokenPin::UnreadableProof => {
                            return Err(MemoryError::report("pin proof unreadable"))
                        }
                        BrokenPin::ReportsSuccess | BrokenPin::InvalidInitialProof => {}
                    }
                }
                Ok(proof)
            }
            fn delete_if_unpinned(&self, _payload_ref: &String) -> MemoryResult<()> {
                self.deleted.set(true);
                match self.fault {
                    BrokenPin::ReportsSuccess => Ok(()),
                    _ => Err(MemoryError::InvalidState("delete rejected: still pinned".into())),
                }
            }
        }
        let usage = UsageBreakdown { retained_payload_bytes: 103, ..UsageBreakdown::default() };
        let before = PinnedPayloadObservation {
            pin_count: 1,
            stats: StoreStats { item_count: 1, estimated_bytes: 103 },
            charged_bytes: 103,
            ledger: LedgerSnapshot {
                class_usage: [(MemoryClass::ArtifactCache, usage)].into(),
                owner_usage: [("fault:owner".into(), usage)].into(),
                context_usage: [(MemoryContextKind::Request, usage)].into(),
                live_ticket_count: 1,
                rebuild_in_progress: false,
            },
        };
        for fault in [
            BrokenPin::MissingReference,
            BrokenPin::LostEntry,
            BrokenPin::StoreDebitDrift,
            BrokenPin::ClassKindDrift,
            BrokenPin::TicketAuthorityDrift,
            BrokenPin::UnreadableProof,
            BrokenPin::ReportsSuccess,
            BrokenPin::InvalidInitialProof,
        ] {
            let probe = BrokenPinProbe { fault, deleted: Cell::new(false), before: before.clone() };
            let outcome = classify_pinned_delete(&probe, &"payload-0".into());
            assert!(
                matches!(outcome, FaultOutcome::Escaped { .. }),
                "{fault:?} must escape containment through the live classifier: {outcome:?}"
            );
        }
        // Positive pole runs exactly the same classifier through the real adapter.
        assert!(drive_pin_invalidation_failure().contained());
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
    // a rubber stamp. Both poles call the production `classify_spill_failure`
    // helper: a LYING controller (reports success when it must fail closed)
    // proves `Escaped`, and the honest fail-closed controller proves `Contained`.
    #[test]
    fn spill_fault_classification_reports_escaped_for_a_lying_controller() {
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
        impl SpillCountProbe for LyingSpillController {
            fn spill_count(&self) -> MemoryResult<u64> {
                Ok(0)
            }
        }

        let escaped = classify_spill_failure(&LyingSpillController);
        assert!(!escaped.contained(), "a lying controller must classify as Escaped");
        assert!(matches!(escaped, FaultOutcome::Escaped { .. }));
        assert!(
            escaped.evidence().contains("reported success"),
            "lying evidence must name the false success: {}",
            escaped.evidence()
        );

        // Positive pole: the real fail-closed controller is Contained through the
        // same helper code path.
        let honest = RoutedSpillController::new(InMemorySpillTarget::unavailable());
        let contained = classify_spill_failure(&honest);
        assert!(
            contained.contained(),
            "the honest controller fails closed (Contained): {}",
            contained.evidence()
        );
        assert!(contained.evidence().contains("stayed at 0"));
    }

    #[test]
    fn spill_fault_classification_reports_escaped_when_count_drifts() {
        use quanta_memory_platform_core::MemoryError;

        // Fails closed but moves the inventory: the before/after proof disagrees.
        struct DriftingSpillController {
            count: std::cell::Cell<u64>,
        }
        impl SpillController<String> for DriftingSpillController {
            fn spill(
                &self,
                handle: String,
            ) -> Result<
                quanta_memory_platform_facade::SpillReceipt,
                quanta_memory_platform_facade::SpillFailure<String>,
            > {
                self.count.set(1);
                Err(quanta_memory_platform_facade::SpillFailure::new(
                    MemoryError::capacity("spill target unavailable"),
                    handle,
                ))
            }
        }
        impl SpillCountProbe for DriftingSpillController {
            fn spill_count(&self) -> MemoryResult<u64> {
                Ok(self.count.get())
            }
        }

        let controller = DriftingSpillController { count: std::cell::Cell::new(0) };
        let outcome = classify_spill_failure(&controller);
        assert!(!outcome.contained(), "a drifted count must classify as Escaped");
        assert!(outcome.evidence().contains("inventory proof"));
    }

    #[test]
    fn spill_fault_classification_reports_escaped_when_count_is_unreadable() {
        use quanta_memory_platform_core::MemoryError;

        // Fails closed but the inventory cannot be read back.
        struct UnreadableCountController;
        impl SpillController<String> for UnreadableCountController {
            fn spill(
                &self,
                handle: String,
            ) -> Result<
                quanta_memory_platform_facade::SpillReceipt,
                quanta_memory_platform_facade::SpillFailure<String>,
            > {
                Err(quanta_memory_platform_facade::SpillFailure::new(
                    MemoryError::capacity("spill target unavailable"),
                    handle,
                ))
            }
        }
        impl SpillCountProbe for UnreadableCountController {
            fn spill_count(&self) -> MemoryResult<u64> {
                Err(MemoryError::capacity("inventory unreadable"))
            }
        }

        let outcome = classify_spill_failure(&UnreadableCountController);
        assert!(!outcome.contained(), "an unreadable count must classify as Escaped");
        assert!(outcome.evidence().contains("inventory proof"));
    }
}
