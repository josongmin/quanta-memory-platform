use super::*;

impl AccountingLedger {
    #[must_use]
    pub(crate) fn new(policy: ValidatedResidencyPolicy) -> Self {
        Self {
            policy,
            class_usage: BTreeMap::new(),
            owner_usage: Default::default(),
            context_usage: BTreeMap::new(),
            total_usage: UsageBreakdown::default(),
            non_headroom_bytes: 0,
            instance_id: LEDGER_INSTANCE.fetch_add(1, Ordering::Relaxed),
            ticket_states: BTreeMap::new(),
            next_ticket_id: Some(1),
            rebuild_in_progress: false,
            #[cfg(test)]
            last_release_preflight_axis_probes: 0,
        }
    }

    #[must_use]
    pub fn policy(&self) -> &ResidencyPolicy {
        self.policy.as_policy()
    }

    /// Read-only usage maps for the admission preflight (FUP-003). The
    /// preflight evaluates the same rule engine the attempt path enforces.
    #[must_use]
    pub(crate) fn usage_view(&self) -> crate::domain::admission::UsageView<'_> {
        crate::domain::admission::UsageView {
            class_usage: &self.class_usage,
            owner_usage: {
                #[cfg(test)]
                {
                    OwnerUsageRead(OwnerUsageReadData::Metered(&self.owner_usage))
                }
                #[cfg(not(test))]
                {
                    (&self.owner_usage).into()
                }
            },
            context_usage: &self.context_usage,
            total_usage: &self.total_usage,
        }
    }

    #[must_use]
    pub(crate) fn ticket_id_space_exhausted(&self) -> bool {
        self.next_ticket_id.is_none()
    }

    #[must_use]
    pub(crate) fn live_ticket_limit_reached(&self) -> bool {
        self.ticket_states.len() >= crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
    }

    /// Swap the active residency policy in place, preserving all live usage
    /// counters. This is the live-reconfiguration primitive: the caller is
    /// responsible for having already validated that the new policy does not
    /// shrink any cap below current usage. Counters (class/owner/context) are
    /// left byte-for-byte unchanged, so all accounting invariants are preserved
    /// across the swap.
    pub(crate) fn swap_policy(&mut self, new_policy: ValidatedResidencyPolicy) {
        self.policy = new_policy;
    }

    /// Enter rebuild mode: committed/live writes (reserve/commit/release) are
    /// rejected until [`Self::finish_rebuild`] is called. Rollback of an
    /// incomplete pending reservation is the sole cleanup exception.
    pub fn begin_rebuild(&mut self) {
        self.rebuild_in_progress = true;
    }

    pub fn finish_rebuild(&mut self) {
        self.rebuild_in_progress = false;
    }

    #[must_use]
    pub(crate) fn rebuild_in_progress(&self) -> bool {
        self.rebuild_in_progress
    }

    /// Current usage on a class axis.
    #[must_use]
    pub fn class_usage(&self, class: &MemoryClass) -> UsageBreakdown {
        self.class_usage.get(class).copied().unwrap_or_default()
    }

    /// Current usage on an owner axis.
    #[must_use]
    pub fn owner_usage(&self, owner: &OwnerKey) -> UsageBreakdown {
        self.owner_usage.get(owner).copied().unwrap_or_default()
    }

    /// Exact semantic usage without cloning or scanning reporting maps.
    /// Test-only since the admission engine reads the aggregate through the
    /// preflight usage view (FUP-003).
    #[cfg(test)]
    pub(crate) fn total_semantic_bytes(&self) -> u64 {
        self.total_usage.total()
    }

    /// Exact non-headroom usage without cloning or scanning class maps.
    pub(crate) fn non_headroom_bytes(&self) -> u64 {
        self.non_headroom_bytes
    }

    /// Exact live-scratch usage without cloning or scanning owner maps.
    pub(crate) fn total_live_scratch_bytes(&self) -> u64 {
        self.total_usage.live_scratch_bytes
    }

    /// Canonical legal-state predicate shared by reserve and live reconfigure.
    /// `delta` describes one prospective reservation; `None` validates the
    /// current snapshot against a replacement policy.
    pub(crate) fn validate_policy_postimage(
        policy: &ResidencyPolicy,
        snapshot: &LedgerSnapshot,
        delta: Option<(&MemoryClass, &OwnerKey, u64)>,
    ) -> MemoryResult<()> {
        snapshot.validate_exact()?;
        let delta_bytes = delta.map_or(0, |(_, _, bytes)| bytes);
        let global_after = snapshot
            .total_owner_bytes()
            .checked_add(delta_bytes)
            .ok_or_else(|| MemoryError::capacity("global postimage exceeds u64"))?;
        if global_after > policy.global_hard_limit_bytes {
            return Err(MemoryError::capacity(format!(
                "global hard limit {} below postimage {global_after}",
                policy.global_hard_limit_bytes
            )));
        }

        let class_after = |class: &MemoryClass| -> MemoryResult<u64> {
            let current = match snapshot.class_usage.get(class) {
                Some(usage) => usage.checked_total()?,
                None => 0,
            };
            if delta.is_some_and(|(candidate, _, _)| candidate == class) {
                current
                    .checked_add(delta_bytes)
                    .ok_or_else(|| MemoryError::capacity("class postimage exceeds u64"))
            } else {
                Ok(current)
            }
        };
        let owner_after = |owner: &OwnerKey| -> MemoryResult<u64> {
            let current = match snapshot.owner_usage.get(owner) {
                Some(usage) => usage.checked_total()?,
                None => 0,
            };
            if delta.is_some_and(|(_, candidate, _)| candidate == owner) {
                current
                    .checked_add(delta_bytes)
                    .ok_or_else(|| MemoryError::capacity("owner postimage exceeds u64"))
            } else {
                Ok(current)
            }
        };

        let mut non_headroom_after = 0u64;
        let mut unused_class_floor = 0u64;
        let mut total_borrow = 0u64;
        for (class, budget) in &policy.classes {
            let used = class_after(class)?;
            if *class != MemoryClass::Headroom {
                non_headroom_after = non_headroom_after
                    .checked_add(used)
                    .ok_or_else(|| MemoryError::capacity("non-headroom postimage exceeds u64"))?;
                unused_class_floor = unused_class_floor
                    .checked_add(budget.reserved_bytes.saturating_sub(used))
                    .ok_or_else(|| MemoryError::capacity("class reserved floor exceeds u64"))?;
            }
            if used > budget.hard_cap_bytes {
                if !budget.borrow_from_shared_pool {
                    return Err(MemoryError::capacity(format!(
                        "non-borrowing class {class:?} hard cap {} below postimage {used}",
                        budget.hard_cap_bytes
                    )));
                }
                total_borrow = total_borrow
                    .checked_add(used - budget.hard_cap_bytes)
                    .ok_or_else(|| MemoryError::capacity("class borrow exceeds u64"))?;
            }
        }
        if let Some((class, _, _)) = delta {
            if !policy.classes.contains_key(class) {
                return Err(MemoryError::capacity(format!(
                    "class {class:?} is not configured in the active residency policy"
                )));
            }
        }
        for (class, usage) in &snapshot.class_usage {
            if usage.checked_total()? > 0 && !policy.classes.contains_key(class) {
                return Err(MemoryError::capacity(format!(
                    "live class {class:?} is absent from policy"
                )));
            }
        }
        if total_borrow > policy.shared_burst_pool_bytes {
            return Err(MemoryError::capacity(format!(
                "class borrow {total_borrow} exceeds shared pool {}",
                policy.shared_burst_pool_bytes
            )));
        }
        let non_headroom_band =
            policy
                .global_hard_limit_bytes
                .checked_sub(policy.headroom_bytes)
                .ok_or_else(|| MemoryError::capacity("headroom exceeds global hard limit"))?;
        let protected_postimage = non_headroom_after
            .checked_add(unused_class_floor)
            .ok_or_else(|| MemoryError::capacity("class protected postimage exceeds u64"))?;
        if protected_postimage > non_headroom_band {
            return Err(MemoryError::capacity(format!(
                "non-headroom usage {non_headroom_after} plus unused class floors \
                 {unused_class_floor} exceeds band {non_headroom_band}"
            )));
        }

        for (owner, quota) in &policy.owner_quotas {
            let used = owner_after(owner)?;
            if used > quota.hard_cap_bytes {
                return Err(MemoryError::capacity(format!(
                    "owner {owner} hard cap {} below postimage {used}",
                    quota.hard_cap_bytes
                )));
            }
        }
        Ok(())
    }

    /// Reserve `bytes` against (class, owner, context).
    ///
    /// Fail-closed: every cap check happens BEFORE any counter mutation, so a
    /// denied reserve leaves the ledger byte-for-byte unchanged.
    ///
    /// Cap precedence (outermost first): rebuild gate, global hard limit
    /// (invariant 5, headroom-aware), owner hard cap, class hard cap — which a
    /// borrowing class may exceed only up to the shared burst pool (invariant 6).
    pub fn reserve(
        &mut self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        if self.rebuild_in_progress {
            return Err(MemoryError::InvalidState(
                "rebuild in progress: live reserve rejected".to_string(),
            ));
        }

        // Identifier exhaustion is permanent for this ledger. Check it before
        // byte caps so an impossible request cannot provoke host reclaim.
        let id = self.next_ticket_id.ok_or_else(|| {
            MemoryError::InvalidState("reservation ticket id space exhausted".to_string())
        })?;

        // This authority bound is independent of byte budgets: admitting a
        // zero-byte ticket still grows ticket state and per-ticket bookkeeping.
        if self.live_ticket_limit_reached() {
            return Err(MemoryError::capacity(format!(
                "live reservation ticket limit {} reached",
                crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER
            )));
        }
        let owner_key_bytes = owner.len();
        if owner_key_bytes > crate::domain::management_limits::MAX_OWNER_KEY_BYTES {
            return Err(MemoryError::capacity(format!(
                "owner identifier is {owner_key_bytes} bytes; maximum is {}",
                crate::domain::management_limits::MAX_OWNER_KEY_BYTES
            )));
        }

        // --- pure checks, no mutation yet ---
        //
        // The admission engine is the single rule implementation shared with
        // the broker preflight (FUP-003): same checks, same order, same
        // byte-identical errors. This is equivalent to
        // `validate_policy_postimage(..., Some(delta))` plus the
        // reserve-specific overflow guards, without cloning maps.

        let usage = self.usage_view();
        let request = crate::domain::admission::AdmissionRequest {
            class: &class,
            owner: &owner,
            context,
            kind,
            bytes,
        };
        crate::domain::admission::check_ledger_postimage(self.policy(), &usage, &request)
            .map_err(|failure| failure.into_error())?;
        // --- all checks passed: mutate ---
        self.next_ticket_id = id.checked_add(1);
        self.ticket_states.insert(id, TicketState::Pending);

        // A zero-byte reserve is a real, live, single-use ticket but must not
        // create empty axis entries, so snapshots stay canonical (a 0-byte
        // reserve+release is a true no-op on the counters).
        if bytes > 0 {
            self.class_usage.entry(class.clone()).or_default().add(kind, bytes);
            self.owner_usage.entry(owner.clone()).or_default().add(kind, bytes);
            self.context_usage.entry(context).or_default().add(kind, bytes);
            self.total_usage.add(kind, bytes);
            if class != MemoryClass::Headroom {
                self.non_headroom_bytes = self
                    .non_headroom_bytes
                    .checked_add(bytes)
                    .expect("ledger reserve preflight checked non-headroom aggregate");
            }
        }

        Ok(ReservationTicket { id, ledger: self.instance_id, class, owner, context, kind, bytes })
    }

    /// Commit a reservation. In this model a committed reservation keeps its
    /// charge (reserve already moved the counters); commit only finalizes the
    /// ticket. It returns the now-charged byte count for caller bookkeeping.
    ///
    /// Commit is rejected during rebuild because finalizing a charge is a live
    /// write against the active accounting state.
    pub fn commit(&mut self, ticket: &ReservationTicket) -> MemoryResult<u64> {
        if self.rebuild_in_progress {
            return Err(MemoryError::InvalidState(
                "rebuild in progress: live commit rejected".to_string(),
            ));
        }
        // Invariant 7: a ticket issued by a different ledger (even one sharing
        // this ledger's id space) is never live here.
        if ticket.ledger != self.instance_id {
            return Err(MemoryError::InvalidState(format!(
                "commit rejected: ticket {} was issued by a different ledger",
                ticket.id
            )));
        }
        match self.ticket_states.get_mut(&ticket.id) {
            Some(state @ TicketState::Pending) => {
                *state = TicketState::Committed;
                Ok(ticket.bytes)
            }
            Some(TicketState::Committed) => Err(MemoryError::InvalidState(format!(
                "commit rejected: ticket {} is already committed",
                ticket.id
            ))),
            None => Err(MemoryError::InvalidState(format!(
                "commit rejected: ticket {} is not live (already released or never issued)",
                ticket.id
            ))),
        }
    }

    /// Cancel a pending reservation even while rebuild fencing is active.
    /// This is rollback of an incomplete write, not a new live write.
    pub(crate) fn rollback_pending(&mut self, ticket: &ReservationTicket) -> MemoryResult<()> {
        self.validate_ledger_identity(ticket, "pending rollback")?;
        match self.ticket_states.get(&ticket.id) {
            Some(TicketState::Pending) => {
                self.preflight_release_batch(std::slice::from_ref(ticket))?;
                self.apply_release(ticket);
                Ok(())
            }
            Some(TicketState::Committed) => Err(MemoryError::InvalidState(format!(
                "pending rollback rejected: ticket {} is already committed",
                ticket.id
            ))),
            None => Err(MemoryError::InvalidState(format!(
                "pending rollback rejected: ticket {} is not live",
                ticket.id
            ))),
        }
    }

    /// Release a reservation/charge, subtracting from class, owner, and context
    /// usage. Used both for rolled-back reservations and for committed bytes
    /// that are later deleted/evicted.
    pub fn release(&mut self, ticket: &ReservationTicket) -> MemoryResult<()> {
        self.validate_release_ticket(ticket)?;
        self.preflight_release_batch(std::slice::from_ref(ticket))?;
        self.apply_release(ticket);
        Ok(())
    }

    /// Replace one live committed charge with another as one ledger mutation.
    ///
    /// The old ticket is removed before admission so an overwrite is charged on
    /// the net postimage, but a denied replacement restores that exact ticket by
    /// direct owner-state rollback. Rollback never re-enters admission and cannot
    /// fail: no policy, scheduler, or external callback participates in it.
    pub(crate) fn replace_committed(
        &mut self,
        old: &ReservationTicket,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        self.validate_committed_ticket(old)?;
        self.preflight_release_batch(std::slice::from_ref(old))?;
        self.apply_release(old);

        let replacement = match self.reserve(class, owner, context, kind, bytes) {
            Ok(ticket) => ticket,
            Err(error) => {
                self.restore_released_ticket_v1(old);
                return Err(error);
            }
        };
        if let Err(error) = self.commit(&replacement) {
            self.apply_release(&replacement);
            self.restore_released_ticket_v1(old);
            return Err(error);
        }
        Ok(replacement)
    }

    /// Release a batch of reservations/charges atomically.
    ///
    /// The full batch is pre-validated before any counter moves, so a rebuild or
    /// invalid ticket cannot partially refund the ledger.
    pub fn release_batch(&mut self, tickets: &[ReservationTicket]) -> MemoryResult<()> {
        // An empty batch is a true no-op: it touches no counters, so it must
        // succeed even during a rebuild. (A charged store's no-op sweep/truncate
        // that reclaimed nothing would otherwise be wrongly rejected mid-rebuild.)
        if tickets.is_empty() {
            return Ok(());
        }
        if self.rebuild_in_progress {
            return Err(MemoryError::InvalidState(
                "rebuild in progress: live release rejected".to_string(),
            ));
        }
        let mut seen = BTreeSet::new();
        for ticket in tickets {
            self.validate_release_ticket(ticket)?;
            if !seen.insert(ticket.id) {
                return Err(MemoryError::InvalidState(format!(
                    "release rejected: duplicate ticket {} in one batch",
                    ticket.id
                )));
            }
        }
        self.preflight_release_batch(tickets)?;
        for ticket in tickets {
            self.apply_release(ticket);
        }
        Ok(())
    }

    fn preflight_release_batch(&mut self, tickets: &[ReservationTicket]) -> MemoryResult<()> {
        #[cfg(test)]
        {
            self.last_release_preflight_axis_probes = 0;
        }
        #[cfg(test)]
        let owner_probes = self.owner_usage.start_probes();
        let mut class_refunds = BTreeMap::<(MemoryClass, ChargeKind), u64>::new();
        let mut owner_refunds = BTreeMap::<(OwnerKey, ChargeKind), u64>::new();
        let mut context_refunds = BTreeMap::<(MemoryContextKind, ChargeKind), u64>::new();
        let mut total_refunds = UsageBreakdown::default();
        let mut non_headroom_refund = 0u64;
        for ticket in tickets {
            if ticket.bytes == 0 {
                continue;
            }
            for (slot, label) in [
                (
                    class_refunds.entry((ticket.class.clone(), ticket.kind)).or_default(),
                    "class release batch exceeds u64",
                ),
                (
                    owner_refunds.entry((ticket.owner.clone(), ticket.kind)).or_default(),
                    "owner release batch exceeds u64",
                ),
            ] {
                *slot = slot
                    .checked_add(ticket.bytes)
                    .ok_or_else(|| MemoryError::InvalidState(label.into()))?;
            }
            let context_slot = context_refunds.entry((ticket.context, ticket.kind)).or_default();
            *context_slot = context_slot.checked_add(ticket.bytes).ok_or_else(|| {
                MemoryError::InvalidState("context release batch exceeds u64".into())
            })?;
            total_refunds.add(ticket.kind, ticket.bytes);
            if ticket.class != MemoryClass::Headroom {
                non_headroom_refund =
                    non_headroom_refund.checked_add(ticket.bytes).ok_or_else(|| {
                        MemoryError::InvalidState("non-headroom release batch exceeds u64".into())
                    })?;
            }
        }

        for ((class, kind), bytes) in &class_refunds {
            #[cfg(test)]
            {
                self.last_release_preflight_axis_probes += 1;
            }
            let usage = self
                .class_usage
                .get(class)
                .ok_or_else(|| MemoryError::InvalidState("release class axis is missing".into()))?;
            usage.slot(*kind).checked_sub(*bytes).ok_or_else(|| {
                MemoryError::InvalidState("release exceeds charged class slot".into())
            })?;
        }
        for ((owner, kind), bytes) in &owner_refunds {
            let usage = self
                .owner_usage
                .get(owner)
                .ok_or_else(|| MemoryError::InvalidState("release owner axis is missing".into()))?;
            usage.slot(*kind).checked_sub(*bytes).ok_or_else(|| {
                MemoryError::InvalidState("release exceeds charged owner slot".into())
            })?;
        }
        for ((context, kind), bytes) in &context_refunds {
            #[cfg(test)]
            {
                self.last_release_preflight_axis_probes += 1;
            }
            let usage = self.context_usage.get(context).ok_or_else(|| {
                MemoryError::InvalidState("release context axis is missing".into())
            })?;
            usage.slot(*kind).checked_sub(*bytes).ok_or_else(|| {
                MemoryError::InvalidState("release exceeds charged context slot".into())
            })?;
        }
        for kind in
            [ChargeKind::RetainedPayload, ChargeKind::RetainedMetadata, ChargeKind::LiveScratch]
        {
            self.total_usage.slot(kind).checked_sub(total_refunds.slot(kind)).ok_or_else(|| {
                MemoryError::InvalidState("release exceeds charged aggregate slot".into())
            })?;
        }
        self.non_headroom_bytes.checked_sub(non_headroom_refund).ok_or_else(|| {
            MemoryError::InvalidState("release exceeds non-headroom aggregate".into())
        })?;
        #[cfg(test)]
        {
            self.last_release_preflight_axis_probes += owner_probes.key_visits();
        }
        Ok(())
    }

    fn validate_release_ticket(&self, ticket: &ReservationTicket) -> MemoryResult<()> {
        if self.rebuild_in_progress {
            return Err(MemoryError::InvalidState(
                "rebuild in progress: live release rejected".to_string(),
            ));
        }
        // Invariant 7, fail-closed BEFORE mutation. A ticket from a different
        // ledger is rejected first (its id may collide with a live local id),
        // then a replayed/cloned/forged ticket is rejected by the live set, so
        // usage can never drift below truth.
        self.validate_ledger_identity(ticket, "release")?;
        if !self.ticket_states.contains_key(&ticket.id) {
            return Err(MemoryError::InvalidState(format!(
                "release rejected: ticket {} is not live (already released or never issued)",
                ticket.id
            )));
        }
        Ok(())
    }

    fn validate_committed_ticket(&self, ticket: &ReservationTicket) -> MemoryResult<()> {
        self.validate_release_ticket(ticket)?;
        if self.ticket_states.get(&ticket.id) != Some(&TicketState::Committed) {
            return Err(MemoryError::InvalidState(format!(
                "operation requires committed ticket {}, but it is pending",
                ticket.id
            )));
        }
        Ok(())
    }

    fn validate_ledger_identity(
        &self,
        ticket: &ReservationTicket,
        operation: &str,
    ) -> MemoryResult<()> {
        if ticket.ledger != self.instance_id {
            return Err(MemoryError::InvalidState(format!(
                "{operation} rejected: ticket {} was issued by a different ledger",
                ticket.id
            )));
        }
        Ok(())
    }

    fn apply_release(&mut self, ticket: &ReservationTicket) {
        self.ticket_states.remove(&ticket.id);
        // Subtract the ticket's exact byte magnitude with a `u64` (never an
        // `i64` delta): a ticket with `bytes > i64::MAX` must subtract its full
        // magnitude, not a wrapped value. After subtracting, prune any axis entry
        // that reached zero so snapshots stay canonical.
        if let Some(usage) = self.class_usage.get_mut(&ticket.class) {
            usage.sub(ticket.kind, ticket.bytes);
        }
        if self.class_usage.get(&ticket.class).is_some_and(|u| u.total() == 0) {
            self.class_usage.remove(&ticket.class);
        }
        if let Some(usage) = self.owner_usage.get_mut(&ticket.owner) {
            usage.sub(ticket.kind, ticket.bytes);
        }
        if self.owner_usage.get(&ticket.owner).is_some_and(|u| u.total() == 0) {
            self.owner_usage.remove(&ticket.owner);
        }
        if let Some(usage) = self.context_usage.get_mut(&ticket.context) {
            usage.sub(ticket.kind, ticket.bytes);
        }
        if self.context_usage.get(&ticket.context).is_some_and(|u| u.total() == 0) {
            self.context_usage.remove(&ticket.context);
        }
        self.total_usage.sub(ticket.kind, ticket.bytes);
        if ticket.class != MemoryClass::Headroom {
            self.non_headroom_bytes = self
                .non_headroom_bytes
                .checked_sub(ticket.bytes)
                .expect("ledger release preflight checked non-headroom aggregate");
        }
    }

    /// Restore a ticket removed by this ledger's in-lock replacement primitive.
    /// The caller has already validated the ticket and no operation can
    /// interleave, so this is a deterministic inverse of `apply_release` rather
    /// than a second admission attempt.
    fn restore_released_ticket_v1(&mut self, ticket: &ReservationTicket) {
        self.ticket_states.insert(ticket.id, TicketState::Committed);
        if ticket.bytes == 0 {
            return;
        }
        self.class_usage.entry(ticket.class.clone()).or_default().add(ticket.kind, ticket.bytes);
        self.owner_usage.entry(ticket.owner.clone()).or_default().add(ticket.kind, ticket.bytes);
        self.context_usage.entry(ticket.context).or_default().add(ticket.kind, ticket.bytes);
        self.total_usage.add(ticket.kind, ticket.bytes);
        if ticket.class != MemoryClass::Headroom {
            self.non_headroom_bytes = self
                .non_headroom_bytes
                .checked_add(ticket.bytes)
                .expect("released ticket restoration returns a previously charged aggregate");
        }
    }

    /// Immutable usage snapshot for reporting and invariant assertions.
    #[must_use]
    pub fn snapshot(&self) -> LedgerSnapshot {
        LedgerSnapshot {
            class_usage: self.class_usage.clone(),
            owner_usage: {
                #[cfg(test)]
                {
                    self.owner_usage.iter().map(|(owner, usage)| (owner.clone(), *usage)).collect()
                }
                #[cfg(not(test))]
                {
                    self.owner_usage.clone()
                }
            },
            context_usage: self.context_usage.clone(),
            live_ticket_count: self.ticket_states.len(),
            rebuild_in_progress: self.rebuild_in_progress,
        }
    }
}
