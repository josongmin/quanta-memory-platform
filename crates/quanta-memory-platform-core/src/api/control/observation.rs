use super::*;

impl LiveProcessMemoryBroker {
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
    /// holds bytes whose new hard cap is below its usage, or any class with a
    /// live ticket (including a zero-byte ticket) dropped from the new policy;
    /// any owner quota in the new policy below live owner usage; any
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

    pub(super) fn try_live_reconfigure_inner(
        &self,
        new_policy: ResidencyPolicy,
    ) -> MemoryResult<()> {
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
        for (ticket_id, attribution) in &core.ticket_attribution {
            if !new_policy.classes.contains_key(&attribution.class) {
                return Err(MemoryError::InvalidState(format!(
                    "live reconfig denied: class {:?} still owns live ticket {ticket_id}",
                    attribution.class
                )));
            }
            *live_ticket_owners.entry(&attribution.owner).or_insert(0) += 1;
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
            // Removing an owner's quota would silently uncap a live owner,
            // exactly like dropping a live class from the policy; deny it
            // symmetric with the class-drop rule.
            if core.ledger.policy().owner_quotas.contains_key(owner)
                && !new_policy.owner_quotas.contains_key(owner)
            {
                return Err(MemoryError::InvalidState(format!(
                    "live reconfig denied: owner {owner} holds {live_tickets} live ticket(s) \
                     ({owner_used} live bytes); removing its quota would silently uncap it"
                )));
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
        for attribution in core.ticket_attribution.values() {
            if let Some(group) = new_policy.owner_groups.get(&attribution.owner) {
                let slot = live_group_usage.entry(group).or_insert(0);
                *slot = slot.checked_add(attribution.bytes).ok_or_else(|| {
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
        }
        // The marginal owner/class snapshots cannot reconstruct which class
        // consumed a given group's bytes. Replay the exact live ticket joint
        // axis, preserving Headroom's participation in group caps but not in
        // non-headroom reserved-floor consumption.
        for attribution in core.ticket_attribution.values() {
            if let Some(group) = new_policy.owner_groups.get(&attribution.owner) {
                candidate_scheduler.record_usage(group, &attribution.class, attribution.bytes)?;
            }
        }
        candidate_scheduler
            .protected_floor_fits(non_headroom_used, new_non_headroom_ceiling)
            .map_err(|error| MemoryError::InvalidState(format!("live reconfig denied: {error}")))?;

        // Accepted: install the new policy by rebuilding the ledger's view while
        // preserving usage counters.
        core.install_policy(validated, OverrideSource::RuntimeReconfigure, candidate_scheduler)?;
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
