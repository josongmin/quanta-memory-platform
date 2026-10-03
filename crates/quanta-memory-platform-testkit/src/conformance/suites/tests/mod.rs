mod cases_01;

use super::*;
use crate::conformance::target::{AccountingView, AllocatorAdvisoryView};
use crate::fake::{AccountingFidelity, FakeProcessBroker, FakeReservation};
use proptest::prelude::*;
use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClass, MemoryClassBudget,
    MemoryContextKind, MemoryEvent, MemoryEventSink, MemoryPreset, ReclaimPriority,
    ResidencyPolicy, ResourceGroup, StoreBindingHandle,
};
use quanta_memory_platform_facade::MemoryPlatformBuilder;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

fn all_held(verdicts: &[AssertionVerdict]) -> bool {
    verdicts.iter().all(AssertionVerdict::held)
}

/// Families that were VIOLATED in a verdict list (deduplicated, order kept).
fn violated(verdicts: &[AssertionVerdict]) -> Vec<ContractFamily> {
    let mut out = Vec::new();
    for v in verdicts.iter().filter(|v| !v.held()) {
        if !out.contains(&v.family) {
            out.push(v.family);
        }
    }
    out
}

#[derive(Clone, Copy, Debug)]
enum BrokerFault {
    BlackHoleAccounting,
    CommitSwallowing,
    ReleaseSwallowing,
    DuplicateCommitAccepting,
    WrongContextAxis,
    WrongChargeKindAxis,
    WrongClassKindAxis,
    WrongContextKindAxis,
    SwapKindAcrossClassBuckets,
    StaleTicketAccepting,
    ObservationError,
    TransferDoubleOwner,
}

struct BrokenBrokerTarget {
    inner: FakeProcessBroker,
    fault: BrokerFault,
}

impl BrokenBrokerTarget {
    fn new(fault: BrokerFault) -> Self {
        Self { inner: FakeProcessBroker::honest(), fault }
    }
}

impl BrokerConformanceTarget for BrokenBrokerTarget {
    type Ticket = FakeReservation;

    fn do_reserve(
        &mut self,
        class_axis: usize,
        owner: &str,
        kind: ChargeAxis,
        bytes: u64,
    ) -> Result<Self::Ticket, String> {
        BrokerConformanceTarget::do_reserve(&mut self.inner, class_axis, owner, kind, bytes)
    }

    fn do_commit(&mut self, ticket: &Self::Ticket) -> Result<(), String> {
        match self.fault {
            BrokerFault::CommitSwallowing => Ok(()),
            BrokerFault::DuplicateCommitAccepting => {
                BrokerConformanceTarget::do_commit(&mut self.inner, ticket).or(Ok(()))
            }
            _ => BrokerConformanceTarget::do_commit(&mut self.inner, ticket),
        }
    }

    fn do_release(&mut self, ticket: &Self::Ticket) -> Result<(), String> {
        match self.fault {
            BrokerFault::ReleaseSwallowing => Ok(()),
            BrokerFault::StaleTicketAccepting => {
                BrokerConformanceTarget::do_release(&mut self.inner, ticket).or(Ok(()))
            }
            _ => BrokerConformanceTarget::do_release(&mut self.inner, ticket),
        }
    }

    type RebuildFence = <crate::fake::FakeProcessBroker as BrokerConformanceTarget>::RebuildFence;

    fn do_begin_rebuild(&mut self) -> Result<Self::RebuildFence, String> {
        BrokerConformanceTarget::do_begin_rebuild(&mut self.inner)
    }

    fn do_finish_rebuild(&mut self, fence: Self::RebuildFence) -> Result<(), String> {
        BrokerConformanceTarget::do_finish_rebuild(&mut self.inner, fence)
    }

    fn view(&self) -> Result<AccountingView, String> {
        if matches!(self.fault, BrokerFault::ObservationError) {
            return Err("injected snapshot failure".into());
        }
        if matches!(self.fault, BrokerFault::BlackHoleAccounting) {
            return Ok(AccountingView::default());
        }
        let mut view = BrokerConformanceTarget::view(&self.inner)?;
        match self.fault {
            BrokerFault::WrongContextAxis => {
                view.context_axis_bytes = [0, view.total_context_bytes, 0, 0, 0];
            }
            BrokerFault::WrongChargeKindAxis => {
                view.total_retained_metadata_bytes = view
                    .total_retained_metadata_bytes
                    .saturating_add(view.total_retained_payload_bytes);
                view.total_retained_payload_bytes = 0;
            }
            // FUP-004: payload<->metadata swap confined to ONE class
            // bucket. Every scalar total still balances; only the per-axis
            // kind vectors diverge.
            BrokerFault::WrongClassKindAxis => {
                view.class_kind_bytes[0].swap(0, 1);
            }
            BrokerFault::WrongContextKindAxis => {
                view.context_kind_bytes[0].swap(0, 1);
            }
            // FUP-004: move payload across class buckets while preserving
            // every aggregate (axis totals AND per-kind global totals).
            // Aggregate checks are blind to this; only the bucket x kind
            // model comparison bites.
            BrokerFault::SwapKindAcrossClassBuckets => {
                let moved = view.class_kind_bytes[0][0].min(1_000);
                view.class_kind_bytes[0][0] -= moved;
                view.class_kind_bytes[1][0] = view.class_kind_bytes[1][0].saturating_add(moved);
                view.class_axis_bytes[0] -= moved;
                view.class_axis_bytes[1] = view.class_axis_bytes[1].saturating_add(moved);
            }
            BrokerFault::TransferDoubleOwner => {
                view.total_owner_bytes = view.total_owner_bytes.saturating_add(1_000);
                view.owner_axis_bytes[1] = view.owner_axis_bytes[1].saturating_add(1_000);
            }
            _ => {}
        }
        Ok(view)
    }

    fn observe_allocator(
        &self,
        observation: &AllocatorObservation,
    ) -> Result<AllocatorAdvisoryView, String> {
        BrokerConformanceTarget::observe_allocator(&self.inner, observation)
    }
}

fn assert_broken_target_rejected(fault: BrokerFault, exact_families: &[ContractFamily]) {
    let mut target = BrokenBrokerTarget::new(fault);
    let verdicts = run_broker_accounting_suite(&mut target);
    assert!(!all_held(&verdicts), "{fault:?} must be rejected");
    assert_eq!(
        violated(&verdicts),
        exact_families,
        "{fault:?} must localize to its exact expected family set: {verdicts:?}"
    );
}

mod model;
use model::{run_independent_trace, IndependentAccountingModel, ModelTicket, ModelTicketState};

#[derive(Default)]
struct ReentrantSink {
    broker: Mutex<Option<LiveProcessMemoryBroker>>,
    entered: AtomicBool,
    reentries: AtomicUsize,
}

impl MemoryEventSink for ReentrantSink {
    fn emit(&self, _event: MemoryEvent) {
        if self.entered.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(broker) = self.broker.lock().expect("sink broker lock").clone() {
            broker.observe().expect("reentrant observation must not deadlock");
            self.reentries.fetch_add(1, Ordering::Release);
        }
    }
}

// --- FUP-004: per-axis kind validation ---------------------------------
//
// Aggregate totals can balance while one axis hides a kind swap. The suite
// must reject axis-confined swaps (per-axis kind vectors) and
// aggregate-preserving cross-bucket swaps (model bucket x kind comparison).

struct UnderbillingPreparedTarget<'a, T>(&'a T);

impl<T: PreparedPublicationConformanceTarget> PreparedPublicationConformanceTarget
    for UnderbillingPreparedTarget<'_, T>
{
    type Token = T::Token;

    fn prepare(
        &self,
        identity: &[u8],
        payload: &[u8],
    ) -> Result<(Self::Token, PrepareDispositionV1), PreparedPublicationErrorV1> {
        self.0.prepare(identity, payload)
    }

    fn resolve(&self, token: &Self::Token) -> Result<Vec<u8>, PreparedPublicationErrorV1> {
        self.0.resolve(token)
    }

    fn abort(&self, token: &Self::Token) -> Result<AbortDispositionV1, PreparedPublicationErrorV1> {
        self.0.abort(token)
    }

    fn orphan_tokens(
        &self,
        referenced_tokens: &[Self::Token],
    ) -> Result<Vec<Self::Token>, PreparedPublicationErrorV1> {
        self.0.orphan_tokens(referenced_tokens)
    }

    fn gc_orphans(
        &self,
        referenced_tokens: &[Self::Token],
    ) -> Result<usize, PreparedPublicationErrorV1> {
        self.0.gc_orphans(referenced_tokens)
    }

    fn stats(&self) -> Result<PreparedPublicationStatsV1, PreparedPublicationErrorV1> {
        self.0.stats().map(|mut stats| {
            stats.charged_bytes = 0;
            stats.tombstone_bytes = 0;
            stats
        })
    }
}

#[derive(Debug, Default)]
struct BrokenPreparedTarget {
    prepares: std::sync::Mutex<u64>,
}

impl PreparedPublicationConformanceTarget for BrokenPreparedTarget {
    type Token = u64;

    fn prepare(
        &self,
        _identity: &[u8],
        _payload: &[u8],
    ) -> Result<(Self::Token, PrepareDispositionV1), PreparedPublicationErrorV1> {
        let mut prepares = self.prepares.lock().map_err(|_| {
            PreparedPublicationErrorV1::Storage(MemoryError::InvalidState("poisoned".to_string()))
        })?;
        *prepares = prepares.saturating_add(1);
        Ok((*prepares, PrepareDispositionV1::Applied))
    }

    fn resolve(&self, _token: &Self::Token) -> Result<Vec<u8>, PreparedPublicationErrorV1> {
        Ok(b"payload".to_vec())
    }

    fn abort(
        &self,
        _token: &Self::Token,
    ) -> Result<AbortDispositionV1, PreparedPublicationErrorV1> {
        Ok(AbortDispositionV1::Aborted)
    }

    fn orphan_tokens(
        &self,
        _referenced_tokens: &[Self::Token],
    ) -> Result<Vec<Self::Token>, PreparedPublicationErrorV1> {
        Ok(Vec::new())
    }

    fn gc_orphans(
        &self,
        _referenced_tokens: &[Self::Token],
    ) -> Result<usize, PreparedPublicationErrorV1> {
        Ok(0)
    }

    fn stats(&self) -> Result<PreparedPublicationStatsV1, PreparedPublicationErrorV1> {
        let prepares = *self.prepares.lock().map_err(|_| {
            PreparedPublicationErrorV1::Storage(MemoryError::InvalidState("poisoned".to_string()))
        })?;
        Ok(PreparedPublicationStatsV1 {
            prepared_count: prepares as usize,
            charged_bytes: prepares.saturating_mul(7),
            ..PreparedPublicationStatsV1::default()
        })
    }
}

#[derive(Debug)]
struct RootBlindPreparedTarget {
    service: PreparedPublicationService<Vec<u8>, Budgeted>,
}

impl RootBlindPreparedTarget {
    fn new() -> Self {
        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("platform");
        Self {
            service: platform
                .prepared_publication_builder::<Vec<u8>>()
                .bind(StoreBindingHandle::new(
                    MemoryClass::ManifestResidency,
                    "root-blind-prepared-conformance".to_string(),
                    None,
                ))
                .build()
                .expect("bound prepared builder"),
        }
    }
}

impl PreparedPublicationConformanceTarget for RootBlindPreparedTarget {
    type Token = PreparedPublicationTokenV1;

    fn prepare(
        &self,
        identity: &[u8],
        payload: &[u8],
    ) -> Result<(Self::Token, PrepareDispositionV1), PreparedPublicationErrorV1> {
        PreparedPublicationConformanceTarget::prepare(&self.service, identity, payload)
    }

    fn resolve(&self, token: &Self::Token) -> Result<Vec<u8>, PreparedPublicationErrorV1> {
        self.service.resolve(token)
    }

    fn abort(&self, token: &Self::Token) -> Result<AbortDispositionV1, PreparedPublicationErrorV1> {
        self.service.abort(token)
    }

    fn orphan_tokens(
        &self,
        _referenced_tokens: &[Self::Token],
    ) -> Result<Vec<Self::Token>, PreparedPublicationErrorV1> {
        self.service.orphan_tokens(&[])
    }

    fn gc_orphans(
        &self,
        _referenced_tokens: &[Self::Token],
    ) -> Result<usize, PreparedPublicationErrorV1> {
        self.service.gc_orphans(&[])
    }

    fn stats(&self) -> Result<PreparedPublicationStatsV1, PreparedPublicationErrorV1> {
        self.service.stats()
    }
}

#[derive(Debug, Default)]
struct BrokenTranscriptTarget {
    rows: std::sync::Mutex<Vec<TranscriptRow>>,
}

impl TranscriptAppendConformanceTarget for BrokenTranscriptTarget {
    fn append(
        &self,
        _session_id: &[u8],
        sequence: u64,
        message_id: &[u8],
        body: &[u8],
    ) -> Result<TranscriptAppendDispositionV1, TranscriptAppendErrorV1> {
        self.rows
            .lock()
            .map_err(|_| {
                TranscriptAppendErrorV1::Storage(MemoryError::InvalidState("poisoned".to_string()))
            })?
            .push((sequence, message_id.to_vec(), body.to_vec()));
        Ok(TranscriptAppendDispositionV1::Applied)
    }

    fn load(&self, _session_id: &[u8]) -> Result<Vec<TranscriptRow>, MemoryError> {
        self.rows
            .lock()
            .map(|rows| rows.clone())
            .map_err(|_| MemoryError::InvalidState("poisoned".to_string()))
    }

    fn truncate_to(&self, _session_id: &[u8], max_messages: usize) -> Result<(), MemoryError> {
        self.rows
            .lock()
            .map(|mut rows| rows.truncate(max_messages))
            .map_err(|_| MemoryError::InvalidState("poisoned".to_string()))
    }

    fn retained_bytes(&self) -> Result<u64, MemoryError> {
        self.rows
            .lock()
            .map(|rows| rows.iter().map(|(_, _, body)| body.len() as u64).sum())
            .map_err(|_| MemoryError::InvalidState("poisoned".to_string()))
    }
}

struct UnderbillingTranscriptTarget<'a, T>(&'a T);

impl<T: TranscriptAppendConformanceTarget> TranscriptAppendConformanceTarget
    for UnderbillingTranscriptTarget<'_, T>
{
    fn append(
        &self,
        session_id: &[u8],
        sequence: u64,
        message_id: &[u8],
        body: &[u8],
    ) -> Result<TranscriptAppendDispositionV1, TranscriptAppendErrorV1> {
        self.0.append(session_id, sequence, message_id, body)
    }

    fn load(&self, session_id: &[u8]) -> Result<Vec<TranscriptRow>, MemoryError> {
        self.0.load(session_id)
    }

    fn truncate_to(&self, session_id: &[u8], max_messages: usize) -> Result<(), MemoryError> {
        self.0.truncate_to(session_id, max_messages)
    }

    fn retained_bytes(&self) -> Result<u64, MemoryError> {
        Ok(0)
    }
}

// -----------------------------------------------------------------------
// META-REQUIREMENT: every AccountingFidelity break variant is rejected.
// -----------------------------------------------------------------------

/// Every non-honest [`AccountingFidelity`] the fake can model. Adding a new
/// break variant to the enum and forgetting to list it here will leave a hole
/// in the meta-coverage; this is the single place that enumerates them.
fn all_break_variants() -> Vec<AccountingFidelity> {
    // Compile-time tie to the enum: this exhaustive match (no wildcard) fails
    // to compile when a new AccountingFidelity variant is added, forcing the
    // author to also list it below. Without this, a new break variant could
    // silently escape `suite_rejects_every_accounting_fidelity_break_variant`.
    fn _exhaustiveness_guard(v: AccountingFidelity) {
        match v {
            AccountingFidelity::Honest
            | AccountingFidelity::UnderReportOwnerBytes(_)
            | AccountingFidelity::DivergeClassFromOwner(_)
            | AccountingFidelity::DivergeOwnerPartition(_) => {}
        }
    }
    vec![
        AccountingFidelity::UnderReportOwnerBytes(100),
        AccountingFidelity::DivergeClassFromOwner(64),
        AccountingFidelity::DivergeOwnerPartition(48),
    ]
}

// Guards that the hand-maintained `all_break_variants()` list stays complete:
// every entry is a distinct break variant (not Honest) and maps to a family.

/// The contract family each break variant must localize to. A variant that
/// failed to localize here (or localized to the wrong family) would mean the
/// suite caught "something" but not the SPECIFIC contract the lie violates.
fn expected_family(variant: AccountingFidelity) -> ContractFamily {
    match variant {
        // Under-reporting owner bytes / inflating class both desync class from
        // owner -> invariant 1.
        AccountingFidelity::UnderReportOwnerBytes(_)
        | AccountingFidelity::DivergeClassFromOwner(_) => ContractFamily::ClassOwnerBalance,
        // Inflating the totals while leaving the partition honest keeps class
        // == owner but breaks owner == partition_sum -> invariant 2.
        AccountingFidelity::DivergeOwnerPartition(_) => ContractFamily::OwnerPartition,
        AccountingFidelity::Honest => unreachable!("Honest is not a break variant"),
    }
}

// -----------------------------------------------------------------------
// ReserveFailClosed: a target that MUTATES on a denied write is rejected.
// -----------------------------------------------------------------------

/// A conformance target that lies about fail-closed: it admits (mutates) a
/// reserve even while the rebuild window is open. The new in-suite
/// `ReserveFailClosed` check must catch this. Implemented locally because the
/// stock fake is honest and the trait does not expose a "be dishonest" knob.
struct MutateOnDenyTarget {
    owner_payload_bytes: u64,
    class_payload_bytes: u64,
    rebuild: bool,
    next_id: u64,
}

impl MutateOnDenyTarget {
    fn new() -> Self {
        Self { owner_payload_bytes: 0, class_payload_bytes: 0, rebuild: false, next_id: 1 }
    }
}

impl BrokerConformanceTarget for MutateOnDenyTarget {
    type Ticket = u64;
    type RebuildFence = ();

    fn do_reserve(
        &mut self,
        _class_axis: usize,
        _owner: &str,
        _kind: ChargeAxis,
        bytes: u64,
    ) -> Result<Self::Ticket, String> {
        // THE LIE: while rebuilding, return Err (so the RebuildWriteFence
        // check still sees a rejection) but mutate the counters first. A
        // correct broker validates and bails BEFORE touching state.
        if self.rebuild {
            self.owner_payload_bytes = self.owner_payload_bytes.saturating_add(bytes);
            self.class_payload_bytes = self.class_payload_bytes.saturating_add(bytes);
            return Err("rebuild in progress".to_string());
        }
        self.owner_payload_bytes = self.owner_payload_bytes.saturating_add(bytes);
        self.class_payload_bytes = self.class_payload_bytes.saturating_add(bytes);
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        Ok(id)
    }

    fn do_commit(&mut self, _ticket: &Self::Ticket) -> Result<(), String> {
        Ok(())
    }

    fn do_release(&mut self, _ticket: &Self::Ticket) -> Result<(), String> {
        self.owner_payload_bytes = 0;
        self.class_payload_bytes = 0;
        Ok(())
    }

    fn do_begin_rebuild(&mut self) -> Result<(), String> {
        self.rebuild = true;
        Ok(())
    }

    fn do_finish_rebuild(&mut self, _fence: Self::RebuildFence) -> Result<(), String> {
        self.rebuild = false;
        Ok(())
    }

    fn class_axes(&self) -> &'static [&'static str] {
        &["a", "b"]
    }

    fn view(&self) -> Result<AccountingView, String> {
        Ok(AccountingView {
            total_class_bytes: self.class_payload_bytes,
            total_owner_bytes: self.owner_payload_bytes,
            total_context_bytes: self.owner_payload_bytes,
            context_axis_bytes: [self.owner_payload_bytes, 0, 0, 0, 0],
            class_axis_bytes: [self.class_payload_bytes, 0],
            owner_axis_bytes: [self.owner_payload_bytes, 0],
            group_axis_bytes: [0, 0],
            class_kind_bytes: [[self.class_payload_bytes, 0, 0], [0, 0, 0]],
            owner_kind_bytes: [[self.owner_payload_bytes, 0, 0], [0, 0, 0]],
            context_kind_bytes: [
                [self.owner_payload_bytes, 0, 0],
                [0, 0, 0],
                [0, 0, 0],
                [0, 0, 0],
                [0, 0, 0],
            ],
            live_ticket_count: 0,
            total_retained_payload_bytes: self.owner_payload_bytes,
            total_retained_metadata_bytes: 0,
            total_live_scratch_bytes: 0,
            rebuild_in_progress: self.rebuild,
        })
    }

    fn observe_allocator(
        &self,
        observation: &AllocatorObservation,
    ) -> Result<AllocatorAdvisoryView, String> {
        let semantic_bytes = self.owner_payload_bytes;
        Ok(AllocatorAdvisoryView {
            semantic_bytes,
            allocator_resident_bytes: observation.resident_bytes,
            mismatch: observation.resident_bytes.is_some_and(|resident| resident != semantic_bytes),
            drift_bytes: observation
                .resident_bytes
                .map(|resident| i128::from(resident) - i128::from(semantic_bytes)),
        })
    }
}

struct ObserveMutatesTarget(std::cell::Cell<u64>);

impl ObserveMutatesTarget {
    fn new() -> Self {
        Self(std::cell::Cell::new(0))
    }
}

impl BrokerConformanceTarget for ObserveMutatesTarget {
    type Ticket = ();
    type RebuildFence = ();

    fn do_reserve(
        &mut self,
        _class_axis: usize,
        _owner: &str,
        _kind: ChargeAxis,
        bytes: u64,
    ) -> Result<Self::Ticket, String> {
        self.0.set(self.0.get().saturating_add(bytes));
        Ok(())
    }

    fn do_commit(&mut self, _ticket: &Self::Ticket) -> Result<(), String> {
        Ok(())
    }

    fn do_release(&mut self, _ticket: &Self::Ticket) -> Result<(), String> {
        self.0.set(0);
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
        let bytes = self.0.get();
        Ok(AccountingView {
            total_class_bytes: bytes,
            total_owner_bytes: bytes,
            total_context_bytes: bytes,
            context_axis_bytes: [bytes, 0, 0, 0, 0],
            class_axis_bytes: [bytes, 0],
            owner_axis_bytes: [bytes, 0],
            group_axis_bytes: [0, 0],
            class_kind_bytes: [[bytes, 0, 0], [0, 0, 0]],
            owner_kind_bytes: [[bytes, 0, 0], [0, 0, 0]],
            context_kind_bytes: [[bytes, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0]],
            live_ticket_count: 0,
            total_retained_payload_bytes: bytes,
            total_retained_metadata_bytes: 0,
            total_live_scratch_bytes: 0,
            rebuild_in_progress: false,
        })
    }

    fn observe_allocator(
        &self,
        observation: &AllocatorObservation,
    ) -> Result<AllocatorAdvisoryView, String> {
        let before = self.0.get();
        self.0.set(before.saturating_add(1));
        Ok(AllocatorAdvisoryView {
            semantic_bytes: before,
            allocator_resident_bytes: observation.resident_bytes,
            mismatch: observation.resident_bytes.is_some_and(|resident| resident != before),
            drift_bytes: observation
                .resident_bytes
                .map(|resident| i128::from(resident) - i128::from(before)),
        })
    }
}

// -----------------------------------------------------------------------
// commit -> release returns the balance to zero (honest fake and real).
// -----------------------------------------------------------------------

// -----------------------------------------------------------------------
// Cap-deny AGREEMENT: honest fake and real broker deny the same way.
// -----------------------------------------------------------------------

fn live_broker() -> LiveProcessMemoryBroker {
    let residency = ResidencyPolicy::new(10_000_000)
        .with_class_budget(
            MemoryClass::ArtifactCache,
            MemoryClassBudget {
                reserved_bytes: 0,
                soft_cap_bytes: 4_000,
                hard_cap_bytes: 6_000,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        )
        .with_class_budget(
            MemoryClass::EvidenceCache,
            MemoryClassBudget {
                reserved_bytes: 0,
                soft_cap_bytes: 4_000,
                hard_cap_bytes: 6_000,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        );
    quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Ci,
            quanta_memory_platform_core::ExecutionBudget::default(),
            residency,
        ))
        .build()
        .expect("valid residency")
}

/// A spill target that CLAIMS to accept but retains nothing (drops the handle),
/// so a restore can never bring it back — the SpillRouting fault.
#[cfg(feature = "fixtures")]
#[derive(Debug, Default)]
struct DroppingSpillTarget;

#[cfg(feature = "fixtures")]
impl quanta_memory_platform_facade::SpillTarget<String> for DroppingSpillTarget {
    fn is_available(&self) -> bool {
        true
    }
    fn accept(
        &self,
        _handle: String,
    ) -> Result<
        quanta_memory_platform_facade::SpillReceipt,
        quanta_memory_platform_facade::SpillFailure<String>,
    > {
        Ok(quanta_memory_platform_facade::SpillReceipt::InProcess { inventory_after: 0 })
    }
    // restore uses the trait default -> always Ok(None): nothing comes back.
}

#[cfg(feature = "fixtures")]
impl quanta_memory_platform_facade::SpillInventory for DroppingSpillTarget {
    fn retained_count(&self) -> quanta_memory_platform_core::MemoryResult<u64> {
        Ok(0)
    }
}

// The SpillRouting verdict must be able to VIOLATE: driving the SUITE's own
// `spill_round_trip_verdict` against an honest in-memory target HOLDS, against a
// handle-dropping target VIOLATES. A check that passed the dropping target —
// or a weakened suite helper — would be a rubber stamp.
