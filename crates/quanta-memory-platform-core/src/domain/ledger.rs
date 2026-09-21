//! Process-wide memory accounting ledger.
//!
//! This is the semantic accounting SSOT for the broker. It owns the
//! reserve / commit / release algebra and enforces the global invariants:
//!
//! 1. `sum(class usage) == sum(owner usage)`
//! 2. `sum(owner usage) == retained payload bytes + retained metadata bytes
//!     + live scratch reservations`
//! 3. `reserved <= soft <= hard` (validated when the policy is installed)
//! 4. committed/live writes are forbidden while `rebuild_in_progress`; rollback
//!    of an incomplete pending reservation remains allowed so abandoned
//!    capacity permits cannot strand reserved bytes during a rebuild
//! 5. `sum(owner usage) <= global_hard_limit_bytes`; non-`Headroom` charges are
//!    additionally capped at `global_hard_limit_bytes - headroom_bytes`, so the
//!    reserved headroom band is only reachable by the `Headroom` class.
//! 6. a class may exceed its own hard cap only when `borrow_from_shared_pool`
//!    is set, and total over-cap borrowing across all borrowing classes is
//!    bounded by `shared_burst_pool_bytes`.
//! 7. reservation tickets are single-use for release: each issued ticket is
//!    tracked until released, so a cloned/replayed ticket cannot double-subtract
//!    (a second release, or a commit after release, is a typed `InvalidState`).
//!
//! Allocator telemetry is *advisory*: it never mutates these counters.
//!
//! The ledger is single-threaded and synchronous; the public broker wraps it
//! behind a lock. Every mutating operation either fully succeeds or leaves the
//! ledger untouched (fail-closed before mutation).

use std::collections::BTreeMap;
#[cfg(feature = "live_control")]
use std::collections::BTreeSet;
#[cfg(feature = "live_control")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "live_control")]
use crate::domain::config::ValidatedResidencyPolicy;
use crate::domain::context::{MemoryContextKind, OwnerKey};
use crate::domain::errors::{MemoryError, MemoryResult};
use crate::domain::policy::MemoryClass;
#[cfg(feature = "live_control")]
use crate::domain::policy::ResidencyPolicy;

/// Process-global source of per-ledger instance tags. Ticket ids are only unique
/// *within* a ledger (each ledger's counter starts at 1), so every issued ticket
/// also carries its ledger's instance tag; commit/release reject a ticket whose
/// tag does not match this ledger, defeating cross-ledger id-space collisions
/// (invariant 7).
#[cfg(feature = "live_control")]
static LEDGER_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// Why a charge is held: a payload byte, a metadata byte, or a live scratch
/// reservation. Invariant 2 partitions all owner usage across these kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChargeKind {
    /// Retained payload bytes.
    RetainedPayload,
    /// Retained metadata bytes (pin tables, indexes, generation metadata).
    RetainedMetadata,
    /// Live scratch reservation (temporary working buffer, not yet released).
    LiveScratch,
}

/// Internal ledger capability. Public consumers receive broker-bound RAII
/// authorities instead; exposing this cloneable identity would allow a caller
/// to discard the only release path while the charge remains live.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "live_control")]
pub(crate) struct ReservationTicket {
    /// Per-ledger id (unique within the issuing ledger only).
    id: u64,
    /// Instance tag of the issuing ledger (guards against cross-ledger misuse).
    ledger: u64,
    class: MemoryClass,
    owner: OwnerKey,
    context: MemoryContextKind,
    kind: ChargeKind,
    bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(feature = "live_control")]
enum TicketState {
    Pending,
    Committed,
}

#[cfg(feature = "live_control")]
impl ReservationTicket {
    #[must_use]
    pub(crate) fn id(&self) -> u64 {
        self.id
    }
    #[must_use]
    pub(crate) fn class(&self) -> &MemoryClass {
        &self.class
    }
    #[must_use]
    pub(crate) fn owner(&self) -> &OwnerKey {
        &self.owner
    }
    #[must_use]
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
    #[must_use]
    pub(crate) fn kind(&self) -> ChargeKind {
        self.kind
    }
    #[must_use]
    pub(crate) fn context(&self) -> MemoryContextKind {
        self.context
    }
}

/// Per-charge-kind usage split for one accounting axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsageBreakdown {
    pub retained_payload_bytes: u64,
    pub retained_metadata_bytes: u64,
    pub live_scratch_bytes: u64,
}

impl UsageBreakdown {
    /// Exact total for a representable breakdown.
    ///
    /// # Panics
    /// Panics if a caller-constructed breakdown exceeds `u64::MAX`. Ledger
    /// mutations reject such a postimage before publishing it.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.retained_payload_bytes
            .checked_add(self.retained_metadata_bytes)
            .and_then(|sum| sum.checked_add(self.live_scratch_bytes))
            .expect("ledger usage breakdown exceeds u64")
    }

    pub fn checked_total(&self) -> MemoryResult<u64> {
        self.retained_payload_bytes
            .checked_add(self.retained_metadata_bytes)
            .and_then(|sum| sum.checked_add(self.live_scratch_bytes))
            .ok_or_else(|| MemoryError::InvalidState("ledger usage breakdown exceeds u64".into()))
    }

    #[cfg(feature = "live_control")]
    fn slot(&self, kind: ChargeKind) -> u64 {
        match kind {
            ChargeKind::RetainedPayload => self.retained_payload_bytes,
            ChargeKind::RetainedMetadata => self.retained_metadata_bytes,
            ChargeKind::LiveScratch => self.live_scratch_bytes,
        }
    }

    #[cfg(feature = "live_control")]
    fn slot_mut(&mut self, kind: ChargeKind) -> &mut u64 {
        match kind {
            ChargeKind::RetainedPayload => &mut self.retained_payload_bytes,
            ChargeKind::RetainedMetadata => &mut self.retained_metadata_bytes,
            ChargeKind::LiveScratch => &mut self.live_scratch_bytes,
        }
    }

    /// Exact axis mutation after admission has checked the full postimage.
    #[cfg(feature = "live_control")]
    fn add(&mut self, kind: ChargeKind, bytes: u64) {
        let slot = self.slot_mut(kind);
        *slot = slot.checked_add(bytes).expect("ledger reserve preflight checked axis slot");
    }

    /// Exact axis mutation after release has checked all three axes.
    #[cfg(feature = "live_control")]
    fn sub(&mut self, kind: ChargeKind, bytes: u64) {
        let slot = self.slot_mut(kind);
        *slot = slot.checked_sub(bytes).expect("ledger release preflight checked axis slot");
    }
}

/// Immutable snapshot of the ledger usage, suitable for reporting and for
/// asserting the global invariants from outside the ledger.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LedgerSnapshot {
    pub class_usage: BTreeMap<MemoryClass, UsageBreakdown>,
    pub owner_usage: BTreeMap<OwnerKey, UsageBreakdown>,
    pub context_usage: BTreeMap<MemoryContextKind, UsageBreakdown>,
    /// Pending plus committed reservation authorities that have not terminated.
    /// A zero-byte reservation still contributes one live authority.
    pub live_ticket_count: usize,
    pub rebuild_in_progress: bool,
}

impl LedgerSnapshot {
    fn checked_axis_total<K: Ord>(axis: &BTreeMap<K, UsageBreakdown>) -> MemoryResult<u64> {
        axis.values().try_fold(0u64, |sum, usage| {
            sum.checked_add(usage.checked_total()?)
                .ok_or_else(|| MemoryError::InvalidState("ledger axis total exceeds u64".into()))
        })
    }

    pub fn checked_total_class_bytes(&self) -> MemoryResult<u64> {
        Self::checked_axis_total(&self.class_usage)
    }

    pub fn checked_total_owner_bytes(&self) -> MemoryResult<u64> {
        Self::checked_axis_total(&self.owner_usage)
    }

    pub fn checked_total_context_bytes(&self) -> MemoryResult<u64> {
        Self::checked_axis_total(&self.context_usage)
    }

    pub fn checked_total_retained_payload_bytes(&self) -> MemoryResult<u64> {
        self.owner_usage.values().try_fold(0u64, |sum, usage| {
            sum.checked_add(usage.retained_payload_bytes)
                .ok_or_else(|| MemoryError::InvalidState("ledger payload total exceeds u64".into()))
        })
    }

    pub fn checked_total_retained_metadata_bytes(&self) -> MemoryResult<u64> {
        self.owner_usage.values().try_fold(0u64, |sum, usage| {
            sum.checked_add(usage.retained_metadata_bytes).ok_or_else(|| {
                MemoryError::InvalidState("ledger metadata total exceeds u64".into())
            })
        })
    }

    pub fn checked_total_live_scratch_bytes(&self) -> MemoryResult<u64> {
        self.owner_usage.values().try_fold(0u64, |sum, usage| {
            sum.checked_add(usage.live_scratch_bytes)
                .ok_or_else(|| MemoryError::InvalidState("ledger scratch total exceeds u64".into()))
        })
    }

    #[cfg(feature = "live_control")]
    pub(crate) fn validate_exact(&self) -> MemoryResult<()> {
        let class = Self::checked_axis_total(&self.class_usage)?;
        let owner = Self::checked_axis_total(&self.owner_usage)?;
        let context = Self::checked_axis_total(&self.context_usage)?;
        if class != owner || owner != context {
            return Err(MemoryError::InvalidState("ledger accounting axes are unbalanced".into()));
        }
        let kinds = self.owner_usage.values().try_fold(0u64, |sum, usage| {
            sum.checked_add(usage.retained_payload_bytes)
                .and_then(|sum| sum.checked_add(usage.retained_metadata_bytes))
                .and_then(|sum| sum.checked_add(usage.live_scratch_bytes))
                .ok_or_else(|| {
                    MemoryError::InvalidState("ledger kind partition exceeds u64".into())
                })
        })?;
        if kinds != owner {
            return Err(MemoryError::InvalidState("ledger kind partition is unbalanced".into()));
        }
        Ok(())
    }

    #[must_use]
    pub fn total_class_bytes(&self) -> u64 {
        self.checked_total_class_bytes().expect("ledger class total exceeds u64")
    }

    #[must_use]
    pub fn total_owner_bytes(&self) -> u64 {
        self.checked_total_owner_bytes().expect("ledger owner total exceeds u64")
    }

    /// Total bytes on the context axis. The context axis is the third accounting
    /// dimension (alongside class and owner); every reserve/release touches all
    /// three, so this total must equal the owner total at every point.
    #[must_use]
    pub fn total_context_bytes(&self) -> u64 {
        self.checked_total_context_bytes().expect("ledger context total exceeds u64")
    }

    /// Context axis balances against the owner axis (the third-axis analogue of
    /// invariant 1).
    #[must_use]
    pub fn context_owner_balanced(&self) -> bool {
        self.total_context_bytes() == self.total_owner_bytes()
    }

    #[must_use]
    pub fn total_retained_payload_bytes(&self) -> u64 {
        self.checked_total_retained_payload_bytes().expect("ledger payload total exceeds u64")
    }

    #[must_use]
    pub fn total_retained_metadata_bytes(&self) -> u64 {
        self.checked_total_retained_metadata_bytes().expect("ledger metadata total exceeds u64")
    }

    #[must_use]
    pub fn total_live_scratch_bytes(&self) -> u64 {
        self.checked_total_live_scratch_bytes().expect("ledger scratch total exceeds u64")
    }

    /// Invariant 1: class totals equal owner totals.
    #[must_use]
    pub fn class_owner_balanced(&self) -> bool {
        self.total_class_bytes() == self.total_owner_bytes()
    }

    /// Invariant 2: owner usage partitions exactly into payload + metadata +
    /// live scratch.
    #[must_use]
    pub fn owner_partition_balanced(&self) -> bool {
        self.total_retained_payload_bytes()
            .checked_add(self.total_retained_metadata_bytes())
            .and_then(|sum| sum.checked_add(self.total_live_scratch_bytes()))
            .is_some_and(|sum| sum == self.total_owner_bytes())
    }
}

/// The accounting ledger. Owns class/owner/context counters and the active
/// residency policy.
///
/// Intentionally non-`Clone`: reservation tickets are minted against one live
/// ledger epoch, so duplicating the ledger state would re-open cross-ledger
/// replay hazards.
#[derive(Debug)]
#[cfg(feature = "live_control")]
pub(crate) struct AccountingLedger {
    policy: ValidatedResidencyPolicy,
    class_usage: BTreeMap<MemoryClass, UsageBreakdown>,
    owner_usage: BTreeMap<OwnerKey, UsageBreakdown>,
    context_usage: BTreeMap<MemoryContextKind, UsageBreakdown>,
    /// This ledger's process-unique instance tag, stamped on every issued ticket.
    instance_id: u64,
    /// Canonical reservation state machine. An id exists exactly while its
    /// charge is live, and can transition only Pending -> Committed -> absent.
    ticket_states: BTreeMap<u64, TicketState>,
    /// `None` means the u64 id space was exhausted. IDs never wrap or reuse.
    next_ticket_id: Option<u64>,
    rebuild_in_progress: bool,
}

#[cfg(feature = "live_control")]
impl AccountingLedger {
    #[must_use]
    pub(crate) fn new(policy: ValidatedResidencyPolicy) -> Self {
        Self {
            policy,
            class_usage: BTreeMap::new(),
            owner_usage: BTreeMap::new(),
            context_usage: BTreeMap::new(),
            instance_id: LEDGER_INSTANCE.fetch_add(1, Ordering::Relaxed),
            ticket_states: BTreeMap::new(),
            next_ticket_id: Some(1),
            rebuild_in_progress: false,
        }
    }

    #[must_use]
    pub fn policy(&self) -> &ResidencyPolicy {
        self.policy.as_policy()
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

        // --- pure checks, no mutation yet ---

        let snapshot = self.snapshot();
        Self::validate_policy_postimage(self.policy(), &snapshot, Some((&class, &owner, bytes)))?;
        self.context_usage
            .get(&context)
            .copied()
            .unwrap_or_default()
            .checked_total()?
            .checked_add(bytes)
            .ok_or_else(|| MemoryError::capacity("context reserve total exceeds u64"))?;
        for usage in [
            self.class_usage.get(&class),
            self.owner_usage.get(&owner),
            self.context_usage.get(&context),
        ] {
            if usage.map_or(0, |u| u.slot(kind)).checked_add(bytes).is_none() {
                return Err(MemoryError::capacity("ledger charge-kind slot exceeds u64"));
            }
        }
        // --- all checks passed: mutate ---
        let id = self.next_ticket_id.ok_or_else(|| {
            MemoryError::InvalidState("reservation ticket id space exhausted".to_string())
        })?;
        self.next_ticket_id = id.checked_add(1);
        self.ticket_states.insert(id, TicketState::Pending);

        // A zero-byte reserve is a real, live, single-use ticket but must not
        // create empty axis entries, so snapshots stay canonical (a 0-byte
        // reserve+release is a true no-op on the counters).
        if bytes > 0 {
            self.class_usage.entry(class.clone()).or_default().add(kind, bytes);
            self.owner_usage.entry(owner.clone()).or_default().add(kind, bytes);
            self.context_usage.entry(context).or_default().add(kind, bytes);
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

    fn preflight_release_batch(&self, tickets: &[ReservationTicket]) -> MemoryResult<()> {
        let mut snapshot = self.snapshot();
        snapshot.validate_exact()?;
        for ticket in tickets {
            if ticket.bytes == 0 {
                continue;
            }
            for usage in [
                snapshot.class_usage.get_mut(&ticket.class),
                snapshot.owner_usage.get_mut(&ticket.owner),
                snapshot.context_usage.get_mut(&ticket.context),
            ] {
                let Some(usage) = usage else {
                    return Err(MemoryError::InvalidState("release axis is missing".into()));
                };
                let slot = usage.slot_mut(ticket.kind);
                *slot = slot.checked_sub(ticket.bytes).ok_or_else(|| {
                    MemoryError::InvalidState("release exceeds charged axis slot".into())
                })?;
            }
        }
        snapshot.validate_exact()
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
    }

    /// Immutable usage snapshot for reporting and invariant assertions.
    #[must_use]
    pub fn snapshot(&self) -> LedgerSnapshot {
        LedgerSnapshot {
            class_usage: self.class_usage.clone(),
            owner_usage: self.owner_usage.clone(),
            context_usage: self.context_usage.clone(),
            live_ticket_count: self.ticket_states.len(),
            rebuild_in_progress: self.rebuild_in_progress,
        }
    }
}

#[cfg(all(test, feature = "live_control"))]
mod tests {
    use super::*;
    use crate::domain::config::ValidatedResidencyPolicy;
    use crate::domain::policy::{MemoryClassBudget, OwnerQuota, ReclaimPriority, ResidencyPolicy};

    fn class_budget(soft: u64, hard: u64, borrow: bool) -> MemoryClassBudget {
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: soft,
            hard_cap_bytes: hard,
            borrow_from_shared_pool: borrow,
            reclaim_priority: ReclaimPriority::Normal,
        }
    }

    /// Ledger unit tests that are not testing class admission use an explicitly
    /// permissive class map. Production `ResidencyPolicy::new` is fail-closed
    /// with zero budgets and never receives this test-only expansion.
    fn ledger_from_policy(mut policy: ResidencyPolicy) -> AccountingLedger {
        let global = policy.global_hard_limit_bytes;
        for class in MemoryClass::mandatory() {
            let budget = policy.classes.get_mut(&class).expect("mandatory class materialized");
            if budget.hard_cap_bytes == 0 {
                budget.soft_cap_bytes = global;
                budget.hard_cap_bytes = global;
            }
        }
        let validated = ValidatedResidencyPolicy::try_from(policy).expect("valid test policy");
        AccountingLedger::new(validated)
    }

    fn reserve(
        ledger: &mut AccountingLedger,
        class: MemoryClass,
        owner: &str,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        ledger.reserve(
            class,
            owner.to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            bytes,
        )
    }

    // ---- #1: the global hard limit is enforced on reserve (was advisory) ----

    #[test]
    fn reserve_denied_when_global_hard_limit_would_be_exceeded() {
        // Permissive class budgets and no owner quota: only global can deny.
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
        reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 600).expect("fits under global");
        let before = ledger.snapshot();

        let denied = reserve(&mut ledger, MemoryClass::EvidenceCache, "o2", 600);
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "total 1200 > global 1000 must be a typed capacity error, got {denied:?}"
        );
        // fail-closed: denied reserve leaves the ledger byte-for-byte unchanged.
        assert_eq!(ledger.snapshot(), before, "denied global reserve must not mutate");
        assert_eq!(ledger.snapshot().total_owner_bytes(), 600);
    }

    #[test]
    fn reserve_for_unconfigured_class_is_denied_without_mutation() {
        let validated = ValidatedResidencyPolicy::try_from(ResidencyPolicy::new(1_000))
            .expect("mandatory classes are total");
        let mut ledger = AccountingLedger::new(validated);
        let before = ledger.snapshot();

        let denied = reserve(&mut ledger, MemoryClass::SpillFileCache, "future", 1);

        assert!(matches!(denied, Err(MemoryError::Capacity(_))));
        assert_eq!(ledger.snapshot(), before, "unconfigured-class denial must not mutate");
    }

    #[test]
    fn global_ceiling_reserves_headroom_for_non_headroom_classes() {
        // global 1000, headroom 200 -> non-headroom classes share at most 800,
        // but the Headroom class itself may draw into the reserved band.
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000).with_headroom_bytes(200));

        let denied = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 900);
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "900 > (1000 - 200 headroom) must be denied for a non-headroom class"
        );
        assert_eq!(ledger.snapshot().total_owner_bytes(), 0, "denial must not mutate");

        // The Headroom class is allowed to consume the reserved band up to global.
        reserve(&mut ledger, MemoryClass::Headroom, "o", 900)
            .expect("Headroom class may use the reserved band up to the global ceiling");
        assert_eq!(ledger.snapshot().total_owner_bytes(), 900);
    }

    #[test]
    fn headroom_usage_does_not_consume_protected_non_headroom_band_twice() {
        let mut policy = ResidencyPolicy::new(1_000).with_headroom_bytes(200);
        policy.classes.insert(
            MemoryClass::Headroom,
            MemoryClassBudget {
                reserved_bytes: 0,
                soft_cap_bytes: 200,
                hard_cap_bytes: 200,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Last,
            },
        );
        let mut ledger = ledger_from_policy(policy);

        reserve(&mut ledger, MemoryClass::Headroom, "headroom", 200).expect("headroom band");
        reserve(&mut ledger, MemoryClass::ArtifactCache, "work", 800)
            .expect("non-headroom owns its separate 800-byte band");
        assert_eq!(ledger.snapshot().total_owner_bytes(), 1_000);
    }

    #[test]
    fn reserved_floor_protects_class_from_peer_consumption() {
        let mut policy = ResidencyPolicy::new(1_000);
        policy.classes.insert(
            MemoryClass::ArtifactCache,
            MemoryClassBudget {
                reserved_bytes: 400,
                soft_cap_bytes: 700,
                hard_cap_bytes: 1_000,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        );
        policy.classes.insert(
            MemoryClass::EvidenceCache,
            MemoryClassBudget {
                reserved_bytes: 0,
                soft_cap_bytes: 1_000,
                hard_cap_bytes: 1_000,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        );
        let mut ledger = ledger_from_policy(policy);

        assert!(matches!(
            reserve(&mut ledger, MemoryClass::EvidenceCache, "peer", 601),
            Err(MemoryError::Capacity(_))
        ));
        assert_eq!(ledger.snapshot().total_owner_bytes(), 0, "denial is fail-closed");
        reserve(&mut ledger, MemoryClass::EvidenceCache, "peer", 600).expect("unprotected band");
        reserve(&mut ledger, MemoryClass::ArtifactCache, "protected", 400)
            .expect("reserved class can consume its floor");
        assert_eq!(ledger.snapshot().total_owner_bytes(), 1_000);
    }

    // ---- #4: reservation tickets are idempotent (no double-release corruption) ----

    #[test]
    fn double_release_of_a_cloned_ticket_is_rejected_and_does_not_corrupt() {
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
        let ticket =
            reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 100_000).expect("reserve");
        ledger.commit(&ticket).expect("commit");

        // First release returns the bytes.
        ledger.release(&ticket).expect("first release succeeds");
        let after_first = ledger.snapshot();
        assert_eq!(after_first.total_owner_bytes(), 0, "release returns the bytes once");

        // A cloned ticket released a second time MUST be rejected, and must not
        // drive any counter below the truth (invariants 1 & 2 stay intact).
        let twin = ticket.clone();
        let second = ledger.release(&twin);
        assert!(
            matches!(second, Err(MemoryError::InvalidState(_))),
            "double release must be a typed invalid-state error, got {second:?}"
        );
        assert_eq!(ledger.snapshot(), after_first, "rejected double release must not mutate");
        assert!(ledger.snapshot().class_owner_balanced());
        assert!(ledger.snapshot().owner_partition_balanced());
    }

    #[test]
    fn commit_after_release_is_rejected() {
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
        let ticket =
            reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 10_000).expect("reserve");
        ledger.release(&ticket).expect("release uncommitted reservation");

        assert!(
            matches!(ledger.commit(&ticket), Err(MemoryError::InvalidState(_))),
            "committing a released ticket must be rejected"
        );
    }

    #[test]
    fn second_commit_of_same_reservation_is_rejected() {
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
        let ticket =
            reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 10_000).expect("reserve");
        let replay = ticket.clone();
        ledger.commit(&ticket).expect("first commit");
        let before = ledger.snapshot();

        let second = ledger.commit(&replay);

        assert!(matches!(second, Err(MemoryError::InvalidState(_))));
        assert_eq!(ledger.snapshot(), before, "duplicate commit must not mutate counters");
    }

    #[test]
    fn ticket_id_exhaustion_fails_without_reuse() {
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
        ledger.next_ticket_id = Some(u64::MAX);
        let last = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 1)
            .expect("last representable id is issued once");
        assert_eq!(last.id(), u64::MAX);
        let before = ledger.snapshot();

        let exhausted = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 1);

        assert!(matches!(exhausted, Err(MemoryError::InvalidState(_))));
        assert_eq!(ledger.snapshot(), before, "exhaustion must not mutate or reuse an id");
    }

    // ---- #6: borrowing over a class cap is bounded by the shared burst pool ----

    #[test]
    fn borrow_over_class_cap_is_bounded_by_shared_burst_pool() {
        // global is roomy so only the class cap / burst pool gate the reserve.
        let mut ledger = ledger_from_policy(
            ResidencyPolicy::new(10_000_000)
                .with_shared_burst_pool_bytes(100)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(400, 600, true)),
        );

        // Fill to the hard cap (no borrow yet).
        reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 600).expect("up to hard cap");
        // Borrow 50 of the 100-byte pool: admitted.
        reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 50).expect("borrow within pool");
        // A further 60 would push total borrow to 110 > 100 pool: denied.
        let denied = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 60);
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "borrow beyond the shared burst pool must be denied, got {denied:?}"
        );
        assert_eq!(ledger.class_usage(&MemoryClass::ArtifactCache).total(), 650);
    }

    #[test]
    fn shared_burst_pool_is_shared_across_borrowing_classes() {
        let mut ledger = ledger_from_policy(
            ResidencyPolicy::new(10_000_000)
                .with_shared_burst_pool_bytes(100)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(400, 600, true))
                .with_class_budget(MemoryClass::EvidenceCache, class_budget(400, 600, true)),
        );
        reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 660).expect("borrow 60 of 100");
        // Evidence wants to borrow 60 too -> total borrow 120 > 100 pool: denied.
        let denied = reserve(&mut ledger, MemoryClass::EvidenceCache, "o", 660);
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "second class cannot exceed the pool already drawn by the first"
        );
        // But borrowing only 40 more (total 100) is exactly admissible.
        reserve(&mut ledger, MemoryClass::EvidenceCache, "o", 640)
            .expect("borrow that exactly fills the remaining pool is admitted");
    }

    #[test]
    fn borrow_false_still_denies_over_cap_regardless_of_pool() {
        let mut ledger = ledger_from_policy(
            ResidencyPolicy::new(10_000_000)
                .with_shared_burst_pool_bytes(10_000)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(400, 600, false)),
        );
        let denied = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 700);
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "a non-borrowing class cannot use the pool"
        );
    }

    // sanity: an owner quota still trips independently of the new ceilings.
    #[test]
    fn owner_hard_cap_still_enforced() {
        let mut ledger = ledger_from_policy(
            ResidencyPolicy::new(10_000_000)
                .with_owner_quota("o", OwnerQuota { soft_cap_bytes: 300, hard_cap_bytes: 500 }),
        );
        reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 400).expect("under owner cap");
        assert!(matches!(
            reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 200),
            Err(MemoryError::Capacity(_))
        ));
    }

    // An EMPTY batch release is a true no-op and must succeed even during rebuild
    // (a charged store's no-op sweep/truncate that reclaimed nothing must not be
    // wrongly rejected mid-rebuild), while a NON-empty batch stays rebuild-fenced.
    #[test]
    fn empty_release_batch_is_a_noop_even_during_rebuild() {
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
        let t = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 1_000).expect("reserve");
        ledger.commit(&t).expect("commit");
        ledger.begin_rebuild();

        // Empty batch: no-op, succeeds despite the rebuild window.
        ledger.release_batch(&[]).expect("empty batch is a no-op even during rebuild");
        assert_eq!(ledger.snapshot().total_owner_bytes(), 1_000, "no-op batch changed nothing");

        // Non-empty batch: still fenced by the rebuild gate.
        assert!(
            matches!(
                ledger.release_batch(std::slice::from_ref(&t)),
                Err(MemoryError::InvalidState(_))
            ),
            "a non-empty batch release during rebuild must still be rejected"
        );
        ledger.finish_rebuild();
        ledger.release_batch(&[t]).expect("batch release resumes after rebuild");
        assert_eq!(ledger.snapshot().total_owner_bytes(), 0);
    }

    // ===================================================================
    // Scenario group A: ledger accounting invariants (extras).
    // Shared assertion: after every op the global invariants must hold.
    // ===================================================================

    /// Assert invariants 1, 2 and the third-axis (context==owner) balance on a
    /// snapshot. Used as the post-condition of every accounting op below.
    fn assert_axes_balanced(snap: &LedgerSnapshot) {
        assert!(snap.class_owner_balanced(), "invariant 1: class==owner must hold: {snap:?}");
        assert!(
            snap.owner_partition_balanced(),
            "invariant 2: owner partitions into payload+metadata+scratch: {snap:?}"
        );
        assert!(
            snap.context_owner_balanced(),
            "third axis: context total must equal owner total: {snap:?}"
        );
    }

    fn reserve_kind(
        ledger: &mut AccountingLedger,
        class: MemoryClass,
        owner: &str,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        ledger.reserve(class, owner.to_string(), context, kind, bytes)
    }

    // ---- scenario A: zero-byte reserve reserves/commits/releases coherently ----

    #[test]
    fn zero_byte_reserve_commits_and_releases_coherently() {
        // scenario A: a 0-byte ticket is a real, live, single-use ticket. It
        // must commit (return 0), release once, and stay invariant-balanced;
        // its second release is rejected (invariant 7 idempotency).
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
        let before = ledger.snapshot();

        let ticket = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 0)
            .expect("a zero-byte reserve is admitted");
        assert_eq!(ticket.bytes(), 0, "ticket carries 0 bytes");
        assert_eq!(ledger.snapshot().live_ticket_count, 1);
        // A 0-byte reserve must not move any counter.
        assert_eq!(ledger.snapshot().total_owner_bytes(), 0, "0 bytes charged nothing");
        assert_axes_balanced(&ledger.snapshot());

        // Commit reports the (zero) charged bytes for an honest, live ticket.
        assert_eq!(ledger.commit(&ticket).expect("commit live zero-byte ticket"), 0);
        assert_eq!(ledger.snapshot().live_ticket_count, 1);

        // First release succeeds and the ledger returns to its pre-reserve state.
        ledger.release(&ticket).expect("first release of zero-byte ticket");
        assert_eq!(ledger.snapshot().live_ticket_count, 0);
        assert_eq!(ledger.snapshot(), before, "release of a 0-byte ticket is a no-op on counters");

        // Invariant 7 still bites for a zero-byte ticket: the second release of
        // the same id is rejected and nothing changes.
        let after_release = ledger.snapshot();
        let second = ledger.release(&ticket);
        assert!(
            matches!(second, Err(MemoryError::InvalidState(_))),
            "second release of a zero-byte ticket must be rejected, got {second:?}"
        );
        assert_eq!(ledger.snapshot(), after_release, "rejected release must not mutate");
    }

    // ---- scenario A: saturating_add near u64::MAX preserves invariant 1 ----

    #[test]
    fn overflow_denial_preserves_exact_live_ticket_accounting() {
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(u64::MAX));
        let first = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", u64::MAX - 10)
            .expect("first fits within validated global policy");
        ledger.commit(&first).expect("first commits");
        let before = ledger.snapshot();
        let second = reserve(&mut ledger, MemoryClass::EvidenceCache, "o", 100);
        assert!(
            matches!(second, Err(MemoryError::Capacity(_))),
            "overflow must deny, got {second:?}"
        );
        assert_eq!(ledger.snapshot(), before, "denial must leave all axes unchanged");
    }

    #[test]
    fn exact_boundary_ticket_survives_first_release() {
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(u64::MAX));
        let first = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", u64::MAX - 10)
            .expect("first reserve");
        ledger.commit(&first).expect("first commit");
        let second = reserve(&mut ledger, MemoryClass::EvidenceCache, "o", 10)
            .expect("sum equals u64::MAX exactly");
        ledger.commit(&second).expect("second commit");
        ledger.release(&first).expect("release first");
        assert_eq!(
            ledger.snapshot().total_owner_bytes(),
            10,
            "second live ticket must remain exactly charged"
        );
        ledger.release(&second).expect("release second");
        assert_eq!(ledger.snapshot().total_owner_bytes(), 0);
    }

    #[test]
    fn unrepresentable_sum_is_denied_without_mutating_any_axis() {
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(u64::MAX));
        let first = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", u64::MAX - 10)
            .expect("first huge reserve fits under u64::MAX global");
        let before = ledger.snapshot();
        assert!(matches!(
            reserve(&mut ledger, MemoryClass::EvidenceCache, "other", 100),
            Err(MemoryError::Capacity(_))
        ));
        assert_eq!(ledger.snapshot(), before);
        ledger.release(&first).expect("original ticket remains live and releasable");
        assert_eq!(ledger.snapshot().total_owner_bytes(), 0);
    }

    #[test]
    fn batch_release_preflights_every_charge_kind_before_first_ticket_mutates() {
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000));
        let first =
            reserve(&mut ledger, MemoryClass::ArtifactCache, "first", 100).expect("first reserve");
        let second = reserve(&mut ledger, MemoryClass::EvidenceCache, "second", 100)
            .expect("second reserve");
        // Keep the class total unchanged while corrupting the second ticket's
        // exact kind slot. A totals-only preflight would miss this mismatch.
        let usage = ledger.class_usage.get_mut(&MemoryClass::EvidenceCache).expect("second class");
        usage.retained_payload_bytes = 0;
        usage.retained_metadata_bytes = 100;
        let before = ledger.snapshot();
        assert!(matches!(
            ledger.release_batch(&[first.clone(), second.clone()]),
            Err(MemoryError::InvalidState(_))
        ));
        assert_eq!(ledger.snapshot(), before, "preflight failure must not release first ticket");
        assert!(ledger.ticket_states.contains_key(&first.id()));
        assert!(ledger.ticket_states.contains_key(&second.id()));
    }

    #[test]
    fn snapshot_validation_rejects_unrepresentable_axis_instead_of_saturating_it() {
        let mut snapshot = LedgerSnapshot::default();
        snapshot.owner_usage.insert(
            "owner".into(),
            UsageBreakdown {
                retained_payload_bytes: u64::MAX,
                retained_metadata_bytes: 1,
                live_scratch_bytes: 0,
            },
        );
        assert!(matches!(snapshot.validate_exact(), Err(MemoryError::InvalidState(_))));
        assert!(std::panic::catch_unwind(|| snapshot.total_owner_bytes()).is_err());
    }

    // ---- scenario A: bytes > i64::MAX must not wrap to a negative delta ----

    #[test]
    fn reserve_above_i64_max_does_not_wrap_to_negative_delta() {
        // scenario A: regression guard for the old `bytes as i64` cast. A charge
        // of bytes > i64::MAX, when routed through i64, wraps to a NEGATIVE delta
        // and SUBTRACTS — silently corrupting the counters. The fixed path adds
        // the full u64 magnitude.
        let big: u64 = (i64::MAX as u64) + 12_345; // > i64::MAX, fits in u64
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(u64::MAX));

        let ticket = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", big)
            .expect("reserve of bytes>i64::MAX is admitted under a u64::MAX global");

        let snap = ledger.snapshot();
        // If the cast had wrapped, this would be a small/garbage value, not `big`.
        assert_eq!(
            snap.total_owner_bytes(),
            big,
            "bytes>i64::MAX must be charged at full magnitude, not a wrapped delta"
        );
        assert_eq!(
            snap.class_usage[&MemoryClass::ArtifactCache].retained_payload_bytes,
            big,
            "class axis charged the full magnitude"
        );
        assert_axes_balanced(&snap);

        // And release must subtract the same full magnitude back to zero.
        ledger.release(&ticket).expect("release the huge ticket");
        let after = ledger.snapshot();
        assert_eq!(after.total_owner_bytes(), 0, "release subtracts the full magnitude");
        assert_axes_balanced(&after);
    }

    // ---- scenario A: cross-ledger ticket misuse is rejected by the idempotency guard ----

    #[test]
    fn ticket_from_one_ledger_cannot_release_on_another() {
        // scenario A: ledgers A and B have overlapping id spaces (both start at
        // 1). A ticket issued by A is not live in B, so releasing it on B must be
        // rejected (invariant 7) and must leave B byte-for-byte unchanged.
        let mut ledger_a = ledger_from_policy(ResidencyPolicy::new(1_000_000));
        let mut ledger_b = ledger_from_policy(ResidencyPolicy::new(1_000_000));

        let a_ticket = reserve(&mut ledger_a, MemoryClass::ArtifactCache, "o", 500).expect("A");
        // Give B its own state with the SAME id (1), so this is not merely an
        // unknown id but a genuine id-space collision.
        let b_ticket = reserve(&mut ledger_b, MemoryClass::EvidenceCache, "o2", 700).expect("B");
        assert_eq!(a_ticket.id(), b_ticket.id(), "overlapping id space (both start at 1)");

        let before_b = ledger_b.snapshot();
        let misuse = ledger_b.release(&a_ticket);
        assert!(
            matches!(misuse, Err(MemoryError::InvalidState(_))),
            "releasing A's ticket on B must be a typed invalid-state error, got {misuse:?}"
        );
        assert_eq!(ledger_b.snapshot(), before_b, "B must be unchanged by the foreign release");
        // B's own ticket still releases honestly afterward.
        ledger_b.release(&b_ticket).expect("B's own ticket still releases");
        assert_eq!(ledger_b.snapshot().total_owner_bytes(), 0);
    }

    // ---- scenario A: next_ticket_id is monotonic and unique across N reserves ----

    #[test]
    fn ticket_ids_are_monotonic_and_unique_across_many_reserves() {
        // scenario A: every reserve must mint a fresh, strictly increasing id so
        // the idempotency set can never confuse two distinct reservations.
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(10_000_000));
        let mut ids = Vec::new();
        let mut prev: Option<u64> = None;
        for i in 0..64u64 {
            let t = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", i).expect("reserve");
            if let Some(p) = prev {
                assert!(t.id() > p, "ids must be strictly increasing: {} !> {p}", t.id());
            }
            prev = Some(t.id());
            ids.push(t.id());
        }
        let unique: BTreeSet<u64> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "every reserve mints a unique id");
    }

    // ---- scenario A: context axis balances against owner axis in a mixed sequence ----

    #[test]
    fn context_axis_balances_with_owner_after_mixed_sequence() {
        // scenario A: the third axis. After a mix of reserves across different
        // contexts/owners/classes (and one release), total context usage must
        // equal total owner usage at every step.
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(10_000_000));

        let steps: &[(MemoryClass, &str, MemoryContextKind, ChargeKind, u64)] = &[
            (
                MemoryClass::ArtifactCache,
                "a",
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                100,
            ),
            (
                MemoryClass::EvidenceCache,
                "b",
                MemoryContextKind::Session,
                ChargeKind::RetainedMetadata,
                200,
            ),
            (
                MemoryClass::TraceIndex,
                "a",
                MemoryContextKind::Background,
                ChargeKind::LiveScratch,
                300,
            ),
            (
                MemoryClass::ArtifactCache,
                "c",
                MemoryContextKind::Generation,
                ChargeKind::RetainedPayload,
                400,
            ),
        ];
        let mut tickets = Vec::new();
        for (class, owner, ctx, kind, bytes) in steps {
            let t = reserve_kind(&mut ledger, class.clone(), owner, *ctx, *kind, *bytes)
                .expect("mixed reserve");
            tickets.push(t);
            let snap = ledger.snapshot();
            assert_eq!(
                snap.total_context_bytes(),
                snap.total_owner_bytes(),
                "context axis must track owner axis after each reserve"
            );
            assert_axes_balanced(&snap);
        }
        // Release one ticket and re-check the third axis still balances.
        ledger.release(&tickets[1]).expect("release the metadata ticket");
        let snap = ledger.snapshot();
        assert_eq!(snap.total_context_bytes(), snap.total_owner_bytes(), "balances after release");
        assert_axes_balanced(&snap);
    }

    // ---- scenario A: wrong-kind/mixed-kind release keeps the partition (invariant 2) ----

    #[test]
    fn mixed_kind_release_preserves_charge_kind_partition() {
        // scenario A: payload and metadata charges live in different partition
        // slots (invariant 2). Releasing the metadata ticket must subtract ONLY
        // from the metadata slot, leaving the payload slot intact and the
        // partition exact.
        let mut ledger = ledger_from_policy(ResidencyPolicy::new(10_000_000));
        let payload = reserve_kind(
            &mut ledger,
            MemoryClass::ArtifactCache,
            "o",
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            500,
        )
        .expect("payload reserve");
        let metadata = reserve_kind(
            &mut ledger,
            MemoryClass::ArtifactCache,
            "o",
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata,
            300,
        )
        .expect("metadata reserve");

        let snap = ledger.snapshot();
        assert_eq!(snap.total_retained_payload_bytes(), 500);
        assert_eq!(snap.total_retained_metadata_bytes(), 300);
        assert_axes_balanced(&snap);

        // Release metadata: payload slot must be untouched.
        ledger.release(&metadata).expect("release metadata ticket");
        let snap = ledger.snapshot();
        assert_eq!(snap.total_retained_metadata_bytes(), 0, "metadata slot drained");
        assert_eq!(
            snap.total_retained_payload_bytes(),
            500,
            "payload slot untouched by metadata release"
        );
        assert_axes_balanced(&snap);

        // Release payload: everything returns to zero, partition still exact.
        ledger.release(&payload).expect("release payload ticket");
        let snap = ledger.snapshot();
        assert_eq!(snap.total_owner_bytes(), 0);
        assert_axes_balanced(&snap);
    }

    // ---- scenario A: deterministic property-style fuzz keeps invariants at every step ----

    #[test]
    fn deterministic_fuzz_preserves_invariants_and_returns_to_zero() {
        // scenario A: a fixed, index-derived (no RNG crate) sequence of reserves
        // and releases across several owners/classes/contexts/kinds. Invariants
        // 1, 2, 5 (<= global) and 6 (within burst pool) must hold at EVERY step,
        // and releasing every live ticket must return all totals to exactly 0.
        let global = 10_000_000u64;
        let mut ledger = ledger_from_policy(
            ResidencyPolicy::new(global)
                .with_shared_burst_pool_bytes(50_000)
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(40_000, 60_000, true))
                .with_class_budget(MemoryClass::EvidenceCache, class_budget(40_000, 60_000, true)),
        );

        let classes = [
            MemoryClass::ArtifactCache,
            MemoryClass::EvidenceCache,
            MemoryClass::TraceIndex, // no budget configured -> only global gates it
        ];
        let owners = ["o0", "o1", "o2"];
        let contexts =
            [MemoryContextKind::Request, MemoryContextKind::Session, MemoryContextKind::Background];
        let kinds =
            [ChargeKind::RetainedPayload, ChargeKind::RetainedMetadata, ChargeKind::LiveScratch];

        let mut live: Vec<ReservationTicket> = Vec::new();

        for i in 0u64..400 {
            // Deterministic pseudo-random decisions derived purely from `i`.
            let do_release = (i % 3 == 2) && !live.is_empty();
            if do_release {
                // Pick a live ticket deterministically and release it.
                let idx = (i.wrapping_mul(2_654_435_761) % (live.len() as u64)) as usize;
                let t = live.swap_remove(idx);
                ledger.release(&t).expect("releasing a live ticket always succeeds");
            } else {
                let class = classes[(i % classes.len() as u64) as usize].clone();
                let owner = owners[((i / 2) % owners.len() as u64) as usize];
                let ctx = contexts[((i / 3) % contexts.len() as u64) as usize];
                let kind = kinds[((i / 5) % kinds.len() as u64) as usize];
                // Modest byte sizes so we mostly stay admissible; some reserves
                // may be denied by a class/global cap — a denial is fine and
                // must not corrupt anything.
                let bytes = 1 + (i.wrapping_mul(97) % 2_000);
                if let Ok(t) = ledger.reserve(class, owner.to_string(), ctx, kind, bytes) {
                    live.push(t);
                }
            }

            // Invariants hold at EVERY step, admitted or denied.
            let snap = ledger.snapshot();
            assert!(snap.class_owner_balanced(), "inv 1 at step {i}: {snap:?}");
            assert!(snap.owner_partition_balanced(), "inv 2 at step {i}: {snap:?}");
            assert!(snap.context_owner_balanced(), "3rd axis at step {i}: {snap:?}");
            assert!(
                snap.total_owner_bytes() <= global,
                "inv 5 at step {i}: total {} > global {global}",
                snap.total_owner_bytes()
            );
            // Inv 6: no borrowing class may exceed cap+its share of the pool; in
            // aggregate, over-cap borrow never exceeds the pool.
            let over_artifact =
                ledger.class_usage(&MemoryClass::ArtifactCache).total().saturating_sub(60_000);
            let over_evidence =
                ledger.class_usage(&MemoryClass::EvidenceCache).total().saturating_sub(60_000);
            assert!(
                over_artifact.saturating_add(over_evidence) <= 50_000,
                "inv 6 at step {i}: total borrow exceeds shared pool"
            );
        }

        // Drain every remaining live ticket: totals must return to exactly 0.
        for t in live.drain(..) {
            ledger.release(&t).expect("final drain release succeeds");
        }
        let snap = ledger.snapshot();
        assert_eq!(snap.total_owner_bytes(), 0, "all owner bytes returned");
        assert_eq!(snap.total_class_bytes(), 0, "all class bytes returned");
        assert_eq!(snap.total_context_bytes(), 0, "all context bytes returned");
        assert_axes_balanced(&snap);
    }
}
