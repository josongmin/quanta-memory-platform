//! Live broker control plane: an accounting-backed broker with reserve /
//! commit / release, resource-group admission, pressure classification, and
//! fail-closed live reconfiguration.
//!
//! This is the concrete control-plane surface. It wraps the semantic
//! [`AccountingLedger`] (SSOT) behind a lock and composes the
//! application-layer scheduler and pressure controller. Allocator telemetry is
//! observed only through the advisory lane and never mutates the ledger.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::adapters::scratch::BrokerBoundScratchAllocator;
use crate::application::controller::LadderPressureController;
use crate::application::scheduler::WeightedResourceGroupScheduler;
use crate::domain::admission::{AdmissionRequest, AdmissionVerdict, PreflightInputs};
use crate::domain::config::{OverrideRecord, OverrideSource, ValidatedResidencyPolicy};
use crate::domain::context::{
    MemoryContextHandle, MemoryContextKind, OwnerKey, RuntimeId, RuntimeKind,
};
use crate::domain::errors::{MemoryError, MemoryResult};
use crate::domain::ledger::{AccountingLedger, ChargeKind, LedgerSnapshot, ReservationTicket};
use crate::domain::policy::{MemoryClass, ReclaimPriority, ResidencyPolicy};
use crate::domain::recharge::{RechargeError, RechargeRecoveryV1};
use crate::domain::stats::{
    AllocatorSnapshot, BrokerObservation, EffectiveMemoryBudgetReport, PressureState,
    ReclaimRuntimeStats,
};
use crate::ports::reclaim::{
    MemoryReclaimer, ReclaimCandidate, ReclaimGroupCandidate, ReclaimRequest, ReclaimTrigger,
};
use crate::ports::scratch::ScratchAllocator;
use crate::ports::store::StoreBindingHandle;
#[cfg(test)]
use crate::ports::telemetry::NoopMemoryEventSink;
use crate::ports::telemetry::{
    AllocatorAdvisory, MemoryEvent, MemoryEventSink, MemoryOperation, MemoryOutcome,
};

mod admission;
mod authority;
mod observation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrokerCallbackKind {
    Reclaimer,
}

thread_local! {
    /// Synchronous host callbacks can call back into a broker on the same
    /// thread. Track only active callback identities (not all operations), so
    /// independent concurrent requests on other threads remain unaffected.
    static ACTIVE_BROKER_CALLBACKS: RefCell<Vec<(BrokerCallbackKind, usize)>> = const {
        RefCell::new(Vec::new())
    };
}

struct BrokerCallbackScope {
    kind: BrokerCallbackKind,
    identity: usize,
}

impl BrokerCallbackScope {
    fn enter(kind: BrokerCallbackKind, identity: usize) -> Option<Self> {
        ACTIVE_BROKER_CALLBACKS.with(|active| {
            let mut active = active.borrow_mut();
            if active.contains(&(kind, identity)) {
                return None;
            }
            active.push((kind, identity));
            Some(Self { kind, identity })
        })
    }

    fn is_active(kind: BrokerCallbackKind, identity: usize) -> bool {
        ACTIVE_BROKER_CALLBACKS.with(|active| active.borrow().contains(&(kind, identity)))
    }
}

impl Drop for BrokerCallbackScope {
    fn drop(&mut self) {
        ACTIVE_BROKER_CALLBACKS.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(index) =
                active.iter().rposition(|entry| *entry == (self.kind, self.identity))
            {
                active.remove(index);
            }
        });
    }
}

struct CallbackPermit<'a>(&'a AtomicBool);

impl CallbackPermit<'_> {
    fn try_enter(active: &AtomicBool) -> Option<CallbackPermit<'_>> {
        active
            .compare_exchange(false, true, AtomicOrdering::Acquire, AtomicOrdering::Relaxed)
            .ok()
            .map(|_| CallbackPermit(active))
    }
}

impl Drop for CallbackPermit<'_> {
    fn drop(&mut self) {
        self.0.store(false, AtomicOrdering::Release);
    }
}

/// Retire an opaque host panic payload outside the broker lock. A malicious
/// payload may panic again in `Drop`; leaking that *one* secondary payload is
/// preferable to allowing a host destructor to unwind through accounting.
/// Callers quarantine the callback before invoking this, so the leak cannot
/// grow with repeated broker operations.
fn retire_host_panic_payload(payload: Box<dyn std::any::Any + Send>) {
    if let Err(drop_panic) =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(payload)))
    {
        std::mem::forget(drop_panic);
    }
}

/// A live, accounting-backed process memory broker.
///
/// All mutating operations route through the [`AccountingLedger`] so the global
/// invariants hold. Reserve fails closed (no mutation on deny). Live reconfig
/// rejects illegal narrowing while leaving the active policy untouched.
#[derive(Clone)]
pub struct LiveProcessMemoryBroker {
    inner: Arc<Mutex<BrokerCore>>,
    events: Arc<dyn MemoryEventSink>,
    /// Serialize advisory sink calls across broker clones and threads. Events
    /// arriving while a sink callback is active are dropped to bound reentry.
    event_sink_active: Arc<AtomicBool>,
    /// A panicking host sink is not retried: its opaque panic payload cannot
    /// be guaranteed safe to destroy, so repeated invocation is unbounded.
    event_sink_quarantined: Arc<AtomicBool>,
    /// At most one external reclaim callback may execute for this broker at a
    /// time. This cross-thread gate complements the same-thread reentry guard.
    reclaimer_active: Arc<AtomicBool>,
    reclaimer_quarantined: Arc<AtomicBool>,
}

struct BrokerCore {
    ledger: AccountingLedger,
    scheduler: WeightedResourceGroupScheduler,
    controller: Arc<LadderPressureController>,
    report: EffectiveMemoryBudgetReport,
    /// One issuing-scope record per live ticket, including zero-byte tickets.
    /// This is the joint owner/class/group axis needed to rebuild the derived
    /// scheduler without guessing class attribution from marginal snapshots.
    ticket_attribution: BTreeMap<u64, TicketAttribution>,
    /// Committed charges whose last owning lease was dropped while the rebuild
    /// fence was closed. They remain charged until `finish_rebuild` opens the
    /// fence and releases bounded atomic batches. This queue is a subset of live
    /// ticket authorities, so the ledger ticket cap also bounds its backlog.
    deferred_committed_releases: VecDeque<ReservationTicket>,
    /// Pending tickets whose `Drop` rollback failed outside the normal path
    /// (reachable only through an internal invariant break once reconfigure
    /// guards live-ticket scope). The exact ticket authority is preserved here
    /// instead of being silently discarded, and retried on the next
    /// `finish_rebuild` or accepted reconfigure; failures stay queued.
    deferred_pending_rollbacks: VecDeque<ReservationTicket>,
    /// Monotonic epoch of the current rebuild fence. Only the permit minted
    /// with the matching epoch may finish the fence; a stale or foreign
    /// permit is rejected fail-closed.
    rebuild_epoch: u64,
    /// True when the owning [`RebuildFence`] was dropped without finishing:
    /// the fence stays closed (writes rejected) and is observable through
    /// [`LiveProcessMemoryBroker::rebuild_fence_status`]; only an explicit
    /// new `begin_rebuild` re-adopts it.
    rebuild_abandoned: bool,
    reclaimer: Option<Arc<dyn MemoryReclaimer>>,
    reclaim_stats: ReclaimRuntimeStats,
}

#[derive(Clone, Debug)]
struct TicketAttribution {
    owner: OwnerKey,
    class: MemoryClass,
    group: Option<String>,
    bytes: u64,
}

impl TicketAttribution {
    fn new(ticket: &ReservationTicket, group: Option<String>) -> Self {
        Self {
            owner: ticket.owner().clone(),
            class: ticket.class().clone(),
            group,
            bytes: ticket.bytes(),
        }
    }
}

/// An owned pending reservation. This type is intentionally not `Clone`.
/// Dropping it before commit rolls its class/owner/context/group capacity back,
/// including while rebuild fencing is active.
#[derive(Debug)]
pub struct PendingReservation {
    broker: LiveProcessMemoryBroker,
    ticket: Option<ReservationTicket>,
}

impl PendingReservation {
    #[must_use]
    pub fn id(&self) -> u64 {
        self.ticket.as_ref().expect("pending reservation owns a ticket").id()
    }

    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.ticket.as_ref().expect("pending reservation owns a ticket").bytes()
    }

    /// Consume this pending authority and produce one broker-bound committed
    /// lease. A failed commit rolls the pending reservation back before
    /// returning; dropping the successful lease releases the charge.
    pub fn commit(mut self) -> MemoryResult<ManagedCharge> {
        let ticket = self.ticket.as_ref().expect("pending reservation owns a ticket");
        match self.broker.commit(ticket) {
            Ok(_) => Ok(ManagedCharge {
                broker: self.broker.clone(),
                charge: Some(CommittedCharge {
                    ticket: self.ticket.take().expect("successful commit retains ticket authority"),
                }),
            }),
            Err(error) => {
                // Do not remove the only rollback authority until rollback has
                // succeeded. In particular, a poisoned broker rejects both the
                // commit and the ordinary rollback. Leaving the ticket in
                // `self` makes Drop execute the poison-recovering exact rollback
                // before this method returns the original commit error.
                if self.broker.rollback_pending_ticket(ticket).is_ok() {
                    self.ticket = None;
                }
                Err(error)
            }
        }
    }

    pub(crate) fn rollback(&mut self) -> MemoryResult<()> {
        if let Some(ticket) = self.ticket.as_ref() {
            self.broker.rollback_pending_ticket(ticket)?;
            self.ticket = None;
        }
        Ok(())
    }
}

impl Drop for PendingReservation {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            self.broker.rollback_pending_ticket_on_drop(&ticket);
        }
    }
}

struct LiveRuntimeMemoryView {
    runtime_id: RuntimeId,
    runtime_kind: RuntimeKind,
    scratch: BrokerBoundScratchAllocator,
}

impl LiveRuntimeMemoryView {
    fn new(
        broker: LiveProcessMemoryBroker,
        runtime_id: RuntimeId,
        runtime_kind: RuntimeKind,
    ) -> Self {
        Self {
            scratch: BrokerBoundScratchAllocator::new(broker, runtime_id.clone()),
            runtime_id,
            runtime_kind,
        }
    }
}

impl crate::api::broker::RuntimeMemoryView for LiveRuntimeMemoryView {
    fn runtime_id(&self) -> &RuntimeId {
        &self.runtime_id
    }

    fn runtime_kind(&self) -> RuntimeKind {
        self.runtime_kind
    }

    fn context(&self, kind: MemoryContextKind, owner: OwnerKey) -> MemoryContextHandle {
        MemoryContextHandle::new(self.runtime_id.clone(), kind, owner)
    }

    fn bind_store(&self, class: MemoryClass, owner: OwnerKey) -> StoreBindingHandle {
        StoreBindingHandle::new(class, owner, Some(self.runtime_id.clone()))
    }

    fn scratch(&self) -> &dyn ScratchAllocator {
        &self.scratch
    }
}

/// Internal single-owner committed accounting authority. It is always wrapped
/// by [`ManagedCharge`] before crossing the public boundary.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommittedCharge {
    ticket: ReservationTicket,
}

impl CommittedCharge {
    #[must_use]
    pub(crate) fn id(&self) -> u64 {
        self.ticket.id()
    }

    #[must_use]
    pub(crate) fn bytes(&self) -> u64 {
        self.ticket.bytes()
    }

    #[must_use]
    pub(crate) fn class(&self) -> &MemoryClass {
        self.ticket.class()
    }
}

/// Failed consuming release. The unchanged live charge is returned so the
/// caller never loses accounting authority on an error path.
#[derive(Debug)]
pub(crate) struct ChargeReleaseError {
    pub(crate) error: MemoryError,
    pub(crate) charge: CommittedCharge,
}

/// Recovery disposition for a failed consuming recharge.
#[derive(Debug)]
pub(crate) enum ChargeRechargeRecovery {
    ChargePreserved(CommittedCharge),
}

/// Failed consuming recharge with explicit ownership recovery.
#[derive(Debug)]
pub(crate) struct ChargeRechargeError {
    pub(crate) error: MemoryError,
    pub(crate) recovery: ChargeRechargeRecovery,
}

/// Recovery disposition for a failed [`ManagedCharge::recharge`]. The lease
/// itself retains a usable charge for `ChargePreserved`; callers never have to
/// move accounting authority through an error value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagedChargeRechargeRecovery {
    ChargePreserved,
    RecoveryRequired { rollback_error: MemoryError },
}

/// Failed managed recharge. Inspect `recovery` to distinguish a denied
/// replacement whose old charge remains owned by the lease from an internal
/// rollback failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedChargeRechargeError {
    pub error: MemoryError,
    pub recovery: ManagedChargeRechargeRecovery,
}

/// Broker-bound, single-owner committed accounting lease.
///
/// The lease is intentionally not `Clone`; dropping the final authority releases
/// its charge, or defers that exact release when a rebuild fence is active.
///
/// ```compile_fail
/// # use quanta_memory_platform_core::ManagedCharge;
/// # fn cannot_clone(charge: ManagedCharge) {
/// let duplicate = charge.clone();
/// # }
/// ```
#[derive(Debug)]
pub struct ManagedCharge {
    broker: LiveProcessMemoryBroker,
    charge: Option<CommittedCharge>,
}

impl ManagedCharge {
    /// Whether this live lease was issued by the given broker authority.
    /// Transfer consumers must check this before attaching a charge to a
    /// different store; matching ticket numbers are not broker identity.
    #[must_use]
    pub fn belongs_to(&self, broker: &LiveProcessMemoryBroker) -> bool {
        Arc::ptr_eq(&self.broker.inner, &broker.inner)
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.charge.is_some()
    }

    #[must_use]
    pub fn id(&self) -> Option<u64> {
        self.charge.as_ref().map(CommittedCharge::id)
    }

    #[must_use]
    pub fn bytes(&self) -> Option<u64> {
        self.charge.as_ref().map(CommittedCharge::bytes)
    }

    #[must_use]
    pub fn class(&self) -> Option<&MemoryClass> {
        self.charge.as_ref().map(CommittedCharge::class)
    }

    #[must_use]
    pub fn owner(&self) -> Option<&OwnerKey> {
        self.charge.as_ref().map(|charge| charge.ticket.owner())
    }

    #[must_use]
    pub fn context(&self) -> Option<MemoryContextKind> {
        self.charge.as_ref().map(|charge| charge.ticket.context())
    }

    #[must_use]
    pub fn kind(&self) -> Option<ChargeKind> {
        self.charge.as_ref().map(|charge| charge.ticket.kind())
    }

    /// Release this charge explicitly. A failed release restores the unchanged
    /// committed authority into this lease so the caller can retry or drop it.
    pub fn release(&mut self) -> MemoryResult<()> {
        let charge = self.charge.take().ok_or_else(|| {
            MemoryError::InvalidState("managed charge is already released".to_string())
        })?;
        match self.broker.release_owned(charge) {
            Ok(()) => Ok(()),
            Err(ChargeReleaseError { error, charge }) => {
                self.charge = Some(charge);
                Err(error)
            }
        }
    }

    /// Replace this live charge atomically. A normal denial restores the old (or
    /// ledger-restored) committed authority into this lease before returning.
    pub fn recharge(
        &mut self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> Result<(), ManagedChargeRechargeError> {
        let old = self.charge.take().ok_or_else(|| ManagedChargeRechargeError {
            error: MemoryError::InvalidState("managed charge is already released".to_string()),
            recovery: ManagedChargeRechargeRecovery::RecoveryRequired {
                rollback_error: MemoryError::InvalidState(
                    "no committed authority remains to restore".to_string(),
                ),
            },
        })?;
        match self.broker.recharge_owned(old, class, owner, context, kind, bytes) {
            Ok(replacement) => {
                self.charge = Some(replacement);
                Ok(())
            }
            Err(ChargeRechargeError {
                error,
                recovery: ChargeRechargeRecovery::ChargePreserved(charge),
            }) => {
                self.charge = Some(charge);
                Err(ManagedChargeRechargeError {
                    error,
                    recovery: ManagedChargeRechargeRecovery::ChargePreserved,
                })
            }
        }
    }
}

impl Drop for ManagedCharge {
    fn drop(&mut self) {
        if let Some(charge) = self.charge.take() {
            self.broker.release_managed_charge_on_drop(charge);
        }
    }
}

/// Observable state of the rebuild fence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebuildFenceStatus {
    /// No fence: admission is open.
    Unfenced,
    /// One live coordinator holds the fence permit.
    Active,
    /// The owning permit was dropped without finishing: writes stay
    /// rejected until an explicit new `begin_rebuild` re-adopts the fence.
    Abandoned,
}

/// Receipt separating fence completion from deferred-authority cleanup.
///
/// `fence_open` reports the fence transition itself; the remaining counts
/// and `drain_error` report the bounded-batch cleanup that runs after the
/// fence opens. A drain error never re-closes the fence — retry cleanup
/// through the next accepted fence finish (an accepted live reconfigure
/// retries only deferred pending rollbacks, not deferred committed
/// releases). On a broker-lock failure the remaining counts are reported
/// as `usize::MAX` sentinels alongside the typed `drain_error`; treat any
/// `usize::MAX` count as "unknown", never as a literal depth.
///
/// A caller compiling with warnings denied cannot silently discard cleanup:
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use quanta_memory_platform_core::{MemoryResult, RebuildFence};
/// fn bad(fence: RebuildFence) -> MemoryResult<()> {
///     fence.finish()?;
///     Ok(())
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use = "inspect fence completion and deferred cleanup outcome"]
pub struct RebuildReceipt {
    /// True when this finish committed the fence-open transition.
    pub fence_open: bool,
    /// Deferred pending-rollback authorities still queued after this call.
    pub pending_rollbacks_remaining: usize,
    /// Deferred committed-release authorities still queued after this call.
    pub committed_releases_remaining: usize,
    /// First error hit while draining deferred authorities, if any.
    pub drain_error: Option<MemoryError>,
}

/// Non-Clone, epoch-bound authority over one rebuild fence.
///
/// Obtained from [`LiveProcessMemoryBroker::begin_rebuild`]. Dropping it
/// without [`Self::finish`] abandons the fence closed (observable via
/// [`LiveProcessMemoryBroker::rebuild_fence_status`]) instead of silently
/// reopening writes; recovery is an explicit new `begin_rebuild`.
///
/// A caller compiling with warnings denied cannot discard the permit:
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use quanta_memory_platform_core::{LiveProcessMemoryBroker, MemoryResult};
/// fn bad(broker: &LiveProcessMemoryBroker) -> MemoryResult<()> {
///     broker.begin_rebuild()?;
///     Ok(())
/// }
/// ```
#[must_use = "finish or explicitly recover this rebuild fence"]
pub struct RebuildFence {
    broker: LiveProcessMemoryBroker,
    epoch: u64,
    finished: bool,
}

impl RebuildFence {
    /// Commit the fence open and drain deferred authorities in bounded
    /// batches. Fence completion and the cleanup outcome are reported
    /// separately in the returned receipt.
    pub fn finish(mut self) -> MemoryResult<RebuildReceipt> {
        self.finished = true;
        self.broker.finish_rebuild_epoch(self.epoch)
    }

    /// The epoch this permit authorizes.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

impl Drop for RebuildFence {
    fn drop(&mut self) {
        if !self.finished {
            self.broker.abandon_rebuild_epoch(self.epoch);
        }
    }
}

impl std::fmt::Debug for RebuildFence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RebuildFence")
            .field("epoch", &self.epoch)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl LiveProcessMemoryBroker {
    /// Build a live broker from a validated effective report. The residency
    /// policy is re-validated here so a broker can never come up on an invalid
    /// policy.
    #[cfg(test)]
    pub(crate) fn from_report(report: EffectiveMemoryBudgetReport) -> MemoryResult<Self> {
        Self::from_report_with_reclaimer(report, None)
    }

    #[cfg(test)]
    pub(crate) fn from_report_with_reclaimer(
        report: EffectiveMemoryBudgetReport,
        reclaimer: Option<Arc<dyn MemoryReclaimer>>,
    ) -> MemoryResult<Self> {
        Self::from_report_with_reclaimer_and_events(
            report,
            reclaimer,
            Arc::new(NoopMemoryEventSink),
        )
    }

    pub(crate) fn from_report_with_reclaimer_and_events(
        report: EffectiveMemoryBudgetReport,
        reclaimer: Option<Arc<dyn MemoryReclaimer>>,
        events: Arc<dyn MemoryEventSink>,
    ) -> MemoryResult<Self> {
        let validated = ValidatedResidencyPolicy::try_from(report.residency.clone())?;
        let residency = validated.as_policy();
        let scheduler = WeightedResourceGroupScheduler::new(
            residency.resource_groups.clone(),
            residency.shared_burst_pool_bytes,
        );
        let controller = Arc::new(LadderPressureController::new(
            residency.global_hard_limit_bytes,
            crate::application::controller::PressureThresholds::default(),
            residency.initial_pressure,
        ));
        let ledger = AccountingLedger::new(validated);
        Ok(Self {
            inner: Arc::new(Mutex::new(BrokerCore {
                ledger,
                scheduler,
                controller,
                report,
                ticket_attribution: BTreeMap::new(),
                deferred_committed_releases: VecDeque::new(),
                deferred_pending_rollbacks: VecDeque::new(),
                rebuild_epoch: 0,
                rebuild_abandoned: false,
                reclaimer,
                reclaim_stats: ReclaimRuntimeStats::default(),
            })),
            events,
            event_sink_active: Arc::new(AtomicBool::new(false)),
            event_sink_quarantined: Arc::new(AtomicBool::new(false)),
            reclaimer_active: Arc::new(AtomicBool::new(false)),
            reclaimer_quarantined: Arc::new(AtomicBool::new(false)),
        })
    }

    fn lock(&self) -> MemoryResult<std::sync::MutexGuard<'_, BrokerCore>> {
        self.inner.lock().map_err(|_| MemoryError::poisoned("broker lock"))
    }

    fn outcome(error: &MemoryError) -> MemoryOutcome {
        MemoryOutcome::from_error(error)
    }

    fn emit(&self, event: MemoryEvent) {
        if self.event_sink_quarantined.load(AtomicOrdering::Acquire) {
            return;
        }
        // Thread-local detection is insufficient when a sink spawns a thread
        // that observes this broker. Serialize by broker identity across all
        // threads; recursive/concurrent advisory events may be dropped.
        let Some(_callback_permit) = CallbackPermit::try_enter(&self.event_sink_active) else {
            return;
        };
        if self.event_sink_quarantined.load(AtomicOrdering::Acquire) {
            return;
        }
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.events.emit(event);
        })) {
            self.event_sink_quarantined.store(true, AtomicOrdering::Release);
            retire_host_panic_payload(payload);
        }
    }

    /// Record one advisory, bounded-cardinality event. This is the facade/host
    /// bridge for operations whose authority lives outside the core broker
    /// (store close, spill, restore and tombstone saturation). The sink cannot
    /// mutate accounting through this API, and a sink panic is contained.
    pub fn record_advisory_event(&self, event: MemoryEvent) {
        self.emit(event);
    }

    fn emit_result<T>(
        &self,
        operation: MemoryOperation,
        bytes: Option<u64>,
        result: &MemoryResult<T>,
    ) {
        let outcome = result.as_ref().map_or_else(Self::outcome, |_| MemoryOutcome::Succeeded);
        self.emit(MemoryEvent::new(operation, outcome, bytes));
    }
}

impl std::fmt::Debug for LiveProcessMemoryBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveProcessMemoryBroker").finish_non_exhaustive()
    }
}

impl BrokerCore {
    /// Install an accepted policy while preserving live usage counters.
    ///
    /// The ledger keeps every class/owner/context counter byte-for-byte and
    /// only swaps the active policy in place, so all accounting invariants hold
    /// across the reconfiguration. The scheduler and controller are rebuilt
    /// from the new policy.
    fn install_policy(
        &mut self,
        validated: ValidatedResidencyPolicy,
        source: OverrideSource,
        scheduler: WeightedResourceGroupScheduler,
    ) -> MemoryResult<()> {
        // Snapshot live usage BEFORE the swap so the rebuilt scheduler can be
        // re-seeded from it (a fresh scheduler starts at zero usage, which would
        // otherwise make post-reconfigure admission see phantom-empty groups and
        // silently drop the anti-starvation guarantee).
        let snapshot = self.ledger.snapshot();
        snapshot.validate_exact()?;
        let next_revision = self
            .report
            .policy_revision
            .checked_add(1)
            .ok_or_else(|| MemoryError::InvalidState("policy revision exhausted".into()))?;
        let new_policy = validated.as_policy().clone();
        self.ledger.swap_policy(validated);
        self.scheduler = scheduler;
        let controller = LadderPressureController::new(
            new_policy.global_hard_limit_bytes,
            crate::application::controller::PressureThresholds::default(),
            new_policy.initial_pressure,
        );
        // Seed cached pressure from LIVE usage, symmetric with the scheduler reseed
        // above: a fresh controller would otherwise revert cached pressure to
        // `initial_pressure` (a latent fail-open if a cached-pressure getter is ever
        // surfaced — a loaded broker would briefly read Normal after a reconfigure).
        controller.update_from_semantic(snapshot.total_owner_bytes());
        self.controller = Arc::new(controller);
        // Keep the report's residency in sync with the active policy.
        self.report.residency = new_policy;
        self.report.policy_revision = next_revision;
        let source_label = source.label().to_string();
        self.report.active_policy_source.clone_from(&source_label);
        if !self.report.override_sources.contains(&source_label) {
            self.report.override_sources.push(source_label);
        }
        // FUP-005: the active-residency provenance is a single CURRENT record,
        // not an append-only journal. Upsert it so N accepted reconfigures add
        // at most one entry on top of the initial resolver provenance; the
        // initial records (residency.base, per-field overrides) are never
        // touched. A rejected reconfigure returns before this point, so the
        // whole provenance vector is preserved on denial.
        match self
            .report
            .override_provenance
            .iter_mut()
            .find(|record| record.field == "residency.active")
        {
            Some(record) => record.source = source,
            None => self
                .report
                .override_provenance
                .push(OverrideRecord::new(source, "residency.active")),
        }
        Ok(())
    }
}

impl crate::api::broker::ProcessMemoryBroker for LiveProcessMemoryBroker {
    fn attach_runtime(
        &self,
        runtime_id: crate::domain::context::RuntimeId,
        runtime_kind: crate::domain::context::RuntimeKind,
    ) -> MemoryResult<Arc<dyn crate::api::broker::RuntimeMemoryView>> {
        Ok(Arc::new(LiveRuntimeMemoryView::new(self.clone(), runtime_id, runtime_kind)))
    }

    fn effective_report(&self) -> EffectiveMemoryBudgetReport {
        // The trait signature is infallible, so on lock poison we cannot return
        // an error. We must NOT fabricate a fictitious policy (e.g. a 4 GiB
        // Production budget): advertising a budget the broker was never built
        // with is a fail-open lie that downstream consumers would trust. Instead
        // we recover the *real* report that was last installed, which the
        // poisoned guard still owns, so the trait method stays byte-for-byte
        // consistent with the inherent `effective_report` on the healthy path
        // and remains truthful even under poison.
        match self.inner.lock() {
            Ok(core) => core.report.clone(),
            Err(poisoned) => poisoned.into_inner().report.clone(),
        }
    }

    fn observe(&self) -> MemoryResult<BrokerObservation> {
        LiveProcessMemoryBroker::observe(self)
    }
}

#[cfg(test)]
mod tests;
