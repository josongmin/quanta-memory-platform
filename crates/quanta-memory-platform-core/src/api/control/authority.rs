use super::*;

impl LiveProcessMemoryBroker {
    /// Release on an already-held lock guard (ledger release + group bookkeeping).
    /// Scheduler state is preflighted and transactionally restored if the
    /// ledger rejects the ticket, avoiding a full scheduler clone.
    pub(super) fn attribution_for(
        core: &BrokerCore,
        ticket: &ReservationTicket,
    ) -> MemoryResult<TicketAttribution> {
        let attribution = core.ticket_attribution.get(&ticket.id()).ok_or_else(|| {
            MemoryError::InvalidState("live ticket has no issuing-scope attribution".into())
        })?;
        if attribution.owner != *ticket.owner()
            || attribution.class != *ticket.class()
            || attribution.bytes != ticket.bytes()
        {
            return Err(MemoryError::InvalidState(
                "live ticket issuing-scope attribution disagrees with authority".into(),
            ));
        }
        Ok(attribution.clone())
    }

    pub(super) fn release_on(
        core: &mut BrokerCore,
        ticket: &ReservationTicket,
    ) -> MemoryResult<()> {
        let attribution = Self::attribution_for(core, ticket)?;
        if let Some(group) = &attribution.group {
            core.scheduler.usage_after_release(group, ticket.bytes())?;
            core.scheduler.record_release(group, ticket.class(), ticket.bytes())?;
        }
        if let Err(error) = core.ledger.release(ticket) {
            if let Some(group) = &attribution.group {
                core.scheduler.record_usage(group, ticket.class(), ticket.bytes()).map_err(|rollback| {
                    MemoryError::InvalidState(format!(
                        "ledger release failed ({error}); scheduler rollback also failed ({rollback})"
                    ))
                })?;
            }
            return Err(error);
        }
        core.ticket_attribution.remove(&ticket.id());
        Ok(())
    }

    pub(super) fn rollback_pending_on(
        core: &mut BrokerCore,
        ticket: &ReservationTicket,
    ) -> MemoryResult<()> {
        let attribution = Self::attribution_for(core, ticket)?;
        if let Some(group) = &attribution.group {
            core.scheduler.usage_after_release(group, ticket.bytes())?;
            core.scheduler.record_release(group, ticket.class(), ticket.bytes())?;
        }
        if let Err(error) = core.ledger.rollback_pending(ticket) {
            if let Some(group) = &attribution.group {
                core.scheduler.record_usage(group, ticket.class(), ticket.bytes()).map_err(|rollback| {
                    MemoryError::InvalidState(format!(
                        "ledger pending rollback failed ({error}); scheduler rollback also failed ({rollback})"
                    ))
                })?;
            }
            return Err(error);
        }
        core.ticket_attribution.remove(&ticket.id());
        Ok(())
    }

    pub(super) fn rollback_pending_ticket(&self, ticket: &ReservationTicket) -> MemoryResult<()> {
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
    pub(super) fn rollback_pending_ticket_on_drop(&self, ticket: &ReservationTicket) {
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
    pub(super) fn drain_deferred_pending_rollbacks_batch(core: &mut BrokerCore) -> usize {
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

    pub(super) fn drain_deferred_committed_releases_batch(
        core: &mut BrokerCore,
    ) -> MemoryResult<usize> {
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
    pub(super) fn release_managed_charge_on_drop(&self, charge: CommittedCharge) {
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

    pub(super) fn release_many_on(
        core: &mut BrokerCore,
        tickets: &[ReservationTicket],
    ) -> MemoryResult<()> {
        // Validate and stage both scheduler axes before touching the ledger.
        // A batch may mix Headroom and non-Headroom tickets in one group, so a
        // scalar group refund would erase the protected-floor distinction.
        let mut next_scheduler = core.scheduler.clone();
        for ticket in tickets {
            let attribution = Self::attribution_for(core, ticket)?;
            if let Some(group) = &attribution.group {
                next_scheduler.record_release(group, ticket.class(), ticket.bytes())?;
            }
        }
        core.ledger.release_batch(tickets)?;
        core.scheduler = next_scheduler;
        for ticket in tickets {
            core.ticket_attribution.remove(&ticket.id());
        }
        Ok(())
    }

    /// Shared reserve path on an already-held lock guard (admission, the
    /// execution-scratch budget, the ledger reserve, and scheduler/ticket
    /// bookkeeping), so `reserve` and `reserve_committed` compose it without
    /// re-locking.
    pub(super) fn reserve_on(
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

        if core.ledger.ticket_id_space_exhausted() {
            return Err(MemoryError::InvalidState(
                "reservation ticket id space exhausted".to_string(),
            ));
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
        if let Some(group_name) = &group {
            if let Err(error) = core.scheduler.record_usage(group_name, ticket.class(), bytes) {
                core.ledger.rollback_pending(&ticket).map_err(|rollback| {
                    MemoryError::InvalidState(format!(
                        "scheduler commit failed ({error}); ledger rollback also failed ({rollback})"
                    ))
                })?;
                return Err(error);
            }
        }
        core.ticket_attribution.insert(ticket.id(), TicketAttribution::new(&ticket, group));
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
    /// The whole slice is preflighted before any proportional ticket vector
    /// is built, in a fixed precedence: batch length against the broker's
    /// live-ticket cap (a longer active batch cannot exist), broker affinity,
    /// active lease state, then the checked byte sum. Any validation or
    /// rebuild failure leaves every lease unchanged. An empty batch is a
    /// successful no-op.
    pub fn release_managed_many(&self, charges: &mut [ManagedCharge]) -> MemoryResult<()> {
        // Advisory event payload only: a saturating sum so preflight failures
        // keep reporting the batch's byte magnitude. The authoritative
        // checked sum below still runs after the active-lease scan and can
        // never mask an inactive lease as zero.
        let advisory_bytes = charges
            .iter()
            .map(|charge| charge.bytes().unwrap_or(0))
            .fold(0u64, u64::saturating_add);
        // Preflight with no ticket vector: an invalid batch is rejected before
        // an O(batch length) allocation. A batch larger than the live-ticket
        // cap can never be a valid active batch of this broker.
        if charges.len() > crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER {
            let result = Err(MemoryError::InvalidState(format!(
                "managed release batch of {} leases exceeds the maximum live tickets per broker ({})",
                charges.len(),
                crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
            )));
            self.emit_result(MemoryOperation::Release, Some(advisory_bytes), &result);
            return result;
        }
        for charge in charges.iter() {
            if !Arc::ptr_eq(&self.inner, &charge.broker.inner) {
                let result = Err(MemoryError::InvalidState(
                    "managed charge batch contains a foreign broker lease".to_string(),
                ));
                self.emit_result(MemoryOperation::Release, Some(advisory_bytes), &result);
                return result;
            }
            if charge.charge.is_none() {
                let result = Err(MemoryError::InvalidState(
                    "managed charge batch contains an already released lease".to_string(),
                ));
                self.emit_result(MemoryOperation::Release, Some(advisory_bytes), &result);
                return result;
            }
        }
        // Every lease is now proven active, so `bytes()` cannot silently map
        // an inactive lease onto a zero-byte release.
        let bytes = charges.iter().try_fold(0u64, |sum, charge| {
            sum.checked_add(charge.bytes().expect("preflight proved the lease active")).ok_or_else(
                || MemoryError::InvalidState("managed release byte total exceeds u64".into()),
            )
        });
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(error) => {
                let result = Err(error);
                self.emit_result(MemoryOperation::Release, Some(advisory_bytes), &result);
                return result;
            }
        };
        let mut tickets = Vec::with_capacity(charges.len());
        for charge in charges.iter() {
            tickets.push(
                charge.charge.as_ref().expect("preflight proved the lease active").ticket.clone(),
            );
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

    /// Enter rebuild mode and mint the sole authority that may finish it.
    ///
    /// The returned [`RebuildFence`] is intentionally not `Clone` and is
    /// bound to this broker's current rebuild epoch:
    ///
    /// - a second `begin_rebuild` while a live fence is held is rejected
    ///   (one coordinator owns one fence);
    /// - a fence left behind by a dropped permit is **abandoned**: writes
    ///   stay rejected and the state is observable via
    ///   [`Self::rebuild_fence_status`]; an explicit new `begin_rebuild`
    ///   re-adopts it under a fresh epoch (deterministic recovery that
    ///   never silently reopens writes);
    /// - `RebuildFence::finish` verifies the epoch, so a stale or foreign
    ///   permit cannot open another coordinator's fence.
    pub fn begin_rebuild(&self) -> MemoryResult<RebuildFence> {
        let mut core = self.lock()?;
        if core.ledger.rebuild_in_progress() && !core.rebuild_abandoned {
            return Err(MemoryError::InvalidState(
                "rebuild already in progress: a second live coordinator cannot open over \
                 the active fence"
                    .to_string(),
            ));
        }
        // Resolve every fallible step BEFORE mutating any fence state: an
        // exhausted epoch must not strand a fence with no permit and must
        // not flip an abandoned fence to active on the Err path.
        let epoch = core
            .rebuild_epoch
            .checked_add(1)
            .ok_or_else(|| MemoryError::InvalidState("rebuild epoch exhausted u64".to_string()))?;
        // Explicit recovery: re-adopt the abandoned fence under a new epoch.
        // Deferred authorities stay queued and exact; the clear happens only
        // now that every fallible step has succeeded.
        core.rebuild_abandoned = false;
        core.ledger.begin_rebuild();
        core.rebuild_epoch = epoch;
        drop(core);
        Ok(RebuildFence { broker: self.clone(), epoch, finished: false })
    }

    /// Observable rebuild fence state: no fence, one live fence, or a fence
    /// abandoned by a dropped permit (writes still rejected).
    pub fn rebuild_fence_status(&self) -> MemoryResult<RebuildFenceStatus> {
        let core = self.lock()?;
        Ok(if !core.ledger.rebuild_in_progress() {
            RebuildFenceStatus::Unfenced
        } else if core.rebuild_abandoned {
            RebuildFenceStatus::Abandoned
        } else {
            RebuildFenceStatus::Active
        })
    }

    /// Epoch-bound fence completion used by [`RebuildFence::finish`].
    pub(super) fn finish_rebuild_epoch(&self, epoch: u64) -> MemoryResult<RebuildReceipt> {
        {
            let mut core = self.lock()?;
            if !core.ledger.rebuild_in_progress() {
                return Err(MemoryError::InvalidState(
                    "no rebuild fence is active to finish".to_string(),
                ));
            }
            if core.rebuild_epoch != epoch {
                return Err(MemoryError::InvalidState(format!(
                    "stale or foreign rebuild fence: permit epoch {epoch}, active epoch {}",
                    core.rebuild_epoch
                )));
            }
            core.ledger.finish_rebuild();
            core.rebuild_abandoned = false;
        }
        // The fence is open. Drain deferred authorities in bounded batches,
        // reporting the cleanup outcome separately from the fence transition.
        let (pending, committed, drain_error) = self.drain_deferred_queues();
        Ok(RebuildReceipt {
            fence_open: true,
            pending_rollbacks_remaining: pending,
            committed_releases_remaining: committed,
            drain_error,
        })
    }

    /// Mark a dropped (unfinished) fence abandoned if it still owns the
    /// current epoch; a stale permit must not abandon a newer coordinator's
    /// fence.
    pub(super) fn abandon_rebuild_epoch(&self, epoch: u64) {
        let mut core = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if core.ledger.rebuild_in_progress() && core.rebuild_epoch == epoch {
            core.rebuild_abandoned = true;
        }
    }

    /// Bounded-batch drain of the deferred queues. Returns the remaining
    /// queue depths and the first drain error, if any. A drain interrupted
    /// by another coordinator's fence reports a distinct typed error instead
    /// of blaming this caller's own release.
    pub(super) fn drain_deferred_queues(&self) -> (usize, usize, Option<MemoryError>) {
        let mut drain_error: Option<MemoryError> = None;
        loop {
            let mut core = match self.lock() {
                Ok(core) => core,
                Err(error) => return (usize::MAX, usize::MAX, Some(error)),
            };
            if drain_error.is_none() && core.ledger.rebuild_in_progress() {
                // Another coordinator re-entered a fence between our batches.
                // The queues are preserved; report it distinctly instead of
                // failing a release with a misleading "live release rejected".
                drain_error = Some(MemoryError::InvalidState(
                    "rebuild fence re-entered by another coordinator; deferred authorities \
                     remain queued"
                        .to_string(),
                ));
                return (
                    core.deferred_pending_rollbacks.len(),
                    core.deferred_committed_releases.len(),
                    drain_error,
                );
            }
            if core.deferred_pending_rollbacks.is_empty()
                && core.deferred_committed_releases.is_empty()
            {
                return (
                    core.deferred_pending_rollbacks.len(),
                    core.deferred_committed_releases.len(),
                    drain_error,
                );
            }
            let queued_before =
                core.deferred_pending_rollbacks.len() + core.deferred_committed_releases.len();
            let pending_resolved = Self::drain_deferred_pending_rollbacks_batch(&mut core);
            let committed_released = match Self::drain_deferred_committed_releases_batch(&mut core)
            {
                Ok(released) => released,
                Err(error) => {
                    drain_error = Some(error);
                    return (
                        core.deferred_pending_rollbacks.len(),
                        core.deferred_committed_releases.len(),
                        drain_error,
                    );
                }
            };
            let queued_after =
                core.deferred_pending_rollbacks.len() + core.deferred_committed_releases.len();
            if queued_after >= queued_before && pending_resolved == 0 && committed_released == 0 {
                drain_error = Some(MemoryError::InvalidState(format!(
                    "rebuild cleanup made no progress; {queued_after} deferred ticket \
                     authorities remain",
                )));
                return (
                    core.deferred_pending_rollbacks.len(),
                    core.deferred_committed_releases.len(),
                    drain_error,
                );
            }
        }
    }
}
