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
    /// At most one external reclaim callback may execute for this broker at a
    /// time. This cross-thread gate complements the same-thread reentry guard.
    reclaimer_active: Arc<AtomicBool>,
}

struct BrokerCore {
    ledger: AccountingLedger,
    scheduler: WeightedResourceGroupScheduler,
    controller: Arc<LadderPressureController>,
    report: EffectiveMemoryBudgetReport,
    /// Resource group each live ticket was admitted under, captured at reserve
    /// time so `release` returns capacity to the ISSUING group even after a live
    /// reconfigure remaps owners (reserve/release stay symmetric).
    ticket_groups: BTreeMap<u64, String>,
    /// Issuing owner of each live ticket (pending or committed), including
    /// zero-byte tickets that hold no positive usage entry. Live reconfigure
    /// must not remap/remove an owner mapping while any ticket — zero-byte or
    /// not — still draws release authority from it (FUP-007: zero usage is not
    /// the absence of authority). Entries mirror `ticket_groups` lifetimes.
    ticket_owners: BTreeMap<u64, OwnerKey>,
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
    reclaimer: Option<Arc<dyn MemoryReclaimer>>,
    reclaim_stats: ReclaimRuntimeStats,
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
                ticket_groups: BTreeMap::new(),
                ticket_owners: BTreeMap::new(),
                deferred_committed_releases: VecDeque::new(),
                deferred_pending_rollbacks: VecDeque::new(),
                reclaimer,
                reclaim_stats: ReclaimRuntimeStats::default(),
            })),
            events,
            event_sink_active: Arc::new(AtomicBool::new(false)),
            reclaimer_active: Arc::new(AtomicBool::new(false)),
        })
    }

    fn lock(&self) -> MemoryResult<std::sync::MutexGuard<'_, BrokerCore>> {
        self.inner.lock().map_err(|_| MemoryError::poisoned("broker lock"))
    }

    fn outcome(error: &MemoryError) -> MemoryOutcome {
        MemoryOutcome::from_error(error)
    }

    fn emit(&self, event: MemoryEvent) {
        // Thread-local detection is insufficient when a sink spawns a thread
        // that observes this broker. Serialize by broker identity across all
        // threads; recursive/concurrent advisory events may be dropped.
        let Some(_callback_permit) = CallbackPermit::try_enter(&self.event_sink_active) else {
            return;
        };
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.events.emit(event);
        })) {
            // A telemetry sink has no accounting authority. Contain a hostile
            // panic payload rather than letting observation abort a caller.
            std::mem::forget(payload);
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

    /// Reserve bytes against (class, owner, context). Fail-closed before
    /// mutation. Consults the resource-group scheduler when the owner maps to a
    /// resource group (weighted-fair admission), and enforces the execution
    /// scratch budget for live-scratch charges — both BEFORE any ledger mutation,
    /// so a denial leaves the ledger byte-for-byte unchanged.
    pub(crate) fn reserve(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        self.reserve_with_reclaim(class, owner, context, kind, bytes, false)
    }

    /// Canonical single-owner reservation path. If the returned value is
    /// abandoned, its `Drop` implementation rolls the pending capacity back.
    pub fn reserve_owned(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<PendingReservation> {
        let result = self
            .reserve(class, owner, context, kind, bytes)
            .map(|ticket| PendingReservation { broker: self.clone(), ticket: Some(ticket) });
        self.emit_result(MemoryOperation::Reserve, Some(bytes), &result);
        result
    }

    /// Reserve AND commit under a SINGLE lock acquisition, so no live reconfigure
    /// or rebuild can interleave between the reservation and its commit (which
    /// would otherwise strand reserved bytes behind a discarded ticket). This is
    /// the atomic charge primitive the facade charged-store seam relies on.
    fn reserve_committed(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        self.reserve_with_reclaim(class, owner, context, kind, bytes, true)
    }

    /// Canonical atomic reserve+commit path returning non-clone authority.
    fn reserve_committed_owned(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<CommittedCharge> {
        self.reserve_committed(class, owner, context, kind, bytes)
            .map(|ticket| CommittedCharge { ticket })
    }

    /// Builder-first committed accounting path for long-lived facade objects.
    /// The returned non-clone lease owns both the charge and its issuing broker,
    /// so explicit failure and `Drop` cannot discard release authority.
    pub fn reserve_managed_charge(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ManagedCharge> {
        let result = self
            .reserve_committed_owned(class, owner, context, kind, bytes)
            .map(|charge| ManagedCharge { broker: self.clone(), charge: Some(charge) });
        self.emit_result(MemoryOperation::Reserve, Some(bytes), &result);
        result
    }

    /// Consume an internal committed charge. On denial the same charge is
    /// returned to its broker-bound [`ManagedCharge`] owner.
    fn release_owned(&self, charge: CommittedCharge) -> Result<(), ChargeReleaseError> {
        let bytes = charge.bytes();
        let result = match self.release(&charge.ticket) {
            Ok(()) => Ok(()),
            Err(error) => Err(ChargeReleaseError { error, charge }),
        };
        let outcome = result
            .as_ref()
            .map_or_else(|error| Self::outcome(&error.error), |_| MemoryOutcome::Succeeded);
        self.emit(MemoryEvent::new(MemoryOperation::Release, outcome, Some(bytes)));
        result
    }

    /// Consume an old committed charge and atomically return its replacement.
    /// Every failure branch returns either a live charge or an explicit recovery
    /// failure; ownership is never silently discarded.
    fn recharge_owned(
        &self,
        old: CommittedCharge,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> Result<CommittedCharge, ChargeRechargeError> {
        let result = match self.recharge(&old.ticket, class, owner, context, kind, bytes) {
            Ok(ticket) => Ok(CommittedCharge { ticket }),
            Err(RechargeError { error, recovery: RechargeRecoveryV1::OldChargePreserved }) => {
                Err(ChargeRechargeError {
                    error,
                    recovery: ChargeRechargeRecovery::ChargePreserved(old),
                })
            }
        };
        let outcome = result
            .as_ref()
            .map_or_else(|error| Self::outcome(&error.error), |_| MemoryOutcome::Succeeded);
        self.emit(MemoryEvent::new(MemoryOperation::Recharge, outcome, Some(bytes)));
        result
    }

    /// Atomically swap one committed charge for another under a SINGLE lock:
    /// release `old`, then reserve+commit a new charge. All-or-nothing — this is
    /// the primitive a charged store uses for an in-place overwrite.
    ///
    /// - `Ok(new)`: the swap happened; `old` is consumed (released) and `new` is
    ///   the fresh committed receipt.
    /// - `Err(..OldChargePreserved)`: nothing was mutated and the original old
    ///   ticket remains valid.
    /// - other recovery states are reserved for generic accounting sinks; this
    ///   broker's ledger replacement never needs external rollback.
    ///
    /// Because release-then-charge happens under one lock, an overwrite never
    /// transiently double-counts against caps/group admission, and no rebuild can
    /// interleave to strand bytes.
    pub(crate) fn recharge(
        &self,
        old: &ReservationTicket,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> Result<ReservationTicket, RechargeError> {
        use crate::application::scheduler::ResourceGroupScheduler;

        let mut core = self.lock().map_err(|error| RechargeError {
            error,
            recovery: RechargeRecoveryV1::OldChargePreserved,
        })?;

        // Validate resource-group admission against the replacement postimage,
        // not against a transient double charge. A cloned candidate gives us an
        // infallible scheduler commit after the ledger replacement succeeds.
        let old_group = core.ticket_groups.get(&old.id()).cloned();
        let new_group = core.ledger.policy().owner_groups.get(&owner).cloned();
        let mut next_scheduler = core.scheduler.clone();
        if let Some(group) = &old_group {
            next_scheduler.record_release(group, old.bytes()).map_err(|error| RechargeError {
                error,
                recovery: RechargeRecoveryV1::OldChargePreserved,
            })?;
        }
        if let Some(group) = &new_group {
            if !next_scheduler.admits(group, bytes) {
                return Err(RechargeError {
                    error: MemoryError::capacity(format!(
                        "resource group {group} admission denied for owner {owner} recharge of {bytes} bytes"
                    )),
                    recovery: RechargeRecoveryV1::OldChargePreserved,
                });
            }
            next_scheduler.record_usage(group, bytes).map_err(|error| RechargeError {
                error,
                recovery: RechargeRecoveryV1::OldChargePreserved,
            })?;
        }

        // Recharge is an admission operation too. Validate the same complete
        // resource-group floor postimage as reserve, after removing the old
        // non-headroom charge and adding the replacement. Without this check a
        // peer group could consume another group's reserved floor by growing an
        // already-live charge instead of issuing a fresh reserve.
        let non_headroom_current = core.ledger.non_headroom_bytes();
        let without_old = if *old.class() == MemoryClass::Headroom {
            non_headroom_current
        } else {
            non_headroom_current.checked_sub(old.bytes()).ok_or_else(|| RechargeError {
                error: MemoryError::InvalidState(
                    "non-headroom recharge old charge exceeds live usage".into(),
                ),
                recovery: RechargeRecoveryV1::OldChargePreserved,
            })?
        };
        let non_headroom_after = if class == MemoryClass::Headroom {
            without_old
        } else {
            without_old.checked_add(bytes).ok_or_else(|| RechargeError {
                error: MemoryError::capacity("non-headroom recharge total exceeds u64"),
                recovery: RechargeRecoveryV1::OldChargePreserved,
            })?
        };
        let non_headroom_band = core
            .ledger
            .policy()
            .global_hard_limit_bytes
            .checked_sub(core.ledger.policy().headroom_bytes)
            .ok_or_else(|| RechargeError {
                error: MemoryError::capacity("headroom exceeds global hard limit"),
                recovery: RechargeRecoveryV1::OldChargePreserved,
            })?;
        next_scheduler.protected_floor_fits(non_headroom_after, non_headroom_band).map_err(
            |error| RechargeError { error, recovery: RechargeRecoveryV1::OldChargePreserved },
        )?;

        // The execution scratch ceiling is a broker-owned admission axis, not a
        // ledger class cap. Evaluate the same replacement postimage used by the
        // ledger so the atomic primitive cannot bypass this outer authority.
        if let Some(max) = core.report.execution.max_execution_scratch_bytes {
            let max = u64::try_from(max).unwrap_or(u64::MAX);
            let current = core.ledger.total_live_scratch_bytes();
            let without_old = if old.kind() == ChargeKind::LiveScratch {
                current.checked_sub(old.bytes()).ok_or_else(|| RechargeError {
                    error: MemoryError::InvalidState(
                        "scratch recharge old charge exceeds live usage".into(),
                    ),
                    recovery: RechargeRecoveryV1::OldChargePreserved,
                })?
            } else {
                current
            };
            let live_after = if kind == ChargeKind::LiveScratch {
                without_old.checked_add(bytes).ok_or_else(|| RechargeError {
                    error: MemoryError::capacity("execution scratch recharge total exceeds u64"),
                    recovery: RechargeRecoveryV1::OldChargePreserved,
                })?
            } else {
                without_old
            };
            if live_after > max {
                return Err(RechargeError {
                    error: MemoryError::capacity(format!(
                        "execution scratch budget {max} exceeded by recharge to {live_after}"
                    )),
                    recovery: RechargeRecoveryV1::OldChargePreserved,
                });
            }
        }

        let replacement =
            core.ledger.replace_committed(old, class, owner, context, kind, bytes).map_err(
                |error| RechargeError { error, recovery: RechargeRecoveryV1::OldChargePreserved },
            )?;
        core.scheduler = next_scheduler;
        core.ticket_groups.remove(&old.id());
        core.ticket_owners.remove(&old.id());
        core.ticket_owners.insert(replacement.id(), replacement.owner().clone());
        if let Some(group) = new_group {
            core.ticket_groups.insert(replacement.id(), group);
        }
        Ok(replacement)
    }

    /// Reserve + commit on an already-held lock guard, rolling the reservation
    /// back if the (unreachable on this path) commit ever fails.
    fn reserve_committed_on(
        core: &mut BrokerCore,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        let ticket = Self::reserve_on(core, class, owner, context, kind, bytes)?;
        if let Err(err) = core.ledger.commit(&ticket) {
            // Unreachable on this single-lock path (the ticket is live and no
            // rebuild can interleave), but stay fail-closed: undo the reservation
            // and its group bookkeeping rather than strand the bytes.
            let _ = Self::release_on(core, &ticket);
            return Err(err);
        }
        Ok(ticket)
    }

    fn soft_threshold_request(
        core: &BrokerCore,
        class: &MemoryClass,
        owner: &OwnerKey,
        bytes: u64,
    ) -> MemoryResult<Option<ReclaimRequest>> {
        let policy = core.ledger.policy();
        let class_current = core.ledger.class_usage(class).checked_total()?;
        let class_after = class_current
            .checked_add(bytes)
            .ok_or_else(|| MemoryError::capacity("class prospective usage exceeds u64"))?;
        let class_crossed = policy.classes.get(class).is_some_and(|budget| {
            class_current <= budget.soft_cap_bytes && class_after > budget.soft_cap_bytes
        });

        let owner_current = core.ledger.owner_usage(owner).checked_total()?;
        let owner_after = owner_current
            .checked_add(bytes)
            .ok_or_else(|| MemoryError::capacity("owner prospective usage exceeds u64"))?;
        let owner_crossed = policy.owner_quotas.get(owner).is_some_and(|quota| {
            owner_current <= quota.soft_cap_bytes && owner_after > quota.soft_cap_bytes
        });

        let group = policy.owner_groups.get(owner).cloned();
        let group_crossed = group.as_ref().is_some_and(|name| {
            policy.resource_groups.get(name).is_some_and(|budget| {
                let current = core.scheduler.current_usage(name);
                current <= budget.soft_cap_bytes
                    && current.checked_add(bytes).is_some_and(|after| after > budget.soft_cap_bytes)
            })
        });

        if !(class_crossed || owner_crossed || group_crossed) {
            return Ok(None);
        }
        Self::reclaim_request(core, class, owner, group, bytes, ReclaimTrigger::SoftThreshold)
            .map(Some)
    }

    fn reclaim_request(
        core: &BrokerCore,
        class: &MemoryClass,
        owner: &OwnerKey,
        group: Option<String>,
        bytes: u64,
        trigger: ReclaimTrigger,
    ) -> MemoryResult<ReclaimRequest> {
        let snapshot = core.ledger.snapshot();
        snapshot.validate_exact()?;
        let prospective = snapshot
            .total_owner_bytes()
            .checked_add(bytes)
            .ok_or_else(|| MemoryError::capacity("reclaim prospective usage exceeds u64"))?;
        let classified = core.controller.classify_semantic(prospective);
        let pressure = match (trigger, classified) {
            (ReclaimTrigger::SoftThreshold, PressureState::Normal) => PressureState::Soft,
            (ReclaimTrigger::CapacityDenial, PressureState::Normal | PressureState::Soft) => {
                PressureState::Hard
            }
            _ => classified,
        };
        let ticket_count_full = core.ledger.live_ticket_limit_reached();
        let mut candidates: Vec<ReclaimCandidate> = core
            .ledger
            .policy()
            .classes
            .iter()
            // A zero-byte charge can occupy a live ticket even in a class
            // with a zero byte cap. At the ticket-count ceiling the host may
            // need to evict that authority, so do not hide such classes from
            // the one bounded reclaim attempt.
            .filter(|(_, budget)| budget.hard_cap_bytes > 0 || ticket_count_full)
            .map(|(candidate, budget)| ReclaimCandidate {
                class: candidate.clone(),
                priority: budget.reclaim_priority,
            })
            .collect();
        candidates.sort_by_key(|candidate| {
            let priority = match candidate.priority {
                ReclaimPriority::Last => 0,
                ReclaimPriority::Normal => 1,
                ReclaimPriority::First => 2,
            };
            (priority, candidate.class.clone())
        });
        let mut group_candidates: Vec<ReclaimGroupCandidate> = core
            .ledger
            .policy()
            .resource_groups
            .iter()
            .map(|(name, budget)| ReclaimGroupCandidate {
                group: name.clone(),
                priority: budget.reclaim_priority,
            })
            .collect();
        group_candidates.sort_by_key(|candidate| {
            let priority = match candidate.priority {
                ReclaimPriority::Last => 0,
                ReclaimPriority::Normal => 1,
                ReclaimPriority::First => 2,
            };
            (priority, candidate.group.clone())
        });
        Ok(ReclaimRequest {
            trigger,
            requested_class: class.clone(),
            requested_owner: owner.clone(),
            resource_group: group,
            requested_bytes: bytes,
            pressure,
            rungs: core.controller.ladder_for(pressure),
            candidates,
            group_candidates,
            max_attempts: 1,
        })
    }

    fn run_reclaim(&self, reclaimer: Arc<dyn MemoryReclaimer>, request: &ReclaimRequest) {
        let Some(reclaimer_permit) = CallbackPermit::try_enter(&self.reclaimer_active) else {
            // Concurrent plans may still make progress without host work if the
            // live postimage admits them; otherwise their normal remeasurement
            // returns a typed denial. Never start a recursive/concurrent storm.
            self.emit(MemoryEvent::new(
                MemoryOperation::Reclaim,
                MemoryOutcome::Busy,
                Some(request.requested_bytes),
            ));
            return;
        };
        let broker_identity = Arc::as_ptr(&self.inner) as usize;
        // The entry guard in `reserve_with_reclaim` rejects same-broker reserve
        // reentry before it can prepare another callback. This local guard is a
        // second boundary for future internal callers of this method.
        let Some(callback_scope) =
            BrokerCallbackScope::enter(BrokerCallbackKind::Reclaimer, broker_identity)
        else {
            return;
        };
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reclaimer.reclaim(request)));
        drop(callback_scope);
        drop(reclaimer_permit);
        let mut core = match self.lock() {
            Ok(core) => core,
            Err(error) => {
                self.emit(MemoryEvent::new(
                    MemoryOperation::Reclaim,
                    Self::outcome(&error),
                    Some(request.requested_bytes),
                ));
                return;
            }
        };
        core.reclaim_stats.attempts += 1;
        let event_outcome = match outcome {
            Ok(Ok(outcome)) => {
                core.reclaim_stats.successes += 1;
                core.reclaim_stats.reported_reclaimed_bytes += u128::from(outcome.reclaimed_bytes);
                MemoryOutcome::Succeeded
            }
            Ok(Err(_)) => {
                core.reclaim_stats.failures += 1;
                MemoryOutcome::CallbackFailed
            }
            Err(payload) => {
                core.reclaim_stats.failures += 1;
                // A hostile panic payload may itself panic on Drop. This is an
                // exceptional, bounded leak that keeps a client callback from
                // aborting the process during panic-payload retirement.
                std::mem::forget(payload);
                MemoryOutcome::CallbackFailed
            }
        };
        drop(core);
        self.emit(MemoryEvent::new(
            MemoryOperation::Reclaim,
            event_outcome,
            Some(request.requested_bytes),
        ));
    }

    /// Pure admission preflight over locked state (FUP-003): evaluates the
    /// shared admission engine against the live ledger, scheduler, and report
    /// without mutating anything. The verdict decides the reclaim plan BEFORE
    /// any callback can fire, so terminally impossible and blocked requests
    /// never touch a host callback.
    fn admission_preflight(
        core: &BrokerCore,
        class: &MemoryClass,
        owner: &OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> AdmissionVerdict {
        let owner_group = core.ledger.policy().owner_groups.get(owner).map(String::as_str);
        let inputs = PreflightInputs {
            policy: core.ledger.policy(),
            usage: core.ledger.usage_view(),
            non_headroom_bytes: core.ledger.non_headroom_bytes(),
            live_scratch_bytes: core.ledger.total_live_scratch_bytes(),
            execution_scratch_max: core.report.execution.max_execution_scratch_bytes,
            fair_burst_shares: core.scheduler.fair_burst_shares(),
            group_current_usage: owner_group.map_or(0, |group| core.scheduler.current_usage(group)),
            unused_group_floors: core.scheduler.unused_reserved_floor_bytes(),
            owner_group,
            request: AdmissionRequest { class, owner, context, kind, bytes },
            rebuild_in_progress: core.ledger.rebuild_in_progress(),
            ticket_id_space_exhausted: core.ledger.ticket_id_space_exhausted(),
            live_ticket_limit_reached: core.ledger.live_ticket_limit_reached(),
        };
        crate::domain::admission::preflight(&inputs)
    }

    fn reserve_with_reclaim(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
        commit: bool,
    ) -> MemoryResult<ReservationTicket> {
        let broker_identity = Arc::as_ptr(&self.inner) as usize;
        if BrokerCallbackScope::is_active(BrokerCallbackKind::Reclaimer, broker_identity) {
            return Err(MemoryError::operation_in_progress(
                "reentrant reserve from this broker's memory reclaimer",
            ));
        }

        // Reject oversized identifiers before preflight can invoke host reclaim
        // callbacks or mutate diagnostic counters. `reserve_on` and the ledger
        // repeat the check at the mutation authority boundary.
        if owner.len() > crate::domain::management_limits::MAX_OWNER_KEY_BYTES {
            return Err(MemoryError::capacity(format!(
                "owner identifier is {} bytes; maximum is {}",
                owner.len(),
                crate::domain::management_limits::MAX_OWNER_KEY_BYTES
            )));
        }

        // Locked preflight FIRST: Blocked and Terminal verdicts return before
        // any callback is even built. A predicted failure goes straight to the
        // hard (capacity-denial) plan — a soft request is only proactive, sent
        // when the request itself is admissible but crosses a soft threshold.
        enum Plan {
            Admit(Option<(Arc<dyn MemoryReclaimer>, ReclaimRequest)>),
            Hard(Arc<dyn MemoryReclaimer>, ReclaimRequest),
        }
        let plan = {
            let mut core = self.lock()?;
            match Self::admission_preflight(&core, &class, &owner, context, kind, bytes) {
                AdmissionVerdict::Blocked { reason } => {
                    core.reclaim_stats.blocked_denials += 1;
                    return Err(reason.into_error());
                }
                AdmissionVerdict::TerminalDenial { breach } => {
                    core.reclaim_stats.terminal_denials += 1;
                    return Err(breach.into_error());
                }
                AdmissionVerdict::Admit => {
                    let request = Self::soft_threshold_request(&core, &class, &owner, bytes)?;
                    let work = request.and_then(|request| {
                        core.reclaimer.clone().map(|reclaimer| (reclaimer, request))
                    });
                    Plan::Admit(work)
                }
                AdmissionVerdict::ReclaimCandidate { breach } => {
                    let group = core.ledger.policy().owner_groups.get(&owner).cloned();
                    let request = Self::reclaim_request(
                        &core,
                        &class,
                        &owner,
                        group,
                        bytes,
                        ReclaimTrigger::CapacityDenial,
                    )?;
                    match core.reclaimer.clone() {
                        Some(reclaimer) => Plan::Hard(reclaimer, request),
                        // No callback available: the live breach is final,
                        // exactly as if the attempt had run.
                        None => {
                            core.reclaim_stats.terminal_denials += 1;
                            return Err(breach.into_error());
                        }
                    }
                }
            }
        };
        match plan {
            Plan::Admit(soft_work) => {
                if let Some((reclaimer, request)) = soft_work {
                    self.run_reclaim(reclaimer, &request);
                }
                let mut core = self.lock()?;
                if commit {
                    Self::reserve_committed_on(&mut core, class, owner, context, kind, bytes)
                } else {
                    Self::reserve_on(&mut core, class, owner, context, kind, bytes)
                }
            }
            Plan::Hard(reclaimer, request) => {
                self.run_reclaim(reclaimer, &request);
                // Final authority is the remeasured semantic state, never the
                // callback's claimed byte count. One retry is the bounded
                // exhaustion contract: no second callback whatever the
                // remeasure says.
                let mut core = self.lock()?;
                match Self::admission_preflight(&core, &class, &owner, context, kind, bytes) {
                    AdmissionVerdict::Admit => {
                        if commit {
                            Self::reserve_committed_on(
                                &mut core,
                                class.clone(),
                                owner.clone(),
                                context,
                                kind,
                                bytes,
                            )
                        } else {
                            Self::reserve_on(&mut core, class, owner, context, kind, bytes)
                        }
                    }
                    AdmissionVerdict::Blocked { reason } => {
                        core.reclaim_stats.blocked_denials += 1;
                        Err(reason.into_error())
                    }
                    AdmissionVerdict::TerminalDenial { breach }
                    | AdmissionVerdict::ReclaimCandidate { breach } => {
                        core.reclaim_stats.terminal_denials += 1;
                        Err(breach.into_error())
                    }
                }
            }
        }
    }

    /// Release on an already-held lock guard (ledger release + group bookkeeping).
    /// Scheduler state is preflighted and transactionally restored if the
    /// ledger rejects the ticket, avoiding a full scheduler clone.
    fn release_on(core: &mut BrokerCore, ticket: &ReservationTicket) -> MemoryResult<()> {
        let group = core.ticket_groups.get(&ticket.id()).cloned();
        if let Some(group) = &group {
            core.scheduler.usage_after_release(group, ticket.bytes())?;
            core.scheduler.record_release(group, ticket.bytes())?;
        }
        if let Err(error) = core.ledger.release(ticket) {
            if let Some(group) = &group {
                core.scheduler.record_usage(group, ticket.bytes()).map_err(|rollback| {
                    MemoryError::InvalidState(format!(
                        "ledger release failed ({error}); scheduler rollback also failed ({rollback})"
                    ))
                })?;
            }
            return Err(error);
        }
        core.ticket_groups.remove(&ticket.id());
        core.ticket_owners.remove(&ticket.id());
        Ok(())
    }

    fn rollback_pending_on(core: &mut BrokerCore, ticket: &ReservationTicket) -> MemoryResult<()> {
        let group = core.ticket_groups.get(&ticket.id()).cloned();
        if let Some(group) = &group {
            core.scheduler.usage_after_release(group, ticket.bytes())?;
            core.scheduler.record_release(group, ticket.bytes())?;
        }
        if let Err(error) = core.ledger.rollback_pending(ticket) {
            if let Some(group) = &group {
                core.scheduler.record_usage(group, ticket.bytes()).map_err(|rollback| {
                    MemoryError::InvalidState(format!(
                        "ledger pending rollback failed ({error}); scheduler rollback also failed ({rollback})"
                    ))
                })?;
            }
            return Err(error);
        }
        core.ticket_groups.remove(&ticket.id());
        core.ticket_owners.remove(&ticket.id());
        Ok(())
    }

    fn rollback_pending_ticket(&self, ticket: &ReservationTicket) -> MemoryResult<()> {
        let mut core = self.lock()?;
        Self::rollback_pending_on(&mut core, ticket)
    }

    /// Emergency RAII cleanup for an abandoned pending permit.
    ///
    /// Public mutations remain fail-closed on a poisoned broker lock. Drop is
    /// different: losing the only pending-ticket authority would permanently
    /// strand class/owner/context/group capacity. Recover the poisoned guard and
    /// perform only the exact pending rollback. The rollback validates the
    /// ledger instance, ticket contents, and Pending state before subtraction;
    /// it cannot release a committed charge.
    fn rollback_pending_ticket_on_drop(&self, ticket: &ReservationTicket) {
        let mut core = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        // FUP-007: never silently discard the last pending authority. A rollback
        // failure here is unreachable without an internal invariant break (live
        // reconfigure can no longer remove the ticket's group scope), but if it
        // ever happens the exact ticket is preserved in the deferred queue for a
        // later retry instead of stranding charged capacity with no owner.
        if Self::rollback_pending_on(&mut core, ticket).is_err() {
            core.deferred_pending_rollbacks.push_back(ticket.clone());
        }
    }

    /// Best-effort retry of deferred pending rollbacks. Each rollback is
    /// individually fail-closed; tickets that still fail stay queued with their
    /// authority intact. Called after `finish_rebuild` and after an accepted
    /// reconfigure (which may have restored a missing group scope).
    fn drain_deferred_pending_rollbacks_batch(core: &mut BrokerCore) -> usize {
        // Each lock acquisition handles at most one bounded batch; queue size
        // itself is bounded by the live-ticket admission cap.
        let count = core
            .deferred_pending_rollbacks
            .len()
            .min(crate::domain::management_limits::MAX_DEFERRED_TICKET_DRAIN_BATCH);
        let mut resolved = 0;
        for _ in 0..count {
            let Some(ticket) = core.deferred_pending_rollbacks.pop_front() else {
                break;
            };
            if Self::rollback_pending_on(core, &ticket).is_err() {
                core.deferred_pending_rollbacks.push_back(ticket);
            } else {
                resolved += 1;
            }
        }
        resolved
    }

    fn drain_deferred_committed_releases_batch(core: &mut BrokerCore) -> MemoryResult<usize> {
        let count = core
            .deferred_committed_releases
            .len()
            .min(crate::domain::management_limits::MAX_DEFERRED_TICKET_DRAIN_BATCH);
        let deferred: Vec<_> =
            (0..count).filter_map(|_| core.deferred_committed_releases.pop_front()).collect();
        if let Err(error) = Self::release_many_on(core, &deferred) {
            // Batch release is atomic; restore exact authority and FIFO order.
            for ticket in deferred.into_iter().rev() {
                core.deferred_committed_releases.push_front(ticket);
            }
            return Err(error);
        }
        Ok(deferred.len())
    }

    /// Emergency cleanup for the last owner of a committed charge.
    ///
    /// Recovering a poisoned mutex is intentionally confined to this exact
    /// authority-preserving cleanup path. A healthy/open ledger validates and
    /// releases the original ticket. During rebuild, the exact ticket is queued
    /// under the same broker lock and remains conservatively charged until
    /// `finish_rebuild` drains the batch.
    fn release_managed_charge_on_drop(&self, charge: CommittedCharge) {
        let ticket = charge.ticket;
        let bytes = ticket.bytes();
        let mut core = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if core.ledger.rebuild_in_progress() {
            core.deferred_committed_releases.push_back(ticket);
            drop(core);
            self.emit(MemoryEvent::new(MemoryOperation::Release, MemoryOutcome::Busy, Some(bytes)));
            return;
        }

        // A live ManagedCharge is issued by this broker and cannot be cloned, so
        // non-rebuild failure is unreachable without an internal invariant break.
        // Retain the exact ticket in the deferred owner queue instead of silently
        // discarding authority if that invariant is ever violated.
        let outcome = if Self::release_on(&mut core, &ticket).is_err() {
            core.deferred_committed_releases.push_back(ticket);
            MemoryOutcome::InvalidState
        } else {
            MemoryOutcome::Succeeded
        };
        drop(core);
        self.emit(MemoryEvent::new(MemoryOperation::Release, outcome, Some(bytes)));
    }

    fn release_many_on(core: &mut BrokerCore, tickets: &[ReservationTicket]) -> MemoryResult<()> {
        let mut group_refunds = BTreeMap::<String, u64>::new();
        for ticket in tickets {
            if let Some(group) = core.ticket_groups.get(&ticket.id()) {
                let refund = group_refunds.entry(group.clone()).or_default();
                *refund = refund.checked_add(ticket.bytes()).ok_or_else(|| {
                    MemoryError::InvalidState("resource-group batch release exceeds u64".into())
                })?;
            }
        }
        for (group, refund) in &group_refunds {
            core.scheduler.usage_after_release(group, *refund)?;
        }
        let mut applied: Vec<(String, u64)> = Vec::with_capacity(group_refunds.len());
        for (group, refund) in &group_refunds {
            if let Err(error) = core.scheduler.record_release(group, *refund) {
                for (applied_group, applied_refund) in applied {
                    core.scheduler
                        .record_usage(&applied_group, applied_refund)
                        .expect("preflighted scheduler batch rollback");
                }
                return Err(error);
            }
            applied.push((group.clone(), *refund));
        }
        if let Err(error) = core.ledger.release_batch(tickets) {
            for (group, refund) in applied {
                core.scheduler.record_usage(&group, refund).map_err(|rollback| {
                    MemoryError::InvalidState(format!(
                        "ledger batch release failed ({error}); scheduler rollback also failed ({rollback})"
                    ))
                })?;
            }
            return Err(error);
        }
        for ticket in tickets {
            core.ticket_groups.remove(&ticket.id());
            core.ticket_owners.remove(&ticket.id());
        }
        Ok(())
    }

    /// Shared reserve path on an already-held lock guard (admission, the
    /// execution-scratch budget, the ledger reserve, and scheduler/ticket
    /// bookkeeping), so `reserve` and `reserve_committed` compose it without
    /// re-locking.
    fn reserve_on(
        core: &mut BrokerCore,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        use crate::application::scheduler::ResourceGroupScheduler;

        let owner_key_bytes = owner.len();
        if owner_key_bytes > crate::domain::management_limits::MAX_OWNER_KEY_BYTES {
            return Err(MemoryError::capacity(format!(
                "owner identifier is {owner_key_bytes} bytes; maximum is {}",
                crate::domain::management_limits::MAX_OWNER_KEY_BYTES
            )));
        }

        // D: resource-group admission. An owner mapped to a group must pass the
        // weighted-fair scheduler (anti-starvation) before it can reserve.
        let group = core.ledger.policy().owner_groups.get(&owner).cloned();
        if let Some(ref group) = group {
            if !core.scheduler.admits(group, bytes) {
                return Err(MemoryError::capacity(format!(
                    "resource group {group} admission denied for owner {owner} reserve of {bytes} bytes"
                )));
            }
            core.scheduler.usage_after_growth(group, bytes)?;
        }
        if class != MemoryClass::Headroom {
            let non_headroom_current = core.ledger.non_headroom_bytes();
            let non_headroom_after = non_headroom_current
                .checked_add(bytes)
                .ok_or_else(|| MemoryError::capacity("non-headroom reserve total exceeds u64"))?;
            let band = core
                .ledger
                .policy()
                .global_hard_limit_bytes
                .checked_sub(core.ledger.policy().headroom_bytes)
                .ok_or_else(|| MemoryError::capacity("headroom exceeds global hard limit"))?;
            core.scheduler.protected_floor_fits_after_growth(
                group.as_deref(),
                bytes,
                non_headroom_after,
                band,
            )?;
        }

        // ExecutionBudget: the memory-relevant cap is `max_execution_scratch_bytes`,
        // enforced here for live-scratch charges. The other execution caps
        // (wall-time, subqueries, trace-edges, output) are runtime concerns and
        // are host-enforced (surfaced in the effective report for the host).
        if kind == ChargeKind::LiveScratch {
            if let Some(max) = core.report.execution.max_execution_scratch_bytes {
                let max = u64::try_from(max).unwrap_or(u64::MAX);
                let live_after =
                    core.ledger.total_live_scratch_bytes().checked_add(bytes).ok_or_else(|| {
                        MemoryError::capacity("execution scratch reserve total exceeds u64")
                    })?;
                if live_after > max {
                    return Err(MemoryError::capacity(format!(
                        "execution scratch budget {max} exceeded by reserve to {live_after}"
                    )));
                }
            }
        }

        let ticket = core.ledger.reserve(class, owner, context, kind, bytes)?;
        // Keep scheduler usage in sync so admission reflects live group usage, and
        // remember the issuing group so release is symmetric across reconfigure.
        // The issuing owner is recorded for every ticket — including zero-byte
        // tickets with no positive usage entry — so live reconfigure can guard
        // the ticket's release scope (FUP-007).
        core.ticket_owners.insert(ticket.id(), ticket.owner().clone());
        if let Some(group) = group {
            if let Err(error) = core.scheduler.record_usage(&group, bytes) {
                core.ticket_owners.remove(&ticket.id());
                core.ledger.rollback_pending(&ticket).map_err(|rollback| {
                    MemoryError::InvalidState(format!(
                        "scheduler commit failed ({error}); ledger rollback also failed ({rollback})"
                    ))
                })?;
                return Err(error);
            }
            core.ticket_groups.insert(ticket.id(), group);
        }
        Ok(ticket)
    }

    /// Commit a previously reserved ticket.
    pub(crate) fn commit(&self, ticket: &ReservationTicket) -> MemoryResult<u64> {
        let mut core = self.lock()?;
        core.ledger.commit(ticket)
    }

    /// Release a ticket (rollback of a reservation or eviction of a committed
    /// charge). Returns the released bytes to the owner's resource group (if any)
    /// only after the ledger release succeeds, so a rejected (replayed) release
    /// never returns phantom group capacity.
    pub(crate) fn release(&self, ticket: &ReservationTicket) -> MemoryResult<()> {
        let mut core = self.lock()?;
        // `release_on` returns capacity to the group the ticket was ADMITTED under
        // (recorded at reserve time), not the current policy's mapping — so
        // reserve/release stay symmetric even after a live reconfigure remaps the
        // owner — and is fail-closed before any scheduler mutation.
        Self::release_on(&mut core, ticket)
    }

    /// Atomically release active managed charges issued by this broker.
    ///
    /// Broker affinity and active authority are checked before the ledger is
    /// touched. Any validation/rebuild failure leaves every lease unchanged.
    pub fn release_managed_many(&self, charges: &mut [ManagedCharge]) -> MemoryResult<()> {
        let bytes = charges.iter().try_fold(0u64, |sum, charge| {
            sum.checked_add(charge.bytes().unwrap_or(0)).ok_or_else(|| {
                MemoryError::InvalidState("managed release byte total exceeds u64".into())
            })
        });
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(error) => {
                let result = Err(error);
                self.emit_result(MemoryOperation::Release, None, &result);
                return result;
            }
        };
        let mut tickets = Vec::with_capacity(charges.len());
        for charge in charges.iter() {
            if !Arc::ptr_eq(&self.inner, &charge.broker.inner) {
                let result = Err(MemoryError::InvalidState(
                    "managed charge batch contains a foreign broker lease".to_string(),
                ));
                self.emit_result(MemoryOperation::Release, Some(bytes), &result);
                return result;
            }
            let committed = charge.charge.as_ref().ok_or_else(|| {
                MemoryError::InvalidState(
                    "managed charge batch contains an already released lease".to_string(),
                )
            });
            let committed = match committed {
                Ok(committed) => committed,
                Err(error) => {
                    let result = Err(error);
                    self.emit_result(MemoryOperation::Release, Some(bytes), &result);
                    return result;
                }
            };
            tickets.push(committed.ticket.clone());
        }

        let result = (|| {
            let mut core = self.lock()?;
            Self::release_many_on(&mut core, &tickets)?;
            for charge in charges {
                charge.charge = None;
            }
            Ok(())
        })();
        self.emit_result(MemoryOperation::Release, Some(bytes), &result);
        result
    }

    /// Ask resource-group admission whether `group_name` may grow by `bytes`.
    pub fn admits_group(&self, group_name: &str, bytes: u64) -> MemoryResult<bool> {
        use crate::application::scheduler::ResourceGroupScheduler;
        let core = self.lock()?;
        Ok(core.scheduler.admits(group_name, bytes))
    }

    /// Enter rebuild mode: subsequent live writes are rejected until
    /// [`Self::finish_rebuild`].
    pub fn begin_rebuild(&self) -> MemoryResult<()> {
        let mut core = self.lock()?;
        core.ledger.begin_rebuild();
        Ok(())
    }

    pub fn finish_rebuild(&self) -> MemoryResult<()> {
        {
            let mut core = self.lock()?;
            core.ledger.finish_rebuild();
        }
        // The existing contract opens admission before deferred committed
        // releases are applied. Keep that ordering, but release the lock between
        // bounded batches so a large (yet ticket-bounded) backlog cannot create
        // one unbounded critical section. Charges remain conservatively counted
        // until their own atomic batch succeeds.
        loop {
            let mut core = self.lock()?;
            if core.deferred_pending_rollbacks.is_empty()
                && core.deferred_committed_releases.is_empty()
            {
                return Ok(());
            }
            let queued_before =
                core.deferred_pending_rollbacks.len() + core.deferred_committed_releases.len();
            let pending_resolved = Self::drain_deferred_pending_rollbacks_batch(&mut core);
            let committed_released = Self::drain_deferred_committed_releases_batch(&mut core)?;
            let queued_after =
                core.deferred_pending_rollbacks.len() + core.deferred_committed_releases.len();
            if queued_after >= queued_before && pending_resolved == 0 && committed_released == 0 {
                return Err(MemoryError::InvalidState(format!(
                    "rebuild cleanup made no progress; {} deferred ticket authorities remain",
                    queued_after
                )));
            }
        }
    }

    /// Current pressure classification from semantic usage (not allocator
    /// telemetry).
    pub fn classify_pressure(&self) -> MemoryResult<PressureState> {
        let result = (|| {
            let core = self.lock()?;
            let snapshot = core.ledger.snapshot();
            snapshot.validate_exact()?;
            let used = snapshot.checked_total_owner_bytes()?;
            Ok(core.controller.update_from_semantic(used))
        })();
        self.emit_result(MemoryOperation::Pressure, None, &result);
        result
    }

    /// Observe an allocator snapshot. Advisory only: returns a comparison
    /// against semantic accounting and provably does NOT mutate the ledger
    /// (the advisory telemetry contract). The returned advisory carries the mismatch signal.
    pub fn observe_allocator(
        &self,
        snapshot: &AllocatorSnapshot,
    ) -> MemoryResult<AllocatorAdvisory> {
        let result = (|| {
            let core = self.lock()?;
            let semantic_snapshot = core.ledger.snapshot();
            semantic_snapshot.validate_exact()?;
            let semantic = semantic_snapshot.checked_total_owner_bytes()?;
            Ok(AllocatorAdvisory::compare(snapshot, semantic))
        })();
        let outcome = result.as_ref().map_or_else(Self::outcome, |advisory| {
            if advisory.mismatch {
                MemoryOutcome::AllocatorMismatch
            } else {
                MemoryOutcome::Succeeded
            }
        });
        let bytes = result.as_ref().ok().map(|advisory| advisory.semantic_bytes);
        self.emit(MemoryEvent::new(MemoryOperation::ObserveAllocator, outcome, bytes));
        result
    }

    /// Attempt a live reconfiguration. Fail-closed: an illegal narrowing leaves
    /// the active policy and ledger untouched and returns a typed error.
    ///
    /// Deny rules, evaluated against LIVE usage (not just the new policy's keys):
    /// a new global hard limit below current usage; any class that currently
    /// holds bytes whose new hard cap is below its usage OR that is dropped from
    /// the new policy entirely (which would silently uncap it); any owner quota
    /// in the new policy whose hard cap is below that owner's current usage; any
    /// owner→resource-group remap/removal while that owner still holds live
    /// bytes OR any live ticket (a zero-byte ticket carries release authority
    /// without positive usage, so usage alone cannot authorize the remap).
    /// Allowed: raising caps, adding headroom, owner-quota / reclaim-priority
    /// adjustments that still honor live usage without remapping live owners.
    pub fn try_live_reconfigure(&self, new_policy: ResidencyPolicy) -> MemoryResult<()> {
        let result = self.try_live_reconfigure_inner(new_policy);
        self.emit_result(MemoryOperation::Reconfigure, None, &result);
        result
    }

    fn try_live_reconfigure_inner(&self, new_policy: ResidencyPolicy) -> MemoryResult<()> {
        let validated = ValidatedResidencyPolicy::try_from(new_policy)?;
        let new_policy = validated.as_policy();
        let mut core = self.lock()?;

        let snapshot = core.ledger.snapshot();
        snapshot.validate_exact()?;
        AccountingLedger::validate_policy_postimage(new_policy, &snapshot, None)
            .map_err(|error| MemoryError::InvalidState(format!("live reconfig denied: {error}")))?;
        let current_total = snapshot.total_owner_bytes();
        let headroom_used =
            snapshot.class_usage.get(&MemoryClass::Headroom).map_or(0, |u| u.total());
        let non_headroom_used = current_total.saturating_sub(headroom_used);
        let new_non_headroom_ceiling =
            new_policy.global_hard_limit_bytes.saturating_sub(new_policy.headroom_bytes);

        // FUP-007: an owner mapping change is denied while ANY live ticket —
        // pending or committed, zero-byte or not — still draws release authority
        // from the issuing scope. Zero-byte tickets hold no positive usage entry,
        // so usage alone cannot prove the absence of authority; the broker's
        // exact live-ticket owner index is the authority oracle here. This scan
        // is O(live tickets) on the slow control path only; reserve/release hot
        // paths do not scan.
        let mut live_ticket_owners: BTreeMap<&OwnerKey, usize> = BTreeMap::new();
        for owner in core.ticket_owners.values() {
            *live_ticket_owners.entry(owner).or_insert(0) += 1;
        }
        let current_owner_groups = &core.ledger.policy().owner_groups;
        for owner in snapshot
            .owner_usage
            .keys()
            .chain(live_ticket_owners.keys().copied())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let owner_used = snapshot.owner_usage.get(owner).map_or(0, |usage| usage.total());
            let live_tickets = live_ticket_owners.get(owner).copied().unwrap_or(0);
            if owner_used == 0 && live_tickets == 0 {
                continue;
            }
            let current_group = current_owner_groups.get(owner);
            let next_group = new_policy.owner_groups.get(owner);
            if current_group != next_group {
                return Err(MemoryError::InvalidState(format!(
                    "live reconfig denied: owner {owner} holds {live_tickets} live ticket(s) \
                     ({owner_used} live bytes) but resource-group mapping changes from \
                     {current_group:?} to {next_group:?}"
                )));
            }
        }

        // A new resource-GROUP hard cap must accommodate the group's current live
        // usage, mirroring the class/owner checks above: lowering a group's cap
        // below what its mapped owners already hold would install a cap the group
        // instantly violates (a fail-open of the group bound). Owner->group mapping
        // changes for live owners are already rejected, so summing live owner usage
        // by the new mapping reflects the true live load per group.
        let mut live_group_usage: BTreeMap<&String, u64> = BTreeMap::new();
        for (owner, usage) in &snapshot.owner_usage {
            if let Some(group) = new_policy.owner_groups.get(owner) {
                let slot = live_group_usage.entry(group).or_insert(0);
                *slot = slot.checked_add(usage.total()).ok_or_else(|| {
                    MemoryError::InvalidState("live resource-group usage exceeds u64".into())
                })?;
            }
        }
        use crate::application::scheduler::ResourceGroupScheduler;
        let mut candidate_scheduler = WeightedResourceGroupScheduler::new(
            new_policy.resource_groups.clone(),
            new_policy.shared_burst_pool_bytes,
        );
        for (group, used) in &live_group_usage {
            if !candidate_scheduler.admits(group, *used) {
                return Err(MemoryError::InvalidState(format!(
                    "live reconfig denied: resource group {group} current usage {used} violates \
                     hard/burst policy"
                )));
            }
            candidate_scheduler.record_usage(group, *used)?;
        }
        candidate_scheduler
            .protected_floor_fits(non_headroom_used, new_non_headroom_ceiling)
            .map_err(|error| MemoryError::InvalidState(format!("live reconfig denied: {error}")))?;

        // Accepted: install the new policy by rebuilding the ledger's view while
        // preserving usage counters.
        core.install_policy(validated, OverrideSource::RuntimeReconfigure)?;
        // A restored group scope may unblock preserved pending authority.
        Self::drain_deferred_pending_rollbacks_batch(&mut core);
        Ok(())
    }

    /// Immutable ledger snapshot for invariant assertions and reporting.
    pub fn ledger_snapshot(&self) -> MemoryResult<LedgerSnapshot> {
        let core = self.lock()?;
        let snapshot = core.ledger.snapshot();
        snapshot.validate_exact()?;
        Ok(snapshot)
    }

    /// The effective budget report this broker was built from.
    pub fn effective_report(&self) -> MemoryResult<EffectiveMemoryBudgetReport> {
        let core = self.lock()?;
        Ok(core.report.clone())
    }

    /// Bounded reclaim activity observed by this broker. Callback-reported
    /// bytes are telemetry only; final admission always uses ledger remeasurement.
    pub fn reclaim_runtime_stats(&self) -> MemoryResult<ReclaimRuntimeStats> {
        let core = self.lock()?;
        Ok(core.reclaim_stats.clone())
    }

    /// Observe ledger, policy provenance, pressure and reclaim counters from
    /// one broker-lock snapshot. A poisoned or invalid broker is a typed error;
    /// no zero/default observation is fabricated.
    pub fn observe(&self) -> MemoryResult<BrokerObservation> {
        let result = (|| {
            let core = self.lock()?;
            let ledger = core.ledger.snapshot();
            ledger.validate_exact()?;
            let semantic_bytes = ledger.checked_total_owner_bytes()?;
            let pressure = core.controller.update_from_semantic(semantic_bytes);
            let resource_group_usage = core
                .report
                .residency
                .resource_groups
                .keys()
                .map(|name| (name.clone(), core.scheduler.current_usage(name)))
                .collect();
            Ok(BrokerObservation {
                ledger,
                resource_group_usage,
                report: core.report.clone(),
                pressure,
                reclaim: core.reclaim_stats.clone(),
                semantic_bytes,
            })
        })();
        let bytes = result.as_ref().ok().map(|observation| observation.semantic_bytes);
        self.emit_result(MemoryOperation::ObserveBroker, bytes, &result);
        result
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
        let mut scheduler = WeightedResourceGroupScheduler::new(
            new_policy.resource_groups.clone(),
            new_policy.shared_burst_pool_bytes,
        );
        for (owner, group) in &new_policy.owner_groups {
            let used = snapshot.owner_usage.get(owner).map_or(0, |u| u.total());
            if used > 0 {
                scheduler.record_usage(group, used)?;
            }
        }
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
mod tests {
    use super::*;
    use crate::api::broker::ProcessMemoryBroker;
    use crate::domain::context::RuntimeKind;
    use crate::domain::policy::{
        MemoryClassBudget, OwnerQuota, ReclaimPriority, ResidencyPolicy, ResourceGroup,
    };
    use crate::domain::stats::PressureState;
    use crate::ports::scratch::ScratchRequest;

    fn report_with(mut residency: ResidencyPolicy) -> EffectiveMemoryBudgetReport {
        // Most control-plane tests isolate global/owner/group behavior. Expand
        // only the constructor's explicit zero defaults into permissive test
        // caps; a removed class stays removed so totality tests still bite.
        let global = residency.global_hard_limit_bytes;
        for budget in residency.classes.values_mut() {
            if budget.reserved_bytes == 0
                && budget.soft_cap_bytes == 0
                && budget.hard_cap_bytes == 0
            {
                budget.soft_cap_bytes = global;
                budget.hard_cap_bytes = global;
            }
        }
        EffectiveMemoryBudgetReport {
            preset: crate::domain::policy::MemoryPreset::Production,
            execution: crate::domain::policy::ExecutionBudget::default(),
            residency,
            override_sources: vec!["test".to_string()],
            override_provenance: Vec::new(),
            policy_revision: 1,
            active_policy_source: "test".to_string(),
        }
    }

    fn class_budget(reserved: u64, soft: u64, hard: u64, borrow: bool) -> MemoryClassBudget {
        MemoryClassBudget {
            reserved_bytes: reserved,
            soft_cap_bytes: soft,
            hard_cap_bytes: hard,
            borrow_from_shared_pool: borrow,
            reclaim_priority: ReclaimPriority::Normal,
        }
    }

    fn base_residency() -> ResidencyPolicy {
        ResidencyPolicy::new(1_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400_000, 600_000, false))
            .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 400_000, 600_000, false))
            .with_owner_quota(
                "owner:a",
                OwnerQuota { soft_cap_bytes: 300_000, hard_cap_bytes: 500_000 },
            )
    }

    fn broker() -> LiveProcessMemoryBroker {
        LiveProcessMemoryBroker::from_report(report_with(base_residency()))
            .expect("base residency is valid")
    }

    #[test]
    fn overlong_owner_is_denied_before_reclaim_or_accounting() {
        let b = broker();
        let before = b.ledger_snapshot().expect("before snapshot");
        let owner = "o".repeat(crate::domain::management_limits::MAX_OWNER_KEY_BYTES + 1);
        let result = b.reserve_owned(
            MemoryClass::ArtifactCache,
            owner,
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            0,
        );
        assert!(matches!(result, Err(MemoryError::Capacity(_))));
        assert_eq!(b.ledger_snapshot().expect("after snapshot"), before);
        assert_eq!(b.reclaim_runtime_stats().expect("reclaim stats").attempts, 0);
    }

    #[test]
    fn scratch_reserve_cap_overflow_is_denied_without_mutation() {
        let mut report = report_with(ResidencyPolicy::new(u64::MAX));
        report.execution.max_execution_scratch_bytes = Some(usize::MAX);
        let b = LiveProcessMemoryBroker::from_report(report).expect("validated policy");
        let first = b
            .reserve(
                MemoryClass::ArtifactCache,
                "o".into(),
                MemoryContextKind::Request,
                ChargeKind::LiveScratch,
                u64::MAX - 10,
            )
            .expect("first reserve");
        b.commit(&first).expect("commit first");
        let before = b.ledger_snapshot().expect("before");
        let second = b.reserve(
            MemoryClass::EvidenceCache,
            "o".into(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            100,
        );
        assert!(
            matches!(second, Err(MemoryError::Capacity(_))),
            "scratch cap overflow must deny: {second:?}"
        );
        assert_eq!(b.ledger_snapshot().expect("after denial"), before);
        b.release(&first).expect("first ticket remains live");
        assert_eq!(b.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }

    #[test]
    fn scratch_recharge_cap_overflow_preserves_original_ticket() {
        let mut report = report_with(ResidencyPolicy::new(u64::MAX));
        report.execution.max_execution_scratch_bytes = Some(usize::MAX);
        let b = LiveProcessMemoryBroker::from_report(report).expect("validated policy");
        let first = b
            .reserve(
                MemoryClass::ArtifactCache,
                "o".into(),
                MemoryContextKind::Request,
                ChargeKind::LiveScratch,
                u64::MAX - 11,
            )
            .expect("first reserve");
        b.commit(&first).expect("commit first");
        let old = b
            .reserve(
                MemoryClass::EvidenceCache,
                "o".into(),
                MemoryContextKind::Request,
                ChargeKind::LiveScratch,
                1,
            )
            .expect("old reserve");
        b.commit(&old).expect("commit old");
        let before = b.ledger_snapshot().expect("before");
        let replacement = b.recharge(
            &old,
            MemoryClass::EvidenceCache,
            "o".into(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            100,
        );
        assert!(
            matches!(replacement, Err(RechargeError { error: MemoryError::Capacity(_), .. })),
            "scratch recharge overflow must deny: {replacement:?}"
        );
        assert_eq!(b.ledger_snapshot().expect("after denial"), before);
        b.release(&old).expect("old ticket remains live");
        b.release(&first).expect("first ticket remains live");
        assert_eq!(b.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }

    #[test]
    fn dropping_pending_reservation_rolls_back_all_axes() {
        let b = broker();
        let before = b.ledger_snapshot().expect("before");
        {
            let pending = b
                .reserve_owned(
                    MemoryClass::ArtifactCache,
                    "owner:a".to_string(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    100_000,
                )
                .expect("reserve pending");
            assert_eq!(pending.bytes(), 100_000);
            assert_eq!(b.ledger_snapshot().expect("reserved").total_owner_bytes(), 100_000);
        }

        assert_eq!(b.ledger_snapshot().expect("after drop"), before);
    }

    #[test]
    fn dropping_pending_reservation_during_rebuild_does_not_strand_capacity() {
        let b = broker();
        let pending = b
            .reserve_owned(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("reserve pending");
        b.begin_rebuild().expect("begin rebuild");

        drop(pending);

        let snapshot = b.ledger_snapshot().expect("snapshot");
        assert!(snapshot.rebuild_in_progress);
        assert_eq!(snapshot.total_owner_bytes(), 0);
        b.finish_rebuild().expect("finish rebuild");
    }

    #[test]
    fn failed_pending_commit_during_rebuild_rolls_back_capacity() {
        let b = broker();
        let pending = b
            .reserve_owned(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("reserve pending");
        b.begin_rebuild().expect("begin rebuild");

        assert!(matches!(pending.commit(), Err(MemoryError::InvalidState(_))));
        assert_eq!(b.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
        b.finish_rebuild().expect("finish rebuild");
    }

    #[test]
    fn pending_drop_restores_resource_group_capacity() {
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 100,
            hard_cap_bytes: 100,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let residency = ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 10_000, 10_000, false))
            .with_resource_group("g", group)
            .with_owner_group("owner:g", "g");
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("broker");
        let pending = b
            .reserve_owned(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100,
            )
            .expect("fill group capacity");
        assert!(!b.admits_group("g", 1).expect("probe full group"));

        drop(pending);

        assert!(b.admits_group("g", 100).expect("capacity restored"));
        assert_eq!(b.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
    }

    fn managed_charge(broker: &LiveProcessMemoryBroker, bytes: u64) -> ManagedCharge {
        broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                bytes,
            )
            .expect("reserve managed charge")
    }

    #[test]
    fn deferred_committed_cleanup_is_batched_and_drains_large_backlog() {
        let b = broker();
        let count = crate::domain::management_limits::MAX_DEFERRED_TICKET_DRAIN_BATCH * 2 + 17;
        let charges: Vec<_> = (0..count).map(|_| managed_charge(&b, 0)).collect();
        b.begin_rebuild().expect("begin rebuild");
        drop(charges);
        assert_eq!(b.inner.lock().expect("core").deferred_committed_releases.len(), count);

        // Exercise the critical-section unit directly: the first pass may
        // consume only one configured batch, leaving the exact remainder queued.
        {
            let mut core = b.inner.lock().expect("core");
            core.ledger.finish_rebuild();
            assert_eq!(
                LiveProcessMemoryBroker::drain_deferred_committed_releases_batch(&mut core)
                    .expect("first bounded batch"),
                crate::domain::management_limits::MAX_DEFERRED_TICKET_DRAIN_BATCH
            );
            assert_eq!(
                core.deferred_committed_releases.len(),
                count - crate::domain::management_limits::MAX_DEFERRED_TICKET_DRAIN_BATCH
            );
        }
        b.finish_rebuild().expect("finish remaining batches");
        let snapshot = b.ledger_snapshot().expect("snapshot");
        assert_eq!(snapshot.live_ticket_count, 0);
        assert_eq!(snapshot.total_owner_bytes(), 0);
        assert!(b.inner.lock().expect("core").deferred_committed_releases.is_empty());
    }

    #[test]
    fn managed_charge_drop_on_healthy_broker_releases_exact_capacity() {
        let b = broker();
        let charge = managed_charge(&b, 100_000);
        assert_eq!(charge.bytes(), Some(100_000));
        assert_eq!(charge.owner().map(String::as_str), Some("owner:a"));
        assert_eq!(charge.context(), Some(MemoryContextKind::Request));
        assert_eq!(charge.kind(), Some(ChargeKind::RetainedPayload));
        assert_eq!(b.ledger_snapshot().expect("charged").total_owner_bytes(), 100_000);

        drop(charge);

        assert_eq!(b.ledger_snapshot().expect("released").total_owner_bytes(), 0);
    }

    #[test]
    fn managed_charge_drop_during_rebuild_defers_until_finish() {
        let b = broker();
        let charge = managed_charge(&b, 100_000);
        b.begin_rebuild().expect("begin rebuild");

        drop(charge);

        let fenced = b.ledger_snapshot().expect("fenced snapshot");
        assert!(fenced.rebuild_in_progress);
        assert_eq!(fenced.total_owner_bytes(), 100_000, "deferred release stays conservative");

        b.finish_rebuild().expect("finish drains deferred charge");
        let drained = b.ledger_snapshot().expect("drained snapshot");
        assert!(!drained.rebuild_in_progress);
        assert_eq!(drained.total_owner_bytes(), 0);
    }

    #[test]
    fn managed_charge_explicit_release_failure_retains_authority() {
        let b = broker();
        let mut charge = managed_charge(&b, 100_000);
        let id = charge.id();
        b.begin_rebuild().expect("begin rebuild");

        let error = charge.release().expect_err("release is fenced during rebuild");
        assert!(matches!(error, MemoryError::InvalidState(_)));
        assert!(charge.is_active());
        assert_eq!(charge.id(), id);
        assert_eq!(b.ledger_snapshot().expect("still charged").total_owner_bytes(), 100_000);

        b.finish_rebuild().expect("finish rebuild");
        charge.release().expect("retry exact release");
        assert!(!charge.is_active());
        assert_eq!(b.ledger_snapshot().expect("released").total_owner_bytes(), 0);
    }

    #[test]
    fn managed_charge_batch_failure_retains_every_lease() {
        let b = broker();
        let mut charges = [managed_charge(&b, 100_000), managed_charge(&b, 120_000)];
        let ids = [charges[0].id(), charges[1].id()];
        b.begin_rebuild().expect("begin rebuild");

        b.release_managed_many(&mut charges).expect_err("batch release is fenced");
        assert!(charges.iter().all(ManagedCharge::is_active));
        assert_eq!([charges[0].id(), charges[1].id()], ids);
        assert_eq!(b.ledger_snapshot().expect("unchanged").total_owner_bytes(), 220_000);

        b.finish_rebuild().expect("finish rebuild");
        b.release_managed_many(&mut charges).expect("atomic retry");
        assert!(charges.iter().all(|charge| !charge.is_active()));
        assert_eq!(b.ledger_snapshot().expect("released").total_owner_bytes(), 0);
    }

    #[test]
    fn managed_charge_batch_rejects_foreign_broker_before_mutation() {
        let a = broker();
        let b = broker();
        let mut charges = [managed_charge(&a, 100_000), managed_charge(&b, 120_000)];

        let error = a
            .release_managed_many(&mut charges)
            .expect_err("foreign broker lease must reject the complete batch");
        assert!(matches!(error, MemoryError::InvalidState(_)));
        assert!(charges.iter().all(ManagedCharge::is_active));
        assert_eq!(a.ledger_snapshot().expect("a unchanged").total_owner_bytes(), 100_000);
        assert_eq!(b.ledger_snapshot().expect("b unchanged").total_owner_bytes(), 120_000);

        drop(charges);
        assert_eq!(a.ledger_snapshot().expect("a drop cleanup").total_owner_bytes(), 0);
        assert_eq!(b.ledger_snapshot().expect("b drop cleanup").total_owner_bytes(), 0);
    }

    #[test]
    fn managed_charge_recharge_denial_restores_old_authority() {
        let b = broker();
        let mut charge = managed_charge(&b, 100_000);
        let original_id = charge.id();

        let denied = charge
            .recharge(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                700_000,
            )
            .expect_err("replacement over class cap is denied");
        assert_eq!(denied.recovery, ManagedChargeRechargeRecovery::ChargePreserved);
        assert_eq!(charge.id(), original_id);
        assert_eq!(charge.bytes(), Some(100_000));
        assert_eq!(b.ledger_snapshot().expect("old preserved").total_owner_bytes(), 100_000);

        charge
            .recharge(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                200_000,
            )
            .expect("legal replacement");
        assert_ne!(charge.id(), original_id);
        assert_eq!(charge.bytes(), Some(200_000));
        drop(charge);
        assert_eq!(b.ledger_snapshot().expect("drop replacement").total_owner_bytes(), 0);
    }

    #[test]
    fn managed_recharge_at_live_ticket_cap_keeps_broker_authority_maps_exact() {
        let broker = LiveProcessMemoryBroker::from_report(report_with(ResidencyPolicy::new(1_000)))
            .expect("broker");
        let mut charges =
            Vec::with_capacity(crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER);
        for _ in 0..crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER {
            charges.push(
                broker
                    .reserve_managed_charge(
                        MemoryClass::ArtifactCache,
                        "owner:cap".to_string(),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        0,
                    )
                    .expect("zero-byte authority below cap"),
            );
        }
        let source = charges.last_mut().expect("source charge");
        let old_id = source.id();
        source
            .recharge(
                MemoryClass::ArtifactCache,
                "owner:cap".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1,
            )
            .expect("managed replacement needs no extra ticket slot");
        assert_ne!(source.id(), old_id);
        let snapshot = broker.ledger_snapshot().expect("snapshot");
        snapshot.validate_exact().expect("axes after managed swap");
        assert_eq!(
            snapshot.live_ticket_count,
            crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
        );
        assert_eq!(snapshot.total_owner_bytes(), 1);
        drop(charges);
        sep23_assert_fully_drained(&broker);
    }

    #[test]
    fn replace_denial_returns_original_live_charge() {
        let b = broker();
        let charge = b
            .reserve_committed_owned(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("seed committed charge");

        let denied = b
            .recharge_owned(
                charge,
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                700_000,
            )
            .expect_err("replacement over class cap is denied");
        let ChargeRechargeRecovery::ChargePreserved(charge) = denied.recovery;
        assert_eq!(charge.bytes(), 100_000);
        assert_eq!(b.ledger_snapshot().expect("snapshot").total_owner_bytes(), 100_000);
        b.release_owned(charge).expect("release preserved charge");
    }

    #[test]
    fn owned_charge_sequence_matches_exact_reference_model() {
        let b = broker();
        let mut live = Vec::<ManagedCharge>::new();
        let mut model_bytes = 0u64;

        for step in 0..100u64 {
            match step % 4 {
                0 => {
                    let pending = b
                        .reserve_owned(
                            MemoryClass::ArtifactCache,
                            "owner:a".to_string(),
                            MemoryContextKind::Request,
                            ChargeKind::RetainedPayload,
                            100,
                        )
                        .expect("pending reserve");
                    drop(pending);
                }
                1 => {
                    let charge = b
                        .reserve_owned(
                            MemoryClass::ArtifactCache,
                            "owner:a".to_string(),
                            MemoryContextKind::Request,
                            ChargeKind::RetainedPayload,
                            100,
                        )
                        .expect("reserve")
                        .commit()
                        .expect("commit");
                    live.push(charge);
                    model_bytes += 100;
                }
                2 if !live.is_empty() => {
                    let mut charge = live.pop().expect("non-empty");
                    model_bytes -= charge.bytes().expect("active charge");
                    charge.release().expect("release");
                }
                _ if !live.is_empty() => {
                    let mut replacement = live.pop().expect("non-empty");
                    model_bytes -= replacement.bytes().expect("active charge");
                    replacement
                        .recharge(
                            MemoryClass::ArtifactCache,
                            "owner:a".to_string(),
                            MemoryContextKind::Request,
                            ChargeKind::RetainedPayload,
                            150,
                        )
                        .expect("replacement fits");
                    model_bytes += replacement.bytes().expect("replacement active");
                    live.push(replacement);
                }
                _ => {}
            }

            let snapshot = b.ledger_snapshot().expect("snapshot");
            assert_eq!(snapshot.total_owner_bytes(), model_bytes, "step {step}");
            assert!(snapshot.class_owner_balanced(), "step {step}");
            assert!(snapshot.owner_partition_balanced(), "step {step}");
            assert!(snapshot.context_owner_balanced(), "step {step}");
        }

        while let Some(mut charge) = live.pop() {
            model_bytes -= charge.bytes().expect("active charge");
            charge.release().expect("drain");
        }
        assert_eq!(model_bytes, 0);
        assert_eq!(b.ledger_snapshot().expect("final").total_owner_bytes(), 0);
    }

    // --- Invariant 1 & 2: class total == owner total == payload+metadata+scratch
    #[test]
    fn reserve_commit_keeps_class_owner_and_partition_balanced() {
        let b = broker();
        let t1 = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("reserve within caps must succeed");
        b.commit(&t1).expect("commit must succeed");

        let t2 = b
            .reserve(
                MemoryClass::EvidenceCache,
                "owner:a".to_string(),
                MemoryContextKind::Session,
                ChargeKind::RetainedMetadata,
                50_000,
            )
            .expect("second reserve within caps must succeed");
        b.commit(&t2).expect("commit must succeed");

        let snap = b.ledger_snapshot().expect("snapshot");
        // Invariant 1
        assert_eq!(
            snap.total_class_bytes(),
            snap.total_owner_bytes(),
            "class total must equal owner total"
        );
        assert!(snap.class_owner_balanced());
        // Invariant 2
        assert!(snap.owner_partition_balanced(), "owner usage must partition into kinds");
        assert_eq!(snap.total_owner_bytes(), 150_000);
        assert_eq!(snap.total_retained_payload_bytes(), 100_000);
        assert_eq!(snap.total_retained_metadata_bytes(), 50_000);
        assert_eq!(snap.total_live_scratch_bytes(), 0);
    }

    // --- Negative path 1: reserve denied BEFORE mutation (owner hard cap)
    #[test]
    fn reserve_denied_owner_hard_cap_leaves_ledger_untouched() {
        let b = broker();
        // Charge owner:a up to 400k (under 500k hard cap).
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                400_000,
            )
            .expect("first reserve fits");
        b.commit(&t).expect("commit");
        let before = b.ledger_snapshot().expect("snapshot before");

        // Now request 200k more -> 600k > 500k owner hard cap. Must deny.
        let denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            200_000,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "must be typed capacity error");

        let after = b.ledger_snapshot().expect("snapshot after");
        assert_eq!(
            before.total_owner_bytes(),
            after.total_owner_bytes(),
            "denied reserve must NOT mutate accounting"
        );
        assert_eq!(after.total_owner_bytes(), 400_000);
    }

    // --- Negative path 3: class hard cap exceeded when class may not borrow
    #[test]
    fn reserve_denied_class_hard_cap_no_borrow() {
        // ArtifactCache hard cap 600k, borrow=false. Use a roomy owner with no
        // quota so the *class* cap is what trips, not the owner cap.
        let residency = ResidencyPolicy::new(1_000_000).with_class_budget(
            MemoryClass::ArtifactCache,
            class_budget(0, 400_000, 600_000, false),
        );
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");

        // owner:unbounded has no quota entry -> only class cap applies.
        let denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:unbounded".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            650_000,
        );
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "class hard cap breach (no borrow) must be a typed capacity error"
        );
        // negative pair: a borrow-enabled class with a funded burst pool accepts
        // the same overflow (the 50k over-cap fits the 100k shared pool).
        let borrow_residency = ResidencyPolicy::new(1_000_000)
            .with_shared_burst_pool_bytes(100_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400_000, 600_000, true));
        let b2 =
            LiveProcessMemoryBroker::from_report(report_with(borrow_residency)).expect("valid");
        assert!(
            b2.reserve(
                MemoryClass::ArtifactCache,
                "owner:unbounded".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                650_000,
            )
            .is_ok(),
            "borrow-enabled class admits overflow within the shared burst pool"
        );
    }

    // --- #6 at the broker surface: borrowing past the burst pool is denied
    #[test]
    fn reserve_borrow_beyond_shared_pool_denied() {
        let residency = ResidencyPolicy::new(10_000_000)
            .with_shared_burst_pool_bytes(40_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400_000, 600_000, true));
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
        // 650k over a 600k cap = 50k borrow > 40k pool -> denied.
        let denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:unbounded".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            650_000,
        );
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "borrow exceeding the shared burst pool must be a typed capacity error"
        );
    }

    // --- Invariant: release subtracts and keeps balance
    #[test]
    fn release_returns_bytes_and_preserves_invariants() {
        let b = broker();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                120_000,
            )
            .expect("reserve");
        b.commit(&t).expect("commit");
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 120_000);

        b.release(&t).expect("release");
        let snap = b.ledger_snapshot().expect("snap");
        assert_eq!(snap.total_owner_bytes(), 0, "release must subtract owner usage");
        assert!(snap.class_owner_balanced());
        assert!(snap.owner_partition_balanced());
    }

    // --- Negative path 6: rebuild-in-progress rejects writes
    #[test]
    fn rebuild_in_progress_rejects_reserve_commit_release() {
        let b = broker();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                10_000,
            )
            .expect("reserve before rebuild");
        b.begin_rebuild().expect("begin rebuild");

        let r = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            10_000,
        );
        assert!(matches!(r, Err(MemoryError::InvalidState(_))), "reserve rejected during rebuild");
        assert!(
            matches!(b.commit(&t), Err(MemoryError::InvalidState(_))),
            "commit rejected during rebuild"
        );
        assert!(
            matches!(b.release(&t), Err(MemoryError::InvalidState(_))),
            "release rejected during rebuild"
        );

        // positive pair: after finishing rebuild, writes resume.
        b.finish_rebuild().expect("finish rebuild");
        assert!(b.commit(&t).is_ok(), "commit resumes after rebuild");
    }

    // --- Invariant 5: denied live-reconfig leaves old active policy untouched
    #[test]
    fn live_reconfig_shrink_below_usage_denied_and_noop() {
        let b = broker();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                300_000,
            )
            .expect("reserve");
        b.commit(&t).expect("commit");

        let before = b.effective_report().expect("report").residency.global_hard_limit_bytes;

        // Attempt to shrink global hard limit below current usage (300k).
        let shrink = ResidencyPolicy::new(100_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 80_000, 100_000, false));
        let denied = b.try_live_reconfigure(shrink);
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "shrink below usage must be denied"
        );

        let after = b.effective_report().expect("report").residency.global_hard_limit_bytes;
        assert_eq!(before, after, "denied reconfig must leave active policy untouched");
        // The ledger must still be intact too.
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 300_000);
    }

    #[test]
    fn live_reconfig_class_hard_cap_below_class_usage_denied() {
        let b = broker();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                300_000,
            )
            .expect("reserve");
        b.commit(&t).expect("commit");

        // New policy shrinks ArtifactCache hard cap to 200k < 300k used.
        let bad = base_residency().with_class_budget(
            MemoryClass::ArtifactCache,
            class_budget(0, 150_000, 200_000, false),
        );
        assert!(
            matches!(b.try_live_reconfigure(bad), Err(MemoryError::InvalidState(_))),
            "class cap below class usage must be denied"
        );
    }

    // --- #5: a class holding live bytes cannot be silently dropped (fail-open)
    #[test]
    fn live_reconfig_dropping_a_class_with_live_usage_is_denied() {
        let b = broker();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                300_000,
            )
            .expect("reserve");
        b.commit(&t).expect("commit");

        // New policy raises the global limit but OMITS ArtifactCache entirely,
        // which previously left that class uncapped (fail-open). Must be denied.
        let dropped = ResidencyPolicy::new(2_000_000).with_class_budget(
            MemoryClass::EvidenceCache,
            class_budget(0, 400_000, 600_000, false),
        );
        let denied = b.try_live_reconfigure(dropped);
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "dropping a class that holds live bytes must be denied, got {denied:?}"
        );
        // untouched
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 300_000);
        assert_eq!(
            b.effective_report().expect("report").residency.global_hard_limit_bytes,
            1_000_000,
            "denied reconfig must leave the active policy untouched"
        );
    }

    // --- #5: an owner quota below that owner's live usage is denied
    #[test]
    fn live_reconfig_owner_quota_below_owner_usage_is_denied() {
        let b = broker();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                300_000,
            )
            .expect("reserve");
        b.commit(&t).expect("commit");

        // owner:a holds 300k; install a quota with a 200k hard cap -> contradiction.
        let bad = base_residency().with_owner_quota(
            "owner:a",
            OwnerQuota { soft_cap_bytes: 150_000, hard_cap_bytes: 200_000 },
        );
        assert!(
            matches!(b.try_live_reconfigure(bad), Err(MemoryError::InvalidState(_))),
            "owner quota below current owner usage must be denied"
        );
    }

    // --- audit fix: raising headroom over live non-Headroom usage is denied
    // (invariant 5 band must keep holding for the live state, not just future
    // reserves). Without the check, the reconfigure would silently strand 800k of
    // non-Headroom bytes inside the newly reserved headroom band.
    #[test]
    fn live_reconfig_raising_headroom_over_non_headroom_usage_is_denied() {
        // Roomy class + no owner quota so ONLY the headroom band can bite.
        let residency = ResidencyPolicy::new(1_000_000).with_class_budget(
            MemoryClass::ArtifactCache,
            class_budget(0, 800_000, 1_000_000, false),
        );
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:free".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                800_000,
            )
            .expect("reserve 800k non-Headroom");
        b.commit(&t).expect("commit");

        // Raise headroom to 300k: effective non-Headroom ceiling 700k < 800k live.
        let raised =
            ResidencyPolicy::new(1_000_000).with_headroom_bytes(300_000).with_class_budget(
                MemoryClass::ArtifactCache,
                class_budget(0, 800_000, 1_000_000, false),
            );
        let denied = b.try_live_reconfigure(raised);
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "raising headroom above the free band must be denied, got {denied:?}"
        );
        // Untouched: still 0 headroom in the active policy.
        assert_eq!(b.effective_report().expect("report").residency.headroom_bytes, 0);

        // A headroom of 100k (ceiling 900k >= 800k live) is legal and accepted.
        let legal = ResidencyPolicy::new(1_000_000).with_headroom_bytes(100_000).with_class_budget(
            MemoryClass::ArtifactCache,
            class_budget(0, 800_000, 1_000_000, false),
        );
        b.try_live_reconfigure(legal).expect("headroom within the free band is legal");
        assert_eq!(b.effective_report().expect("report").residency.headroom_bytes, 100_000);
    }

    // --- audit fix: lowering a resource-group hard cap below the group's live
    // usage is denied (symmetric with the class/owner/headroom live-usage checks);
    // without it the reconfigure would install a group cap the group already breaks.
    #[test]
    fn live_reconfig_lowering_group_cap_below_live_group_usage_is_denied() {
        use crate::domain::policy::ResourceGroup;
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50_000,
            hard_cap_bytes: 100_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let residency = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
            .with_resource_group("g", group.clone())
            .with_owner_group("owner:g", "g");
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
        let t = b
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                90_000,
            )
            .expect("group usage 90k");

        // Lower group g's cap to 50k (< 90k live), mapping unchanged -> denied.
        let lowered = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
            .with_resource_group("g", ResourceGroup { hard_cap_bytes: 50_000, ..group })
            .with_owner_group("owner:g", "g");
        let denied = b.try_live_reconfigure(lowered);
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "lowering a group cap below live group usage must be denied, got {denied:?}"
        );
        let observation = b.observe().expect("lock-consistent group observation");
        assert_eq!(observation.resource_group_usage.get("g"), Some(&90_000));
        assert_eq!(observation.ledger.live_ticket_count, 1);

        // Raising the cap to 120k (>= 90k live) is legal and accepted.
        let raised = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
            .with_resource_group("g", ResourceGroup { hard_cap_bytes: 120_000, ..group })
            .with_owner_group("owner:g", "g");
        b.try_live_reconfigure(raised).expect("raising the group cap over live usage is legal");
        b.release(&t).expect("release");
        let drained = b.observe().expect("drained group observation");
        assert_eq!(drained.resource_group_usage.get("g"), Some(&0));
        assert_eq!(drained.ledger.live_ticket_count, 0);
    }

    // --- positive pair: a legal reconfig (raising caps) is accepted
    #[test]
    fn live_reconfig_raise_caps_accepted_and_preserves_usage() {
        let b = broker();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                300_000,
            )
            .expect("reserve");
        b.commit(&t).expect("commit");

        let bigger = ResidencyPolicy::new(2_000_000)
            .with_class_budget(
                MemoryClass::ArtifactCache,
                class_budget(0, 800_000, 1_200_000, false),
            )
            .with_owner_quota(
                "owner:a",
                OwnerQuota { soft_cap_bytes: 600_000, hard_cap_bytes: 900_000 },
            );
        b.try_live_reconfigure(bigger).expect("raising caps is legal");

        let snap = b.ledger_snapshot().expect("snap");
        assert_eq!(snap.total_owner_bytes(), 300_000, "usage preserved across legal reconfig");
        assert!(snap.class_owner_balanced());
        assert_eq!(
            b.effective_report().expect("report").residency.global_hard_limit_bytes,
            2_000_000
        );
    }

    // --- Invariant 6: allocator telemetry mismatch does not mutate accounting
    #[test]
    fn allocator_advisory_mismatch_does_not_mutate_accounting() {
        let b = broker();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("reserve");
        b.commit(&t).expect("commit");
        let before = b.ledger_snapshot().expect("snap").total_owner_bytes();

        // Allocator claims a wildly different resident value.
        let snapshot =
            AllocatorSnapshot { resident_bytes: Some(999_999), ..AllocatorSnapshot::default() };
        let advisory = b.observe_allocator(&snapshot).expect("observe");
        assert!(advisory.mismatch, "advisory must flag the disagreement");
        assert_eq!(
            advisory.semantic_bytes, before,
            "advisory reports semantic SSOT, not allocator"
        );
        assert_eq!(advisory.drift_bytes(), Some(999_999i128 - before as i128));

        let after = b.ledger_snapshot().expect("snap").total_owner_bytes();
        assert_eq!(before, after, "advisory observation must NOT change semantic accounting");
    }

    // --- pressure classification is semantic, not allocator-driven
    #[test]
    fn pressure_classifies_from_semantic_usage() {
        let b = broker();
        // Global hard limit is 1_000_000. Default thresholds soft=75%.
        assert_eq!(b.classify_pressure().expect("classify"), PressureState::Normal);

        // Charge owner:a to 450k via two owners to cross 75% globally.
        let t1 = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                400_000,
            )
            .expect("reserve a");
        b.commit(&t1).expect("commit");
        let t2 = b
            .reserve(
                MemoryClass::EvidenceCache,
                "owner:c".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                400_000,
            )
            .expect("reserve c");
        b.commit(&t2).expect("commit");
        // 800k / 1_000_000 = 80% -> Soft (>=75) but < hard (90).
        assert_eq!(b.classify_pressure().expect("classify"), PressureState::Soft);
    }

    // === Scenario group C: control-plane extras ===========================

    // scenario C: reconfigure atomicity. A DENIED try_live_reconfigure must
    // leave scheduler + controller + report ALL unchanged. We capture the full
    // observable state (effective_report residency, ledger snapshot, pressure
    // classification, scheduler admission) BEFORE the denied op and assert each
    // is identical AFTER -- fail-closed, all-or-nothing.
    #[test]
    fn denied_reconfigure_leaves_scheduler_controller_and_report_unchanged() {
        let residency = base_residency().with_resource_group(
            "group:a",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 100_000,
                hard_cap_bytes: 200_000,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        );
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
        // Drive usage to 800k so classification is Soft and so a shrink is illegal.
        let t1 = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                400_000,
            )
            .expect("reserve a");
        b.commit(&t1).expect("commit");
        let t2 = b
            .reserve(
                MemoryClass::EvidenceCache,
                "owner:c".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                400_000,
            )
            .expect("reserve c");
        b.commit(&t2).expect("commit");

        // Capture the entire observable surface before the denied op.
        let report_before = b.effective_report().expect("report");
        let snap_before = b.ledger_snapshot().expect("snap");
        let pressure_before = b.classify_pressure().expect("classify");
        let admits_before = b.admits_group("group:a", 150_000).expect("admits");
        assert_eq!(pressure_before, PressureState::Soft, "precondition: 80% is Soft");

        // Illegal narrowing: global limit 500k < current usage 800k -> denied.
        let shrink = ResidencyPolicy::new(500_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400_000, 450_000, false))
            .with_class_budget(
                MemoryClass::EvidenceCache,
                class_budget(0, 400_000, 450_000, false),
            );
        let denied = b.try_live_reconfigure(shrink);
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "shrink below live usage must be denied, got {denied:?}"
        );

        // Every facet must be byte-for-byte identical after the denied op.
        let report_after = b.effective_report().expect("report");
        let snap_after = b.ledger_snapshot().expect("snap");
        let pressure_after = b.classify_pressure().expect("classify");
        let admits_after = b.admits_group("group:a", 150_000).expect("admits");
        assert_eq!(report_before.residency, report_after.residency, "report residency unchanged");
        assert_eq!(
            snap_before.total_owner_bytes(),
            snap_after.total_owner_bytes(),
            "ledger snapshot unchanged"
        );
        assert_eq!(snap_before.class_usage, snap_after.class_usage, "class usage unchanged");
        assert_eq!(
            pressure_before, pressure_after,
            "controller classification unchanged (still Soft against the old 1M limit)"
        );
        assert_eq!(admits_before, admits_after, "scheduler admission unchanged");
    }

    // scenario C: an ACCEPTED legal raise rebuilds derived state. Usage that was
    // Soft under the old global limit must become Normal under a larger one,
    // proving the controller was rebuilt from the new policy (not stale).
    #[test]
    fn accepted_reconfigure_rebuilds_controller_to_new_global_limit() {
        let b = broker();
        let t1 = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                400_000,
            )
            .expect("reserve a");
        b.commit(&t1).expect("commit");
        let t2 = b
            .reserve(
                MemoryClass::EvidenceCache,
                "owner:c".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                400_000,
            )
            .expect("reserve c");
        b.commit(&t2).expect("commit");
        // 800k / 1M = 80% -> Soft under the old limit.
        assert_eq!(b.classify_pressure().expect("classify"), PressureState::Soft);

        // Raise the global limit to 4M (and class caps to fit). Now 800k / 4M =
        // 20% -> Normal. Caps only rise so the reconfig is legal.
        let bigger = ResidencyPolicy::new(4_000_000)
            .with_class_budget(
                MemoryClass::ArtifactCache,
                class_budget(0, 1_000_000, 2_000_000, false),
            )
            .with_class_budget(
                MemoryClass::EvidenceCache,
                class_budget(0, 1_000_000, 2_000_000, false),
            );
        b.try_live_reconfigure(bigger).expect("raising the global limit is legal");

        assert_eq!(
            b.classify_pressure().expect("classify"),
            PressureState::Normal,
            "controller rebuilt: 800k is now Normal against the new 4M limit"
        );
        // Usage is preserved across the rebuild.
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 800_000);
        assert_eq!(
            b.effective_report().expect("report").residency.global_hard_limit_bytes,
            4_000_000,
            "report residency reflects the accepted new global limit"
        );
    }

    #[test]
    fn reconfigure_rejects_policy_missing_drained_mandatory_class() {
        let b = broker();
        let before = b.effective_report().expect("report");
        let mut incomplete = ResidencyPolicy::new(2_000_000);
        incomplete.classes.remove(&MemoryClass::GenerationMetadata);

        let result = b.try_live_reconfigure(incomplete);

        assert!(matches!(result, Err(MemoryError::Config(_))));
        assert_eq!(b.effective_report().expect("report"), before);
        assert_eq!(b.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
    }

    #[test]
    fn reconfigure_missing_class_denial_preserves_policy_scheduler_and_usage() {
        let b = broker();
        let ticket = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("seed live usage");
        b.commit(&ticket).expect("commit");
        let report_before = b.effective_report().expect("report");
        let snapshot_before = b.ledger_snapshot().expect("snapshot");
        let admission_before = b.admits_group("absent", 1).expect("admission probe");

        let mut incomplete = ResidencyPolicy::new(2_000_000);
        incomplete.classes.remove(&MemoryClass::ArtifactCache);
        let result = b.try_live_reconfigure(incomplete);

        assert!(matches!(result, Err(MemoryError::Config(_))));
        assert_eq!(b.effective_report().expect("report"), report_before);
        assert_eq!(b.ledger_snapshot().expect("snapshot"), snapshot_before);
        assert_eq!(b.admits_group("absent", 1).expect("admission probe"), admission_before);
    }

    // scenario C: pressure threshold boundaries. Default thresholds are
    // soft=75%, hard=90%, fatal=98% and the classifier uses `>=` (inclusive
    // lower bound) on `used*100/limit`. We drive usage to exactly the boundary
    // fractions of the 1_000_000 global limit and pin the precise transitions,
    // documenting the >= (not >) behavior: 750k (75%) is Soft, 749_999 (<75%)
    // is Normal; 900k (90%) is Hard; 980k (98%) is Fatal.
    #[test]
    fn pressure_thresholds_are_inclusive_at_exact_boundary_fractions() {
        // (used_bytes, expected) over the 1_000_000 limit. `>=` semantics:
        // the boundary value itself crosses up into the higher band.
        let cases: &[(u64, PressureState)] = &[
            (0, PressureState::Normal),
            (749_999, PressureState::Normal), // just below 75% stays Normal
            (750_000, PressureState::Soft),   // exactly 75% -> Soft (>= boundary)
            (899_999, PressureState::Soft),   // just below 90% stays Soft
            (900_000, PressureState::Hard),   // exactly 90% -> Hard
            (979_999, PressureState::Hard),   // just below 98% stays Hard
            (980_000, PressureState::Fatal),  // exactly 98% -> Fatal
        ];
        for &(used, expected) in cases {
            // Fresh broker per case so usage is deterministic and isolated. The
            // single class/owner caps are set at the global limit so the *only*
            // thing under test is the controller's threshold classification, not
            // an incidental class/owner cap denial.
            let roomy = ResidencyPolicy::new(1_000_000).with_class_budget(
                MemoryClass::ArtifactCache,
                class_budget(0, 1_000_000, 1_000_000, false),
            );
            let b = LiveProcessMemoryBroker::from_report(report_with(roomy)).expect("valid");
            let t = b
                .reserve(
                    MemoryClass::ArtifactCache,
                    "owner:boundary".to_string(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    used,
                )
                .expect("reserve at boundary fits within the at-limit class cap");
            b.commit(&t).expect("commit");
            assert_eq!(
                b.classify_pressure().expect("classify"),
                expected,
                "usage {used} / 1_000_000 must classify as {expected:?} (>= boundary)"
            );
        }
    }

    // scenario C: rebuild re-entrancy. begin_rebuild twice then finish once must
    // yield coherent state: after two begins the ledger is rebuilding and writes
    // are rejected; a single finish clears it and writes resume. A finish when
    // NOT rebuilding is a no-op (returns Ok, leaves the flag false) -- pinned to
    // the actual ledger behavior.
    #[test]
    fn rebuild_is_reentrant_double_begin_single_finish_then_idle_finish_is_noop() {
        let b = broker();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                10_000,
            )
            .expect("reserve before rebuild");

        // Two begins: idempotent, still a single logical rebuild in progress.
        b.begin_rebuild().expect("first begin");
        b.begin_rebuild().expect("second begin is idempotent");
        assert!(
            b.ledger_snapshot().expect("snap").rebuild_in_progress,
            "after two begins the ledger is rebuilding"
        );
        assert!(
            matches!(b.commit(&t), Err(MemoryError::InvalidState(_))),
            "writes rejected while rebuilding (after double-begin)"
        );

        // A SINGLE finish clears the (single) rebuild -- begin is not a counter.
        b.finish_rebuild().expect("finish");
        assert!(
            !b.ledger_snapshot().expect("snap").rebuild_in_progress,
            "one finish clears the rebuild flag set by two begins"
        );
        assert!(b.commit(&t).is_ok(), "writes resume after the single finish");

        // Finishing again while NOT rebuilding is a no-op: Ok, flag stays false.
        b.finish_rebuild().expect("idle finish is a no-op, not an error");
        assert!(
            !b.ledger_snapshot().expect("snap").rebuild_in_progress,
            "idle finish leaves the flag false"
        );
    }

    // scenario C (IMPL): the ProcessMemoryBroker::effective_report trait impl
    // must return the SAME real report as the inherent method on the healthy
    // path, and must NOT fabricate a fictitious Production budget. We assert the
    // two are byte-for-byte equal and that the report is the one we built from
    // (a 1M global limit), proving the fabricated 4GiB Production fallback is
    // gone.
    #[test]
    fn trait_effective_report_matches_inherent_and_is_not_fabricated() {
        use crate::api::broker::ProcessMemoryBroker;
        let b = broker();
        let inherent = b.effective_report().expect("inherent report");
        let via_trait = <LiveProcessMemoryBroker as ProcessMemoryBroker>::effective_report(&b);
        assert_eq!(
            inherent, via_trait,
            "trait impl must mirror the inherent report exactly on the healthy path"
        );
        // The real report carries the 1M limit we built from, NOT a fabricated
        // 4 GiB Production budget.
        assert_eq!(via_trait.residency.global_hard_limit_bytes, 1_000_000);
        assert_ne!(
            via_trait.residency.global_hard_limit_bytes,
            4 * 1024 * 1024 * 1024,
            "must not return the old fabricated Production fallback"
        );
        assert!(
            !via_trait.override_sources.iter().any(|s| s == "poisoned-fallback"),
            "healthy report must not carry the fabricated poisoned-fallback marker"
        );
    }

    // --- D: resource-group admission gates the reserve, fail-closed ---
    #[test]
    fn reserve_gated_by_resource_group_admission_fail_closed() {
        use crate::domain::policy::ResourceGroup;
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50_000,
            hard_cap_bytes: 100_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        // Roomy global so ONLY the group admission can deny; owner mapped to "g".
        let residency = ResidencyPolicy::new(10_000_000)
            .with_resource_group("g", group)
            .with_owner_group("owner:g", "g");
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");

        // Within the group hard cap: admitted.
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80_000,
            )
            .expect("within group cap");
        b.commit(&t).expect("commit");

        // Over the group cap (80k+40k=120k > 100k, no burst pool) -> denied.
        let denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            40_000,
        );
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "group admission denial must be a typed capacity error, got {denied:?}"
        );
        // Fail-closed: the denied reserve did not move the ledger.
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 80_000);

        // Releasing returns the group's burst capacity so a later reserve fits.
        b.release(&t).expect("release");
        assert!(
            b.reserve(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                90_000,
            )
            .is_ok(),
            "released group capacity is re-admissible"
        );
    }

    #[test]
    fn owner_without_group_mapping_is_not_gated_by_admission() {
        use crate::domain::policy::ResourceGroup;
        let tiny = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 1,
            hard_cap_bytes: 1,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        // The group exists but "owner:free" is NOT mapped to it.
        let residency = ResidencyPolicy::new(10_000_000).with_resource_group("g", tiny);
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
        assert!(
            b.reserve(
                MemoryClass::ArtifactCache,
                "owner:free".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                5_000_000,
            )
            .is_ok(),
            "an unmapped owner is gated only by class/owner/global caps, not by any group"
        );
    }

    // --- ExecutionBudget: the memory-relevant scratch cap is enforced ---
    #[test]
    fn execution_scratch_budget_caps_live_scratch_and_spares_other_kinds() {
        let report = EffectiveMemoryBudgetReport {
            execution: crate::domain::policy::ExecutionBudget {
                max_execution_scratch_bytes: Some(100_000),
                ..crate::domain::policy::ExecutionBudget::unlimited()
            },
            ..report_with(ResidencyPolicy::new(10_000_000))
        };
        let b = LiveProcessMemoryBroker::from_report(report).expect("valid");

        let t = b
            .reserve(
                MemoryClass::ExecutionScratch,
                "o".to_string(),
                MemoryContextKind::Request,
                ChargeKind::LiveScratch,
                80_000,
            )
            .expect("within scratch budget");
        b.commit(&t).expect("commit");

        let denied = b.reserve(
            MemoryClass::ExecutionScratch,
            "o".to_string(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            40_000,
        );
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "live-scratch over the execution scratch budget must be denied, got {denied:?}"
        );
        assert_eq!(
            b.ledger_snapshot().expect("snap").total_live_scratch_bytes(),
            80_000,
            "denied scratch reserve is fail-closed"
        );

        // A non-scratch (payload) charge is NOT gated by the scratch budget.
        assert!(
            b.reserve(
                MemoryClass::ArtifactCache,
                "o".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                5_000_000,
            )
            .is_ok(),
            "the execution scratch budget bounds only live scratch, not retained payload"
        );
    }

    #[test]
    fn execution_scratch_budget_accepts_exact_cap_and_denies_one_more() {
        let report = EffectiveMemoryBudgetReport {
            execution: crate::domain::policy::ExecutionBudget {
                max_execution_scratch_bytes: Some(100_000),
                ..crate::domain::policy::ExecutionBudget::unlimited()
            },
            ..report_with(ResidencyPolicy::new(10_000_000))
        };
        let broker = LiveProcessMemoryBroker::from_report(report).expect("valid");
        let at_cap = broker
            .reserve(
                MemoryClass::ExecutionScratch,
                "boundary-owner".to_string(),
                MemoryContextKind::Request,
                ChargeKind::LiveScratch,
                100_000,
            )
            .expect("the exact configured cap is admissible");
        broker.commit(&at_cap).expect("commit at cap");

        let denied = broker.reserve(
            MemoryClass::ExecutionScratch,
            "boundary-owner".to_string(),
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            1,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        assert_eq!(
            broker.ledger_snapshot().expect("snapshot").total_live_scratch_bytes(),
            100_000,
            "the over-cap attempt leaves semantic accounting unchanged"
        );
    }

    fn runtime_scratch_broker(cap: usize) -> LiveProcessMemoryBroker {
        let report = EffectiveMemoryBudgetReport {
            execution: crate::domain::policy::ExecutionBudget {
                max_execution_scratch_bytes: Some(cap),
                ..crate::domain::policy::ExecutionBudget::unlimited()
            },
            ..report_with(ResidencyPolicy::new(10_000_000))
        };
        LiveProcessMemoryBroker::from_report(report).expect("valid scratch broker")
    }

    #[test]
    fn live_runtime_scratch_charge_appears_in_ledger_snapshot() {
        let b = runtime_scratch_broker(100);
        let runtime = b
            .attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime)
            .expect("attach runtime");
        let context = runtime.context(MemoryContextKind::Request, "owner:scratch".to_string());
        let mut reservation = runtime
            .scratch()
            .reserve_scratch_scope(&context, ScratchRequest::new("working-set", 60))
            .expect("scratch reserve");

        let snapshot = b.ledger_snapshot().expect("snapshot");
        assert_eq!(snapshot.total_live_scratch_bytes(), 60);
        assert_eq!(snapshot.total_owner_bytes(), 60);
        assert_eq!(
            snapshot.class_usage.get(&MemoryClass::ExecutionScratch).map(|usage| usage.total()),
            Some(60)
        );
        assert_eq!(snapshot.owner_usage.get("owner:scratch").map(|usage| usage.total()), Some(60));
        assert_eq!(
            snapshot.context_usage.get(&MemoryContextKind::Request).map(|usage| usage.total()),
            Some(60)
        );
        assert!(reservation.is_enforced());

        reservation.release().expect("explicit release");
        assert_eq!(b.ledger_snapshot().expect("after release").total_owner_bytes(), 0);
    }

    #[test]
    fn live_runtime_scratch_over_cap_is_denied_without_mutation() {
        let b = runtime_scratch_broker(100);
        let runtime = b
            .attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime)
            .expect("attach runtime");
        let context = runtime.context(MemoryContextKind::Request, "owner:scratch".to_string());
        let held = runtime
            .scratch()
            .reserve_scratch_scope(&context, ScratchRequest::new("held", 80))
            .expect("initial reserve");
        let before = b.ledger_snapshot().expect("before denial");

        let denied =
            runtime.scratch().reserve_scratch_scope(&context, ScratchRequest::new("denied", 21));
        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        assert_eq!(b.ledger_snapshot().expect("after denial"), before);

        drop(held);
        assert_eq!(b.ledger_snapshot().expect("after drop").total_owner_bytes(), 0);
    }

    #[test]
    fn runtime_scratch_and_direct_broker_charge_share_one_limit() {
        let b = runtime_scratch_broker(100);
        let direct = b
            .reserve_committed(
                MemoryClass::ExecutionScratch,
                "owner:direct".to_string(),
                MemoryContextKind::Request,
                ChargeKind::LiveScratch,
                60,
            )
            .expect("direct scratch charge");
        let runtime = b
            .attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime)
            .expect("attach runtime");
        let context = runtime.context(MemoryContextKind::Request, "owner:runtime".to_string());

        let denied = runtime
            .scratch()
            .reserve_scratch_scope(&context, ScratchRequest::new("shared-limit", 41));
        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        assert_eq!(b.ledger_snapshot().expect("snapshot").total_live_scratch_bytes(), 60);
        b.release(&direct).expect("release direct charge");
    }

    #[test]
    fn dropping_runtime_scratch_during_rebuild_returns_capacity() {
        let b = runtime_scratch_broker(100);
        let runtime = b
            .attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime)
            .expect("attach runtime");
        let context = runtime.context(MemoryContextKind::Request, "owner:scratch".to_string());
        let reservation = runtime
            .scratch()
            .reserve_scratch_scope(&context, ScratchRequest::new("temporary", 100))
            .expect("fill capacity");
        b.begin_rebuild().expect("begin rebuild");

        drop(reservation);

        let snapshot = b.ledger_snapshot().expect("snapshot");
        assert!(snapshot.rebuild_in_progress);
        assert_eq!(snapshot.total_owner_bytes(), 0);
        b.finish_rebuild().expect("finish rebuild");
    }

    #[test]
    fn runtime_scratch_rejects_foreign_runtime_context_without_mutation() {
        let b = runtime_scratch_broker(100);
        let runtime = b
            .attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime)
            .expect("attach runtime");
        let foreign = MemoryContextHandle::new(
            "runtime-b".to_string(),
            MemoryContextKind::Request,
            "owner:scratch".to_string(),
        );
        let before = b.ledger_snapshot().expect("before");

        let denied =
            runtime.scratch().reserve_scratch_scope(&foreign, ScratchRequest::new("foreign", 1));
        assert!(matches!(denied, Err(MemoryError::InvalidState(_))));
        assert_eq!(b.ledger_snapshot().expect("after"), before);
    }

    #[test]
    fn runtime_scratch_release_restores_resource_group_capacity() {
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 100,
            hard_cap_bytes: 100,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let residency = ResidencyPolicy::new(10_000)
            .with_class_budget(
                MemoryClass::ExecutionScratch,
                class_budget(0, 10_000, 10_000, false),
            )
            .with_resource_group("runtime-group", group)
            .with_owner_group("owner:scratch", "runtime-group");
        let report = EffectiveMemoryBudgetReport {
            execution: crate::domain::policy::ExecutionBudget {
                max_execution_scratch_bytes: Some(10_000),
                ..crate::domain::policy::ExecutionBudget::unlimited()
            },
            ..report_with(residency)
        };
        let b = LiveProcessMemoryBroker::from_report(report).expect("broker");
        let runtime = b
            .attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime)
            .expect("attach runtime");
        let context = runtime.context(MemoryContextKind::Request, "owner:scratch".to_string());
        let mut reservation = runtime
            .scratch()
            .reserve_scratch_scope(&context, ScratchRequest::new("fills-group", 100))
            .expect("fill group capacity");
        assert!(!b.admits_group("runtime-group", 1).expect("group is full"));

        reservation.release().expect("explicit scratch release");

        assert!(b.admits_group("runtime-group", 100).expect("group capacity restored"));
        assert_eq!(b.ledger_snapshot().expect("after release").total_owner_bytes(), 0);
    }

    // A recharge whose NEW live-scratch charge exceeds the execution scratch budget
    // (not a class/owner cap) must also be denied AND restore the old scratch charge
    // — proving the restore path is exercised for the scratch-budget denial source,
    // not just cap denials.
    #[test]
    fn recharge_denied_by_scratch_budget_preserves_old_scratch_charge() {
        let report = EffectiveMemoryBudgetReport {
            execution: crate::domain::policy::ExecutionBudget {
                max_execution_scratch_bytes: Some(100_000),
                ..crate::domain::policy::ExecutionBudget::unlimited()
            },
            ..report_with(ResidencyPolicy::new(10_000_000))
        };
        let b = LiveProcessMemoryBroker::from_report(report).expect("valid");
        let old = b
            .reserve_committed(
                MemoryClass::ExecutionScratch,
                "o".to_string(),
                MemoryContextKind::Request,
                ChargeKind::LiveScratch,
                80_000,
            )
            .expect("seed 80k scratch (within 100k budget)");

        // Recharge to 120k > 100k scratch budget -> denied without consuming old.
        let err = b
            .recharge(
                &old,
                MemoryClass::ExecutionScratch,
                "o".to_string(),
                MemoryContextKind::Request,
                ChargeKind::LiveScratch,
                120_000,
            )
            .expect_err("over scratch budget must be denied");
        assert!(matches!(err.error, MemoryError::Capacity(_)));
        assert_eq!(err.recovery, RechargeRecoveryV1::OldChargePreserved);
        let snap = b.ledger_snapshot().expect("snap");
        assert_eq!(snap.total_live_scratch_bytes(), 80_000, "old scratch charge restored exactly");
        assert!(snap.class_owner_balanced() && snap.owner_partition_balanced(), "axes balanced");
        b.release(&old).expect("release preserved old ticket");
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
    }

    // --- J: concurrency (broker is Arc<Mutex>, Clone) ---
    #[test]
    fn concurrent_reserves_never_exceed_global_limit_no_toctou() {
        let b = LiveProcessMemoryBroker::from_report(report_with(ResidencyPolicy::new(1_000_000)))
            .expect("valid");
        let mut handles = Vec::new();
        for i in 0..16 {
            let b = b.clone();
            handles.push(std::thread::spawn(move || {
                let mut admitted = 0u64;
                for _ in 0..100 {
                    if let Ok(t) = b.reserve(
                        MemoryClass::ArtifactCache,
                        format!("o{i}"),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        1_000,
                    ) {
                        let _ = b.commit(&t);
                        admitted = admitted.saturating_add(1_000);
                    }
                }
                admitted
            }));
        }
        let total_admitted: u64 =
            handles.into_iter().map(|h| h.join().expect("thread panicked")).sum();
        let snap = b.ledger_snapshot().expect("snap");
        assert!(
            snap.total_owner_bytes() <= 1_000_000,
            "global limit must hold under concurrent reserves (no TOCTOU): {}",
            snap.total_owner_bytes()
        );
        assert_eq!(
            snap.total_owner_bytes(),
            total_admitted,
            "no torn updates: the ledger equals exactly the sum of admitted reserves"
        );
        assert!(snap.class_owner_balanced());
    }

    #[test]
    fn broker_clones_share_one_ledger() {
        let b1 = broker();
        let b2 = b1.clone();
        let t = b1
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("reserve via clone 1");
        b1.commit(&t).expect("commit");
        assert_eq!(
            b2.ledger_snapshot().expect("snap").total_owner_bytes(),
            100_000,
            "a clone observes the other clone's charge (shared ledger)"
        );
        b2.release(&t).expect("release via clone 2");
        assert_eq!(
            b1.ledger_snapshot().expect("snap").total_owner_bytes(),
            0,
            "release via one clone is seen by the other"
        );
    }

    #[test]
    fn broker_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LiveProcessMemoryBroker>();
    }

    // --- audit fix [2]: an accepted live reconfigure must NOT reset scheduler
    // group usage to zero (phantom-empty groups would re-open admission).
    #[test]
    fn accepted_reconfigure_preserves_scheduler_group_usage() {
        use crate::domain::policy::ResourceGroup;
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50_000,
            hard_cap_bytes: 100_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let residency = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 150_000, 300_000, false))
            .with_resource_group("g", group.clone())
            .with_owner_group("owner:g", "g");
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                90_000,
            )
            .expect("within group cap");
        b.commit(&t).expect("commit");

        // Legal raise that KEEPS the class, the group, and the owner mapping.
        let raised = ResidencyPolicy::new(20_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 400_000, false))
            .with_resource_group("g", group)
            .with_owner_group("owner:g", "g");
        b.try_live_reconfigure(raised).expect("raising caps is legal");

        // The group still holds 90k of live usage post-reconfigure, so a further
        // 20k (-> 110k > 100k cap) must still be denied. If the scheduler had been
        // reset to zero usage, this would wrongly be admitted.
        let denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            20_000,
        );
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "scheduler group usage must survive an accepted reconfigure, got {denied:?}"
        );
    }

    #[test]
    fn remapping_live_owner_group_is_denied_fail_closed() {
        use crate::domain::policy::ResourceGroup;

        let g1 = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50_000,
            hard_cap_bytes: 100_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let g2 = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50_000,
            hard_cap_bytes: 100_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let residency = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 150_000, 300_000, false))
            .with_resource_group("g1", g1.clone())
            .with_resource_group("g2", g2.clone())
            .with_owner_group("owner:g", "g1");
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
        let ticket = b
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                90_000,
            )
            .expect("baseline charge");

        let remapped = ResidencyPolicy::new(20_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 400_000, false))
            .with_resource_group("g1", g1)
            .with_resource_group("g2", g2)
            .with_owner_group("owner:g", "g2");
        let denied = b.try_live_reconfigure(remapped);
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "live owner-group remap must fail closed while bytes are still live, got {denied:?}"
        );

        let still_denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:g".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            20_000,
        );
        assert!(
            matches!(still_denied, Err(MemoryError::Capacity(_))),
            "failed remap must leave original scheduler truth intact, got {still_denied:?}"
        );

        b.release(&ticket).expect("release baseline");
        let remapped_after_drain = ResidencyPolicy::new(20_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 400_000, false))
            .with_resource_group(
                "g1",
                ResourceGroup {
                    reserved_bytes: 0,
                    soft_cap_bytes: 50_000,
                    hard_cap_bytes: 100_000,
                    burst_percent: 0,
                    reclaim_priority: ReclaimPriority::Normal,
                    admission_weight: 1,
                },
            )
            .with_resource_group(
                "g2",
                ResourceGroup {
                    reserved_bytes: 0,
                    soft_cap_bytes: 50_000,
                    hard_cap_bytes: 100_000,
                    burst_percent: 0,
                    reclaim_priority: ReclaimPriority::Normal,
                    admission_weight: 1,
                },
            )
            .with_owner_group("owner:g", "g2");
        b.try_live_reconfigure(remapped_after_drain)
            .expect("remap is legal after live bytes drain");
    }

    // --- audit fix [0]: the atomic charge primitive fails closed during rebuild
    // (no inter-lock window can strand reserved bytes behind a discarded ticket).
    #[test]
    fn reserve_committed_during_rebuild_is_fail_closed_no_strand() {
        let b = broker();
        b.begin_rebuild().expect("begin rebuild");
        let before = b.ledger_snapshot().expect("snap").total_owner_bytes();
        let denied = b.reserve_committed(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            100_000,
        );
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "reserve_committed during rebuild must be rejected, got {denied:?}"
        );
        assert_eq!(
            b.ledger_snapshot().expect("snap").total_owner_bytes(),
            before,
            "a rejected atomic charge must strand nothing (ledger unchanged)"
        );
    }

    // reserve_committed on the happy path charges exactly once (reserve+commit).
    #[test]
    fn reserve_committed_charges_exactly_once() {
        let b = broker();
        let t = b
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                120_000,
            )
            .expect("atomic charge within caps");
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 120_000);
        // The returned ticket is a normal committed charge: releasing it nets zero.
        b.release(&t).expect("release");
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
    }

    // --- audit fix [3]/[8]/[28]: atomic recharge (in-place overwrite) ---
    #[test]
    fn recharge_swaps_committed_charge_atomically() {
        let b = broker();
        let old = b
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("charge old");
        let new = b
            .recharge(
                &old,
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                50_000,
            )
            .expect("recharge to smaller value");
        // Exactly the new charge remains; the old is consumed (released).
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 50_000);
        assert!(
            matches!(b.release(&old), Err(MemoryError::InvalidState(_))),
            "the old ticket is consumed by a successful recharge"
        );
        b.release(&new).expect("release new");
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
    }

    #[test]
    fn recharge_denied_preserves_original_old_ticket() {
        let b = broker(); // owner:a hard cap 500k
        let old = b
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                400_000,
            )
            .expect("charge old");
        // Recharge to 600k > owner cap 500k -> denied with no old-state mutation.
        let err = b
            .recharge(
                &old,
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                600_000,
            )
            .expect_err("over owner cap must be denied");
        assert!(matches!(err.error, MemoryError::Capacity(_)));
        assert_eq!(err.recovery, RechargeRecoveryV1::OldChargePreserved);
        // The old bytes are still charged (no loss) AND every accounting axis is
        // balanced — a wrong-axis restore (e.g. restoring a different ChargeKind)
        // would pass the owner-total check but break the partition/class balance.
        let snap = b.ledger_snapshot().expect("snap");
        assert_eq!(snap.total_owner_bytes(), 400_000);
        assert!(snap.class_owner_balanced(), "class==owner after restore");
        assert!(snap.owner_partition_balanced(), "partition intact after restore");
        assert_eq!(
            snap.total_retained_payload_bytes(),
            400_000,
            "restored to the SAME charge kind"
        );
        b.release(&old).expect("preserved ticket releases the old bytes");
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
    }

    // A DENIED recharge whose NEW charge kind differs from the OLD must restore on
    // the OLD axis (kind), not the new one. A wrong-axis restore would keep owner
    // totals balanced (so the existing test passes) yet move bytes to the wrong
    // partition — this pins the partition, not just the grand total.
    #[test]
    fn denied_recharge_preserves_the_old_axis_not_the_new_one() {
        let b = broker(); // owner:a hard cap 500k
        let old = b
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload, // OLD kind = payload
                400_000,
            )
            .expect("charge old as payload");

        // Recharge to RetainedMetadata 600k > owner cap 500k -> denied; restore must
        // land back on RetainedPayload (the OLD kind), leaving metadata at zero.
        let err = b
            .recharge(
                &old,
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedMetadata, // NEW kind differs
                600_000,
            )
            .expect_err("over owner cap denied");
        assert!(matches!(err.error, MemoryError::Capacity(_)));
        assert_eq!(err.recovery, RechargeRecoveryV1::OldChargePreserved);
        let snap = b.ledger_snapshot().expect("snap");
        assert_eq!(
            snap.total_retained_payload_bytes(),
            400_000,
            "restore landed on the OLD payload axis"
        );
        assert_eq!(
            snap.total_retained_metadata_bytes(),
            0,
            "restore must NOT move bytes to the new (metadata) axis"
        );
        assert!(
            snap.owner_partition_balanced(),
            "partition balanced after wrong-axis-safe restore"
        );
        b.release(&old).expect("release preserved ticket");
    }

    #[test]
    fn recharge_during_rebuild_leaves_old_intact() {
        let b = broker();
        let old = b
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("charge old");
        b.begin_rebuild().expect("begin rebuild");
        let err = b
            .recharge(
                &old,
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                50_000,
            )
            .expect_err("recharge during rebuild is rejected");
        assert!(matches!(err.error, MemoryError::InvalidState(_)));
        assert_eq!(err.recovery, RechargeRecoveryV1::OldChargePreserved);
        assert_eq!(
            b.ledger_snapshot().expect("snap").total_owner_bytes(),
            100_000,
            "rebuild recharge strands nothing and leaves the old charge intact"
        );
        // After rebuild the ORIGINAL ticket is still valid.
        b.finish_rebuild().expect("finish");
        b.release(&old).expect("original ticket still valid after rebuild");
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
    }

    #[test]
    fn recharge_does_not_transiently_double_count_group_admission() {
        use crate::domain::policy::ResourceGroup;
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50_000,
            hard_cap_bytes: 100_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let residency = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 150_000, 300_000, false))
            .with_resource_group("g", group)
            .with_owner_group("owner:g", "g");
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");

        let old = b
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                90_000,
            )
            .expect("group usage 90k");

        // Overwrite to exactly the group cap (100k). A naive charge-new-then-refund
        // would transiently see 90k+100k=190k > 100k group cap and be DENIED; the
        // atomic recharge (release-then-charge under one lock) admits it.
        let new = b
            .recharge(
                &old,
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("atomic recharge avoids transient double-count against the group");
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 100_000);
        // The group is now exactly at cap: a further 1-byte reserve is denied.
        assert!(matches!(
            b.reserve(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1,
            ),
            Err(MemoryError::Capacity(_))
        ));
        b.release(&new).expect("release");
    }

    // A DENIED recharge on a grouped owner must preserve the old charge and keep
    // scheduler group usage unchanged — no release/restore drift.
    #[test]
    fn denied_recharge_on_grouped_owner_preserves_group_usage() {
        use crate::domain::policy::ResourceGroup;
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50_000,
            hard_cap_bytes: 100_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        // owner:g mapped to group g (cap 100k); class cap 80k so the NEW charge is
        // denied by the class while the group has room.
        let residency = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 60_000, 80_000, false))
            .with_resource_group("g", group)
            .with_owner_group("owner:g", "g");
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");

        let old = b
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                70_000,
            )
            .expect("seed 70k (group usage 70k)");

        // Recharge to 90k > class cap 80k -> denied; old 70k remains live.
        let err = b
            .recharge(
                &old,
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                90_000,
            )
            .expect_err("over class cap denied");
        assert!(matches!(err.error, MemoryError::Capacity(_)));
        assert_eq!(err.recovery, RechargeRecoveryV1::OldChargePreserved);
        assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 70_000);

        // Group usage is back at exactly 70k: it admits 30k more (-> 100k cap) but
        // not 31k. Symmetric — no phantom group drift from the release/restore.
        assert!(
            b.admits_group("g", 30_000).expect("admits"),
            "group should be at 70k, admitting 30k up to the 100k cap"
        );
        assert!(
            !b.admits_group("g", 31_000).expect("admits"),
            "group must be exactly at 70k usage (no phantom drift), so 31k overflows"
        );
        b.release(&old).expect("release preserved ticket");
    }

    #[test]
    fn recharge_cannot_consume_peer_group_reserved_floor() {
        use crate::domain::policy::ResourceGroup;
        let protected = ResourceGroup {
            reserved_bytes: 400,
            soft_cap_bytes: 500,
            hard_cap_bytes: 600,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::First,
            admission_weight: 1,
        };
        let peer = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 700,
            hard_cap_bytes: 1_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Last,
            admission_weight: 1,
        };
        let policy = ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 1_000, 1_000, false))
            .with_resource_group("protected", protected)
            .with_resource_group("peer", peer)
            .with_owner_group("protected-owner", "protected")
            .with_owner_group("peer-owner", "peer");
        let broker = LiveProcessMemoryBroker::from_report(report_with(policy)).expect("broker");
        let old = broker
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "peer-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                500,
            )
            .expect("seed peer usage inside unprotected band");

        let denied = broker
            .recharge(
                &old,
                MemoryClass::ArtifactCache,
                "peer-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                601,
            )
            .expect_err("recharge must preserve the other group's 400-byte floor");
        assert!(matches!(denied.error, MemoryError::Capacity(_)));
        assert_eq!(denied.recovery, RechargeRecoveryV1::OldChargePreserved);
        assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 500);

        let replacement = broker
            .recharge(
                &old,
                MemoryClass::ArtifactCache,
                "peer-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                600,
            )
            .expect("replacement exactly at the unprotected boundary is legal");
        assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 600);
        broker.release(&replacement).expect("release replacement");
    }

    fn poison_broker_lock(broker: &LiveProcessMemoryBroker) {
        let clone = broker.clone();
        let panicked = std::thread::spawn(move || {
            let _guard = clone.inner.lock().expect("lock");
            panic!("poison broker lock for test");
        })
        .join();
        assert!(panicked.is_err(), "poison helper must panic while holding the lock");
    }

    #[test]
    fn managed_charge_drop_recovers_poisoned_lock_for_exact_release() {
        let b = broker();
        let charge = managed_charge(&b, 100_000);
        poison_broker_lock(&b);

        drop(charge);

        let core = b.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let snapshot = core.ledger.snapshot();
        assert_eq!(snapshot.total_owner_bytes(), 0);
        assert!(snapshot.class_owner_balanced() && snapshot.owner_partition_balanced());
        assert!(core.deferred_committed_releases.is_empty());
    }

    #[test]
    fn poisoned_runtime_scratch_explicit_release_preserves_then_drop_recovers_authority() {
        let b = runtime_scratch_broker(100);
        let runtime = b
            .attach_runtime("runtime-a".to_string(), RuntimeKind::Runtime)
            .expect("attach runtime");
        let context = runtime.context(MemoryContextKind::Request, "owner:scratch".to_string());
        let mut reservation = runtime
            .scratch()
            .reserve_scratch_scope(&context, ScratchRequest::new("poison-cleanup", 100))
            .expect("reserve scratch");
        poison_broker_lock(&b);

        let denied = reservation.release();
        assert!(matches!(denied, Err(MemoryError::Poisoned(_))));
        assert!(reservation.is_enforced(), "failed explicit release retains retry authority");

        drop(reservation);

        let core = b.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let snapshot = core.ledger.snapshot();
        assert_eq!(snapshot.total_owner_bytes(), 0);
        assert!(snapshot.class_owner_balanced() && snapshot.owner_partition_balanced());
    }

    #[test]
    fn poisoned_pending_commit_retains_ticket_until_drop_rollback() {
        let b = broker();
        let pending = b
            .reserve_owned(
                MemoryClass::ArtifactCache,
                "owner:pending-poison".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100_000,
            )
            .expect("pending reservation");
        poison_broker_lock(&b);

        let denied = pending.commit().expect_err("poisoned commit must fail closed");
        assert!(matches!(denied, MemoryError::Poisoned(_)));

        // `commit` consumes `pending`; its Drop path must already have recovered
        // the poisoned lock and rolled the exact pending ticket back.
        let core = b.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let snapshot = core.ledger.snapshot();
        assert_eq!(snapshot.total_owner_bytes(), 0);
        assert_eq!(snapshot.total_context_bytes(), 0);
        assert!(snapshot.class_owner_balanced());
        assert!(snapshot.owner_partition_balanced());
        assert!(snapshot.context_owner_balanced());
        assert!(core.ticket_groups.is_empty());
        assert!(core.ticket_owners.is_empty());
        assert!(core.deferred_pending_rollbacks.is_empty());
    }

    #[test]
    fn poisoned_broker_fails_closed_and_trait_report_stays_truthful() {
        use crate::api::broker::ProcessMemoryBroker;

        let b = broker();
        let ticket = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                10_000,
            )
            .expect("reserve before poison");
        let report_before = <LiveProcessMemoryBroker as ProcessMemoryBroker>::effective_report(&b);
        poison_broker_lock(&b);

        assert!(matches!(
            b.reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1,
            ),
            Err(MemoryError::Poisoned(_))
        ));
        assert!(matches!(b.commit(&ticket), Err(MemoryError::Poisoned(_))));
        assert!(matches!(b.release(&ticket), Err(MemoryError::Poisoned(_))));
        assert!(matches!(b.begin_rebuild(), Err(MemoryError::Poisoned(_))));
        assert!(matches!(b.finish_rebuild(), Err(MemoryError::Poisoned(_))));
        assert!(matches!(
            b.try_live_reconfigure(ResidencyPolicy::new(2_000_000)),
            Err(MemoryError::Poisoned(_))
        ));
        assert!(matches!(b.ledger_snapshot(), Err(MemoryError::Poisoned(_))));
        assert!(matches!(
            b.observe_allocator(&AllocatorSnapshot {
                resident_bytes: Some(123),
                ..AllocatorSnapshot::default()
            }),
            Err(MemoryError::Poisoned(_))
        ));
        assert!(matches!(b.effective_report(), Err(MemoryError::Poisoned(_))));
        assert!(matches!(b.observe(), Err(MemoryError::Poisoned(_))));

        let report_after = <LiveProcessMemoryBroker as ProcessMemoryBroker>::effective_report(&b);
        assert_eq!(report_after, report_before, "trait report must stay truthful under poison");
        assert_eq!(report_after.residency.global_hard_limit_bytes, 1_000_000);
        assert!(
            !report_after.override_sources.iter().any(|s| s == "poisoned-fallback"),
            "trait report must not fabricate a poison fallback marker"
        );
    }

    #[derive(Default)]
    struct RecordingEventSink {
        events: Mutex<Vec<MemoryEvent>>,
        broker: Mutex<Option<LiveProcessMemoryBroker>>,
    }

    impl MemoryEventSink for RecordingEventSink {
        fn emit(&self, event: MemoryEvent) {
            if let Some(broker) = self.broker.lock().expect("recorder broker lock").as_ref() {
                // This would deadlock if the broker invoked the sink while
                // holding its mutex.
                broker.effective_report().expect("event callback runs outside broker lock");
            }
            self.events.lock().expect("recorder events lock").push(event);
        }
    }

    #[test]
    fn bounded_events_execute_outside_lock_and_cover_major_broker_outcomes() {
        let sink = Arc::new(RecordingEventSink::default());
        let broker = crate::ProcessMemoryBrokerBuilder::new()
            .with_preset(crate::MemoryPreset::Development)
            .with_event_sink(sink.clone())
            .build()
            .expect("broker");
        *sink.broker.lock().expect("install broker") = Some(broker.clone());

        let mut charge = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "bounded-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                64,
            )
            .expect("reserve");
        charge.release().expect("release");
        let denied = broker.reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "bounded-owner".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            u64::MAX,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        let observation = broker.observe().expect("observation");
        assert_eq!(observation.semantic_bytes, 0);
        assert_eq!(observation.report.policy_revision, 1);

        let events = sink.events.lock().expect("events").clone();
        assert!(events.contains(&MemoryEvent::new(
            MemoryOperation::Reserve,
            MemoryOutcome::Succeeded,
            Some(64),
        )));
        assert!(events.contains(&MemoryEvent::new(
            MemoryOperation::Release,
            MemoryOutcome::Succeeded,
            Some(64),
        )));
        assert!(events.contains(&MemoryEvent::new(
            MemoryOperation::Reserve,
            MemoryOutcome::CapacityDenied,
            Some(u64::MAX),
        )));
        assert!(events.contains(&MemoryEvent::new(
            MemoryOperation::ObserveBroker,
            MemoryOutcome::Succeeded,
            Some(0),
        )));
    }

    struct PanickingEventSink;

    impl MemoryEventSink for PanickingEventSink {
        fn emit(&self, _event: MemoryEvent) {
            panic!("telemetry failure");
        }
    }

    #[test]
    fn telemetry_panic_cannot_change_or_abort_accounting() {
        let broker = crate::ProcessMemoryBrokerBuilder::new()
            .with_preset(crate::MemoryPreset::Development)
            .with_event_sink(Arc::new(PanickingEventSink))
            .build()
            .expect("broker");
        let charge = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "panic-sink-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                32,
            )
            .expect("telemetry panic is contained after accounting commit");
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 32);
        drop(charge);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    struct RecursiveObserveEventSink {
        broker: Mutex<Option<LiveProcessMemoryBroker>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl MemoryEventSink for RecursiveObserveEventSink {
        fn emit(&self, _event: MemoryEvent) {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let broker = self.broker.lock().expect("broker slot").clone();
            if let Some(broker) = broker {
                std::thread::spawn(move || {
                    broker.observe().expect("read observation remains available in event callback");
                })
                .join()
                .expect("nested event observation thread");
            }
        }
    }

    #[test]
    fn event_sink_observation_reentry_delivers_outer_event_once() {
        let sink = Arc::new(RecursiveObserveEventSink {
            broker: Mutex::new(None),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let broker = crate::ProcessMemoryBrokerBuilder::new()
            .with_preset(crate::MemoryPreset::Development)
            .with_event_sink(sink.clone())
            .build()
            .expect("broker");
        *sink.broker.lock().expect("broker slot") = Some(broker.clone());

        broker.record_advisory_event(MemoryEvent::new(
            MemoryOperation::ObserveAllocator,
            MemoryOutcome::Succeeded,
            None,
        ));

        assert_eq!(
            sink.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the nested observe event must be suppressed at the same sink boundary"
        );
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    }

    struct RecursiveReserveReclaimer {
        broker: Mutex<Option<LiveProcessMemoryBroker>>,
        calls: std::sync::atomic::AtomicUsize,
        nested_error: Mutex<Option<MemoryError>>,
    }

    impl MemoryReclaimer for RecursiveReserveReclaimer {
        fn reclaim(
            &self,
            _request: &ReclaimRequest,
        ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let broker = self.broker.lock().expect("broker slot").clone();
            if let Some(broker) = broker {
                let nested = broker.reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    "nested-reclaim".into(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    1,
                );
                *self.nested_error.lock().expect("nested error") = nested.err();
            }
            Ok(crate::ports::reclaim::ReclaimOutcome::default())
        }
    }

    #[test]
    fn same_broker_reclaimer_reserve_reentry_is_bounded_and_typed() {
        let reclaimer = Arc::new(RecursiveReserveReclaimer {
            broker: Mutex::new(None),
            calls: std::sync::atomic::AtomicUsize::new(0),
            nested_error: Mutex::new(None),
        });
        let policy = ResidencyPolicy::new(100)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        *reclaimer.broker.lock().expect("broker slot") = Some(broker.clone());
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "outer".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100,
            )
            .expect("fill hard cap");

        let denied = broker.reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "outer".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        assert_eq!(reclaimer.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(matches!(
            reclaimer.nested_error.lock().expect("nested error").as_ref(),
            Some(MemoryError::OperationInProgress(_))
        ));
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 100);
        drop(baseline);
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }

    struct ThreadSpawnReclaimer {
        broker: Mutex<Option<LiveProcessMemoryBroker>>,
        calls: std::sync::atomic::AtomicUsize,
        nested_error: Mutex<Option<MemoryError>>,
    }

    impl MemoryReclaimer for ThreadSpawnReclaimer {
        fn reclaim(
            &self,
            _request: &ReclaimRequest,
        ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let broker = self.broker.lock().expect("broker slot").clone();
            if let Some(broker) = broker {
                let nested = std::thread::spawn(move || {
                    broker.reserve_managed_charge(
                        MemoryClass::ArtifactCache,
                        "thread-reentry".into(),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        1,
                    )
                })
                .join()
                .expect("nested reserve thread");
                *self.nested_error.lock().expect("nested error") = nested.err();
            }
            Ok(crate::ports::reclaim::ReclaimOutcome::default())
        }
    }

    #[test]
    fn concurrent_reclaimer_reentry_does_not_start_a_callback_storm() {
        let reclaimer = Arc::new(ThreadSpawnReclaimer {
            broker: Mutex::new(None),
            calls: std::sync::atomic::AtomicUsize::new(0),
            nested_error: Mutex::new(None),
        });
        let policy = ResidencyPolicy::new(100)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        *reclaimer.broker.lock().expect("broker slot") = Some(broker.clone());
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "outer".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100,
            )
            .expect("fill hard cap");

        let denied = broker.reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "outer".into(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        assert_eq!(reclaimer.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(matches!(
            reclaimer.nested_error.lock().expect("nested error").as_ref(),
            Some(MemoryError::Capacity(_))
        ));
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 100);
        drop(baseline);
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }

    #[test]
    fn independent_outer_reserves_share_one_in_flight_reclaimer() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::mpsc;
        use std::time::Duration;

        struct HeldReclaimer {
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
            calls: AtomicUsize,
        }
        impl MemoryReclaimer for HeldReclaimer {
            fn reclaim(
                &self,
                _request: &ReclaimRequest,
            ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
                self.calls.fetch_add(1, AtomicOrdering::SeqCst);
                self.entered.send(()).expect("parent observes callback entry");
                self.release
                    .lock()
                    .expect("release channel")
                    .recv()
                    .expect("parent releases callback");
                Ok(crate::ports::reclaim::ReclaimOutcome::default())
            }
        }

        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let reclaimer = Arc::new(HeldReclaimer {
            entered: entered_tx,
            release: Mutex::new(release_rx),
            calls: AtomicUsize::new(0),
        });
        let policy = ResidencyPolicy::new(100)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:outer".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100,
            )
            .expect("fill hard cap");

        let first_broker = broker.clone();
        let first = std::thread::spawn(move || {
            first_broker.reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:first".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1,
            )
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).expect("first callback entered");
        let (second_tx, second_rx) = mpsc::channel();
        let second_broker = broker.clone();
        let second = std::thread::spawn(move || {
            let result = second_broker.reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:second".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1,
            );
            second_tx.send(result).expect("parent receives second outcome");
        });
        let second_while_first_held = second_rx.recv_timeout(Duration::from_secs(5));
        release_tx.send(()).expect("release first callback even on failed second outcome");
        let first_result = first.join().expect("first thread");
        second.join().expect("second thread");
        assert!(matches!(second_while_first_held, Ok(Err(MemoryError::Capacity(_)))));
        assert!(matches!(first_result, Err(MemoryError::Capacity(_))));
        assert_eq!(reclaimer.calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(broker.ledger_snapshot().expect("unchanged").total_owner_bytes(), 100);
        drop(baseline);
        sep23_assert_fully_drained(&broker);
    }

    #[test]
    fn forged_snapshot_has_fallible_totals_without_panicking() {
        let mut snapshot = LedgerSnapshot::default();
        snapshot.owner_usage.insert(
            "a".into(),
            crate::UsageBreakdown {
                retained_payload_bytes: u64::MAX,
                retained_metadata_bytes: 0,
                live_scratch_bytes: 0,
            },
        );
        snapshot.owner_usage.insert(
            "b".into(),
            crate::UsageBreakdown {
                retained_payload_bytes: 1,
                retained_metadata_bytes: 0,
                live_scratch_bytes: 0,
            },
        );
        assert!(matches!(snapshot.checked_total_owner_bytes(), Err(MemoryError::InvalidState(_))));
    }

    #[test]
    fn concurrent_reserve_and_reconfigure_preserve_group_cap_truth() {
        use crate::domain::policy::ResourceGroup;

        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50_000,
            hard_cap_bytes: 100_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let residency = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 150_000, 300_000, false))
            .with_resource_group("g", group.clone())
            .with_owner_group("owner:g", "g");
        let b = LiveProcessMemoryBroker::from_report(report_with(residency)).expect("valid");
        let baseline = b
            .reserve_committed(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                90_000,
            )
            .expect("baseline charge");

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let reconfigure_broker = b.clone();
        let reconfigure_barrier = barrier.clone();
        let reconfigure_thread = std::thread::spawn(move || {
            reconfigure_barrier.wait();
            for i in 0..128u64 {
                let raised = ResidencyPolicy::new(20_000_000 + i)
                    .with_class_budget(
                        MemoryClass::ArtifactCache,
                        class_budget(0, 200_000, 400_000, false),
                    )
                    .with_resource_group("g", group.clone())
                    .with_owner_group("owner:g", "g");
                reconfigure_broker.try_live_reconfigure(raised).expect("accepted raise");
            }
        });

        let reserve_broker = b.clone();
        let reserve_barrier = barrier.clone();
        let reserve_thread = std::thread::spawn(move || {
            reserve_barrier.wait();
            let mut denied = 0usize;
            let mut admitted = 0usize;
            for _ in 0..128 {
                match reserve_broker.reserve(
                    MemoryClass::ArtifactCache,
                    "owner:g".to_string(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    20_000,
                ) {
                    Err(MemoryError::Capacity(_)) => denied += 1,
                    Ok(ticket) => {
                        admitted += 1;
                        reserve_broker
                            .release(&ticket)
                            .expect("unexpected admission must be releasable");
                    }
                    Err(other) => {
                        panic!("unexpected reserve error during concurrent test: {other:?}")
                    }
                }
            }
            (denied, admitted)
        });

        barrier.wait();
        reconfigure_thread.join().expect("reconfigure thread");
        let (denied, admitted) = reserve_thread.join().expect("reserve thread");
        assert_eq!(admitted, 0, "group overflow reserve must never be admitted");
        assert_eq!(denied, 128, "every concurrent overflow reserve must be denied");
        let snap = b.ledger_snapshot().expect("snap");
        assert_eq!(snap.total_owner_bytes(), 90_000, "baseline usage must survive unchanged");
        assert!(snap.class_owner_balanced(), "ledger invariants must survive concurrency");
        b.release(&baseline).expect("release baseline");
    }

    #[derive(Default)]
    struct RecordingReclaimer {
        requests: Mutex<Vec<ReclaimRequest>>,
        charge_to_release: Mutex<Option<ManagedCharge>>,
        broker_for_reentry: Mutex<Option<LiveProcessMemoryBroker>>,
        reconfigure_inside: Mutex<Option<ResidencyPolicy>>,
        reconfigure_attempts: Mutex<Vec<MemoryResult<()>>>,
        forged_reported_bytes: Mutex<Option<u64>>,
    }

    impl MemoryReclaimer for RecordingReclaimer {
        fn reclaim(
            &self,
            request: &ReclaimRequest,
        ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
            self.requests.lock().expect("requests").push(request.clone());
            if let Some(broker) = self.broker_for_reentry.lock().expect("broker").as_ref() {
                broker.effective_report().expect("callback must run outside broker lock");
                if let Some(policy) = self.reconfigure_inside.lock().expect("policy").take() {
                    let outcome = broker.try_live_reconfigure(policy);
                    self.reconfigure_attempts.lock().expect("attempts").push(outcome);
                }
            }
            // A forged report lies about reclaimed bytes while freeing nothing:
            // telemetry records the lie, but admission must remeasure the
            // ledger and never trust it.
            if let Some(forged) = self.forged_reported_bytes.lock().expect("forged").take() {
                return Ok(crate::ports::reclaim::ReclaimOutcome { reclaimed_bytes: forged });
            }
            let mut reclaimed = 0;
            if let Some(mut charge) = self.charge_to_release.lock().expect("charge").take() {
                reclaimed = charge.bytes().expect("live charge");
                charge.release().expect("callback release");
            }
            Ok(crate::ports::reclaim::ReclaimOutcome { reclaimed_bytes: reclaimed })
        }
    }

    struct PanickingReclaimer {
        calls: Mutex<u32>,
    }

    impl MemoryReclaimer for PanickingReclaimer {
        fn reclaim(
            &self,
            _request: &ReclaimRequest,
        ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
            *self.calls.lock().expect("calls") += 1;
            panic!("reclaimer panic");
        }
    }

    #[test]
    fn soft_cap_crossing_triggers_reclaim_without_immediate_hard_deny() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let policy = ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 50, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        *reclaimer.broker_for_reentry.lock().expect("broker") = Some(broker.clone());

        let charge = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "soft-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                60,
            )
            .expect("soft crossing remains admissible below hard");
        let requests = reclaimer.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].trigger, ReclaimTrigger::SoftThreshold);
        assert_eq!(requests[0].max_attempts, 1);
        drop(requests);
        assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 1);
        drop(charge);
    }

    #[test]
    fn owner_and_group_soft_caps_each_trigger_the_same_reclaim_lane() {
        let owner_reclaimer = Arc::new(RecordingReclaimer::default());
        let owner_policy = ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false))
            .with_owner_quota("soft-owner", OwnerQuota { soft_cap_bytes: 10, hard_cap_bytes: 100 });
        let owner_broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(owner_policy),
            Some(owner_reclaimer.clone()),
        )
        .expect("owner broker");
        let owner_charge = owner_broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "soft-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                11,
            )
            .expect("owner soft crossing");
        assert_eq!(owner_reclaimer.requests.lock().expect("requests").len(), 1);

        let group_reclaimer = Arc::new(RecordingReclaimer::default());
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 10,
            hard_cap_bytes: 100,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let group_policy = ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 100, 100, false))
            .with_resource_group("soft-group", group)
            .with_owner_group("group-owner", "soft-group");
        let group_broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(group_policy),
            Some(group_reclaimer.clone()),
        )
        .expect("group broker");
        let group_charge = group_broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "group-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                11,
            )
            .expect("group soft crossing");
        let group_requests = group_reclaimer.requests.lock().expect("requests");
        assert_eq!(group_requests.len(), 1);
        assert_eq!(group_requests[0].resource_group.as_deref(), Some("soft-group"));
        drop(group_requests);
        drop(owner_charge);
        drop(group_charge);
    }

    #[test]
    fn hard_cap_denial_reclaims_outside_lock_remeasures_and_retries_once() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let policy = ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 50, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        *reclaimer.broker_for_reentry.lock().expect("broker") = Some(broker.clone());

        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "hard-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline");
        *reclaimer.charge_to_release.lock().expect("charge") = Some(baseline);
        let replacement = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "hard-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                30,
            )
            .expect("retry sees callback release and admits");

        let requests = reclaimer.requests.lock().expect("requests");
        assert_eq!(requests.last().expect("hard request").trigger, ReclaimTrigger::CapacityDenial);
        assert_eq!(requests.last().expect("hard request").max_attempts, 1);
        drop(requests);
        let stats = broker.reclaim_runtime_stats().expect("stats");
        assert_eq!(stats.attempts, 2, "baseline soft signal plus one hard retry");
        assert_eq!(stats.reported_reclaimed_bytes, 80);
        assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 30);
        drop(replacement);
    }

    #[test]
    fn group_reserved_floor_is_unavailable_to_peer_group() {
        let protected = ResourceGroup {
            reserved_bytes: 400,
            soft_cap_bytes: 500,
            hard_cap_bytes: 600,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::First,
            admission_weight: 1,
        };
        let peer = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 700,
            hard_cap_bytes: 1_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Last,
            admission_weight: 1,
        };
        let policy = ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 1_000, 1_000, false))
            .with_resource_group("protected", protected)
            .with_resource_group("peer", peer)
            .with_owner_group("protected-owner", "protected")
            .with_owner_group("peer-owner", "peer");
        let broker = LiveProcessMemoryBroker::from_report(report_with(policy)).expect("broker");

        assert!(matches!(
            broker.reserve(
                MemoryClass::ArtifactCache,
                "peer-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                601,
            ),
            Err(MemoryError::Capacity(_))
        ));
        let peer_ticket = broker
            .reserve(
                MemoryClass::ArtifactCache,
                "peer-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                600,
            )
            .expect("peer may use unprotected band");
        let protected_ticket = broker
            .reserve(
                MemoryClass::ArtifactCache,
                "protected-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                400,
            )
            .expect("protected group consumes its floor");
        broker.release(&peer_ticket).expect("release peer");
        broker.release(&protected_ticket).expect("release protected");
    }

    #[test]
    fn accepted_reconfigure_updates_revision_source_and_matches_headroom_predicate() {
        let mut policy = ResidencyPolicy::new(1_000).with_headroom_bytes(200);
        policy.classes.insert(MemoryClass::Headroom, class_budget(0, 200, 200, false));
        policy.classes.insert(MemoryClass::ArtifactCache, class_budget(0, 800, 800, false));
        let broker =
            LiveProcessMemoryBroker::from_report(report_with(policy.clone())).expect("broker");
        let headroom = broker
            .reserve(
                MemoryClass::Headroom,
                "headroom".into(),
                MemoryContextKind::Request,
                ChargeKind::LiveScratch,
                200,
            )
            .expect("headroom");
        let work = broker
            .reserve(
                MemoryClass::ArtifactCache,
                "work".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                800,
            )
            .expect("separate non-headroom band");

        broker.try_live_reconfigure(policy).expect("identity policy remains legal");
        let report = broker.effective_report().expect("report");
        assert_eq!(report.policy_revision, 2);
        assert_eq!(report.active_policy_source, "runtime_reconfigure");
        assert_eq!(
            report.override_provenance.last(),
            Some(&OverrideRecord::new(OverrideSource::RuntimeReconfigure, "residency.active"))
        );
        assert_eq!(report.residency.headroom_bytes, 200);
        broker.release(&headroom).expect("release headroom");
        broker.release(&work).expect("release work");
    }

    #[test]
    fn identity_reconfigure_succeeds_during_legal_class_borrow() {
        let policy = ResidencyPolicy::new(1_000)
            .with_shared_burst_pool_bytes(100)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 50, 100, true));
        let broker =
            LiveProcessMemoryBroker::from_report(report_with(policy.clone())).expect("broker");
        let ticket = broker
            .reserve(
                MemoryClass::ArtifactCache,
                "borrower".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                150,
            )
            .expect("legal 50-byte class borrow");
        broker.try_live_reconfigure(policy).expect("identity reconfigure preserves legal borrow");
        broker.release(&ticket).expect("release");
    }

    #[test]
    fn identity_reconfigure_succeeds_during_legal_group_burst() {
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50,
            hard_cap_bytes: 100,
            burst_percent: 50,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let policy = ResidencyPolicy::new(1_000)
            .with_shared_burst_pool_bytes(100)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 300, 500, false))
            .with_resource_group("burstable", group)
            .with_owner_group("group-owner", "burstable");
        let broker =
            LiveProcessMemoryBroker::from_report(report_with(policy.clone())).expect("broker");
        let ticket = broker
            .reserve(
                MemoryClass::ArtifactCache,
                "group-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                150,
            )
            .expect("legal 50-byte group burst");
        broker.try_live_reconfigure(policy).expect("identity reconfigure preserves legal burst");
        broker.release(&ticket).expect("release");
    }

    #[test]
    fn reclaim_priority_orders_victims_deterministically() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let mut last = class_budget(0, 10, 100, false);
        last.reclaim_priority = ReclaimPriority::Last;
        let mut normal = class_budget(0, 100, 100, false);
        normal.reclaim_priority = ReclaimPriority::Normal;
        let mut first = class_budget(0, 100, 100, false);
        first.reclaim_priority = ReclaimPriority::First;
        let group_with_priority = |priority| ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50,
            hard_cap_bytes: 100,
            burst_percent: 0,
            reclaim_priority: priority,
            admission_weight: 1,
        };
        let policy = ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, last)
            .with_class_budget(MemoryClass::EvidenceCache, normal)
            .with_class_budget(MemoryClass::TraceIndex, first)
            .with_resource_group("last-group", group_with_priority(ReclaimPriority::Last))
            .with_resource_group("normal-group", group_with_priority(ReclaimPriority::Normal))
            .with_resource_group("first-group", group_with_priority(ReclaimPriority::First));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        let charge = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "priority-owner".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                11,
            )
            .expect("soft crossing");
        let requests = reclaimer.requests.lock().expect("requests");
        let candidates = &requests[0].candidates;
        let last_pos = candidates
            .iter()
            .position(|candidate| candidate.class == MemoryClass::ArtifactCache)
            .expect("last candidate");
        let normal_pos = candidates
            .iter()
            .position(|candidate| candidate.class == MemoryClass::EvidenceCache)
            .expect("normal candidate");
        let first_pos = candidates
            .iter()
            .position(|candidate| candidate.class == MemoryClass::TraceIndex)
            .expect("first candidate");
        assert!(last_pos < normal_pos && normal_pos < first_pos);
        let groups = &requests[0].group_candidates;
        assert_eq!(groups[0].group, "last-group");
        assert_eq!(groups[1].group, "normal-group");
        assert_eq!(groups[2].group, "first-group");
        drop(requests);
        drop(charge);
    }

    // --- FUP-007: zero-byte ticket scope lifetime ---------------------------
    //
    // A zero-byte ticket holds live release/rollback authority without any
    // positive usage entry. Live reconfigure must treat "zero usage" as
    // authority-unknown and consult the exact live-ticket owner index instead.

    fn sep23_grouped_residency() -> ResidencyPolicy {
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 50_000,
            hard_cap_bytes: 100_000,
            burst_percent: 100,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
            .with_resource_group("g", group)
            .with_owner_group("owner:g", "g")
    }

    fn sep23_ungrouped_residency() -> ResidencyPolicy {
        ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
    }

    fn sep23_grouped_broker() -> LiveProcessMemoryBroker {
        LiveProcessMemoryBroker::from_report(report_with(sep23_grouped_residency())).expect("valid")
    }

    fn sep23_zero_committed(broker: &LiveProcessMemoryBroker, owner: &str) -> ManagedCharge {
        broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                owner.to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                0,
            )
            .expect("zero-byte committed charge")
    }

    fn sep23_assert_fully_drained(broker: &LiveProcessMemoryBroker) {
        let snapshot = broker.ledger_snapshot().expect("snapshot");
        snapshot.validate_exact().expect("axes balance after drain");
        assert_eq!(snapshot.total_owner_bytes(), 0, "semantic counters restored");
        assert_eq!(snapshot.total_class_bytes(), 0);
        assert_eq!(snapshot.total_context_bytes(), 0);
        assert_eq!(snapshot.live_ticket_count, 0, "no live ticket remains");
        let core = broker.inner.lock().expect("broker lock");
        assert!(core.ticket_groups.is_empty(), "no issuing-group refs remain");
        assert!(core.ticket_owners.is_empty(), "no issuing-owner refs remain");
        assert!(core.deferred_committed_releases.is_empty(), "no deferred committed backlog");
        assert!(core.deferred_pending_rollbacks.is_empty(), "no deferred pending backlog");
    }

    #[test]
    fn sep23_fup007_zero_byte_committed_blocks_group_removal_until_explicit_release() {
        let b = sep23_grouped_broker();
        let mut charge = sep23_zero_committed(&b, "owner:g");
        assert_eq!(b.ledger_snapshot().expect("snapshot").live_ticket_count, 1);

        // Removing the owner mapping AND the group while the ticket is live must
        // be atomically denied — zero usage is not the absence of authority.
        let denied = b.try_live_reconfigure(sep23_ungrouped_residency());
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "group removal under a live zero-byte ticket must be denied, got {denied:?}"
        );

        // Explicit release still succeeds against the preserved issuing scope.
        charge.release().expect("explicit release of zero-byte charge");
        sep23_assert_fully_drained(&b);

        // With no live ticket left, the identical reconfigure is legal.
        b.try_live_reconfigure(sep23_ungrouped_residency())
            .expect("group removal after final ticket release");
        sep23_assert_fully_drained(&b);
    }

    #[test]
    fn sep23_fup007_zero_byte_committed_drop_after_denied_reconfigure_releases() {
        let b = sep23_grouped_broker();
        let charge = sep23_zero_committed(&b, "owner:g");
        assert!(matches!(
            b.try_live_reconfigure(sep23_ungrouped_residency()),
            Err(MemoryError::InvalidState(_))
        ));
        drop(charge);
        sep23_assert_fully_drained(&b);
        b.try_live_reconfigure(sep23_ungrouped_residency()).expect("removal after Drop");
    }

    #[test]
    fn sep23_fup007_zero_byte_committed_rebuild_fence_drop_drains_on_finish() {
        let b = sep23_grouped_broker();
        let charge = sep23_zero_committed(&b, "owner:g");
        b.begin_rebuild().expect("begin rebuild");
        drop(charge);
        // Under the fence the exact ticket is deferred, still charged.
        assert_eq!(b.ledger_snapshot().expect("fenced").live_ticket_count, 1);
        b.finish_rebuild().expect("finish drains the deferred zero-byte release");
        sep23_assert_fully_drained(&b);
    }

    #[test]
    fn sep23_fup007_zero_byte_pending_blocks_removal_explicit_rollback_unblocks() {
        let b = sep23_grouped_broker();
        let mut pending = b
            .reserve_owned(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                0,
            )
            .expect("zero-byte pending reservation");
        assert!(
            matches!(
                b.try_live_reconfigure(sep23_ungrouped_residency()),
                Err(MemoryError::InvalidState(_))
            ),
            "pending zero-byte authority must also block scope removal"
        );
        pending.rollback().expect("explicit pending rollback");
        sep23_assert_fully_drained(&b);
        b.try_live_reconfigure(sep23_ungrouped_residency()).expect("removal after rollback");
    }

    #[test]
    fn sep23_fup007_zero_byte_pending_drop_rolls_back_without_stranding() {
        let b = sep23_grouped_broker();
        let pending = b
            .reserve_owned(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                0,
            )
            .expect("zero-byte pending reservation");
        assert!(matches!(
            b.try_live_reconfigure(sep23_ungrouped_residency()),
            Err(MemoryError::InvalidState(_))
        ));
        drop(pending);
        sep23_assert_fully_drained(&b);
    }

    #[test]
    fn sep23_fup007_denied_reconfigure_preserves_policy_revision_scheduler_ledger() {
        let b = sep23_grouped_broker();
        let _charge = sep23_zero_committed(&b, "owner:g");
        let before_report = b.effective_report().expect("report");
        let before_snapshot = b.ledger_snapshot().expect("snapshot");
        let before_group = b.admits_group("g", 100_000).expect("group probe");

        let denied = b.try_live_reconfigure(sep23_ungrouped_residency());
        assert!(matches!(denied, Err(MemoryError::InvalidState(_))));

        let after_report = b.effective_report().expect("report");
        assert_eq!(after_report.policy_revision, before_report.policy_revision, "revision frozen");
        assert_eq!(after_report.residency, before_report.residency, "policy frozen");
        assert_eq!(after_report.override_provenance, before_report.override_provenance);
        assert_eq!(b.ledger_snapshot().expect("snapshot"), before_snapshot, "ledger frozen");
        assert_eq!(b.admits_group("g", 100_000).expect("group probe"), before_group);
        assert_eq!(b.observe().expect("observe").resource_group_usage.get("g"), Some(&0));
    }

    #[test]
    fn sep23_fup007_mixed_zero_and_nonzero_partial_termination() {
        let b = sep23_grouped_broker();
        let mut zero = sep23_zero_committed(&b, "owner:g");
        let mut nonzero = b
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:g".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                10_000,
            )
            .expect("nonzero charge");
        assert!(matches!(
            b.try_live_reconfigure(sep23_ungrouped_residency()),
            Err(MemoryError::InvalidState(_))
        ));
        // Terminating only the nonzero ticket still leaves zero-byte authority.
        nonzero.release().expect("release nonzero");
        assert_eq!(b.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
        assert!(
            matches!(
                b.try_live_reconfigure(sep23_ungrouped_residency()),
                Err(MemoryError::InvalidState(_))
            ),
            "remaining zero-byte ticket must keep blocking"
        );
        zero.release().expect("release zero");
        sep23_assert_fully_drained(&b);
        b.try_live_reconfigure(sep23_ungrouped_residency()).expect("removal when fully drained");
    }

    #[test]
    fn sep23_fup007_scope_matrix_remap_cap_ungrouped_terminated() {
        // Owner remap (g -> h) with a live zero-byte ticket: denied.
        let residency_h = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
            .with_resource_group(
                "g",
                ResourceGroup {
                    reserved_bytes: 0,
                    soft_cap_bytes: 50_000,
                    hard_cap_bytes: 100_000,
                    burst_percent: 100,
                    reclaim_priority: ReclaimPriority::Normal,
                    admission_weight: 1,
                },
            )
            .with_resource_group(
                "h",
                ResourceGroup {
                    reserved_bytes: 0,
                    soft_cap_bytes: 50_000,
                    hard_cap_bytes: 100_000,
                    burst_percent: 100,
                    reclaim_priority: ReclaimPriority::Normal,
                    admission_weight: 1,
                },
            )
            .with_owner_group("owner:g", "h");
        let b = sep23_grouped_broker();
        let mut charge = sep23_zero_committed(&b, "owner:g");
        assert!(
            matches!(b.try_live_reconfigure(residency_h), Err(MemoryError::InvalidState(_))),
            "owner remap under a live zero-byte ticket must be denied"
        );

        // Group cap-only change (mapping untouched): accepted despite the ticket.
        let raised_cap = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
            .with_resource_group(
                "g",
                ResourceGroup {
                    reserved_bytes: 0,
                    soft_cap_bytes: 60_000,
                    hard_cap_bytes: 120_000,
                    burst_percent: 100,
                    reclaim_priority: ReclaimPriority::Normal,
                    admission_weight: 1,
                },
            )
            .with_owner_group("owner:g", "g");
        b.try_live_reconfigure(raised_cap).expect("cap-only change keeps the ticket scope");
        charge.release().expect("release after cap change");

        // Terminated tickets never block a later removal.
        b.try_live_reconfigure(sep23_ungrouped_residency()).expect("removal after termination");

        // An ungrouped zero-byte ticket does not pin group "g".
        let b2 = sep23_grouped_broker();
        let mut stray = sep23_zero_committed(&b2, "owner:stray");
        b2.try_live_reconfigure(sep23_ungrouped_residency())
            .expect("ungrouped ticket must not block unrelated group removal");
        stray.release().expect("release stray");
        sep23_assert_fully_drained(&b2);

        // A different owner's live ticket does not pin owner:g's mapping either,
        // but removing the group both owners share is still denied while any
        // grouped ticket is live.
        let b3 = sep23_grouped_broker();
        let mut other = b3
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:other".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                0,
            )
            .expect("other zero charge");
        // owner:other has no mapping; dropping only owner:g's mapping is denied
        // only if owner:g holds authority — it holds none here, so a policy that
        // keeps both owners unmapped-but-valid is accepted. Removing group "g"
        // while no ticket was issued under it is legal.
        b3.try_live_reconfigure(sep23_ungrouped_residency())
            .expect("no grouped ticket pins group g");
        other.release().expect("release other");
        sep23_assert_fully_drained(&b3);
    }

    // --- FUP-005: bounded reconfigure provenance ---------------------------
    //
    // The active-residency provenance is one CURRENT record, not an
    // ever-growing journal: N accepted reconfigures add at most one entry on
    // top of the initial resolver provenance.

    #[test]
    fn sep23_fup005_ten_thousand_reconfigures_keep_provenance_bounded() {
        let b = broker();
        let initial = b.effective_report().expect("report");
        let initial_len = initial.override_provenance.len();
        let initial_revision = initial.policy_revision;
        for i in 0..10_000u32 {
            let soft = 60_000 + u64::from(i % 2) * 1_000;
            let policy = ResidencyPolicy::new(1_000_000)
                .with_class_budget(
                    MemoryClass::ArtifactCache,
                    class_budget(0, soft, 200_000, false),
                )
                .with_class_budget(
                    MemoryClass::EvidenceCache,
                    class_budget(0, 10_000, 20_000, false),
                );
            b.try_live_reconfigure(policy).expect("cap flap accepted");
        }
        let after = b.effective_report().expect("report");
        assert_eq!(after.policy_revision, initial_revision + 10_000, "revision still counts");
        assert!(
            after.override_provenance.len() <= initial_len + 1,
            "10k reconfigures must add at most one record, got {} over {initial_len}",
            after.override_provenance.len()
        );
        assert_eq!(
            after.override_provenance[..initial_len],
            initial.override_provenance[..],
            "initial resolver provenance is never touched"
        );
        assert_eq!(
            after.override_provenance.last(),
            Some(&OverrideRecord::new(OverrideSource::RuntimeReconfigure, "residency.active")),
            "the single current record names the live source"
        );
        assert_eq!(after.active_policy_source, "runtime_reconfigure");
        // `observe` and `effective_report` share one report object: same
        // active policy, same revision, same bounded provenance.
        assert_eq!(b.observe().expect("observe").report, after);
        sep23_assert_fully_drained(&b);
    }

    #[test]
    fn sep23_fup005_denied_reconfigure_preserves_provenance_byte_for_byte() {
        let b = sep23_grouped_broker();
        // Plant the active record with an accepted cap-only change first.
        let raised_cap = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
            .with_resource_group(
                "g",
                ResourceGroup {
                    reserved_bytes: 0,
                    soft_cap_bytes: 60_000,
                    hard_cap_bytes: 120_000,
                    burst_percent: 100,
                    reclaim_priority: ReclaimPriority::Normal,
                    admission_weight: 1,
                },
            )
            .with_owner_group("owner:g", "g");
        b.try_live_reconfigure(raised_cap).expect("cap-only accepted");
        let _charge = sep23_zero_committed(&b, "owner:g");
        let before = b.effective_report().expect("report");
        assert!(before.override_provenance.iter().any(|r| r.field == "residency.active"));

        let denied = b.try_live_reconfigure(sep23_ungrouped_residency());
        assert!(matches!(denied, Err(MemoryError::InvalidState(_))));
        let after = b.effective_report().expect("report");
        assert_eq!(after.override_provenance, before.override_provenance, "denial preserves all");
        assert_eq!(after.policy_revision, before.policy_revision);
        assert_eq!(after.active_policy_source, before.active_policy_source);
    }

    // --- FUP-003: admission preflight + reclaim plans ------------------------
    //
    // The preflight decides BEFORE any callback: terminally impossible and
    // blocked requests fail without touching the host, a predicted failure
    // goes straight to the hard plan, and only an admissible request that
    // crosses a soft threshold gets a proactive soft request.

    fn sep23_fup003_matrix_broker(reclaimer: &Arc<RecordingReclaimer>) -> LiveProcessMemoryBroker {
        let policy = ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800, 1_000, false))
            .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false))
            .with_owner_quota(
                "owner:quota",
                OwnerQuota { soft_cap_bytes: 800, hard_cap_bytes: 900 },
            )
            .with_resource_group(
                "g",
                ResourceGroup {
                    reserved_bytes: 0,
                    soft_cap_bytes: 800,
                    hard_cap_bytes: 900,
                    burst_percent: 0,
                    reclaim_priority: ReclaimPriority::Normal,
                    admission_weight: 1,
                },
            )
            .with_owner_group("owner:grouped", "g");
        let mut report = report_with(policy);
        report.execution.max_execution_scratch_bytes = Some(50);
        LiveProcessMemoryBroker::from_report_with_reclaimer(report, Some(reclaimer.clone()))
            .expect("matrix broker")
    }

    #[test]
    fn sep23_fup003_terminal_matrix_never_fires_callback() {
        use crate::domain::admission::{AdmissionVerdict, LimitAxis};
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let b = sep23_fup003_matrix_broker(&reclaimer);
        let pristine = b.ledger_snapshot().expect("snapshot");
        let mut terminals = 0u128;

        // (label, class, owner, kind, bytes, expected axis, expected deficit)
        let cases = [
            (
                "class hard, no borrow",
                MemoryClass::EvidenceCache,
                "owner:plain",
                ChargeKind::RetainedPayload,
                101u64,
                "Class",
                1u64,
            ),
            (
                "global hard",
                MemoryClass::Headroom,
                "owner:plain",
                ChargeKind::RetainedPayload,
                1_001,
                "Global",
                1,
            ),
            (
                "owner hard",
                MemoryClass::ArtifactCache,
                "owner:quota",
                ChargeKind::RetainedPayload,
                901,
                "Owner",
                1,
            ),
            (
                "group hard+burst",
                MemoryClass::ArtifactCache,
                "owner:grouped",
                ChargeKind::RetainedPayload,
                901,
                "Group",
                1,
            ),
            (
                "execution scratch max",
                MemoryClass::ArtifactCache,
                "owner:plain",
                ChargeKind::LiveScratch,
                51,
                "ExecutionScratch",
                1,
            ),
            (
                "huge bytes",
                MemoryClass::Headroom,
                "owner:plain",
                ChargeKind::RetainedPayload,
                u64::MAX,
                "Global",
                u64::MAX - 1_000,
            ),
        ];
        for (label, class, owner, kind, bytes, axis, deficit) in cases {
            // Every case also crosses a soft threshold, which is exactly when
            // the old soft-first flow would have fired a useless callback.
            let denied = b.reserve(
                class.clone(),
                owner.to_string(),
                MemoryContextKind::Request,
                kind,
                bytes,
            );
            assert!(
                matches!(denied, Err(MemoryError::Capacity(_))),
                "{label}: terminal request must fail capacity, got {denied:?}"
            );
            terminals += 1;
            assert_eq!(
                b.reclaim_runtime_stats().expect("stats").terminal_denials,
                terminals,
                "{label}: terminal counter tracks"
            );
            assert!(
                reclaimer.requests.lock().expect("requests").is_empty(),
                "{label}: no callback for terminal denials"
            );
            // The verdict carries the limiting axis + deficit for observability.
            let core = b.inner.lock().expect("broker lock");
            match LiveProcessMemoryBroker::admission_preflight(
                &core,
                &class,
                &owner.to_string(),
                MemoryContextKind::Request,
                kind,
                bytes,
            ) {
                AdmissionVerdict::TerminalDenial { breach } => {
                    assert_eq!(
                        format!("{:?}", breach.axis).split([' ', '{']).next(),
                        Some(axis),
                        "{label}: axis"
                    );
                    assert_eq!(breach.deficit, deficit, "{label}: deficit");
                    // And the verdict is deterministic: same state, same verdict.
                    let again = LiveProcessMemoryBroker::admission_preflight(
                        &core,
                        &class,
                        &owner.to_string(),
                        MemoryContextKind::Request,
                        kind,
                        bytes,
                    );
                    assert_eq!(again, AdmissionVerdict::TerminalDenial { breach: breach.clone() });
                    let _ = LimitAxis::Global;
                }
                other => panic!("{label}: expected TerminalDenial, got {other:?}"),
            }
            drop(core);
        }
        assert_eq!(b.ledger_snapshot().expect("snapshot"), pristine, "denials mutate nothing");
        assert_eq!(b.reclaim_runtime_stats().expect("stats").attempts, 0);
    }

    #[test]
    fn sep23_fup003_borrowing_class_hard_is_not_terminal_until_pool_exhausted() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let policy = ResidencyPolicy::new(10_000)
            .with_shared_burst_pool_bytes(50)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, true));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("borrow broker");
        // Borrow within the pool admits without any callback.
        let held = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:b".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                120,
            )
            .expect("borrow within pool admits");
        // Crossing soft 90 fires a proactive soft request only — never a hard plan.
        let soft_requests = reclaimer.requests.lock().expect("requests").len();
        assert!(
            reclaimer
                .requests
                .lock()
                .expect("requests")
                .iter()
                .all(|r| r.trigger == ReclaimTrigger::SoftThreshold),
            "borrow admission may only fire proactive soft requests"
        );
        // Beyond hard+pool even at zero usage: terminal, no further callback.
        let denied = broker.reserve(
            MemoryClass::ArtifactCache,
            "owner:b".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            200,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
        assert_eq!(
            reclaimer.requests.lock().expect("requests").len(),
            soft_requests,
            "terminal denial adds no callback"
        );
        assert_eq!(broker.reclaim_runtime_stats().expect("stats").terminal_denials, 1);
        drop(held);
        sep23_assert_fully_drained(&broker);
    }

    #[test]
    fn sep23_fup003_blocked_matrix_never_fires_callback() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let b = sep23_fup003_matrix_broker(&reclaimer);
        // Optional class with no configured budget: the legacy capacity message
        // is preserved byte-for-byte, but no callback fires for a request that
        // was never viable.
        let denied = b.reserve(
            MemoryClass::SpillFileCache,
            "owner:plain".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            10,
        );
        assert_eq!(
            denied,
            Err(MemoryError::capacity(
                "class SpillFileCache is not configured in the active residency policy"
            ))
        );
        assert_eq!(b.reclaim_runtime_stats().expect("stats").blocked_denials, 1);
        assert!(reclaimer.requests.lock().expect("requests").is_empty());

        b.begin_rebuild().expect("begin rebuild");
        let fenced = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:plain".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            10,
        );
        assert_eq!(
            fenced,
            Err(MemoryError::InvalidState("rebuild in progress: live reserve rejected".into()))
        );
        assert_eq!(b.reclaim_runtime_stats().expect("stats").blocked_denials, 2);
        assert!(reclaimer.requests.lock().expect("requests").is_empty());
        b.finish_rebuild().expect("finish rebuild");
        sep23_assert_fully_drained(&b);
    }

    #[test]
    fn sep23_fup003_ticket_exhaustion_blocks_without_callback() {
        use crate::domain::admission::{
            AdmissionBlock, AdmissionRequest, AdmissionVerdict, PreflightInputs, UsageView,
        };
        // A live ledger cannot exhaust u64 ticket ids in a test, so drive the
        // pure preflight directly: exhaustion blocks with the legacy message.
        let policy = ResidencyPolicy::new(1_000_000).with_class_budget(
            MemoryClass::ArtifactCache,
            class_budget(0, 500_000, 1_000_000, false),
        );
        let empty_classes = BTreeMap::new();
        let empty_owners = BTreeMap::new();
        let empty_contexts = BTreeMap::new();
        let empty_total = crate::domain::ledger::UsageBreakdown::default();
        let empty_shares = BTreeMap::new();
        let class = MemoryClass::ArtifactCache;
        let owner = "owner:x".to_string();
        let inputs = PreflightInputs {
            policy: &policy,
            usage: UsageView {
                class_usage: &empty_classes,
                owner_usage: &empty_owners,
                context_usage: &empty_contexts,
                total_usage: &empty_total,
            },
            non_headroom_bytes: 0,
            live_scratch_bytes: 0,
            execution_scratch_max: None,
            fair_burst_shares: &empty_shares,
            group_current_usage: 0,
            unused_group_floors: 0,
            owner_group: None,
            request: AdmissionRequest {
                class: &class,
                owner: &owner,
                context: MemoryContextKind::Request,
                kind: ChargeKind::RetainedPayload,
                bytes: 100,
            },
            rebuild_in_progress: false,
            ticket_id_space_exhausted: true,
            live_ticket_limit_reached: false,
        };
        assert_eq!(
            crate::domain::admission::preflight(&inputs),
            AdmissionVerdict::Blocked { reason: AdmissionBlock::TicketIdSpaceExhausted }
        );
        assert_eq!(
            AdmissionBlock::TicketIdSpaceExhausted.into_error(),
            MemoryError::InvalidState("reservation ticket id space exhausted".into())
        );
    }

    #[test]
    fn live_ticket_count_cap_runs_one_reclaim_then_remeasures() {
        for with_group in [false, true] {
            let reclaimer = Arc::new(RecordingReclaimer::default());
            let mut policy = ResidencyPolicy::new(1_000)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
            if with_group {
                policy = policy
                    .with_resource_group(
                        "g",
                        ResourceGroup {
                            reserved_bytes: 0,
                            soft_cap_bytes: 90,
                            hard_cap_bytes: 100,
                            burst_percent: 0,
                            reclaim_priority: ReclaimPriority::Normal,
                            admission_weight: 1,
                        },
                    )
                    .with_owner_group("owner:cap", "g");
            }
            let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
                report_with(policy),
                Some(reclaimer.clone()),
            )
            .expect("broker");
            let baseline = broker
                .reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    "owner:cap".into(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    80,
                )
                .expect("baseline");
            let mut zero_tickets = Vec::with_capacity(
                crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER - 1,
            );
            for _ in 1..crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER {
                zero_tickets.push(
                    broker
                        .reserve_managed_charge(
                            MemoryClass::ArtifactCache,
                            "owner:cap".into(),
                            MemoryContextKind::Request,
                            ChargeKind::RetainedPayload,
                            0,
                        )
                        .expect("zero-byte ticket below cap"),
                );
            }
            let before = broker.ledger_snapshot().expect("before");
            assert_eq!(
                before.live_ticket_count,
                crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
            );
            assert!(reclaimer.requests.lock().expect("requests").is_empty());

            // No-op reclaim may run once, but the unchanged ledger must still
            // deny. The group case also exercises mixed admission limits.
            let denied = broker.reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:cap".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                30,
            );
            assert!(matches!(denied, Err(MemoryError::Capacity(_))), "{denied:?}");
            if !with_group {
                assert_eq!(
                    denied.err(),
                    Some(MemoryError::capacity(format!(
                        "live reservation ticket limit {} reached",
                        crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
                    )))
                );
            }
            assert_eq!(
                reclaimer.requests.lock().expect("requests").len(),
                1,
                "group={with_group}: one bounded reclaim attempt"
            );
            assert_eq!(broker.ledger_snapshot().expect("after"), before);

            drop(zero_tickets);
            drop(baseline);
            sep23_assert_fully_drained(&broker);
        }
    }

    #[test]
    fn live_ticket_count_cap_can_recover_when_host_releases_zero_byte_charge() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let policy = ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 0, 0, false));
        let mut report = report_with(policy);
        *report.residency.classes.get_mut(&MemoryClass::ArtifactCache).expect("configured class") =
            class_budget(0, 0, 0, false);
        let broker =
            LiveProcessMemoryBroker::from_report_with_reclaimer(report, Some(reclaimer.clone()))
                .expect("broker");
        *reclaimer.charge_to_release.lock().expect("reclaimable charge") = Some(
            broker
                .reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    "owner:reclaimable".into(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    0,
                )
                .expect("host-owned zero-byte charge"),
        );
        let mut retained =
            Vec::with_capacity(crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER - 1);
        for _ in 1..crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER {
            retained.push(
                broker
                    .reserve_managed_charge(
                        MemoryClass::ArtifactCache,
                        "owner:retained".into(),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        0,
                    )
                    .expect("fill authority cap"),
            );
        }
        let admitted = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:new".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                0,
            )
            .expect("callback released one live ticket");
        let requests = reclaimer.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .candidates
                .iter()
                .any(|candidate| candidate.class == MemoryClass::ArtifactCache),
            "zero-byte ticket owners must be eligible victims at the ticket-count cap"
        );
        drop(requests);
        let after = broker.ledger_snapshot().expect("after");
        assert_eq!(
            after.live_ticket_count,
            crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
        );
        assert_eq!(after.total_owner_bytes(), 0);
        drop(admitted);
        drop(retained);
        sep23_assert_fully_drained(&broker);
    }

    #[test]
    fn sep23_fup003_predicted_failure_skips_soft_and_runs_single_hard() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let policy = ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("reclaim broker");
        // 80 does not cross soft 90: no proactive request yet.
        let mut baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline");
        assert!(reclaimer.requests.lock().expect("requests").is_empty());
        // 30 crosses soft (80 -> 110) AND exceeds hard 100, but 30 alone fits:
        // the plan must be a single hard request, never soft-first.
        *reclaimer.charge_to_release.lock().expect("charge") = Some(std::mem::replace(
            &mut baseline,
            broker
                .reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    "owner:r".to_string(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    0,
                )
                .expect("placeholder"),
        ));
        let admitted = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                30,
            )
            .expect("hard reclaim admits the predicted failure");
        let requests = reclaimer.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1, "exactly one external attempt");
        assert_eq!(requests[0].trigger, ReclaimTrigger::CapacityDenial, "hard plan first");
        drop(requests);
        assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 1);
        assert_eq!(broker.reclaim_runtime_stats().expect("stats").terminal_denials, 0);
        drop(admitted);
        drop(baseline);
        sep23_assert_fully_drained(&broker);
    }

    #[test]
    fn sep23_fup003_hard_remeasure_still_denied_is_terminal_without_second_callback() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let policy = ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("reclaim broker");
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline");
        // The callback frees nothing: the remeasure still denies, terminally,
        // with no second external attempt.
        let denied = broker.reserve(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
        assert_eq!(reclaimer.requests.lock().expect("requests").len(), 1);
        assert_eq!(
            reclaimer.requests.lock().expect("requests")[0].trigger,
            ReclaimTrigger::CapacityDenial
        );
        assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 1);
        assert_eq!(broker.reclaim_runtime_stats().expect("stats").terminal_denials, 1);
        drop(baseline);
        sep23_assert_fully_drained(&broker);
    }

    // --- FUP-006: reclaim x reconfigure x release coupling -------------------
    //
    // The full-followup candidate must hold together when a reclaim callback
    // reconfigures the live policy AND releases a charge in one callback: the
    // remeasure admits under exactly one policy generation, and the initial /
    // final revision, decision receipt, ledger axes and scheduler group usage
    // all agree afterwards.

    fn sep23_fup006_grouped_policy(class_hard: u64) -> ResidencyPolicy {
        ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, class_hard, false))
            .with_resource_group(
                "g",
                ResourceGroup {
                    reserved_bytes: 0,
                    soft_cap_bytes: 90,
                    hard_cap_bytes: 100,
                    burst_percent: 0,
                    reclaim_priority: ReclaimPriority::Normal,
                    admission_weight: 1,
                },
            )
            .with_owner_group("owner:r", "g")
    }

    #[test]
    fn sep23_fup006_accepted_reconfigure_inside_callback_remeasures_under_new_policy() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(sep23_fup006_grouped_policy(100)),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        *reclaimer.broker_for_reentry.lock().expect("broker") = Some(broker.clone());
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline");
        let before = broker.observe().expect("observe");
        assert_eq!(before.resource_group_usage.get("g"), Some(&80));
        assert_eq!(before.ledger.live_ticket_count, 1);

        // Inside the hard callback: raise the class cap 100 -> 200 (the group
        // mapping is kept, so the live grouped ticket does not pin it) and
        // release the 80-byte baseline. The remeasure must admit the 30-byte
        // request under the NEW policy generation only.
        *reclaimer.reconfigure_inside.lock().expect("policy") =
            Some(sep23_fup006_grouped_policy(200));
        *reclaimer.charge_to_release.lock().expect("charge") = Some(baseline);
        let admitted = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                30,
            )
            .expect("remeasure under the raised cap admits");

        let attempts = reclaimer.reconfigure_attempts.lock().expect("attempts");
        assert_eq!(attempts.len(), 1, "exactly one in-callback reconfigure");
        assert!(attempts[0].is_ok(), "cap raise is accepted, got {:?}", attempts[0]);
        drop(attempts);
        assert_eq!(reclaimer.requests.lock().expect("requests").len(), 1, "single hard plan");

        let after = broker.observe().expect("observe");
        assert_eq!(
            after.report.policy_revision,
            before.report.policy_revision + 1,
            "initial/final revisions bracket exactly one accepted reconfigure"
        );
        assert_eq!(after.ledger.live_ticket_count, 1, "baseline released, request admitted");
        assert_eq!(after.ledger.total_owner_bytes(), 30);
        assert_eq!(after.ledger.total_class_bytes(), 30);
        assert_eq!(
            after.resource_group_usage.get("g"),
            Some(&30),
            "scheduler group usage tracks the post-callback ledger"
        );
        assert_eq!(after.reclaim.attempts, 1);
        drop(admitted);
        let drained = broker.observe().expect("observe");
        assert_eq!(drained.resource_group_usage.get("g"), Some(&0));
        assert_eq!(drained.report.policy_revision, after.report.policy_revision);
        sep23_assert_fully_drained(&broker);
    }

    // --- Self-audit: max-scale exact fill + 1 byte denies clean -------------
    //
    // At u64::MAX live usage the next byte is a ReclaimCandidate (the byte
    // alone fits everywhere), but the hard plan cannot even be constructed:
    // the prospective aggregate is unrepresentable. Plan construction fails
    // BEFORE any callback fires, the broker stays usable, and the exact fill
    // still drains to zero.

    #[test]
    fn sep23_audit_max_scale_plus_one_byte_denies_clean_without_callback() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let policy = ResidencyPolicy::new(u64::MAX).with_class_budget(
            MemoryClass::ArtifactCache,
            class_budget(0, u64::MAX, u64::MAX, false),
        );
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("max-scale policy validates");
        // Exact-fill one kind slot to u64::MAX in a single reserve.
        let maxed = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:max".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                u64::MAX,
            )
            .expect("exact u64::MAX fill admits");
        assert!(reclaimer.requests.lock().expect("requests").is_empty());
        // One more byte: the preflight sees a reclaimable breach, but the
        // hard plan's prospective aggregate overflows u64, so construction
        // fails before any callback can run.
        let denied = broker.reserve(
            MemoryClass::ArtifactCache,
            "owner:max".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata,
            1,
        );
        assert_eq!(denied, Err(MemoryError::capacity("reclaim prospective usage exceeds u64")));
        assert_eq!(broker.reclaim_runtime_stats().expect("stats").attempts, 0);
        assert!(reclaimer.requests.lock().expect("requests").is_empty());
        // The broker is fully usable afterwards: observe, release, drain.
        assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), u64::MAX);
        drop(maxed);
        sep23_assert_fully_drained(&broker);
        let fresh = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:max".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedMetadata,
                7,
            )
            .expect("broker admits after the blocked attempt");
        drop(fresh);
        sep23_assert_fully_drained(&broker);
    }

    // --- Self-audit P2-2: deferred drains never discard authority -----------
    //
    // The deferred queues only fill on an internal invariant break (or a
    // rebuild race), so no public flow seeds them — but if a drain retry
    // still fails, the exact ticket must stay queued for a later retry, never
    // silently dropped. A foreign ticket (wrong ledger identity) is the
    // deterministic stand-in for an unfailable rollback.

    fn sep23_audit_seed_deferred_foreign_ticket() -> ReservationTicket {
        let foreign = LiveProcessMemoryBroker::from_report(report_with(base_residency()))
            .expect("foreign broker");
        foreign
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:foreign".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                64,
            )
            .expect("foreign ticket")
    }

    #[test]
    fn sep23_audit_failing_pending_drain_preserves_ticket_for_retry() {
        let b = broker();
        let pristine = b.ledger_snapshot().expect("snapshot");
        let foreign = sep23_audit_seed_deferred_foreign_ticket();
        let foreign_id = foreign.id();
        // A Drop-time rollback failure seeds the deferred queue (never swallowed).
        b.rollback_pending_ticket_on_drop(&foreign);
        {
            let core = b.inner.lock().expect("core");
            assert_eq!(core.deferred_pending_rollbacks.len(), 1, "Drop seeds the queue");
            assert_eq!(core.deferred_pending_rollbacks.front().unwrap().id(), foreign_id);
            assert_eq!(core.deferred_pending_rollbacks.front().unwrap().bytes(), 64);
        }
        // A still-failing drain retry keeps the exact ticket queued, and
        // finish_rebuild surfaces the lack of progress without losing authority.
        {
            let mut core = b.inner.lock().expect("core");
            LiveProcessMemoryBroker::drain_deferred_pending_rollbacks_batch(&mut core);
            assert_eq!(core.deferred_pending_rollbacks.len(), 1, "drain preserves failures");
            assert_eq!(core.deferred_pending_rollbacks.front().unwrap().id(), foreign_id);
        }
        b.begin_rebuild().expect("begin rebuild");
        assert!(b.finish_rebuild().is_err(), "stuck cleanup is surfaced");
        assert_eq!(
            b.inner.lock().expect("core").deferred_pending_rollbacks.len(),
            1,
            "finish_rebuild keeps the failing ticket queued"
        );
        assert_eq!(b.ledger_snapshot().expect("snapshot"), pristine, "drain mutates nothing");
        assert_eq!(b.ledger_snapshot().expect("snapshot").live_ticket_count, 0);
        // The broker is unusable-state-free: fresh admission works on top.
        let fresh = b
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                8,
            )
            .expect("broker usable after failed drain");
        drop(fresh);
        // Ledger counters drain to zero; the foreign ticket intentionally
        // stays queued (sep23_assert_fully_drained would reject it).
        let drained = b.ledger_snapshot().expect("snapshot");
        drained.validate_exact().expect("axes balance after drain");
        assert_eq!(drained.total_owner_bytes(), 0);
        assert_eq!(drained.total_class_bytes(), 0);
        assert_eq!(
            b.inner.lock().expect("core").deferred_pending_rollbacks.len(),
            1,
            "foreign authority still preserved at the end"
        );
    }

    #[test]
    fn sep23_audit_failing_committed_drain_restores_whole_batch() {
        let b = broker();
        let pristine = b.ledger_snapshot().expect("snapshot");
        let foreign = sep23_audit_seed_deferred_foreign_ticket();
        let foreign_id = foreign.id();
        b.inner.lock().expect("core").deferred_committed_releases.push_back(foreign);
        b.begin_rebuild().expect("begin rebuild");
        let drained = b.finish_rebuild();
        assert!(drained.is_err(), "a still-failing batch drain surfaces its error");
        let core = b.inner.lock().expect("core");
        assert_eq!(core.deferred_committed_releases.len(), 1, "whole batch restored");
        assert_eq!(core.deferred_committed_releases.front().unwrap().id(), foreign_id);
        drop(core);
        assert_eq!(b.ledger_snapshot().expect("snapshot"), pristine, "drain mutates nothing");
        let drained = b.ledger_snapshot().expect("snapshot");
        drained.validate_exact().expect("axes balance after drain");
        assert_eq!(drained.total_owner_bytes(), 0);
        assert_eq!(
            b.inner.lock().expect("core").deferred_committed_releases.len(),
            1,
            "foreign authority still preserved at the end"
        );
    }

    #[test]
    fn sep23_fup006_denied_reconfigure_inside_callback_preserves_receipt_and_admits() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(sep23_fup006_grouped_policy(100)),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        *reclaimer.broker_for_reentry.lock().expect("broker") = Some(broker.clone());
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline");
        let before = broker.observe().expect("observe");

        // Inside the hard callback: attempt to drop group "g" while the grouped
        // baseline is still live (FUP-007 must deny), then release the baseline.
        // Admission proceeds under the OLD policy with the receipt untouched.
        let ungrouped = ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
        *reclaimer.reconfigure_inside.lock().expect("policy") = Some(ungrouped);
        *reclaimer.charge_to_release.lock().expect("charge") = Some(baseline);
        let admitted = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                30,
            )
            .expect("release still admits under the unchanged policy");

        let attempts = reclaimer.reconfigure_attempts.lock().expect("attempts");
        assert_eq!(attempts.len(), 1);
        assert!(
            matches!(attempts[0], Err(MemoryError::InvalidState(_))),
            "group removal under a live grouped ticket is denied, got {:?}",
            attempts[0]
        );
        drop(attempts);

        let after = broker.observe().expect("observe");
        assert_eq!(
            after.report.policy_revision, before.report.policy_revision,
            "denied reconfigure bumps no revision"
        );
        assert_eq!(
            after.report.override_provenance, before.report.override_provenance,
            "denied reconfigure preserves the provenance receipt byte-for-byte"
        );
        assert_eq!(after.report.active_policy_source, before.report.active_policy_source);
        assert_eq!(after.resource_group_usage.get("g"), Some(&30));
        assert_eq!(after.ledger.total_owner_bytes(), 30);
        drop(admitted);
        sep23_assert_fully_drained(&broker);
    }

    // --- Self-audit P2-4: FUP-003 contract gaps ------------------------------
    //
    // The audit's group-ladder probe, wrong-victim release, forged telemetry,
    // callback panic, and 0-byte silence each get a pinned regression, plus
    // the randomized preflight/attempt agreement test the admission module
    // docstring promises.

    fn sep23_audit_group_ladder_policy(group_soft: u64) -> ResidencyPolicy {
        ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 9_000, 10_000, false))
            .with_resource_group(
                "g",
                ResourceGroup {
                    reserved_bytes: 0,
                    soft_cap_bytes: group_soft,
                    hard_cap_bytes: 50,
                    burst_percent: 0,
                    reclaim_priority: ReclaimPriority::Normal,
                    admission_weight: 1,
                },
            )
            .with_owner_group("owner:r", "g")
    }

    #[test]
    fn sep23_audit_group_denial_runs_single_hard_regardless_of_soft_crossing() {
        // The audit's soft-45 vs soft-30 probe: the first plan for a group
        // hard denial must be Hard in both cases — soft crossing only adds an
        // earlier proactive signal on the seed, never a soft-first plan.
        for group_soft in [45u64, 30u64] {
            let reclaimer = Arc::new(RecordingReclaimer::default());
            let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
                report_with(sep23_audit_group_ladder_policy(group_soft)),
                Some(reclaimer.clone()),
            )
            .expect("broker");
            let baseline = broker
                .reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    "owner:r".to_string(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    40,
                )
                .expect("baseline");
            let seed_requests = reclaimer.requests.lock().expect("requests").len();
            *reclaimer.charge_to_release.lock().expect("charge") = Some(baseline);
            let admitted = broker
                .reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    "owner:r".to_string(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    20,
                )
                .expect("hard reclaim admits the group denial");
            let requests = reclaimer.requests.lock().expect("requests").clone();
            assert_eq!(
                requests.len(),
                seed_requests + 1,
                "soft {group_soft}: exactly one plan for the denial"
            );
            assert_eq!(
                requests.last().expect("hard request").trigger,
                ReclaimTrigger::CapacityDenial,
                "soft {group_soft}: the denial plan is Hard, never soft-first"
            );
            drop(admitted);
            sep23_assert_fully_drained(&broker);
        }
    }

    #[test]
    fn sep23_audit_wrong_victim_release_does_not_admit() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let policy = ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false))
            .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 90, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline");
        let victim = broker
            .reserve_managed_charge(
                MemoryClass::EvidenceCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                50,
            )
            .expect("victim");
        // The callback frees 50 bytes on the WRONG class: the Artifact breach
        // (80 + 30 > 100) survives, so the remeasure denies terminally.
        *reclaimer.charge_to_release.lock().expect("charge") = Some(victim);
        let denied = broker.reserve(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
        assert_eq!(reclaimer.requests.lock().expect("requests").len(), 1);
        assert_eq!(
            reclaimer.requests.lock().expect("requests")[0].trigger,
            ReclaimTrigger::CapacityDenial
        );
        assert_eq!(broker.reclaim_runtime_stats().expect("stats").terminal_denials, 1);
        let snapshot = broker.ledger_snapshot().expect("snapshot");
        assert_eq!(snapshot.total_owner_bytes(), 80, "only the victim left");
        drop(baseline);
        sep23_assert_fully_drained(&broker);
    }

    #[test]
    fn sep23_audit_forged_reclaimed_bytes_are_recorded_but_never_trusted() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let policy = ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline");
        // The callback frees nothing but reports 30 reclaimed bytes.
        *reclaimer.forged_reported_bytes.lock().expect("forged") = Some(30);
        let denied = broker.reserve(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
        let stats = broker.reclaim_runtime_stats().expect("stats");
        assert_eq!(stats.attempts, 1);
        assert_eq!(stats.reported_reclaimed_bytes, 30, "telemetry records the lie");
        assert_eq!(stats.terminal_denials, 1, "admission remeasured the ledger");
        assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 80);
        drop(baseline);
        sep23_assert_fully_drained(&broker);
    }

    #[test]
    fn sep23_audit_panicking_reclaimer_is_contained_and_broker_survives() {
        let reclaimer = Arc::new(PanickingReclaimer { calls: Mutex::new(0) });
        let policy = ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline");
        // The hard plan panics inside the callback: the panic is contained,
        // the remeasure denies on the unmoved ledger, and the broker (and its
        // lock) stays fully usable.
        let denied = broker.reserve(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
        assert_eq!(*reclaimer.calls.lock().expect("calls"), 1, "single attempt");
        let stats = broker.reclaim_runtime_stats().expect("stats");
        assert_eq!(stats.attempts, 1);
        assert_eq!(stats.failures, 1, "panic counts as a callback failure");
        assert_eq!(stats.terminal_denials, 1);
        assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 80);
        drop(baseline);
        sep23_assert_fully_drained(&broker);
        // And a second panicking attempt behaves identically (no poisoning).
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("baseline again");
        let denied = broker.reserve(
            MemoryClass::ArtifactCache,
            "owner:r".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            30,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "got {denied:?}");
        assert_eq!(*reclaimer.calls.lock().expect("calls"), 2);
        drop(baseline);
        sep23_assert_fully_drained(&broker);
    }

    #[test]
    fn reclaimer_release_then_panic_uses_remeasured_ledger_not_callback_result() {
        struct ReleaseThenPanic {
            victim: Mutex<Option<ManagedCharge>>,
            calls: std::sync::atomic::AtomicUsize,
        }

        impl MemoryReclaimer for ReleaseThenPanic {
            fn reclaim(
                &self,
                _request: &ReclaimRequest,
            ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
                self.calls.fetch_add(1, AtomicOrdering::SeqCst);
                let mut victim = self.victim.lock().expect("victim slot").take().expect("victim");
                victim.release().expect("callback releases real ledger authority");
                panic!("callback fails after making progress");
            }
        }

        let reclaimer = Arc::new(ReleaseThenPanic {
            victim: Mutex::new(None),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let policy = ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        let victim = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "victim".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                80,
            )
            .expect("victim charge");
        *reclaimer.victim.lock().expect("victim slot") = Some(victim);

        let admitted = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "request".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                30,
            )
            .expect("post-callback ledger, not callback success, determines admission");
        assert_eq!(reclaimer.calls.load(AtomicOrdering::SeqCst), 1);
        let stats = broker.reclaim_runtime_stats().expect("stats");
        assert_eq!(stats.attempts, 1);
        assert_eq!(stats.failures, 1, "panic remains observable as callback failure");
        assert_eq!(stats.terminal_denials, 0);
        let snapshot = broker.ledger_snapshot().expect("snapshot");
        snapshot.validate_exact().expect("post-callback axes balance");
        assert_eq!(snapshot.total_owner_bytes(), 30);
        assert_eq!(snapshot.live_ticket_count, 1);
        drop(admitted);
        sep23_assert_fully_drained(&broker);
    }

    #[test]
    fn sep23_audit_zero_byte_reserve_fires_no_callback_even_over_soft() {
        let reclaimer = Arc::new(RecordingReclaimer::default());
        let policy = ResidencyPolicy::new(10_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, false));
        let broker = LiveProcessMemoryBroker::from_report_with_reclaimer(
            report_with(policy),
            Some(reclaimer.clone()),
        )
        .expect("broker");
        // Seed past soft 90: the seed itself fires one proactive soft signal.
        let baseline = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                95,
            )
            .expect("baseline");
        assert_eq!(reclaimer.requests.lock().expect("requests").len(), 1);
        // A 0-byte reserve over soft adds nothing, crosses nothing, fires nothing.
        let zero = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "owner:r".to_string(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                0,
            )
            .expect("0-byte admits");
        assert_eq!(
            reclaimer.requests.lock().expect("requests").len(),
            1,
            "no callback for a 0-byte edge that crosses no threshold"
        );
        drop(zero);
        drop(baseline);
        sep23_assert_fully_drained(&broker);
    }

    // --- Randomized preflight/attempt agreement (FUP-003) --------------------
    //
    // The admission module promises this test: the preflight verdict and the
    // no-callback attempt must agree on outcome AND byte-identical error for
    // every request, and every TerminalDenial must also deny on a fresh
    // broker (proving no callback could help). Deterministic xorshift stream,
    // no threads, no timing.

    struct Sep23XorShift64(u64);

    impl Sep23XorShift64 {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn sep23_agreement_policy(shape: u64) -> ResidencyPolicy {
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 800,
            hard_cap_bytes: 900,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        match shape {
            0 => ResidencyPolicy::new(1_000)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800, 1_000, false))
                .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false)),
            1 => ResidencyPolicy::new(1_000)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800, 1_000, false))
                .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false))
                .with_resource_group("g", group)
                .with_owner_group("owner:r", "g"),
            2 => ResidencyPolicy::new(1_000)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800, 1_000, false))
                .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false))
                .with_owner_quota(
                    "owner:q",
                    OwnerQuota { soft_cap_bytes: 800, hard_cap_bytes: 900 },
                ),
            _ => ResidencyPolicy::new(10_000)
                .with_shared_burst_pool_bytes(50)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, true))
                .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false)),
        }
    }

    #[test]
    fn sep23_fup003_preflight_attempt_agreement_randomized() {
        use crate::domain::admission::AdmissionVerdict;
        const EDGE_BYTES: [u64; 20] = [
            0,
            1,
            2,
            63,
            64,
            65,
            99,
            100,
            101,
            799,
            800,
            801,
            899,
            900,
            901,
            999,
            1_000,
            1_001,
            u64::MAX - 1,
            u64::MAX,
        ];
        const CLASSES: [MemoryClass; 5] = [
            MemoryClass::ArtifactCache,
            MemoryClass::EvidenceCache,
            MemoryClass::TraceIndex,
            MemoryClass::Headroom,
            MemoryClass::SpillFileCache,
        ];
        const OWNERS: [&str; 3] = ["owner:plain", "owner:q", "owner:r"];
        const KINDS: [ChargeKind; 3] =
            [ChargeKind::RetainedPayload, ChargeKind::RetainedMetadata, ChargeKind::LiveScratch];
        for shape in 0..4u64 {
            let policy = sep23_agreement_policy(shape);
            // No reclaimer: ReclaimCandidate denies with the live breach and
            // fires nothing, so verdict and attempt are directly comparable.
            let broker =
                LiveProcessMemoryBroker::from_report(report_with(policy.clone())).expect("broker");
            let mut rng = Sep23XorShift64(0x9E37_79B9_7F4A_7C15u64.wrapping_add(shape));
            let mut live: Vec<ManagedCharge> = Vec::new();
            let mut terminal_spot_checks = 0u32;
            for step in 0..500u32 {
                if step % 100 == 99 {
                    // Rebuild-fence agreement inside the evolving state.
                    broker.begin_rebuild().expect("begin rebuild");
                    let class = CLASSES[rng.below(5) as usize].clone();
                    let owner = OWNERS[rng.below(3) as usize].to_string();
                    let core = broker.inner.lock().expect("core");
                    let verdict = LiveProcessMemoryBroker::admission_preflight(
                        &core,
                        &class,
                        &owner,
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        10,
                    );
                    drop(core);
                    assert!(
                        matches!(
                            verdict,
                            AdmissionVerdict::Blocked {
                                reason: crate::domain::admission::AdmissionBlock::RebuildInProgress
                            }
                        ),
                        "shape {shape} step {step}: fenced preflight blocks, got {verdict:?}"
                    );
                    let result = broker.reserve(
                        class,
                        owner,
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        10,
                    );
                    assert_eq!(
                        result,
                        Err(MemoryError::InvalidState(
                            "rebuild in progress: live reserve rejected".into()
                        )),
                        "shape {shape} step {step}: fenced attempt matches"
                    );
                    broker.finish_rebuild().expect("finish rebuild");
                    continue;
                }
                let class = CLASSES[rng.below(5) as usize].clone();
                let owner = OWNERS[rng.below(3) as usize].to_string();
                let kind = KINDS[rng.below(3) as usize];
                let bytes = if rng.below(2) == 0 {
                    EDGE_BYTES[rng.below(20) as usize]
                } else {
                    rng.below(2_000)
                };
                let core = broker.inner.lock().expect("core");
                let verdict = LiveProcessMemoryBroker::admission_preflight(
                    &core,
                    &class,
                    &owner,
                    MemoryContextKind::Request,
                    kind,
                    bytes,
                );
                drop(core);
                let result = broker.reserve_managed_charge(
                    class.clone(),
                    owner.clone(),
                    MemoryContextKind::Request,
                    kind,
                    bytes,
                );
                match verdict {
                    AdmissionVerdict::Admit => {
                        let charge = result.unwrap_or_else(|error| {
                            panic!(
                                "shape {shape} step {step}: preflight Admit but attempt \
                                 denied ({class:?} {owner} {kind:?} {bytes}): {error:?}"
                            )
                        });
                        // Hold most charges to evolve live state; release some
                        // to exercise the release path mid-stream.
                        if live.len() < 24 && rng.below(4) != 0 {
                            live.push(charge);
                        }
                        if live.len() == 24 {
                            live.remove(0);
                        }
                        if rng.below(8) == 0 && !live.is_empty() {
                            live.remove(rng.below(live.len() as u64) as usize);
                        }
                    }
                    AdmissionVerdict::TerminalDenial { breach } => {
                        let expected = breach.into_error();
                        match &result {
                            Err(error) if error == &expected => {}
                            other => panic!(
                                "shape {shape} step {step}: terminal verdict but attempt \
                                 diverged ({class:?} {owner} {kind:?} {bytes}): {other:?} \
                                 vs {expected:?}"
                            ),
                        }
                        // Terminality proof: the same request denies
                        // identically on a fresh broker, so no callback on
                        // live state could have helped.
                        if terminal_spot_checks < 20 {
                            terminal_spot_checks += 1;
                            let fresh =
                                LiveProcessMemoryBroker::from_report(report_with(policy.clone()))
                                    .expect("fresh broker");
                            let fresh_result = fresh
                                .reserve_managed_charge(
                                    class.clone(),
                                    owner.clone(),
                                    MemoryContextKind::Request,
                                    kind,
                                    bytes,
                                )
                                .map(|_| ());
                            assert_eq!(
                                fresh_result,
                                Err(expected),
                                "shape {shape} step {step}: terminal denial reproduces at zero usage"
                            );
                        }
                    }
                    AdmissionVerdict::ReclaimCandidate { breach } => {
                        let expected = breach.into_error();
                        match &result {
                            Err(error) if error == &expected => {}
                            other => panic!(
                                "shape {shape} step {step}: candidate verdict but attempt \
                                 diverged ({class:?} {owner} {kind:?} {bytes}): {other:?} \
                                 vs {expected:?}"
                            ),
                        }
                    }
                    AdmissionVerdict::Blocked { reason } => {
                        let expected = reason.into_error();
                        match &result {
                            Err(error) if error == &expected => {}
                            other => panic!(
                                "shape {shape} step {step}: blocked verdict but attempt \
                                 diverged ({class:?} {owner} {kind:?} {bytes}): {other:?} \
                                 vs {expected:?}"
                            ),
                        }
                    }
                }
            }
            drop(live);
            sep23_assert_fully_drained(&broker);
        }
    }
}
