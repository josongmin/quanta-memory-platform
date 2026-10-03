use super::*;

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

        // Same reentry bound as a reentrant reserve: a recharge is a
        // synchronous release+reserve+commit swap, so a callback running for
        // this broker must not grow or shrink charges reentrantly. The old
        // charge is untouched (OldChargePreserved).
        let broker_identity = Arc::as_ptr(&self.inner) as usize;
        if BrokerCallbackScope::is_active(BrokerCallbackKind::Reclaimer, broker_identity) {
            return Err(RechargeError {
                error: MemoryError::operation_in_progress(
                    "reentrant recharge from this broker's memory reclaimer",
                ),
                recovery: RechargeRecoveryV1::OldChargePreserved,
            });
        }

        let mut core = self.lock().map_err(|error| RechargeError {
            error,
            recovery: RechargeRecoveryV1::OldChargePreserved,
        })?;

        // Validate resource-group admission against the replacement postimage,
        // not against a transient double charge. A cloned candidate gives us an
        // infallible scheduler commit after the ledger replacement succeeds.
        let old_attribution = Self::attribution_for(&core, old).map_err(|error| RechargeError {
            error,
            recovery: RechargeRecoveryV1::OldChargePreserved,
        })?;
        let old_group = old_attribution.group;
        let new_group = core.ledger.policy().owner_groups.get(&owner).cloned();
        let mut next_scheduler = core.scheduler.clone();
        if let Some(group) = &old_group {
            next_scheduler.record_release(group, old.class(), old.bytes()).map_err(|error| {
                RechargeError { error, recovery: RechargeRecoveryV1::OldChargePreserved }
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
            next_scheduler.record_usage(group, &class, bytes).map_err(|error| RechargeError {
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

    pub(super) fn soft_threshold_request(
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

    pub(super) fn reclaim_request(
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

    /// Pure admission preflight over locked state (FUP-003): evaluates the
    /// shared admission engine against the live ledger, scheduler, and report
    /// without mutating anything. The verdict decides the reclaim plan BEFORE
    /// any callback can fire, so terminally impossible and blocked requests
    /// never touch a host callback.
    pub(super) fn admission_preflight(
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
            group_current_non_headroom_usage: owner_group
                .map_or(0, |group| core.scheduler.current_non_headroom_usage(group)),
            unused_group_floors: core.scheduler.unused_reserved_floor_bytes(),
            owner_group,
            request: AdmissionRequest { class, owner, context, kind, bytes },
            rebuild_in_progress: core.ledger.rebuild_in_progress(),
            ticket_id_space_exhausted: core.ledger.ticket_id_space_exhausted(),
            live_ticket_limit_reached: core.ledger.live_ticket_limit_reached(),
        };
        crate::domain::admission::preflight(&inputs)
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
}
