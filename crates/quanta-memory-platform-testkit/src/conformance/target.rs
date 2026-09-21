//! The generic conformance target: a host-independent view of a broker's
//! accounting that the conformance suite can exercise without naming any host.
//!
//! Both the real core broker (`LiveProcessMemoryBroker`) and the testkit
//! [`FakeProcessBroker`](crate::fake::FakeProcessBroker) implement
//! [`BrokerConformanceTarget`], so the SAME suite runs against the real contract
//! and against (honest or deliberately broken) fakes.

use std::cell::RefCell;

use quanta_memory_platform_core::{
    AllocatorAdvisory, AllocatorSnapshot, LiveProcessMemoryBroker, ManagedCharge,
    PendingReservation,
};

use crate::fake::FakeProcessBroker;

/// A neutral, read-only view of a broker's accounting state. This is the only
/// surface the invariant checks read; it contains no host nouns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountingView {
    /// Sum of all per-class usage totals.
    pub total_class_bytes: u64,
    /// Sum of all per-owner usage totals.
    pub total_owner_bytes: u64,
    /// Sum of the request context used by this conformance workload.
    pub total_context_bytes: u64,
    /// Exact totals for the two stable class axes.
    pub class_axis_bytes: [u64; 2],
    /// Exact totals for the two stable owners used by the suite.
    pub owner_axis_bytes: [u64; 2],
    /// Owner retained-payload bytes.
    pub total_retained_payload_bytes: u64,
    /// Owner retained-metadata bytes.
    pub total_retained_metadata_bytes: u64,
    /// Owner live-scratch bytes.
    pub total_live_scratch_bytes: u64,
    /// Whether a rebuild window is currently open.
    pub rebuild_in_progress: bool,
}

/// Host-neutral allocator observation fed into the advisory lane.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllocatorObservation {
    pub resident_bytes: Option<u64>,
}

/// Host-neutral advisory result returned by a conformance target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllocatorAdvisoryView {
    pub semantic_bytes: u64,
    pub allocator_resident_bytes: Option<u64>,
    pub mismatch: bool,
    pub drift_bytes: Option<i128>,
}

impl AccountingView {
    /// The partition sum that owner usage must equal (invariant 2).
    #[must_use]
    pub fn partition_sum(&self) -> u64 {
        self.total_retained_payload_bytes
            .saturating_add(self.total_retained_metadata_bytes)
            .saturating_add(self.total_live_scratch_bytes)
    }
}

/// A broker that can be exercised by the conformance suite.
///
/// The methods are deliberately minimal and host-free: enough to drive the
/// accounting algebra and read back a neutral [`AccountingView`].
pub trait BrokerConformanceTarget {
    /// A reservation handle this target hands back from [`Self::do_reserve`].
    /// `Debug` so the suite can emit evidence about reservation results.
    type Ticket: std::fmt::Debug;

    /// Reserve `bytes` of `kind` against (class label, owner). The class is given
    /// by a stable index into [`Self::class_axes`] so the suite never needs to
    /// know a host's concrete class type.
    fn do_reserve(
        &mut self,
        class_axis: usize,
        owner: &str,
        kind: ChargeAxis,
        bytes: u64,
    ) -> Result<Self::Ticket, String>;

    /// Commit a previously reserved ticket.
    fn do_commit(&mut self, ticket: &Self::Ticket) -> Result<(), String>;

    /// Release a ticket (rollback or eviction).
    fn do_release(&mut self, ticket: &Self::Ticket) -> Result<(), String>;

    /// Open the rebuild window.
    fn do_begin_rebuild(&mut self) -> Result<(), String>;

    /// Close the rebuild window.
    fn do_finish_rebuild(&mut self) -> Result<(), String>;

    /// The class axes available on this target, in stable order. Provided: the
    /// two conformance axes the suite addresses by index. Targets are not forced
    /// to restate it (ISP); one with a different taxonomy may override.
    fn class_axes(&self) -> &'static [&'static str] {
        CONFORMANCE_CLASS_AXES
    }

    /// A neutral read of current accounting.
    fn view(&self) -> Result<AccountingView, String>;

    /// Observe allocator telemetry without mutating semantic accounting.
    fn observe_allocator(
        &self,
        observation: &AllocatorObservation,
    ) -> Result<AllocatorAdvisoryView, String>;
}

/// The charge kind, expressed host-independently for the conformance surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargeAxis {
    RetainedPayload,
    RetainedMetadata,
    LiveScratch,
}

/// The two stable conformance class axes used by the suite. Both targets map
/// these onto two distinct concrete classes.
const CONFORMANCE_CLASS_AXES: &[&str] = &["artifact-cache", "evidence-cache"];

/// Testkit-owned state machine around the production RAII authorities. The
/// conformance suite retains reference-based replay probes, but the core crate
/// no longer exposes cloneable raw ledger tickets to support them.
#[derive(Debug)]
pub struct LiveConformanceTicket {
    state: RefCell<LiveConformanceTicketState>,
}

#[derive(Debug)]
enum LiveConformanceTicketState {
    Pending(PendingReservation),
    Committed(ManagedCharge),
    Released,
}

fn map_charge(axis: ChargeAxis) -> quanta_memory_platform_core::ChargeKind {
    use quanta_memory_platform_core::ChargeKind;
    match axis {
        ChargeAxis::RetainedPayload => ChargeKind::RetainedPayload,
        ChargeAxis::RetainedMetadata => ChargeKind::RetainedMetadata,
        ChargeAxis::LiveScratch => ChargeKind::LiveScratch,
    }
}

fn map_class(axis: usize) -> Result<quanta_memory_platform_core::MemoryClass, String> {
    use quanta_memory_platform_core::MemoryClass;
    match axis {
        0 => Ok(MemoryClass::ArtifactCache),
        1 => Ok(MemoryClass::EvidenceCache),
        other => Err(format!("conformance class axis {other} out of range")),
    }
}

impl BrokerConformanceTarget for FakeProcessBroker {
    type Ticket = crate::fake::FakeReservation;

    fn do_reserve(
        &mut self,
        class_axis: usize,
        owner: &str,
        kind: ChargeAxis,
        bytes: u64,
    ) -> Result<Self::Ticket, String> {
        let class = map_class(class_axis)?;
        self.reserve(class, owner.to_string(), map_charge(kind), bytes).map_err(|e| e.to_string())
    }

    fn do_commit(&mut self, ticket: &Self::Ticket) -> Result<(), String> {
        self.commit(ticket).map(|_| ()).map_err(|e| e.to_string())
    }

    fn do_release(&mut self, ticket: &Self::Ticket) -> Result<(), String> {
        self.release(ticket).map_err(|e| e.to_string())
    }

    fn do_begin_rebuild(&mut self) -> Result<(), String> {
        self.begin_rebuild();
        Ok(())
    }

    fn do_finish_rebuild(&mut self) -> Result<(), String> {
        self.finish_rebuild();
        Ok(())
    }

    // class_axes: uses the trait's provided default (CONFORMANCE_CLASS_AXES).

    fn view(&self) -> Result<AccountingView, String> {
        let snap = self.snapshot();
        let class_axis_bytes = [
            snap.class_usage
                .get(&quanta_memory_platform_core::MemoryClass::ArtifactCache)
                .map_or(0, crate::fake::FakeUsage::total),
            snap.class_usage
                .get(&quanta_memory_platform_core::MemoryClass::EvidenceCache)
                .map_or(0, crate::fake::FakeUsage::total),
        ];
        let owner_axis_bytes = [
            snap.owner_usage.get("owner:conformance-a").map_or(0, crate::fake::FakeUsage::total),
            snap.owner_usage.get("owner:conformance-b").map_or(0, crate::fake::FakeUsage::total),
        ];
        Ok(AccountingView {
            total_class_bytes: snap.total_class_bytes(),
            total_owner_bytes: snap.total_owner_bytes(),
            total_context_bytes: snap.total_owner_bytes(),
            class_axis_bytes,
            owner_axis_bytes,
            total_retained_payload_bytes: snap.total_retained_payload_bytes(),
            total_retained_metadata_bytes: snap.total_retained_metadata_bytes(),
            total_live_scratch_bytes: snap.total_live_scratch_bytes(),
            rebuild_in_progress: snap.rebuild_in_progress,
        })
    }

    fn observe_allocator(
        &self,
        observation: &AllocatorObservation,
    ) -> Result<AllocatorAdvisoryView, String> {
        Ok(advisory_view(AllocatorAdvisory::compare(
            &AllocatorSnapshot {
                resident_bytes: observation.resident_bytes,
                ..AllocatorSnapshot::default()
            },
            self.snapshot().total_owner_bytes(),
        )))
    }
}

impl BrokerConformanceTarget for LiveProcessMemoryBroker {
    type Ticket = LiveConformanceTicket;

    fn do_reserve(
        &mut self,
        class_axis: usize,
        owner: &str,
        kind: ChargeAxis,
        bytes: u64,
    ) -> Result<Self::Ticket, String> {
        use quanta_memory_platform_core::MemoryContextKind;
        let class = map_class(class_axis)?;
        // The real broker takes a context kind; the conformance suite charges all
        // bytes against a single Request context (the context axis is checked by
        // the core's own owner-tests, not by the portable suite).
        self.reserve_owned(
            class,
            owner.to_string(),
            MemoryContextKind::Request,
            map_charge(kind),
            bytes,
        )
        .map(|pending| LiveConformanceTicket {
            state: RefCell::new(LiveConformanceTicketState::Pending(pending)),
        })
        .map_err(|e| e.to_string())
    }

    fn do_commit(&mut self, ticket: &Self::Ticket) -> Result<(), String> {
        let current = ticket.state.replace(LiveConformanceTicketState::Released);
        match current {
            LiveConformanceTicketState::Pending(pending) => match pending.commit() {
                Ok(charge) => {
                    ticket.state.replace(LiveConformanceTicketState::Committed(charge));
                    Ok(())
                }
                Err(error) => Err(error.to_string()),
            },
            LiveConformanceTicketState::Committed(charge) => {
                ticket.state.replace(LiveConformanceTicketState::Committed(charge));
                Err("reservation is already committed".to_string())
            }
            LiveConformanceTicketState::Released => {
                Err("reservation is already released".to_string())
            }
        }
    }

    fn do_release(&mut self, ticket: &Self::Ticket) -> Result<(), String> {
        let current = ticket.state.replace(LiveConformanceTicketState::Released);
        match current {
            LiveConformanceTicketState::Pending(pending) => {
                drop(pending);
                Ok(())
            }
            LiveConformanceTicketState::Committed(mut charge) => match charge.release() {
                Ok(()) => Ok(()),
                Err(error) => {
                    ticket.state.replace(LiveConformanceTicketState::Committed(charge));
                    Err(error.to_string())
                }
            },
            LiveConformanceTicketState::Released => {
                Err("reservation is already released".to_string())
            }
        }
    }

    fn do_begin_rebuild(&mut self) -> Result<(), String> {
        LiveProcessMemoryBroker::begin_rebuild(self).map_err(|e| e.to_string())
    }

    fn do_finish_rebuild(&mut self) -> Result<(), String> {
        LiveProcessMemoryBroker::finish_rebuild(self).map_err(|e| e.to_string())
    }

    // class_axes: uses the trait's provided default (CONFORMANCE_CLASS_AXES).

    fn view(&self) -> Result<AccountingView, String> {
        self.ledger_snapshot()
            .map(|snap| AccountingView {
                total_class_bytes: snap.total_class_bytes(),
                total_owner_bytes: snap.total_owner_bytes(),
                total_context_bytes: snap.total_context_bytes(),
                class_axis_bytes: [
                    snap.class_usage
                        .get(&quanta_memory_platform_core::MemoryClass::ArtifactCache)
                        .map_or(0, quanta_memory_platform_core::UsageBreakdown::total),
                    snap.class_usage
                        .get(&quanta_memory_platform_core::MemoryClass::EvidenceCache)
                        .map_or(0, quanta_memory_platform_core::UsageBreakdown::total),
                ],
                owner_axis_bytes: [
                    snap.owner_usage
                        .get("owner:conformance-a")
                        .map_or(0, quanta_memory_platform_core::UsageBreakdown::total),
                    snap.owner_usage
                        .get("owner:conformance-b")
                        .map_or(0, quanta_memory_platform_core::UsageBreakdown::total),
                ],
                total_retained_payload_bytes: snap.total_retained_payload_bytes(),
                total_retained_metadata_bytes: snap.total_retained_metadata_bytes(),
                total_live_scratch_bytes: snap.total_live_scratch_bytes(),
                rebuild_in_progress: snap.rebuild_in_progress,
            })
            .map_err(|error| error.to_string())
    }

    fn observe_allocator(
        &self,
        observation: &AllocatorObservation,
    ) -> Result<AllocatorAdvisoryView, String> {
        LiveProcessMemoryBroker::observe_allocator(
            self,
            &AllocatorSnapshot {
                resident_bytes: observation.resident_bytes,
                ..AllocatorSnapshot::default()
            },
        )
        .map(advisory_view)
        .map_err(|e| e.to_string())
    }
}

fn advisory_view(advisory: AllocatorAdvisory) -> AllocatorAdvisoryView {
    AllocatorAdvisoryView {
        semantic_bytes: advisory.semantic_bytes,
        allocator_resident_bytes: advisory.allocator_resident_bytes,
        mismatch: advisory.mismatch,
        drift_bytes: advisory.drift_bytes(),
    }
}
