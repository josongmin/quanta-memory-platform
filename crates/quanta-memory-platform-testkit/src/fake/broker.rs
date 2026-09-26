//! In-process fake broker substrate.
//!
//! The fake broker is a self-contained reservation/accounting model that obeys
//! the SAME accounting invariants as the real core broker
//! (`LiveProcessMemoryBroker`):
//!
//! 1. `sum(class usage) == sum(owner usage)`
//! 2. `sum(owner usage) == retained payload + retained metadata + live scratch`
//! 3. live writes are rejected while a rebuild window is open
//!
//! It additionally models the failure surfaces later tickets need to drive
//! generically: pressure states, owner/group quota deny, a rebuild-in-progress
//! window, and reclaim callbacks.
//!
//! Crucially, the fake can be deliberately *broken* (see [`AccountingFidelity`])
//! so the conformance suite can prove it catches an accounting mismatch instead
//! of rubber-stamping it. A conformance suite that cannot reject a broken fake is
//! worthless.

use std::collections::BTreeMap;

use quanta_memory_platform_core::{
    ChargeKind, MemoryClass, MemoryContextKind, MemoryError, MemoryResult, OwnerKey, PressureState,
};

/// A tentative reservation handle issued by the fake broker. Holding one means
/// bytes were provisioned against a class/owner; it must be released to return
/// the bytes (the fake does not distinguish reserve-vs-commit beyond the real
/// model's `commit` no-op, mirroring the core ledger). After partial reclaim,
/// the original handle owns only the unreclaimed remainder. Full reclaim ends
/// its authority, so subsequent commit/release rejects it as stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeReservation {
    id: u64,
    class: MemoryClass,
    owner: OwnerKey,
    group: Option<String>,
    kind: ChargeKind,
    bytes: u64,
}

impl FakeReservation {
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    #[must_use]
    pub fn owner(&self) -> &OwnerKey {
        &self.owner
    }
    #[must_use]
    pub fn class(&self) -> &MemoryClass {
        &self.class
    }
    #[must_use]
    pub fn kind(&self) -> ChargeKind {
        self.kind
    }
}

/// Per-charge-kind usage split for one accounting axis in the fake.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FakeUsage {
    pub retained_payload_bytes: u64,
    pub retained_metadata_bytes: u64,
    pub live_scratch_bytes: u64,
}

impl FakeUsage {
    #[must_use]
    pub fn total(&self) -> u64 {
        self.retained_payload_bytes
            .saturating_add(self.retained_metadata_bytes)
            .saturating_add(self.live_scratch_bytes)
    }

    fn amount(&self, kind: ChargeKind) -> u64 {
        match kind {
            ChargeKind::RetainedPayload => self.retained_payload_bytes,
            ChargeKind::RetainedMetadata => self.retained_metadata_bytes,
            ChargeKind::LiveScratch => self.live_scratch_bytes,
        }
    }

    fn slot_mut(&mut self, kind: ChargeKind) -> &mut u64 {
        match kind {
            ChargeKind::RetainedPayload => &mut self.retained_payload_bytes,
            ChargeKind::RetainedMetadata => &mut self.retained_metadata_bytes,
            ChargeKind::LiveScratch => &mut self.live_scratch_bytes,
        }
    }

    /// Add `bytes` (a `u64` magnitude) to the kind's slot, saturating. Mirrors the
    /// production ledger: a `bytes > i64::MAX` charge must NOT route through an
    /// `i64` delta (the old `bytes as i64` cast wrapped such a value negative and
    /// silently subtracted — the exact corruption the real ledger fixed).
    fn add(&mut self, kind: ChargeKind, bytes: u64) {
        let slot = self.slot_mut(kind);
        *slot = slot.saturating_add(bytes);
    }

    /// Subtract `bytes` (a `u64` magnitude) from the kind's slot, saturating.
    fn sub(&mut self, kind: ChargeKind, bytes: u64) {
        let slot = self.slot_mut(kind);
        *slot = slot.saturating_sub(bytes);
    }
}

/// Immutable snapshot of the fake broker's usage and rebuild state. This is the
/// host-independent surface the conformance/invariant layer reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeBrokerSnapshot {
    pub class_usage: BTreeMap<MemoryClass, FakeUsage>,
    pub owner_usage: BTreeMap<OwnerKey, FakeUsage>,
    pub context_usage: BTreeMap<MemoryContextKind, FakeUsage>,
    pub group_usage: BTreeMap<String, u64>,
    /// Pending plus committed ticket authorities that have not terminated.
    pub live_ticket_count: usize,
    pub rebuild_in_progress: bool,
    pub pressure: PressureState,
    /// When `Some`, the reported class AND owner totals use this value instead of
    /// the field sums, letting a broken fake desync the totals from the per-kind
    /// partition (invariant 2). `None` (the honest default) reports the true sums.
    pub total_override: Option<u64>,
}

impl FakeBrokerSnapshot {
    #[must_use]
    pub fn total_class_bytes(&self) -> u64 {
        self.total_override.unwrap_or_else(|| self.class_usage.values().map(FakeUsage::total).sum())
    }
    #[must_use]
    pub fn total_owner_bytes(&self) -> u64 {
        self.total_override.unwrap_or_else(|| self.owner_usage.values().map(FakeUsage::total).sum())
    }
    #[must_use]
    pub fn total_retained_payload_bytes(&self) -> u64 {
        self.owner_usage.values().map(|u| u.retained_payload_bytes).sum()
    }
    #[must_use]
    pub fn total_retained_metadata_bytes(&self) -> u64 {
        self.owner_usage.values().map(|u| u.retained_metadata_bytes).sum()
    }
    #[must_use]
    pub fn total_live_scratch_bytes(&self) -> u64 {
        self.owner_usage.values().map(|u| u.live_scratch_bytes).sum()
    }
}

/// Whether the fake reports honest accounting or a deliberately corrupted view.
///
/// `Honest` mirrors the real broker exactly. The corrupting variants exist ONLY
/// so the conformance suite can be proven to bite: a suite that passes a fake
/// that under-reports owner usage, or that lets class and owner totals diverge,
/// is not a conformance suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountingFidelity {
    /// Snapshot reflects the true ledger (the contract).
    Honest,
    /// Owner usage is under-reported by a fixed number of bytes per owner axis.
    UnderReportOwnerBytes(u64),
    /// Class totals are inflated so `sum(class) != sum(owner)` (invariant 1 break).
    DivergeClassFromOwner(u64),
    /// The reported class AND owner TOTALS are inflated by a fixed number of bytes
    /// while the per-kind partition fields stay honest.
    ///
    /// This keeps `sum(class) == sum(owner)` (invariant 1 holds) but makes
    /// `sum(owner) != payload + metadata + live_scratch`, isolating an invariant 2
    /// (owner-partition) break. The only way to desync the owner total from its
    /// partition is an independently reported total, since field manipulation
    /// moves both in lockstep.
    DivergeOwnerPartition(u64),
}

/// A fake process memory broker.
///
/// It owns its own reservation ledger and reclaim/quota/pressure model. It is
/// NOT a production type and never participates in a production dependency graph.
#[derive(Clone, Debug)]
pub struct FakeProcessBroker {
    class_usage: BTreeMap<MemoryClass, FakeUsage>,
    owner_usage: BTreeMap<OwnerKey, FakeUsage>,
    context_usage: BTreeMap<MemoryContextKind, FakeUsage>,
    group_usage: BTreeMap<String, u64>,
    owner_hard_caps: BTreeMap<OwnerKey, u64>,
    group_owners: BTreeMap<String, Vec<OwnerKey>>,
    group_hard_caps: BTreeMap<String, u64>,
    next_id: u64,
    tickets: BTreeMap<u64, FakeTicket>,
    rebuild_in_progress: bool,
    pressure: PressureState,
    /// Bytes a single reclaim attempt will free. `0` models reclaim-returns-zero.
    reclaim_yield_bytes: u64,
    fidelity: AccountingFidelity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FakeTicketState {
    Pending,
    Committed,
}

/// Reclaim and release share the same charge provenance and remaining authority.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FakeTicket {
    reservation: FakeReservation,
    state: FakeTicketState,
    remaining_bytes: u64,
}

impl Default for FakeProcessBroker {
    fn default() -> Self {
        Self::honest()
    }
}

impl FakeProcessBroker {
    /// An honest fake with no caps and Normal pressure.
    #[must_use]
    pub fn honest() -> Self {
        Self {
            class_usage: BTreeMap::new(),
            owner_usage: BTreeMap::new(),
            context_usage: BTreeMap::new(),
            group_usage: BTreeMap::new(),
            owner_hard_caps: BTreeMap::new(),
            group_owners: BTreeMap::new(),
            group_hard_caps: BTreeMap::new(),
            next_id: 1,
            tickets: BTreeMap::new(),
            rebuild_in_progress: false,
            pressure: PressureState::Normal,
            reclaim_yield_bytes: u64::MAX,
            fidelity: AccountingFidelity::Honest,
        }
    }

    /// A deliberately broken fake. Used to prove the conformance suite rejects it.
    #[must_use]
    pub fn broken(fidelity: AccountingFidelity) -> Self {
        Self { fidelity, ..Self::honest() }
    }

    #[must_use]
    pub fn with_owner_hard_cap(mut self, owner: impl Into<OwnerKey>, cap_bytes: u64) -> Self {
        self.owner_hard_caps.insert(owner.into(), cap_bytes);
        self
    }

    #[must_use]
    pub fn with_resource_group(
        mut self,
        group: impl Into<String>,
        cap_bytes: u64,
        owners: Vec<OwnerKey>,
    ) -> Self {
        let group = group.into();
        self.group_hard_caps.insert(group.clone(), cap_bytes);
        self.group_owners.insert(group, owners);
        self
    }

    #[must_use]
    pub fn with_pressure(mut self, pressure: PressureState) -> Self {
        self.pressure = pressure;
        self
    }

    #[must_use]
    pub fn with_reclaim_yield_bytes(mut self, bytes: u64) -> Self {
        self.reclaim_yield_bytes = bytes;
        self
    }

    fn group_of(&self, owner: &OwnerKey) -> Option<&String> {
        self.group_owners.iter().find(|(_, owners)| owners.contains(owner)).map(|(group, _)| group)
    }

    pub(crate) fn group_usage_total(&self, group: &str) -> u64 {
        self.group_usage.get(group).copied().unwrap_or(0)
    }

    /// Reserve bytes against (class, owner). Fail-closed before mutation:
    /// the rebuild gate, then the owner hard cap, then the resource-group cap are
    /// checked before any counter moves, matching the real broker's deny order.
    pub fn reserve(
        &mut self,
        class: MemoryClass,
        owner: OwnerKey,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<FakeReservation> {
        if self.rebuild_in_progress {
            return Err(MemoryError::InvalidState(
                "fake broker: rebuild in progress, live reserve rejected".to_string(),
            ));
        }

        let owner_after = self
            .owner_usage
            .get(&owner)
            .map_or(0, FakeUsage::total)
            .checked_add(bytes)
            .ok_or_else(|| MemoryError::capacity("fake broker: owner usage exceeds u64"))?;
        self.class_usage
            .get(&class)
            .map_or(0, FakeUsage::total)
            .checked_add(bytes)
            .ok_or_else(|| MemoryError::capacity("fake broker: class usage exceeds u64"))?;
        self.context_usage
            .get(&MemoryContextKind::Request)
            .map_or(0, FakeUsage::total)
            .checked_add(bytes)
            .ok_or_else(|| MemoryError::capacity("fake broker: context usage exceeds u64"))?;
        if let Some(cap) = self.owner_hard_caps.get(&owner) {
            if owner_after > *cap {
                return Err(MemoryError::capacity(format!(
                    "fake broker: owner {owner} hard cap {cap} exceeded by reserve to {owner_after}"
                )));
            }
        }

        let group = self.group_of(&owner).cloned();
        if let Some(group) = group.as_ref() {
            if let Some(cap) = self.group_hard_caps.get(group) {
                let group_after = self
                    .group_usage_total(group)
                    .checked_add(bytes)
                    .ok_or_else(|| MemoryError::capacity("fake broker: group usage exceeds u64"))?;
                if group_after > *cap {
                    return Err(MemoryError::capacity(format!(
                        "fake broker: resource group {group} hard cap {cap} exceeded by reserve to {group_after}"
                    )));
                }
            }
        }

        let id = self.next_id;
        let next_id = self.next_id.checked_add(1).ok_or_else(|| {
            MemoryError::InvalidState("fake broker: reservation id space exhausted".into())
        })?;
        self.class_usage.entry(class.clone()).or_default().add(kind, bytes);
        self.owner_usage.entry(owner.clone()).or_default().add(kind, bytes);
        self.context_usage.entry(MemoryContextKind::Request).or_default().add(kind, bytes);
        if let Some(group) = group.as_ref() {
            let usage = self.group_usage.entry(group.clone()).or_default();
            *usage =
                usage.checked_add(bytes).expect("fake group admission preflight checked postimage");
        }
        self.next_id = next_id;
        let reservation = FakeReservation { id, class, owner, group, kind, bytes };
        self.tickets.insert(
            id,
            FakeTicket {
                reservation: reservation.clone(),
                state: FakeTicketState::Pending,
                remaining_bytes: bytes,
            },
        );

        Ok(reservation)
    }

    /// Commit a reservation. Mirrors the core ledger: reserve already moved the
    /// counters; commit only finalizes and is rejected during rebuild.
    pub fn commit(&mut self, ticket: &FakeReservation) -> MemoryResult<u64> {
        if self.rebuild_in_progress {
            return Err(MemoryError::InvalidState(
                "fake broker: rebuild in progress, live commit rejected".to_string(),
            ));
        }
        let authority = self
            .tickets
            .get_mut(&ticket.id)
            .filter(|authority| authority.reservation == *ticket)
            .ok_or_else(|| {
                MemoryError::InvalidState(
                    "fake broker: reservation is stale or already released".into(),
                )
            })?;
        match authority.state {
            FakeTicketState::Pending => {
                authority.state = FakeTicketState::Committed;
                Ok(authority.remaining_bytes)
            }
            FakeTicketState::Committed => {
                Err(MemoryError::InvalidState("fake broker: reservation already committed".into()))
            }
        }
    }

    /// Release a reservation/charge, subtracting from class and owner usage.
    pub fn release(&mut self, ticket: &FakeReservation) -> MemoryResult<()> {
        if self.rebuild_in_progress {
            return Err(MemoryError::InvalidState(
                "fake broker: rebuild in progress, live release rejected".to_string(),
            ));
        }
        let bytes = self
            .tickets
            .get(&ticket.id)
            .filter(|authority| authority.reservation == *ticket)
            .map(|authority| authority.remaining_bytes)
            .ok_or_else(|| {
                MemoryError::InvalidState(
                    "fake broker: reservation is stale or already released".into(),
                )
            })?;
        for (axis, available) in [
            (
                "class",
                self.class_usage.get(&ticket.class).map_or(0, |usage| usage.amount(ticket.kind)),
            ),
            (
                "owner",
                self.owner_usage.get(&ticket.owner).map_or(0, |usage| usage.amount(ticket.kind)),
            ),
            (
                "context",
                self.context_usage
                    .get(&MemoryContextKind::Request)
                    .map_or(0, |usage| usage.amount(ticket.kind)),
            ),
        ] {
            if available < bytes {
                return Err(MemoryError::InvalidState(format!(
                    "fake broker: {axis} release exceeds usage"
                )));
            }
        }
        if let Some(group) = ticket.group.as_ref() {
            if self.group_usage.get(group).copied().unwrap_or(0) < bytes {
                return Err(MemoryError::InvalidState(
                    "fake broker: group release exceeds usage".into(),
                ));
            }
        }

        self.tickets.remove(&ticket.id);
        if let Some(usage) = self.class_usage.get_mut(&ticket.class) {
            usage.sub(ticket.kind, bytes);
        }
        if let Some(usage) = self.owner_usage.get_mut(&ticket.owner) {
            usage.sub(ticket.kind, bytes);
        }
        if let Some(usage) = self.context_usage.get_mut(&MemoryContextKind::Request) {
            usage.sub(ticket.kind, bytes);
        }
        if let Some(group) = ticket.group.as_ref() {
            let usage = self.group_usage.get_mut(group).expect("release preflight checked group");
            *usage = usage.checked_sub(bytes).expect("release preflight checked amount");
        }
        Ok(())
    }

    /// Attempt one reclaim sweep, freeing at most the configured yield from the
    /// largest owner. Charge-kind priority is payload, metadata, then scratch;
    /// tickets within a kind are visited by increasing id. Partial reclaim keeps
    /// the ticket live with a smaller release authority; full reclaim terminates
    /// it. The original handle releases its remainder, never its original bytes.
    /// During rebuild or a failed accounting preflight, nothing moves and 0 is returned.
    pub fn reclaim_once(&mut self) -> u64 {
        if self.rebuild_in_progress || self.reclaim_yield_bytes == 0 {
            return 0;
        }
        let target = self
            .owner_usage
            .iter()
            .max_by_key(|(_, usage)| usage.total())
            .map(|(owner, usage)| (owner.clone(), usage.total()));
        let (owner, owner_total) = match target {
            Some((owner, total)) if total > 0 => (owner, total),
            _ => return 0,
        };
        let freed = owner_total.min(self.reclaim_yield_bytes);
        let mut remaining = freed;
        let mut plan = Vec::new();
        let mut owner_delta = FakeUsage::default();
        let mut class_deltas = BTreeMap::<MemoryClass, FakeUsage>::new();
        let mut group_deltas = BTreeMap::<String, u64>::new();
        for kind in
            [ChargeKind::RetainedPayload, ChargeKind::RetainedMetadata, ChargeKind::LiveScratch]
        {
            for (id, authority) in &self.tickets {
                let reservation = &authority.reservation;
                if remaining == 0 || reservation.owner != owner || reservation.kind != kind {
                    continue;
                }
                let take = authority.remaining_bytes.min(remaining);
                if take == 0 {
                    continue;
                }
                plan.push((*id, take));
                // Every delta is bounded by `freed`, so its slot cannot overflow.
                owner_delta.add(kind, take);
                class_deltas.entry(reservation.class.clone()).or_default().add(kind, take);
                if let Some(group) = &reservation.group {
                    let delta = group_deltas.entry(group.clone()).or_default();
                    let Some(after) = delta.checked_add(take) else {
                        return 0;
                    };
                    *delta = after;
                }
                remaining -= take;
            }
        }
        let covers = |usage: &FakeUsage, delta: &FakeUsage| {
            usage.retained_payload_bytes >= delta.retained_payload_bytes
                && usage.retained_metadata_bytes >= delta.retained_metadata_bytes
                && usage.live_scratch_bytes >= delta.live_scratch_bytes
        };
        if remaining != 0
            || !self.owner_usage.get(&owner).is_some_and(|usage| covers(usage, &owner_delta))
            || !self
                .context_usage
                .get(&MemoryContextKind::Request)
                .is_some_and(|usage| covers(usage, &owner_delta))
            || class_deltas.iter().any(|(class, delta)| {
                !self.class_usage.get(class).is_some_and(|usage| covers(usage, delta))
            })
            || group_deltas
                .iter()
                .any(|(group, delta)| self.group_usage.get(group).is_none_or(|usage| usage < delta))
        {
            return 0;
        }
        let subtract = |usage: &mut FakeUsage, delta: &FakeUsage| {
            usage.retained_payload_bytes = usage
                .retained_payload_bytes
                .checked_sub(delta.retained_payload_bytes)
                .expect("reclaim preflight checked payload");
            usage.retained_metadata_bytes = usage
                .retained_metadata_bytes
                .checked_sub(delta.retained_metadata_bytes)
                .expect("reclaim preflight checked metadata");
            usage.live_scratch_bytes = usage
                .live_scratch_bytes
                .checked_sub(delta.live_scratch_bytes)
                .expect("reclaim preflight checked scratch");
        };
        subtract(
            self.owner_usage.get_mut(&owner).expect("reclaim preflight checked owner"),
            &owner_delta,
        );
        subtract(
            self.context_usage
                .get_mut(&MemoryContextKind::Request)
                .expect("reclaim preflight checked context"),
            &owner_delta,
        );
        for (class, delta) in class_deltas {
            subtract(
                self.class_usage.get_mut(&class).expect("reclaim preflight checked class"),
                &delta,
            );
        }
        for (group, delta) in group_deltas {
            let usage = self.group_usage.get_mut(&group).expect("reclaim preflight checked group");
            *usage = usage.checked_sub(delta).expect("reclaim preflight checked group amount");
        }
        for (id, take) in plan {
            let authority = self.tickets.get_mut(&id).expect("planned ticket remains live");
            authority.remaining_bytes = authority
                .remaining_bytes
                .checked_sub(take)
                .expect("reclaim plan checked ticket amount");
            if authority.remaining_bytes == 0 {
                self.tickets.remove(&id);
            }
        }
        freed
    }

    /// Open the rebuild window: subsequent live writes are rejected.
    pub fn begin_rebuild(&mut self) {
        self.rebuild_in_progress = true;
    }

    /// Close the rebuild window.
    pub fn finish_rebuild(&mut self) {
        self.rebuild_in_progress = false;
    }

    #[must_use]
    pub fn rebuild_in_progress(&self) -> bool {
        self.rebuild_in_progress
    }

    #[must_use]
    pub fn pressure(&self) -> PressureState {
        self.pressure
    }

    /// Snapshot the fake's usage. Applies the configured [`AccountingFidelity`]:
    /// an honest fake reports the true ledger; a broken fake corrupts the
    /// reported view (but never the underlying counters).
    #[must_use]
    pub fn snapshot(&self) -> FakeBrokerSnapshot {
        let mut owner_usage = self.owner_usage.clone();
        let mut class_usage = self.class_usage.clone();
        let mut total_override = None;
        match self.fidelity {
            AccountingFidelity::Honest => {}
            AccountingFidelity::UnderReportOwnerBytes(shortfall) => {
                for usage in owner_usage.values_mut() {
                    // Under-report by shaving payload bytes from the reported view.
                    let take = usage.retained_payload_bytes.min(shortfall);
                    usage.retained_payload_bytes -= take;
                }
            }
            AccountingFidelity::DivergeClassFromOwner(extra) => {
                for usage in class_usage.values_mut() {
                    usage.retained_payload_bytes =
                        usage.retained_payload_bytes.saturating_add(extra);
                }
            }
            AccountingFidelity::DivergeOwnerPartition(extra) => {
                // Leave the per-kind partition fields honest; inflate ONLY the
                // reported class AND owner totals by `extra`. Class == owner is
                // preserved (invariant 1 holds) while owner != partition_sum
                // (invariant 2 breaks). The honest owner total and honest class
                // total are equal for a fake built by honest reserves, so one
                // override value drives both totals.
                let honest_total: u64 = owner_usage.values().map(FakeUsage::total).sum();
                total_override = Some(honest_total.saturating_add(extra));
            }
        }
        FakeBrokerSnapshot {
            class_usage,
            owner_usage,
            context_usage: self.context_usage.clone(),
            group_usage: self.group_usage.clone(),
            live_ticket_count: self.tickets.len(),
            rebuild_in_progress: self.rebuild_in_progress,
            pressure: self.pressure,
            total_override,
        }
    }
}

/// Factory entry point for fake brokers (kept for the public test-entry surface).
pub struct FakeBrokerFactory;

impl FakeBrokerFactory {
    /// An honest fake broker ready for conformance exercise.
    #[must_use]
    pub fn honest() -> FakeProcessBroker {
        FakeProcessBroker::honest()
    }

    /// A deliberately broken fake broker for self-testing the conformance suite.
    #[must_use]
    pub fn broken(fidelity: AccountingFidelity) -> FakeProcessBroker {
        FakeProcessBroker::broken(fidelity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honest_reserve_keeps_class_owner_balanced() {
        let mut b = FakeProcessBroker::honest();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                ChargeKind::RetainedPayload,
                1_000,
            )
            .expect("reserve fits");
        b.commit(&t).expect("commit");
        let snap = b.snapshot();
        assert_eq!(snap.total_class_bytes(), snap.total_owner_bytes());
        assert_eq!(snap.total_owner_bytes(), 1_000);
        assert_eq!(snap.total_retained_payload_bytes(), 1_000);
    }

    #[test]
    fn owner_hard_cap_denies_before_mutation() {
        let mut b = FakeProcessBroker::honest().with_owner_hard_cap("owner:a", 1_000);
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                ChargeKind::RetainedPayload,
                800,
            )
            .expect("first fits");
        b.commit(&t).expect("commit");
        let before = b.snapshot().total_owner_bytes();
        // 800 + 400 = 1200 > 1000 hard cap.
        let denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            ChargeKind::RetainedPayload,
            400,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "typed capacity deny");
        let after = b.snapshot().total_owner_bytes();
        assert_eq!(before, after, "denied reserve must not mutate accounting");
        assert_ne!(after, 1_200);
    }

    #[test]
    fn resource_group_cap_denies_aggregate_over_owners() {
        let mut b = FakeProcessBroker::honest().with_resource_group(
            "group:g",
            1_000,
            vec!["owner:a".to_string(), "owner:b".to_string()],
        );
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                ChargeKind::RetainedPayload,
                700,
            )
            .expect("a fits");
        b.commit(&t).expect("commit");
        // owner:b alone is fine, but group total 700 + 400 > 1000.
        let denied = b.reserve(
            MemoryClass::ArtifactCache,
            "owner:b".to_string(),
            ChargeKind::RetainedPayload,
            400,
        );
        assert!(matches!(denied, Err(MemoryError::Capacity(_))), "group cap deny");
        // positive pair: a smaller second reserve fits within the group cap.
        assert!(b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:b".to_string(),
                ChargeKind::RetainedPayload,
                200
            )
            .is_ok());
    }

    #[test]
    fn rebuild_window_rejects_writes_then_resumes() {
        let mut b = FakeProcessBroker::honest();
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                ChargeKind::RetainedPayload,
                10,
            )
            .expect("reserve before rebuild");
        b.begin_rebuild();
        assert!(matches!(
            b.reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                ChargeKind::RetainedPayload,
                10
            ),
            Err(MemoryError::InvalidState(_))
        ));
        assert!(matches!(b.commit(&t), Err(MemoryError::InvalidState(_))));
        assert!(matches!(b.release(&t), Err(MemoryError::InvalidState(_))));
        b.finish_rebuild();
        assert!(b.commit(&t).is_ok(), "writes resume after rebuild");
    }

    #[test]
    fn reclaim_yield_zero_frees_nothing() {
        let mut b = FakeProcessBroker::honest().with_reclaim_yield_bytes(0);
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                ChargeKind::RetainedPayload,
                500,
            )
            .expect("reserve");
        b.commit(&t).expect("commit");
        // NEGATIVE PATH 1: reclaim returns zero under starvation.
        assert_eq!(b.reclaim_once(), 0, "reclaim-returns-zero must free nothing");
        assert_eq!(
            b.snapshot().total_owner_bytes(),
            500,
            "usage unchanged when reclaim yields zero"
        );

        // positive pair: a yielding fake frees bytes and keeps balance.
        let mut b2 = FakeProcessBroker::honest().with_reclaim_yield_bytes(200);
        let t2 = b2
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                ChargeKind::RetainedPayload,
                500,
            )
            .expect("reserve");
        b2.commit(&t2).expect("commit");
        assert_eq!(b2.reclaim_once(), 200);
        let snap = b2.snapshot();
        assert_eq!(snap.total_owner_bytes(), 300);
        assert_eq!(snap.total_class_bytes(), snap.total_owner_bytes(), "reclaim keeps invariant 1");
    }

    #[test]
    fn reclaim_preserves_ticket_provenance_kind_axes_and_group_refunds() {
        let mut broker = FakeProcessBroker::honest()
            .with_resource_group("group", 120, vec!["small".into(), "large".into()])
            .with_reclaim_yield_bytes(50);
        let unrelated = broker
            .reserve(MemoryClass::ArtifactCache, "small".into(), ChargeKind::RetainedPayload, 20)
            .expect("small payload");
        let reclaimed = broker
            .reserve(MemoryClass::EvidenceCache, "large".into(), ChargeKind::RetainedMetadata, 100)
            .expect("large metadata");
        broker.commit(&unrelated).expect("commit unrelated");
        broker.commit(&reclaimed).expect("commit reclaimed");

        assert_eq!(broker.reclaim_once(), 50);
        let snapshot = broker.snapshot();
        let payload = FakeUsage { retained_payload_bytes: 20, ..FakeUsage::default() };
        let metadata = FakeUsage { retained_metadata_bytes: 50, ..FakeUsage::default() };
        assert_eq!(snapshot.class_usage[&MemoryClass::ArtifactCache], payload);
        assert_eq!(snapshot.class_usage[&MemoryClass::EvidenceCache], metadata);
        assert_eq!(snapshot.owner_usage["small"], payload);
        assert_eq!(snapshot.owner_usage["large"], metadata);
        assert_eq!(
            snapshot.context_usage[&MemoryContextKind::Request],
            FakeUsage {
                retained_payload_bytes: 20,
                retained_metadata_bytes: 50,
                live_scratch_bytes: 0,
            }
        );
        assert_eq!(snapshot.group_usage["group"], 70);
        assert_eq!(snapshot.live_ticket_count, 2);
        assert_eq!(reclaimed.bytes(), 100, "immutable handle records its issued size");
        broker.release(&unrelated).expect("reclaim must not corrupt unrelated ticket");
        broker.release(&reclaimed).expect("original handle releases the remaining 50 bytes");
        let empty = broker.snapshot();
        assert_eq!(empty.total_class_bytes(), 0);
        assert_eq!(empty.total_owner_bytes(), 0);
        assert_eq!(empty.context_usage[&MemoryContextKind::Request], FakeUsage::default());
        assert_eq!(empty.group_usage["group"], 0);
        assert_eq!(empty.live_ticket_count, 0);
        let before_stale_release = broker.snapshot();
        assert!(matches!(broker.release(&reclaimed), Err(MemoryError::InvalidState(_))));
        assert_eq!(broker.snapshot(), before_stale_release);
    }

    #[test]
    fn reclaim_terminates_full_tickets_and_preserves_partial_commit_authority() {
        let mut broker = FakeProcessBroker::honest()
            .with_resource_group("group", 100, vec!["owner".into()])
            .with_reclaim_yield_bytes(40);
        let payload = broker
            .reserve(MemoryClass::EvidenceCache, "owner".into(), ChargeKind::RetainedPayload, 30)
            .expect("payload");
        let metadata = broker
            .reserve(MemoryClass::ArtifactCache, "owner".into(), ChargeKind::RetainedMetadata, 50)
            .expect("metadata");
        let scratch = broker
            .reserve(MemoryClass::ArtifactCache, "owner".into(), ChargeKind::LiveScratch, 5)
            .expect("scratch");
        let zero = broker
            .reserve(MemoryClass::ArtifactCache, "owner".into(), ChargeKind::LiveScratch, 0)
            .expect("zero-byte reservation still carries authority");
        assert_eq!(broker.reclaim_once(), 40, "payload-first reclaim spans both classes");
        let snapshot = broker.snapshot();
        assert_eq!(snapshot.class_usage[&MemoryClass::EvidenceCache], FakeUsage::default());
        assert_eq!(snapshot.class_usage[&MemoryClass::ArtifactCache].retained_metadata_bytes, 40);
        assert_eq!(snapshot.total_owner_bytes(), 45);
        assert_eq!(snapshot.total_retained_payload_bytes(), 0);
        assert_eq!(snapshot.total_retained_metadata_bytes(), 40);
        assert_eq!(snapshot.live_ticket_count, 3);
        assert_eq!(snapshot.group_usage["group"], 45);
        assert_eq!(snapshot.total_live_scratch_bytes(), 5);
        assert!(matches!(broker.commit(&payload), Err(MemoryError::InvalidState(_))));
        assert!(matches!(broker.release(&payload), Err(MemoryError::InvalidState(_))));
        assert_eq!(broker.snapshot(), snapshot, "fully reclaimed authority is stale and inert");
        assert_eq!(broker.commit(&metadata).expect("commit remainder"), 40);
        assert_eq!(broker.reclaim_once(), 40);
        assert_eq!(broker.snapshot().live_ticket_count, 2);
        assert_eq!(broker.snapshot().total_live_scratch_bytes(), 5);
        assert_eq!(broker.snapshot().group_usage["group"], 5);
        assert_eq!(broker.reclaim_once(), 5, "scratch is reclaimed after retained kinds");
        assert!(matches!(broker.commit(&scratch), Err(MemoryError::InvalidState(_))));
        assert!(matches!(broker.release(&scratch), Err(MemoryError::InvalidState(_))));
        assert_eq!(broker.snapshot().live_ticket_count, 1, "zero-byte ticket is not reclaimed");
        assert_eq!(broker.snapshot().total_class_bytes(), 0);
        assert!(matches!(broker.release(&metadata), Err(MemoryError::InvalidState(_))));
        assert_eq!(broker.snapshot().group_usage["group"], 0);
        assert_eq!(broker.commit(&zero).expect("zero authority survives reclaim"), 0);
        broker.release(&zero).expect("zero authority releases exactly once");
        assert_eq!(broker.snapshot().live_ticket_count, 0);
        assert!(matches!(broker.release(&zero), Err(MemoryError::InvalidState(_))));
    }

    #[test]
    fn reclaim_rebuild_and_incomplete_group_preflight_leave_all_authority_untouched() {
        let mut broker = FakeProcessBroker::honest()
            .with_resource_group("group", 100, vec!["owner".into()])
            .with_reclaim_yield_bytes(80);
        let ticket = broker
            .reserve(MemoryClass::ArtifactCache, "owner".into(), ChargeKind::RetainedPayload, 100)
            .expect("reservation");
        broker.commit(&ticket).expect("commit");
        broker.begin_rebuild();
        let before = broker.snapshot();
        let tickets = broker.tickets.clone();
        assert_eq!(broker.reclaim_once(), 0);
        assert_eq!(broker.snapshot(), before);
        assert_eq!(broker.tickets, tickets);
        broker.finish_rebuild();

        // A corrupt group cannot authorize a partial publish on the other axes.
        broker.group_usage.insert("group".into(), 50);
        let before = broker.snapshot();
        let tickets = broker.tickets.clone();
        assert_eq!(broker.reclaim_once(), 0);
        assert_eq!(broker.snapshot(), before);
        assert_eq!(broker.tickets, tickets);
        broker.group_usage.insert("group".into(), 100);
        assert_eq!(broker.reclaim_once(), 80);
        broker.release(&ticket).expect("remaining 20-byte authority survives denial");
        assert_eq!(broker.snapshot().total_owner_bytes(), 0);
        assert_eq!(broker.snapshot().group_usage["group"], 0);
    }

    #[test]
    fn broken_under_report_corrupts_only_the_reported_snapshot() {
        let mut b = FakeProcessBroker::broken(AccountingFidelity::UnderReportOwnerBytes(100));
        let t = b
            .reserve(
                MemoryClass::ArtifactCache,
                "owner:a".to_string(),
                ChargeKind::RetainedPayload,
                500,
            )
            .expect("reserve");
        b.commit(&t).expect("commit");
        let snap = b.snapshot();
        // The reported owner total is short by 100 while the class total is honest:
        // this is exactly the mismatch the conformance suite must catch.
        assert_eq!(snap.total_owner_bytes(), 400);
        assert_eq!(snap.total_class_bytes(), 500);
        assert_ne!(snap.total_class_bytes(), snap.total_owner_bytes());
    }
}
