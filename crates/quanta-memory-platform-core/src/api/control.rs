//! Live broker control plane: an accounting-backed broker with reserve /
//! commit / release, resource-group admission, pressure classification, and
//! fail-closed live reconfiguration.
//!
//! This is the concrete control-plane surface for ticket 05. It wraps the
//! semantic [`AccountingLedger`] (SSOT) behind a lock and composes the
//! application-layer scheduler and pressure controller. Allocator telemetry is
//! observed only through the advisory lane and never mutates the ledger.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::application::controller::LadderPressureController;
use crate::application::scheduler::WeightedResourceGroupScheduler;
use crate::domain::config::validate_residency;
use crate::domain::context::{MemoryContextKind, OwnerKey};
use crate::domain::errors::{MemoryError, MemoryResult};
use crate::domain::ledger::{AccountingLedger, ChargeKind, LedgerSnapshot, ReservationTicket};
use crate::domain::policy::{MemoryClass, ResidencyPolicy};
use crate::domain::stats::{AllocatorSnapshot, EffectiveMemoryBudgetReport, PressureState};
use crate::ports::telemetry::AllocatorAdvisory;

/// A live, accounting-backed process memory broker.
///
/// All mutating operations route through the [`AccountingLedger`] so the global
/// invariants hold. Reserve fails closed (no mutation on deny). Live reconfig
/// rejects illegal narrowing while leaving the active policy untouched.
#[derive(Clone)]
pub struct LiveProcessMemoryBroker {
    inner: Arc<Mutex<BrokerCore>>,
}

/// Outcome of a denied [`LiveProcessMemoryBroker::recharge`].
///
/// Accounting disposition after a denied recharge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RechargeRecoveryV1 {
    /// The old charge and its original ticket remain live and unchanged.
    OldChargePreserved,
    /// A sink that cannot replace in place restored the old charge under a new
    /// ticket. The caller must adopt this ticket.
    OldChargeRestored(ReservationTicket),
    /// The old charge was released and could not be restored. The caller must
    /// remove or quarantine the corresponding payload; it must never continue
    /// serving it as charged state.
    RecoveryRequired { rollback_error: MemoryError },
}

/// The new charge was rejected together with an unambiguous accounting
/// disposition for the old charge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RechargeError {
    /// Why the new charge was denied.
    pub error: MemoryError,
    /// What happened to the old charge.
    pub recovery: RechargeRecoveryV1,
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
}

impl LiveProcessMemoryBroker {
    /// Build a live broker from a validated effective report. The residency
    /// policy is re-validated here so a broker can never come up on an invalid
    /// policy.
    pub fn from_report(report: EffectiveMemoryBudgetReport) -> MemoryResult<Self> {
        validate_residency(&report.residency)?;
        let residency = report.residency.clone();
        let scheduler = WeightedResourceGroupScheduler::new(
            residency.resource_groups.clone(),
            residency.shared_burst_pool_bytes,
        );
        let controller = Arc::new(LadderPressureController::new(
            residency.global_hard_limit_bytes,
            crate::application::controller::PressureThresholds::default(),
            residency.initial_pressure,
        ));
        let ledger = AccountingLedger::new(residency);
        Ok(Self {
            inner: Arc::new(Mutex::new(BrokerCore {
                ledger,
                scheduler,
                controller,
                report,
                ticket_groups: BTreeMap::new(),
            })),
        })
    }

    fn lock(&self) -> MemoryResult<std::sync::MutexGuard<'_, BrokerCore>> {
        self.inner.lock().map_err(|_| MemoryError::InvalidState("broker lock poisoned".to_string()))
    }

    /// Reserve bytes against (class, owner, context). Fail-closed before
    /// mutation. Consults the resource-group scheduler when the owner maps to a
    /// resource group (weighted-fair admission), and enforces the execution
    /// scratch budget for live-scratch charges — both BEFORE any ledger mutation,
    /// so a denial leaves the ledger byte-for-byte unchanged.
    pub fn reserve(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        let mut core = self.lock()?;
        Self::reserve_on(&mut core, class, owner, context, kind, bytes)
    }

    /// Reserve AND commit under a SINGLE lock acquisition, so no live reconfigure
    /// or rebuild can interleave between the reservation and its commit (which
    /// would otherwise strand reserved bytes behind a discarded ticket). This is
    /// the atomic charge primitive the facade charged-store seam relies on.
    pub fn reserve_committed(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        let mut core = self.lock()?;
        Self::reserve_committed_on(&mut core, class, owner, context, kind, bytes)
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
    pub fn recharge(
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
            next_scheduler.record_release(group, old.bytes());
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
        }

        // The execution scratch ceiling is a broker-owned admission axis, not a
        // ledger class cap. Evaluate the same replacement postimage used by the
        // ledger so the atomic primitive cannot bypass this outer authority.
        if let Some(max) = core.report.execution.max_execution_scratch_bytes {
            let max = u64::try_from(max).unwrap_or(u64::MAX);
            let current = core.ledger.snapshot().total_live_scratch_bytes();
            let without_old = if old.kind() == ChargeKind::LiveScratch {
                current.saturating_sub(old.bytes())
            } else {
                current
            };
            let live_after = if kind == ChargeKind::LiveScratch {
                without_old.saturating_add(bytes)
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
        if let Some(group) = &new_group {
            next_scheduler.record_usage(group, bytes);
        }
        core.scheduler = next_scheduler;
        core.ticket_groups.remove(&old.id());
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

    /// Release on an already-held lock guard (ledger release + group bookkeeping),
    /// fail-closed before any scheduler mutation.
    fn release_on(core: &mut BrokerCore, ticket: &ReservationTicket) -> MemoryResult<()> {
        core.ledger.release(ticket)?;
        if let Some(group) = core.ticket_groups.remove(&ticket.id()) {
            core.scheduler.record_release(&group, ticket.bytes());
        }
        Ok(())
    }

    fn release_many_on(core: &mut BrokerCore, tickets: &[ReservationTicket]) -> MemoryResult<()> {
        core.ledger.release_batch(tickets)?;
        for ticket in tickets {
            if let Some(group) = core.ticket_groups.remove(&ticket.id()) {
                core.scheduler.record_release(&group, ticket.bytes());
            }
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

        // D: resource-group admission. An owner mapped to a group must pass the
        // weighted-fair scheduler (anti-starvation) before it can reserve.
        let group = core.ledger.policy().owner_groups.get(&owner).cloned();
        if let Some(ref group) = group {
            if !core.scheduler.admits(group, bytes) {
                return Err(MemoryError::capacity(format!(
                    "resource group {group} admission denied for owner {owner} reserve of {bytes} bytes"
                )));
            }
        }

        // ExecutionBudget: the memory-relevant cap is `max_execution_scratch_bytes`,
        // enforced here for live-scratch charges. The other execution caps
        // (wall-time, subqueries, trace-edges, output) are runtime concerns and
        // are host-enforced (surfaced in the effective report for the host).
        if kind == ChargeKind::LiveScratch {
            if let Some(max) = core.report.execution.max_execution_scratch_bytes {
                let max = u64::try_from(max).unwrap_or(u64::MAX);
                let live_after =
                    core.ledger.snapshot().total_live_scratch_bytes().saturating_add(bytes);
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
        if let Some(group) = group {
            core.scheduler.record_usage(&group, bytes);
            core.ticket_groups.insert(ticket.id(), group);
        }
        Ok(ticket)
    }

    /// Commit a previously reserved ticket.
    pub fn commit(&self, ticket: &ReservationTicket) -> MemoryResult<u64> {
        let mut core = self.lock()?;
        core.ledger.commit(ticket)
    }

    /// Release a ticket (rollback of a reservation or eviction of a committed
    /// charge). Returns the released bytes to the owner's resource group (if any)
    /// only after the ledger release succeeds, so a rejected (replayed) release
    /// never returns phantom group capacity.
    pub fn release(&self, ticket: &ReservationTicket) -> MemoryResult<()> {
        let mut core = self.lock()?;
        // `release_on` returns capacity to the group the ticket was ADMITTED under
        // (recorded at reserve time), not the current policy's mapping — so
        // reserve/release stay symmetric even after a live reconfigure remaps the
        // owner — and is fail-closed before any scheduler mutation.
        Self::release_on(&mut core, ticket)
    }

    /// Release a batch of tickets atomically.
    ///
    /// Used by charged facade mutators that must refund multiple tickets before
    /// mutating store state. Either every ticket is released or none are.
    pub fn release_many(&self, tickets: &[ReservationTicket]) -> MemoryResult<()> {
        let mut core = self.lock()?;
        Self::release_many_on(&mut core, tickets)
    }

    /// Ask resource-group admission whether `group_name` may grow by `bytes`.
    pub fn admits_group(&self, group_name: &str, bytes: u64) -> MemoryResult<bool> {
        use crate::application::scheduler::ResourceGroupScheduler;
        let core = self.lock()?;
        Ok(core.scheduler.admits(group_name, bytes))
    }

    /// Record committed resource-group usage (drives fairness/starvation).
    pub fn record_group_usage(&self, group_name: &str, bytes: u64) -> MemoryResult<u64> {
        let mut core = self.lock()?;
        Ok(core.scheduler.record_usage(group_name, bytes))
    }

    /// Enter rebuild mode: subsequent live writes are rejected until
    /// [`Self::finish_rebuild`].
    pub fn begin_rebuild(&self) -> MemoryResult<()> {
        let mut core = self.lock()?;
        core.ledger.begin_rebuild();
        Ok(())
    }

    pub fn finish_rebuild(&self) -> MemoryResult<()> {
        let mut core = self.lock()?;
        core.ledger.finish_rebuild();
        Ok(())
    }

    /// Current pressure classification from semantic usage (not allocator
    /// telemetry).
    pub fn classify_pressure(&self) -> MemoryResult<PressureState> {
        let core = self.lock()?;
        let used = core.ledger.snapshot().total_owner_bytes();
        Ok(core.controller.update_from_semantic(used))
    }

    /// Observe an allocator snapshot. Advisory only: returns a comparison
    /// against semantic accounting and provably does NOT mutate the ledger
    /// (invariant 6). The returned advisory carries the mismatch signal.
    pub fn observe_allocator(
        &self,
        snapshot: &AllocatorSnapshot,
    ) -> MemoryResult<AllocatorAdvisory> {
        let core = self.lock()?;
        let semantic = core.ledger.snapshot().total_owner_bytes();
        Ok(AllocatorAdvisory::compare(snapshot, semantic))
    }

    /// Attempt a live reconfiguration. Fail-closed: an illegal narrowing leaves
    /// the active policy and ledger untouched and returns a typed error.
    ///
    /// Deny rules, evaluated against LIVE usage (not just the new policy's keys):
    /// a new global hard limit below current usage; any class that currently
    /// holds bytes whose new hard cap is below its usage OR that is dropped from
    /// the new policy entirely (which would silently uncap it); any owner quota
    /// in the new policy whose hard cap is below that owner's current usage; any
    /// owner→resource-group remap while that owner still holds live bytes.
    /// Allowed: raising caps, adding headroom, owner-quota / reclaim-priority
    /// adjustments that still honor live usage without remapping live owners.
    pub fn try_live_reconfigure(&self, new_policy: ResidencyPolicy) -> MemoryResult<()> {
        let mut core = self.lock()?;
        // First, the new policy must be structurally valid on its own.
        validate_residency(&new_policy)?;

        let snapshot = core.ledger.snapshot();
        let current_total = snapshot.total_owner_bytes();
        if new_policy.global_hard_limit_bytes < current_total {
            return Err(MemoryError::InvalidState(format!(
                "live reconfig denied: new global hard limit {} below current usage {current_total}",
                new_policy.global_hard_limit_bytes
            )));
        }

        // Invariant 5 (headroom band) must keep holding for the LIVE state, not
        // just for future reserves: non-`Headroom` bytes already resident must fit
        // under the NEW effective non-Headroom ceiling. Raising `headroom_bytes`
        // above the free space would otherwise silently strand existing
        // non-Headroom usage inside the reserved band (a fail-open of invariant 5).
        let headroom_used =
            snapshot.class_usage.get(&MemoryClass::Headroom).map_or(0, |u| u.total());
        let non_headroom_used = current_total.saturating_sub(headroom_used);
        let new_non_headroom_ceiling =
            new_policy.global_hard_limit_bytes.saturating_sub(new_policy.headroom_bytes);
        if non_headroom_used > new_non_headroom_ceiling {
            return Err(MemoryError::InvalidState(format!(
                "live reconfig denied: non-Headroom usage {non_headroom_used} exceeds the new \
                 effective ceiling {new_non_headroom_ceiling} (global {} - headroom {})",
                new_policy.global_hard_limit_bytes, new_policy.headroom_bytes
            )));
        }

        // Fail-closed over the LIVE usage axes, not the new policy's keys: a class
        // that currently holds bytes may neither be dropped from the policy (which
        // would silently uncap it) nor be given a hard cap below its usage.
        for (class, usage) in &snapshot.class_usage {
            let class_used = usage.total();
            if class_used == 0 {
                continue;
            }
            match new_policy.classes.get(class) {
                Some(budget) if budget.hard_cap_bytes >= class_used => {}
                Some(budget) => {
                    return Err(MemoryError::InvalidState(format!(
                        "live reconfig denied: class {class:?} new hard cap {} below current usage {class_used}",
                        budget.hard_cap_bytes
                    )));
                }
                None => {
                    return Err(MemoryError::InvalidState(format!(
                        "live reconfig denied: class {class:?} holds {class_used} live bytes but is \
                         absent from the new policy (would silently uncap it)"
                    )));
                }
            }
        }

        // Any owner quota carried by the new policy must honor that owner's live
        // usage, or the reconfig would install a quota the owner already violates.
        for (owner, quota) in &new_policy.owner_quotas {
            let owner_used = snapshot.owner_usage.get(owner).map_or(0, |u| u.total());
            if quota.hard_cap_bytes < owner_used {
                return Err(MemoryError::InvalidState(format!(
                    "live reconfig denied: owner {owner} new hard cap {} below current usage {owner_used}",
                    quota.hard_cap_bytes
                )));
            }
        }

        let current_owner_groups = &core.ledger.policy().owner_groups;
        for (owner, usage) in &snapshot.owner_usage {
            let owner_used = usage.total();
            if owner_used == 0 {
                continue;
            }
            let current_group = current_owner_groups.get(owner);
            let next_group = new_policy.owner_groups.get(owner);
            if current_group != next_group {
                return Err(MemoryError::InvalidState(format!(
                    "live reconfig denied: owner {owner} holds {owner_used} live bytes but resource-group mapping changes from {:?} to {:?}",
                    current_group, next_group
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
                *slot = slot.saturating_add(usage.total());
            }
        }
        for (group, used) in live_group_usage {
            if let Some(rg) = new_policy.resource_groups.get(group) {
                if rg.hard_cap_bytes < used {
                    return Err(MemoryError::InvalidState(format!(
                        "live reconfig denied: resource group {group} new hard cap {} below current usage {used}",
                        rg.hard_cap_bytes
                    )));
                }
            }
        }

        // Accepted: install the new policy by rebuilding the ledger's view while
        // preserving usage counters.
        core.install_policy(new_policy);
        Ok(())
    }

    /// Immutable ledger snapshot for invariant assertions and reporting.
    pub fn ledger_snapshot(&self) -> MemoryResult<LedgerSnapshot> {
        let core = self.lock()?;
        Ok(core.ledger.snapshot())
    }

    /// The effective budget report this broker was built from.
    pub fn effective_report(&self) -> MemoryResult<EffectiveMemoryBudgetReport> {
        let core = self.lock()?;
        Ok(core.report.clone())
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
    fn install_policy(&mut self, new_policy: ResidencyPolicy) {
        // Snapshot live usage BEFORE the swap so the rebuilt scheduler can be
        // re-seeded from it (a fresh scheduler starts at zero usage, which would
        // otherwise make post-reconfigure admission see phantom-empty groups and
        // silently drop the anti-starvation guarantee).
        let snapshot = self.ledger.snapshot();
        self.ledger.swap_policy(new_policy.clone());
        let mut scheduler = WeightedResourceGroupScheduler::new(
            new_policy.resource_groups.clone(),
            new_policy.shared_burst_pool_bytes,
        );
        for (owner, group) in &new_policy.owner_groups {
            let used = snapshot.owner_usage.get(owner).map_or(0, |u| u.total());
            if used > 0 {
                scheduler.record_usage(group, used);
            }
        }
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
    }
}

impl crate::api::broker::ProcessMemoryBroker for LiveProcessMemoryBroker {
    fn attach_runtime(
        &self,
        runtime_id: crate::domain::context::RuntimeId,
        runtime_kind: crate::domain::context::RuntimeKind,
    ) -> MemoryResult<Arc<dyn crate::api::broker::RuntimeMemoryView>> {
        Ok(Arc::new(crate::api::broker::BasicRuntimeMemoryView::new(runtime_id, runtime_kind)))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::policy::{
        MemoryClassBudget, OwnerQuota, ReclaimPriority, ResidencyPolicy, ResourceGroup,
    };
    use crate::domain::stats::PressureState;

    fn report_with(residency: ResidencyPolicy) -> EffectiveMemoryBudgetReport {
        EffectiveMemoryBudgetReport {
            preset: crate::domain::policy::MemoryPreset::Production,
            execution: crate::domain::policy::ExecutionBudget::default(),
            residency,
            override_sources: vec!["test".to_string()],
            override_provenance: Vec::new(),
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
        // negative mutation: if the cap were not enforced the totals would differ
        assert_ne!(after.total_owner_bytes(), 600_000);
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
            .with_resource_group("g", ResourceGroup { hard_cap_bytes: 50_000, ..group.clone() })
            .with_owner_group("owner:g", "g");
        let denied = b.try_live_reconfigure(lowered);
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "lowering a group cap below live group usage must be denied, got {denied:?}"
        );

        // Raising the cap to 120k (>= 90k live) is legal and accepted.
        let raised = ResidencyPolicy::new(10_000_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
            .with_resource_group("g", ResourceGroup { hard_cap_bytes: 120_000, ..group })
            .with_owner_group("owner:g", "g");
        b.try_live_reconfigure(raised).expect("raising the group cap over live usage is legal");
        b.release(&t).expect("release");
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
            Err(MemoryError::InvalidState(_))
        ));
        assert!(matches!(b.commit(&ticket), Err(MemoryError::InvalidState(_))));
        assert!(matches!(b.release(&ticket), Err(MemoryError::InvalidState(_))));
        assert!(matches!(b.begin_rebuild(), Err(MemoryError::InvalidState(_))));
        assert!(matches!(b.finish_rebuild(), Err(MemoryError::InvalidState(_))));
        assert!(matches!(
            b.try_live_reconfigure(ResidencyPolicy::new(2_000_000)),
            Err(MemoryError::InvalidState(_))
        ));
        assert!(matches!(b.ledger_snapshot(), Err(MemoryError::InvalidState(_))));
        assert!(matches!(
            b.observe_allocator(&AllocatorSnapshot {
                resident_bytes: Some(123),
                ..AllocatorSnapshot::default()
            }),
            Err(MemoryError::InvalidState(_))
        ));
        assert!(matches!(b.effective_report(), Err(MemoryError::InvalidState(_))));

        let report_after = <LiveProcessMemoryBroker as ProcessMemoryBroker>::effective_report(&b);
        assert_eq!(report_after, report_before, "trait report must stay truthful under poison");
        assert_eq!(report_after.residency.global_hard_limit_bytes, 1_000_000);
        assert!(
            !report_after.override_sources.iter().any(|s| s == "poisoned-fallback"),
            "trait report must not fabricate a poison fallback marker"
        );
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
}
