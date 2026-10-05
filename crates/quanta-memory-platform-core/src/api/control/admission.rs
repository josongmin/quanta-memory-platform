use super::*;
use crate::domain::admission::AdmissionBlock;
use crate::ports::reclaim::{ReclaimScope, ReclaimTarget, ReclaimUnit};

#[derive(Clone, Copy)]
enum AdmissionAction<'a> {
    Reserve { commit: bool },
    Replace(&'a ReservationTicket),
}

impl<'a> AdmissionAction<'a> {
    fn old(self) -> Option<&'a ReservationTicket> {
        match self {
            Self::Reserve { .. } => None,
            Self::Replace(old) => Some(old),
        }
    }
}

impl LiveProcessMemoryBroker {
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
    pub(super) fn reserve_committed(
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
    pub(super) fn reserve_committed_owned(
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
    pub(super) fn release_owned(&self, charge: CommittedCharge) -> Result<(), ChargeReleaseError> {
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
    pub(super) fn recharge_owned(
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

    /// Replace a committed charge through the same bounded admission/reclaim
    /// executor as reserve. The old authority remains live across the callout.
    pub(crate) fn recharge(
        &self,
        old: &ReservationTicket,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> Result<ReservationTicket, RechargeError> {
        self.admit_with_reclaim(class, owner, context, kind, bytes, AdmissionAction::Replace(old))
            .map_err(|error| RechargeError {
                error,
                recovery: RechargeRecoveryV1::OldChargePreserved,
            })
    }

    /// Final locked replacement. All admission checks use the shared net read
    /// view; the staged scheduler is published only after the ledger succeeds.
    fn recharge_on(
        core: &mut BrokerCore,
        old: &ReservationTicket,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        let old_attribution = Self::attribution_for(core, old)?;
        let new_group = core.ledger.policy().owner_groups.get(&owner).cloned();
        let mut next_scheduler = core.scheduler.clone();
        if let Some(group) = &old_attribution.group {
            next_scheduler.record_release(group, old.class(), old.bytes())?;
        }
        if let Some(group) = &new_group {
            next_scheduler.record_usage(group, &class, bytes)?;
        }
        let replacement = core.ledger.replace_committed(old, class, owner, context, kind, bytes)?;
        core.scheduler = next_scheduler;
        core.ticket_attribution.remove(&old.id());
        core.ticket_attribution
            .insert(replacement.id(), TicketAttribution::new(&replacement, new_group));
        Ok(replacement)
    }

    /// Reserve + commit on an already-held lock guard, rolling the reservation
    /// back if the (unreachable on this path) commit ever fails.
    pub(super) fn reserve_committed_on(
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
            if let Err(rollback_error) = Self::release_on(core, &ticket) {
                // Keep the exact pending authority for a later retry if even
                // the defensive rollback fails.
                core.deferred_pending_rollbacks.push_back(ticket);
                return Err(MemoryError::InvalidState(format!(
                    "reservation commit failed ({err}); rollback also failed ({rollback_error})"
                )));
            }
            return Err(err);
        }
        Ok(ticket)
    }

    /// Select the first crossing in the stable class -> owner -> group order.
    /// Crossing compares live usage to the net replacement postimage, so an
    /// ordinary same-attribution shrink cannot provoke a soft callback.
    fn soft_threshold_request(
        core: &BrokerCore,
        inputs: &PreflightInputs<'_>,
    ) -> MemoryResult<Option<ReclaimRequest>> {
        let request = &inputs.request;
        let policy = inputs.policy;
        let class_current = core.ledger.class_usage(request.class).checked_total()?;
        let class_after = inputs
            .usage
            .class(request.class)?
            .checked_total()?
            .checked_add(request.bytes)
            .ok_or_else(|| MemoryError::capacity("class prospective usage exceeds u64"))?;
        let owner_current = core.ledger.owner_usage(request.owner).checked_total()?;
        let owner_after = inputs
            .usage
            .owner(request.owner)?
            .checked_total()?
            .checked_add(request.bytes)
            .ok_or_else(|| MemoryError::capacity("owner prospective usage exceeds u64"))?;
        let crossing = if let Some(budget) = policy.classes.get(request.class).filter(|budget| {
            class_current <= budget.soft_cap_bytes && class_after > budget.soft_cap_bytes
        }) {
            Some((ReclaimScope::Class(request.class.clone()), class_after - budget.soft_cap_bytes))
        } else if let Some(quota) = policy.owner_quotas.get(request.owner).filter(|quota| {
            owner_current <= quota.soft_cap_bytes && owner_after > quota.soft_cap_bytes
        }) {
            Some((ReclaimScope::Owner(request.owner.clone()), owner_after - quota.soft_cap_bytes))
        } else if let Some(group) = inputs.owner_group {
            let current = core.scheduler.current_usage(group);
            let after = inputs
                .group_current_usage
                .checked_add(request.bytes)
                .ok_or_else(|| MemoryError::capacity("group prospective usage exceeds u64"))?;
            policy
                .resource_groups
                .get(group)
                .filter(|budget| current <= budget.soft_cap_bytes && after > budget.soft_cap_bytes)
                .map(|budget| {
                    (ReclaimScope::ResourceGroup(group.to_string()), after - budget.soft_cap_bytes)
                })
        } else {
            None
        };
        let Some((scope, minimum_reduction)) = crossing else {
            return Ok(None);
        };
        let target = ReclaimTarget {
            unit: ReclaimUnit::Bytes,
            scope,
            minimum_reduction: Some(minimum_reduction),
        };
        Self::reclaim_request(core, inputs, ReclaimTrigger::SoftThreshold, target).map(Some)
    }

    fn reclaim_request(
        core: &BrokerCore,
        inputs: &PreflightInputs<'_>,
        trigger: ReclaimTrigger,
        target: ReclaimTarget,
    ) -> MemoryResult<ReclaimRequest> {
        let request = &inputs.request;
        // Read only the keyed/aggregate ledger projection. A reclaim request
        // must not clone the complete owner map just to compute its pressure.
        let prospective = inputs.usage.total()?.checked_total()?.checked_add(request.bytes);
        let classified = prospective
            .map_or(PressureState::Fatal, |bytes| core.controller.classify_semantic(bytes));
        let pressure = match (trigger, classified) {
            (ReclaimTrigger::SoftThreshold, PressureState::Normal) => PressureState::Soft,
            (ReclaimTrigger::CapacityDenial, PressureState::Normal | PressureState::Soft) => {
                PressureState::Hard
            }
            _ => classified,
        };
        let ticket_pressure = target.unit == ReclaimUnit::LiveTickets;
        let mut candidates: Vec<ReclaimCandidate> = inputs
            .policy
            .classes
            .iter()
            .filter(|(class, budget)| {
                let live = core.ledger.class_usage(class);
                let relevant = match &target.scope {
                    ReclaimScope::Class(target_class) => *class == target_class,
                    ReclaimScope::NonHeadroom => **class != MemoryClass::Headroom,
                    ReclaimScope::SharedBorrowPool => {
                        budget.borrow_from_shared_pool
                            && (*class == request.class || live.total() > budget.hard_cap_bytes)
                    }
                    ReclaimScope::ExecutionScratch => live.live_scratch_bytes > 0,
                    ReclaimScope::ChargeKind(kind) => match kind {
                        ChargeKind::RetainedPayload => live.retained_payload_bytes > 0,
                        ChargeKind::RetainedMetadata => live.retained_metadata_bytes > 0,
                        ChargeKind::LiveScratch => live.live_scratch_bytes > 0,
                    },
                    ReclaimScope::Global
                    | ReclaimScope::Owner(_)
                    | ReclaimScope::ResourceGroup(_)
                    | ReclaimScope::Context(_) => true,
                };
                relevant && (budget.hard_cap_bytes > 0 || live.total() > 0 || ticket_pressure)
            })
            .map(|(class, budget)| ReclaimCandidate {
                class: class.clone(),
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
        let mut group_candidates: Vec<ReclaimGroupCandidate> = inputs
            .policy
            .resource_groups
            .iter()
            .filter(|(group, _)| match &target.scope {
                ReclaimScope::ResourceGroup(target_group) => *group == target_group,
                _ => true,
            })
            .map(|(group, budget)| ReclaimGroupCandidate {
                group: group.clone(),
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
            requested_class: request.class.clone(),
            requested_owner: request.owner.clone(),
            resource_group: inputs.owner_group.map(str::to_string),
            requested_bytes: request.bytes,
            target,
            pressure,
            rungs: core.controller.ladder_for(pressure),
            candidates,
            group_candidates,
            max_attempts: 1,
        })
    }

    pub(super) fn run_reclaim(
        &self,
        reclaimer: Arc<dyn MemoryReclaimer>,
        request: &ReclaimRequest,
    ) {
        if self.reclaimer_quarantined.load(AtomicOrdering::Acquire) {
            self.emit(MemoryEvent::new(
                MemoryOperation::Reclaim,
                MemoryOutcome::CallbackFailed,
                Some(request.requested_bytes),
            ));
            return;
        }
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
        if self.reclaimer_quarantined.load(AtomicOrdering::Acquire) {
            return;
        }
        let broker_identity = Arc::as_ptr(&self.inner) as usize;
        // The entry guard in `admit_with_reclaim` rejects same-broker admission
        // reentry before it can prepare another callback. This local guard is a
        // second boundary for future internal callers of this method.
        let Some(callback_scope) =
            BrokerCallbackScope::enter(BrokerCallbackKind::Reclaimer, broker_identity)
        else {
            return;
        };
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reclaimer.reclaim(request)));
        if outcome.is_err() {
            self.reclaimer_quarantined.store(true, AtomicOrdering::Release);
        }
        drop(callback_scope);
        drop(reclaimer_permit);
        // Payload destructors are host code too. Never run one under the
        // broker mutex; it may reenter accounting before it panics.
        let outcome = outcome.map_err(retire_host_panic_payload);
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
            Err(()) => {
                core.reclaim_stats.failures += 1;
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

    /// Construct a net read projection without cloning the ledger or owner map.
    /// Old authority is validated before crediting any of its accounting axes.
    fn admission_inputs<'a>(
        core: &'a BrokerCore,
        class: &'a MemoryClass,
        owner: &'a OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
        old: Option<&'a ReservationTicket>,
    ) -> MemoryResult<PreflightInputs<'a>> {
        let policy = core.ledger.policy();
        let owner_group = policy.owner_groups.get(owner).map(String::as_str);
        let mut usage = core.ledger.usage_view();
        let mut non_headroom_bytes = core.ledger.non_headroom_bytes();
        let mut live_scratch_bytes = core.ledger.total_live_scratch_bytes();
        let mut group_current_usage =
            owner_group.map_or(0, |group| core.scheduler.current_usage(group));
        let mut group_current_non_headroom_usage =
            owner_group.map_or(0, |group| core.scheduler.current_non_headroom_usage(group));
        let mut unused_group_floors = core.scheduler.unused_reserved_floor_bytes();
        if let Some(old) = old {
            core.ledger.validate_committed_ticket(old)?;
            let attribution = Self::attribution_for(core, old)?;
            usage = usage.with_credit(old);
            if *old.class() != MemoryClass::Headroom {
                non_headroom_bytes =
                    non_headroom_bytes.checked_sub(old.bytes()).ok_or_else(|| {
                        MemoryError::InvalidState(
                            "replacement credit exceeds non-headroom usage".into(),
                        )
                    })?;
            }
            if old.kind() == ChargeKind::LiveScratch {
                live_scratch_bytes =
                    live_scratch_bytes.checked_sub(old.bytes()).ok_or_else(|| {
                        MemoryError::InvalidState(
                            "replacement credit exceeds live scratch usage".into(),
                        )
                    })?;
            }
            if let Some(old_group) = attribution.group.as_deref() {
                if owner_group == Some(old_group) {
                    group_current_usage =
                        group_current_usage.checked_sub(old.bytes()).ok_or_else(|| {
                            MemoryError::InvalidState(
                                "replacement credit exceeds group usage".into(),
                            )
                        })?;
                    if *old.class() != MemoryClass::Headroom {
                        group_current_non_headroom_usage = group_current_non_headroom_usage
                            .checked_sub(old.bytes())
                            .ok_or_else(|| {
                                MemoryError::InvalidState(
                                    "replacement credit exceeds group non-headroom usage".into(),
                                )
                            })?;
                    }
                }
                if *old.class() != MemoryClass::Headroom {
                    let budget = policy.resource_groups.get(old_group).ok_or_else(|| {
                        MemoryError::InvalidState("replacement issuing group is missing".into())
                    })?;
                    let current = core.scheduler.current_non_headroom_usage(old_group);
                    let after_release = current.checked_sub(old.bytes()).ok_or_else(|| {
                        MemoryError::InvalidState(
                            "replacement credit exceeds issuing group usage".into(),
                        )
                    })?;
                    unused_group_floors = unused_group_floors
                        .checked_sub(u128::from(budget.reserved_bytes.saturating_sub(current)))
                        .and_then(|total| {
                            total.checked_add(u128::from(
                                budget.reserved_bytes.saturating_sub(after_release),
                            ))
                        })
                        .ok_or_else(|| {
                            MemoryError::InvalidState(
                                "replacement group floors are inconsistent".into(),
                            )
                        })?;
                }
            }
        }
        Ok(PreflightInputs {
            policy,
            usage,
            non_headroom_bytes,
            live_scratch_bytes,
            execution_scratch_max: core.report.execution.max_execution_scratch_bytes,
            fair_burst_shares: core.scheduler.fair_burst_shares(),
            group_current_usage,
            group_current_non_headroom_usage,
            unused_group_floors,
            owner_group,
            request: AdmissionRequest { class, owner, context, kind, bytes },
            rebuild_in_progress: core.ledger.rebuild_in_progress(),
            ticket_id_space_exhausted: core.ledger.ticket_id_space_exhausted(),
            live_ticket_limit_reached: core.ledger.live_ticket_limit_reached(),
            ticket_count_delta: u8::from(old.is_none()),
        })
    }

    fn operation_preflight(
        core: &BrokerCore,
        class: &MemoryClass,
        owner: &OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
        old: Option<&ReservationTicket>,
    ) -> AdmissionVerdict {
        match Self::admission_inputs(core, class, owner, context, kind, bytes, old) {
            Ok(inputs) => crate::domain::admission::preflight(&inputs),
            Err(error) => AdmissionVerdict::Blocked { reason: AdmissionBlock::Internal(error) },
        }
    }

    /// Existing reserve oracle entry; production uses the shared executor.
    #[cfg(test)]
    pub(super) fn admission_preflight(
        core: &BrokerCore,
        class: &MemoryClass,
        owner: &OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> AdmissionVerdict {
        Self::operation_preflight(core, class, owner, context, kind, bytes, None)
    }

    pub(super) fn reserve_with_reclaim(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
        commit: bool,
    ) -> MemoryResult<ReservationTicket> {
        self.admit_with_reclaim(
            class,
            owner,
            context,
            kind,
            bytes,
            AdmissionAction::Reserve { commit },
        )
    }

    /// One executor for pending/committed reserve and atomic replacement.
    /// Callback failures never substitute for the current ledger verdict.
    fn admit_with_reclaim(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
        action: AdmissionAction<'_>,
    ) -> MemoryResult<ReservationTicket> {
        let broker_identity = Arc::as_ptr(&self.inner) as usize;
        if BrokerCallbackScope::is_active(BrokerCallbackKind::Reclaimer, broker_identity) {
            let message = match action {
                AdmissionAction::Reserve { .. } => {
                    "reentrant reserve from this broker's memory reclaimer"
                }
                AdmissionAction::Replace(_) => {
                    "reentrant recharge from this broker's memory reclaimer"
                }
            };
            return Err(MemoryError::operation_in_progress(message));
        }
        if owner.len() > crate::domain::management_limits::MAX_OWNER_KEY_BYTES {
            return Err(MemoryError::capacity(format!(
                "owner identifier is {} bytes; maximum is {}",
                owner.len(),
                crate::domain::management_limits::MAX_OWNER_KEY_BYTES,
            )));
        }
        let (work, hard_retry) = {
            let mut core = self.lock()?;
            let inputs =
                Self::admission_inputs(&core, &class, &owner, context, kind, bytes, action.old())?;
            match crate::domain::admission::preflight(&inputs) {
                AdmissionVerdict::Blocked { reason } => {
                    core.reclaim_stats.blocked_denials += 1;
                    return Err(reason.into_error());
                }
                AdmissionVerdict::TerminalDenial { breach } => {
                    core.reclaim_stats.terminal_denials += 1;
                    return Err(breach.into_error());
                }
                AdmissionVerdict::Admit => {
                    let request = Self::soft_threshold_request(&core, &inputs)?;
                    let work = request.and_then(|request| {
                        core.reclaimer.clone().map(|callback| (callback, request))
                    });
                    (work, false)
                }
                AdmissionVerdict::ReclaimCandidate { breach } => {
                    let Some(callback) = core.reclaimer.clone() else {
                        core.reclaim_stats.terminal_denials += 1;
                        return Err(breach.into_error());
                    };
                    let request = Self::reclaim_request(
                        &core,
                        &inputs,
                        ReclaimTrigger::CapacityDenial,
                        breach.reclaim_target(),
                    )?;
                    (Some((callback, request)), true)
                }
            }
        };
        if let Some((callback, request)) = work {
            self.run_reclaim(callback, &request);
        }
        // Rebuild/policy/old-ticket may have changed while the lock was absent.
        // No second callback, including after a soft callback worsens pressure.
        let mut core = self.lock()?;
        match Self::operation_preflight(&core, &class, &owner, context, kind, bytes, action.old()) {
            AdmissionVerdict::Admit => match action {
                AdmissionAction::Reserve { commit: true } => {
                    Self::reserve_committed_on(&mut core, class, owner, context, kind, bytes)
                }
                AdmissionAction::Reserve { commit: false } => {
                    Self::reserve_on(&mut core, class, owner, context, kind, bytes)
                }
                AdmissionAction::Replace(old) => {
                    Self::recharge_on(&mut core, old, class, owner, context, kind, bytes)
                }
            },
            AdmissionVerdict::Blocked { reason } => {
                if hard_retry {
                    core.reclaim_stats.blocked_denials += 1;
                }
                Err(reason.into_error())
            }
            AdmissionVerdict::TerminalDenial { breach }
            | AdmissionVerdict::ReclaimCandidate { breach } => {
                if hard_retry {
                    core.reclaim_stats.terminal_denials += 1;
                }
                Err(breach.into_error())
            }
        }
    }
}
