//! Pure admission preflight (FUP-003).
//!
//! The broker must decide BEFORE touching any callback whether a reservation
//! is admissible, reclaimable-but-denied, terminally impossible, or blocked.
//! This module is that decision as a pure function: the locked domain state
//! plus the request are the only inputs, so the same inputs always produce the
//! same verdict, and unit tests can drive it without a broker.
//!
//! Rule order mirrors the attempt path (`reserve_on` into `ledger.reserve`)
//! exactly, and every denial carries the byte-identical error the attempt
//! would have produced — the preflight never invents messages. The attempt
//! path delegates to these same checks, so the two cannot drift; a randomized
//! consistency test in `api::control` locks the agreement empirically.
//!
//! Classification rule: a live breach is a [`AdmissionVerdict::ReclaimCandidate`]
//! when the same check passes against zero usage (freeing memory can fix it),
//! and a [`AdmissionVerdict::TerminalDenial`] when it still breaches at zero
//! (no callback can help). [`AdmissionVerdict::Blocked`] covers non-capacity
//! refusals (rebuild fence, unknown class, exhausted ticket ID space, internal
//! inconsistency), which never fire a callback either.

use std::collections::BTreeMap;

use crate::domain::context::{MemoryContextKind, OwnerKey};
use crate::domain::errors::MemoryError;
use crate::domain::ledger::{ChargeKind, ReservationTicket, UsageBreakdown};
use crate::domain::policy::{MemoryClass, ResidencyPolicy, ResourceGroup};
use crate::ports::reclaim::{ReclaimScope, ReclaimTarget, ReclaimUnit};

/// One structured capacity limit that a reservation can breach.
///
/// `postimage` is `None` when the denied postimage is not representable as a
/// `u64` (an intermediate overflow); every other breach reports the exact
/// postimage the attempt computed. The carried error is byte-identical to the
/// attempt path's denial — structure is additive, never a rephrasing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LimitBreach {
    pub(crate) axis: LimitAxis,
    pub(crate) limit: u64,
    pub(crate) postimage: Option<u64>,
    pub(crate) deficit: u64,
    error: MemoryError,
}

/// The limiting axis of a [`LimitBreach`], with the components needed to
/// reproduce the attempt path's exact denial message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LimitAxis {
    LiveTicketCount,
    Global,
    Class { class: MemoryClass },
    SharedBorrowPool,
    ProtectedBand { non_headroom_after: u64, unused_class_floor: u64 },
    Owner { owner: OwnerKey },
    ContextTotal { context: MemoryContextKind },
    KindSlot { kind: ChargeKind },
    Group { group: String },
    GroupFloor { group: Option<String>, usage: u64, floors: u128 },
    NonHeadroomTotal,
    ClassFloorSum,
    ExecutionScratch { max: u64 },
}

/// Non-capacity refusal. Blocked requests never fire a reclaim callback and
/// are never re-evaluated against zero usage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AdmissionBlock {
    RebuildInProgress,
    UnknownClass {
        class: MemoryClass,
    },
    TicketIdSpaceExhausted,
    /// Unreachable internal inconsistency, carried with its original error so
    /// the preflight fails closed exactly like the attempt would.
    Internal(MemoryError),
}

/// One evaluation of the admission chain: either a non-capacity block or a
/// structured capacity breach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AdmissionFailure {
    Block(AdmissionBlock),
    Breach(LimitBreach),
}

/// Preflight verdict for one reservation request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AdmissionVerdict {
    /// The attempt would succeed. The caller may still run one proactive soft
    /// reclaim when the request crosses a soft threshold.
    Admit,
    /// Denied, but the same check passes at zero usage: run one hard reclaim
    /// and remeasure exactly once.
    ReclaimCandidate { breach: LimitBreach },
    /// Denied even at zero usage: fail immediately without any callback.
    TerminalDenial { breach: LimitBreach },
    /// Refused without consulting capacity: fail immediately, no callback.
    Blocked { reason: AdmissionBlock },
}

/// Read-only usage maps for one preflight evaluation: live maps for the live
/// verdict, empty maps for the zero-usage terminality probe.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UsageView<'a> {
    pub class_usage: &'a BTreeMap<MemoryClass, UsageBreakdown>,
    pub owner_usage: crate::domain::ledger::OwnerUsageRead<'a>,
    pub context_usage: &'a BTreeMap<MemoryContextKind, UsageBreakdown>,
    pub total_usage: &'a UsageBreakdown,
    /// Validated live committed authority excluded only from the read view.
    /// The real ledger keeps this ticket charged during any host callback.
    pub credit: Option<&'a ReservationTicket>,
}

impl<'a> UsageView<'a> {
    pub(crate) fn with_credit(mut self, ticket: &'a ReservationTicket) -> Self {
        self.credit = Some(ticket);
        self
    }

    fn adjusted(
        &self,
        mut usage: UsageBreakdown,
        matches: bool,
    ) -> Result<UsageBreakdown, MemoryError> {
        if let Some(credit) = self.credit.filter(|_| matches) {
            let slot = match credit.kind() {
                ChargeKind::RetainedPayload => &mut usage.retained_payload_bytes,
                ChargeKind::RetainedMetadata => &mut usage.retained_metadata_bytes,
                ChargeKind::LiveScratch => &mut usage.live_scratch_bytes,
            };
            *slot = slot.checked_sub(credit.bytes()).ok_or_else(|| {
                MemoryError::InvalidState("replacement credit exceeds its live usage axis".into())
            })?;
        }
        Ok(usage)
    }

    pub(crate) fn class(&self, class: &MemoryClass) -> Result<UsageBreakdown, MemoryError> {
        self.adjusted(
            self.class_usage.get(class).copied().unwrap_or_default(),
            self.credit.is_some_and(|credit| credit.class() == class),
        )
    }

    pub(crate) fn owner(&self, owner: &OwnerKey) -> Result<UsageBreakdown, MemoryError> {
        self.adjusted(
            self.owner_usage.get(owner).copied().unwrap_or_default(),
            self.credit.is_some_and(|credit| credit.owner() == owner),
        )
    }

    pub(crate) fn context(
        &self,
        context: MemoryContextKind,
    ) -> Result<UsageBreakdown, MemoryError> {
        self.adjusted(
            self.context_usage.get(&context).copied().unwrap_or_default(),
            self.credit.is_some_and(|credit| credit.context() == context),
        )
    }

    pub(crate) fn total(&self) -> Result<UsageBreakdown, MemoryError> {
        self.adjusted(*self.total_usage, true)
    }
}

/// One reservation request under evaluation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AdmissionRequest<'a> {
    pub class: &'a MemoryClass,
    pub owner: &'a OwnerKey,
    pub context: MemoryContextKind,
    pub kind: ChargeKind,
    pub bytes: u64,
}

/// Complete preflight inputs: every number the attempt path would read.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PreflightInputs<'a> {
    pub policy: &'a ResidencyPolicy,
    pub usage: UsageView<'a>,
    pub non_headroom_bytes: u64,
    pub live_scratch_bytes: u64,
    pub execution_scratch_max: Option<usize>,
    pub fair_burst_shares: &'a BTreeMap<String, u64>,
    pub group_current_usage: u64,
    pub group_current_non_headroom_usage: u64,
    pub unused_group_floors: u128,
    pub owner_group: Option<&'a str>,
    pub request: AdmissionRequest<'a>,
    pub rebuild_in_progress: bool,
    pub ticket_id_space_exhausted: bool,
    pub live_ticket_limit_reached: bool,
    /// Reserve adds one authority; a validated replacement adds none.
    pub ticket_count_delta: u8,
}

impl LimitBreach {
    pub(crate) fn new(
        axis: LimitAxis,
        limit: u64,
        postimage: Option<u64>,
        error: MemoryError,
    ) -> Self {
        let deficit =
            postimage.map(|postimage| postimage.saturating_sub(limit)).unwrap_or(u64::MAX);
        Self { axis, limit, postimage, deficit, error }
    }

    /// The byte-identical denial the attempt path would have produced.
    #[must_use]
    pub(crate) fn into_error(self) -> MemoryError {
        self.error
    }

    pub(crate) fn reclaim_target(&self) -> ReclaimTarget {
        let (unit, scope) = match &self.axis {
            LimitAxis::LiveTicketCount => (ReclaimUnit::LiveTickets, ReclaimScope::Global),
            LimitAxis::Global => (ReclaimUnit::Bytes, ReclaimScope::Global),
            LimitAxis::Class { class } => (ReclaimUnit::Bytes, ReclaimScope::Class(class.clone())),
            LimitAxis::Owner { owner } => (ReclaimUnit::Bytes, ReclaimScope::Owner(owner.clone())),
            LimitAxis::Group { group } => {
                (ReclaimUnit::Bytes, ReclaimScope::ResourceGroup(group.clone()))
            }
            LimitAxis::SharedBorrowPool => (ReclaimUnit::Bytes, ReclaimScope::SharedBorrowPool),
            LimitAxis::ContextTotal { context } => {
                (ReclaimUnit::Bytes, ReclaimScope::Context(*context))
            }
            LimitAxis::KindSlot { kind } => (ReclaimUnit::Bytes, ReclaimScope::ChargeKind(*kind)),
            LimitAxis::ExecutionScratch { .. } => {
                (ReclaimUnit::Bytes, ReclaimScope::ExecutionScratch)
            }
            LimitAxis::ProtectedBand { .. }
            | LimitAxis::GroupFloor { .. }
            | LimitAxis::NonHeadroomTotal
            | LimitAxis::ClassFloorSum => (ReclaimUnit::Bytes, ReclaimScope::NonHeadroom),
        };
        ReclaimTarget { unit, scope, minimum_reduction: self.postimage.map(|_| self.deficit) }
    }
}

impl AdmissionBlock {
    /// The byte-identical refusal the attempt path would have produced.
    #[must_use]
    pub(crate) fn into_error(self) -> MemoryError {
        match self {
            Self::RebuildInProgress => {
                MemoryError::InvalidState("rebuild in progress: live reserve rejected".to_string())
            }
            Self::UnknownClass { class } => MemoryError::capacity(format!(
                "class {class:?} is not configured in the active residency policy"
            )),
            Self::TicketIdSpaceExhausted => {
                MemoryError::InvalidState("reservation ticket id space exhausted".to_string())
            }
            Self::Internal(error) => error,
        }
    }
}

impl AdmissionFailure {
    #[must_use]
    pub(crate) fn into_error(self) -> MemoryError {
        match self {
            Self::Block(reason) => reason.into_error(),
            Self::Breach(breach) => breach.into_error(),
        }
    }
}

fn current_slot(usage: &UsageBreakdown, kind: ChargeKind) -> u64 {
    match kind {
        ChargeKind::RetainedPayload => usage.retained_payload_bytes,
        ChargeKind::RetainedMetadata => usage.retained_metadata_bytes,
        ChargeKind::LiveScratch => usage.live_scratch_bytes,
    }
}

/// Ledger-side admission chain: the exact rule sequence of
/// `validate_reserve_postimage` plus the context-total and charge-kind-slot
/// overflow guards from `reserve`, in attempt order.
pub(crate) fn check_ledger_postimage(
    policy: &ResidencyPolicy,
    usage: &UsageView,
    request: &AdmissionRequest,
) -> Result<(), AdmissionFailure> {
    let class = request.class;
    let owner = request.owner;
    let bytes = request.bytes;
    if !policy.classes.contains_key(class) {
        return Err(AdmissionFailure::Block(AdmissionBlock::UnknownClass { class: class.clone() }));
    }

    // The global aggregate is read fallibly like every other axis total in
    // this chain. Live-ledger totals are representable by construction (each
    // reserve checks total+bytes before mutating), so this arm is
    // defense-in-depth for the pure function — but a pure predicate must fail
    // closed on any input, never panic.
    let global_current = usage
        .total()
        .map_err(|error| AdmissionFailure::Block(AdmissionBlock::Internal(error)))?
        .checked_total()
        .map_err(|error| AdmissionFailure::Block(AdmissionBlock::Internal(error)))?;
    let global_after = global_current.checked_add(bytes).ok_or_else(|| {
        AdmissionFailure::Breach(LimitBreach::new(
            LimitAxis::Global,
            policy.global_hard_limit_bytes,
            None,
            MemoryError::capacity("global postimage exceeds u64"),
        ))
    })?;
    if global_after > policy.global_hard_limit_bytes {
        return Err(AdmissionFailure::Breach(LimitBreach::new(
            LimitAxis::Global,
            policy.global_hard_limit_bytes,
            Some(global_after),
            MemoryError::capacity(format!(
                "global hard limit {} below postimage {global_after}",
                policy.global_hard_limit_bytes
            )),
        )));
    }

    let mut non_headroom_after = 0u64;
    let mut unused_class_floor = 0u64;
    let mut total_borrow = 0u64;
    for (candidate, budget) in &policy.classes {
        let current = usage
            .class(candidate)
            .map_err(|error| AdmissionFailure::Block(AdmissionBlock::Internal(error)))?
            .checked_total()
            .map_err(|error| AdmissionFailure::Block(AdmissionBlock::Internal(error)))?;
        let used = if candidate == class {
            current.checked_add(bytes).ok_or_else(|| {
                AdmissionFailure::Breach(LimitBreach::new(
                    LimitAxis::Class { class: class.clone() },
                    budget.hard_cap_bytes,
                    None,
                    MemoryError::capacity("class postimage exceeds u64"),
                ))
            })?
        } else {
            current
        };
        if *candidate != MemoryClass::Headroom {
            non_headroom_after = non_headroom_after.checked_add(used).ok_or_else(|| {
                AdmissionFailure::Breach(LimitBreach::new(
                    LimitAxis::NonHeadroomTotal,
                    u64::MAX,
                    None,
                    MemoryError::capacity("non-headroom postimage exceeds u64"),
                ))
            })?;
            unused_class_floor = unused_class_floor
                .checked_add(budget.reserved_bytes.saturating_sub(used))
                .ok_or_else(|| {
                    AdmissionFailure::Breach(LimitBreach::new(
                        LimitAxis::ClassFloorSum,
                        u64::MAX,
                        None,
                        MemoryError::capacity("class reserved floor exceeds u64"),
                    ))
                })?;
        }
        if used > budget.hard_cap_bytes {
            if !budget.borrow_from_shared_pool {
                return Err(AdmissionFailure::Breach(LimitBreach::new(
                    LimitAxis::Class { class: candidate.clone() },
                    budget.hard_cap_bytes,
                    Some(used),
                    MemoryError::capacity(format!(
                        "non-borrowing class {candidate:?} hard cap {} below postimage {used}",
                        budget.hard_cap_bytes
                    )),
                )));
            }
            total_borrow =
                total_borrow.checked_add(used - budget.hard_cap_bytes).ok_or_else(|| {
                    AdmissionFailure::Breach(LimitBreach::new(
                        LimitAxis::SharedBorrowPool,
                        policy.shared_burst_pool_bytes,
                        None,
                        MemoryError::capacity("class borrow exceeds u64"),
                    ))
                })?;
        }
    }
    if total_borrow > policy.shared_burst_pool_bytes {
        return Err(AdmissionFailure::Breach(LimitBreach::new(
            LimitAxis::SharedBorrowPool,
            policy.shared_burst_pool_bytes,
            Some(total_borrow),
            MemoryError::capacity(format!(
                "class borrow {total_borrow} exceeds shared pool {}",
                policy.shared_burst_pool_bytes
            )),
        )));
    }
    let non_headroom_band =
        policy.global_hard_limit_bytes.checked_sub(policy.headroom_bytes).ok_or_else(|| {
            AdmissionFailure::Block(AdmissionBlock::Internal(MemoryError::capacity(
                "headroom exceeds global hard limit",
            )))
        })?;
    let protected_postimage =
        non_headroom_after.checked_add(unused_class_floor).ok_or_else(|| {
            AdmissionFailure::Breach(LimitBreach::new(
                LimitAxis::ProtectedBand { non_headroom_after, unused_class_floor },
                non_headroom_band,
                None,
                MemoryError::capacity("class protected postimage exceeds u64"),
            ))
        })?;
    if protected_postimage > non_headroom_band {
        return Err(AdmissionFailure::Breach(LimitBreach::new(
            LimitAxis::ProtectedBand { non_headroom_after, unused_class_floor },
            non_headroom_band,
            Some(protected_postimage),
            MemoryError::capacity(format!(
                "non-headroom usage {non_headroom_after} plus unused class floors \
                 {unused_class_floor} exceeds band {non_headroom_band}"
            )),
        )));
    }

    if let Some(quota) = policy.owner_quotas.get(owner) {
        let owner_after = usage
            .owner(owner)
            .map_err(|error| AdmissionFailure::Block(AdmissionBlock::Internal(error)))?
            .checked_total()
            .map_err(|error| AdmissionFailure::Block(AdmissionBlock::Internal(error)))?
            .checked_add(bytes)
            .ok_or_else(|| {
                AdmissionFailure::Breach(LimitBreach::new(
                    LimitAxis::Owner { owner: owner.clone() },
                    quota.hard_cap_bytes,
                    None,
                    MemoryError::capacity("owner postimage exceeds u64"),
                ))
            })?;
        if owner_after > quota.hard_cap_bytes {
            return Err(AdmissionFailure::Breach(LimitBreach::new(
                LimitAxis::Owner { owner: owner.clone() },
                quota.hard_cap_bytes,
                Some(owner_after),
                MemoryError::capacity(format!(
                    "owner {owner} hard cap {} below postimage {owner_after}",
                    quota.hard_cap_bytes
                )),
            )));
        }
    }

    usage
        .context(request.context)
        .map_err(|error| AdmissionFailure::Block(AdmissionBlock::Internal(error)))?
        .checked_total()
        .map_err(|error| AdmissionFailure::Block(AdmissionBlock::Internal(error)))?
        .checked_add(bytes)
        .ok_or_else(|| {
            AdmissionFailure::Breach(LimitBreach::new(
                LimitAxis::ContextTotal { context: request.context },
                u64::MAX,
                None,
                MemoryError::capacity("context reserve total exceeds u64"),
            ))
        })?;
    for slot in
        [usage.class(class), usage.owner(owner), usage.context(request.context), usage.total()]
    {
        let slot =
            slot.map_err(|error| AdmissionFailure::Block(AdmissionBlock::Internal(error)))?;
        if current_slot(&slot, request.kind).checked_add(bytes).is_none() {
            return Err(AdmissionFailure::Breach(LimitBreach::new(
                LimitAxis::KindSlot { kind: request.kind },
                u64::MAX,
                None,
                MemoryError::capacity("ledger charge-kind slot exceeds u64"),
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
std::thread_local! {
    // Thread-local instrumentation isolates concurrently executing unit tests.
    static FAIR_SHARE_PRECOMPUTE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn fair_share_precompute_calls() -> usize {
    FAIR_SHARE_PRECOMPUTE_CALLS.with(std::cell::Cell::get)
}

/// Precompute Hamilton's largest-remainder allocation. Every group receives
/// the floor of its exact weighted share, then leftover bytes are assigned
/// by descending fractional remainder with the stable group name as the
/// tie break. The sum is exactly the configured pool when total weight is
/// non-zero, including huge inputs.
///
/// This is policy-static math shared by the scheduler (which caches it) and
/// the preflight (which reads the cache); a single implementation keeps the
/// two from drifting.
pub(crate) fn precompute_fair_burst_shares(
    groups: &BTreeMap<String, ResourceGroup>,
    shared_burst_pool_bytes: u64,
) -> BTreeMap<String, u64> {
    #[cfg(test)]
    FAIR_SHARE_PRECOMPUTE_CALLS.with(|calls| calls.set(calls.get() + 1));
    let total: u128 = groups.values().map(|group| u128::from(group.admission_weight)).sum();
    if total == 0 {
        return groups.keys().map(|name| (name.clone(), 0)).collect();
    }
    let pool = u128::from(shared_burst_pool_bytes);
    let mut shares = BTreeMap::new();
    let mut allocated_base = 0u128;
    let mut remainders: Vec<(&str, u128)> = groups
        .iter()
        .map(|(name, candidate)| {
            let numerator = pool * u128::from(candidate.admission_weight);
            let base = numerator / total;
            allocated_base += base;
            shares.insert(
                name.clone(),
                u64::try_from(base).expect("a weighted share cannot exceed the u64 pool"),
            );
            (name.as_str(), numerator % total)
        })
        .collect();
    remainders.sort_by(|(left_name, left_remainder), (right_name, right_remainder)| {
        right_remainder.cmp(left_remainder).then_with(|| left_name.cmp(right_name))
    });
    let leftover = pool - allocated_base;
    for (index, (name, _)) in remainders.into_iter().enumerate() {
        if u128::try_from(index).expect("usize always fits u128") >= leftover {
            break;
        }
        if let Some(share) = shares.get_mut(name) {
            *share += 1;
        }
    }
    shares
}

/// One group's burst allowance: its cached fair share capped by its
/// percentage of the shared pool.
pub(crate) fn burst_allowance(
    shared_burst_pool_bytes: u64,
    fair_share: u64,
    group: &ResourceGroup,
) -> u64 {
    let percentage_cap =
        u64::try_from(u128::from(shared_burst_pool_bytes) * u128::from(group.burst_percent) / 100)
            .unwrap_or(u64::MAX);
    fair_share.min(percentage_cap)
}

/// The absolute admission ceiling of one group: hard cap plus burst
/// allowance, saturating. A request at or below the ceiling with empty usage
/// is always group-admissible.
pub(crate) fn group_admission_ceiling(
    group: &ResourceGroup,
    shared_burst_pool_bytes: u64,
    fair_share: u64,
) -> u64 {
    group.hard_cap_bytes.saturating_add(burst_allowance(shared_burst_pool_bytes, fair_share, group))
}

/// Outcome of [`check_group_admission`]: the boolean decision plus the
/// exact denial facts — WITHOUT formatting the message. The denial message
/// is built only by [`GroupAdmission::into_breach`], so boolean probes
/// (`admits`, and the preflight's zero-state evaluation) never allocate on
/// the deny path. The internal scheduler cardinality test pins reuse of the
/// policy's cached fair-share table after construction.
pub(crate) enum GroupAdmission {
    Admitted,
    Denied { limit: u64, after: Option<u64> },
}

impl GroupAdmission {
    pub(crate) fn admitted(&self) -> bool {
        matches!(self, Self::Admitted)
    }

    /// Materialize the structured breach, formatting the exact attempt-path
    /// message. This is the ONLY place the message is built, so boolean and
    /// error callers cannot drift. Unrepresentable postimages also report the
    /// denial message: on the attempt path `admits` returns false for them
    /// before any overflow guard runs, so parity demands the same text here.
    pub(crate) fn into_breach(self, group: &str, owner: &str, bytes: u64) -> Option<LimitBreach> {
        match self {
            Self::Admitted => None,
            Self::Denied { limit, after } => Some(LimitBreach::new(
                LimitAxis::Group { group: group.to_string() },
                limit,
                after,
                MemoryError::capacity(format!(
                    "resource group {group} admission denied for owner {owner} \
                     reserve of {bytes} bytes"
                )),
            )),
        }
    }
}

/// Scheduler-side admission for one prospective growth: the exact rule
/// sequence of `admits` plus the `usage_after_growth` overflow guard, in
/// attempt order. Returns a lazy decision — no message is formatted here.
/// The scheduler delegates its boolean probe to this; the preflight
/// materializes the structured breach only when it must report one.
pub(crate) fn check_group_admission(
    groups: &BTreeMap<String, ResourceGroup>,
    shared_burst_pool_bytes: u64,
    fair_shares: &BTreeMap<String, u64>,
    current_usage: u64,
    group: &str,
    bytes: u64,
) -> GroupAdmission {
    let Some(budget) = groups.get(group) else {
        return GroupAdmission::Denied { limit: 0, after: Some(bytes) };
    };
    let after = current_usage.checked_add(bytes);
    let admitted = after.is_some_and(|after| {
        if after <= budget.hard_cap_bytes {
            return true;
        }
        let borrow = after - budget.hard_cap_bytes;
        borrow
            <= burst_allowance(
                shared_burst_pool_bytes,
                fair_shares.get(group).copied().unwrap_or(0),
                budget,
            )
    });
    if !admitted {
        // Covers both the ceiling denial and the unrepresentable postimage
        // (`after == None`): `admitted` is only true for `Some`, so no third
        // arm is needed — the overflow case falls out of the same check.
        let ceiling = group_admission_ceiling(
            budget,
            shared_burst_pool_bytes,
            fair_shares.get(group).copied().unwrap_or(0),
        );
        return GroupAdmission::Denied { limit: ceiling, after };
    }
    GroupAdmission::Admitted
}

/// Scheduler-side protected-floor check for one prospective growth: the exact
/// rule sequence of `protected_floor_fits_after_growth`, with all state
/// passed explicitly so both live and zero evaluations share it.
#[allow(
    clippy::too_many_arguments,
    reason = "admission request axes are explicit at this owner boundary"
)]
pub(crate) fn check_group_floor(
    groups: &BTreeMap<String, ResourceGroup>,
    unused_floors: u128,
    group: Option<&str>,
    group_current_non_headroom_usage: u64,
    delta: u64,
    non_headroom_after: u64,
    non_headroom_band: u64,
) -> Result<(), AdmissionFailure> {
    let prospective_unused = if let Some(group_name) = group {
        let Some(budget) = groups.get(group_name) else {
            return Err(AdmissionFailure::Block(AdmissionBlock::Internal(
                MemoryError::InvalidState(format!(
                    "cannot record usage for unknown resource group {group_name}"
                )),
            )));
        };
        let after = group_current_non_headroom_usage.checked_add(delta).ok_or_else(|| {
            AdmissionFailure::Breach(LimitBreach::new(
                LimitAxis::Group { group: group_name.to_string() },
                u64::MAX,
                None,
                MemoryError::capacity("resource-group usage exceeds u64"),
            ))
        })?;
        let unused_before = budget.reserved_bytes.saturating_sub(group_current_non_headroom_usage);
        let unused_after = budget.reserved_bytes.saturating_sub(after);
        unused_floors
            .checked_sub(u128::from(unused_before))
            .and_then(|total| total.checked_add(u128::from(unused_after)))
            .ok_or_else(|| {
                AdmissionFailure::Block(AdmissionBlock::Internal(MemoryError::InvalidState(
                    "resource-group floor aggregate is inconsistent".into(),
                )))
            })?
    } else {
        unused_floors
    };
    let protected_postimage =
        u128::from(non_headroom_after).checked_add(prospective_unused).ok_or_else(|| {
            AdmissionFailure::Breach(LimitBreach::new(
                LimitAxis::GroupFloor {
                    group: group.map(str::to_string),
                    usage: non_headroom_after,
                    floors: prospective_unused,
                },
                non_headroom_band,
                None,
                MemoryError::capacity("resource-group protected postimage exceeds u128"),
            ))
        })?;
    if protected_postimage > u128::from(non_headroom_band) {
        return Err(AdmissionFailure::Breach(LimitBreach::new(
            LimitAxis::GroupFloor {
                group: group.map(str::to_string),
                usage: non_headroom_after,
                floors: prospective_unused,
            },
            non_headroom_band,
            // A wider protected postimage must not become a fake exact MAX deficit.
            u64::try_from(protected_postimage).ok(),
            MemoryError::capacity(format!(
                "resource-group usage {non_headroom_after} plus unused floors {prospective_unused} \
                 exceeds non-headroom band {non_headroom_band}"
            )),
        )));
    }
    Ok(())
}

/// Execution scratch budget for live-scratch charges: the exact rule sequence
/// of the `reserve_on` scratch gate.
pub(crate) fn check_execution_scratch(
    max: Option<usize>,
    live_scratch_bytes: u64,
    kind: ChargeKind,
    bytes: u64,
) -> Result<(), AdmissionFailure> {
    if kind != ChargeKind::LiveScratch {
        return Ok(());
    }
    let Some(max) = max else {
        return Ok(());
    };
    let max = u64::try_from(max).unwrap_or(u64::MAX);
    let live_after = live_scratch_bytes.checked_add(bytes).ok_or_else(|| {
        AdmissionFailure::Breach(LimitBreach::new(
            LimitAxis::ExecutionScratch { max },
            max,
            None,
            MemoryError::capacity("execution scratch reserve total exceeds u64"),
        ))
    })?;
    if live_after > max {
        return Err(AdmissionFailure::Breach(LimitBreach::new(
            LimitAxis::ExecutionScratch { max },
            max,
            Some(live_after),
            MemoryError::capacity(format!(
                "execution scratch budget {max} exceeded by reserve to {live_after}"
            )),
        )));
    }
    Ok(())
}

/// Evaluate the full admission chain once, in exact attempt order: rebuild
/// fence, ticket ID space, group admission, non-headroom floor, execution
/// scratch, live-ticket count, ledger postimage.
///
/// The fence is checked first because it is a non-capacity refusal no reclaim
/// callback can cure: letting a capacity breach win would run the host
/// callback mid-fence and misreport the fence denial as a terminal Capacity
/// denial.
fn evaluate_chain(inputs: &PreflightInputs) -> Result<(), AdmissionFailure> {
    let request = &inputs.request;
    if inputs.rebuild_in_progress {
        return Err(AdmissionFailure::Block(AdmissionBlock::RebuildInProgress));
    }
    // No amount of reclaim can create a new monotonic ticket identifier.
    if inputs.ticket_id_space_exhausted {
        return Err(AdmissionFailure::Block(AdmissionBlock::TicketIdSpaceExhausted));
    }
    if let Some(group) = inputs.owner_group {
        // The chain needs the structured breach on denial, so it materializes
        // here; pure boolean probes (`admits`) skip `into_breach` instead.
        if let Some(breach) = check_group_admission(
            &inputs.policy.resource_groups,
            inputs.policy.shared_burst_pool_bytes,
            inputs.fair_burst_shares,
            inputs.group_current_usage,
            group,
            request.bytes,
        )
        .into_breach(group, request.owner, request.bytes)
        {
            return Err(AdmissionFailure::Breach(breach));
        }
    }
    if *request.class != MemoryClass::Headroom || inputs.usage.credit.is_some() {
        let delta = if *request.class == MemoryClass::Headroom { 0 } else { request.bytes };
        let non_headroom_after = inputs.non_headroom_bytes.checked_add(delta).ok_or_else(|| {
            AdmissionFailure::Breach(LimitBreach::new(
                LimitAxis::NonHeadroomTotal,
                u64::MAX,
                None,
                MemoryError::capacity("non-headroom reserve total exceeds u64"),
            ))
        })?;
        let band = inputs
            .policy
            .global_hard_limit_bytes
            .checked_sub(inputs.policy.headroom_bytes)
            .ok_or_else(|| {
                AdmissionFailure::Block(AdmissionBlock::Internal(MemoryError::capacity(
                    "headroom exceeds global hard limit",
                )))
            })?;
        check_group_floor(
            &inputs.policy.resource_groups,
            inputs.unused_group_floors,
            inputs.owner_group,
            inputs.group_current_non_headroom_usage,
            delta,
            non_headroom_after,
            band,
        )?;
    }
    check_execution_scratch(
        inputs.execution_scratch_max,
        inputs.live_scratch_bytes,
        request.kind,
        request.bytes,
    )?;
    if inputs.ticket_count_delta > 1 {
        return Err(AdmissionFailure::Block(AdmissionBlock::Internal(MemoryError::InvalidState(
            "admission ticket delta must be zero or one".into(),
        ))));
    }
    if inputs.live_ticket_limit_reached && inputs.ticket_count_delta != 0 {
        let limit = crate::domain::management_limits::MAX_LIVE_TICKETS_PER_BROKER as u64;
        return Err(AdmissionFailure::Breach(LimitBreach::new(
            LimitAxis::LiveTicketCount,
            limit,
            Some(limit + 1),
            MemoryError::capacity(format!("live reservation ticket limit {limit} reached")),
        )));
    }
    check_ledger_postimage(inputs.policy, &inputs.usage, request)?;
    Ok(())
}

/// Pure admission verdict: evaluate the chain against live state; on a live
/// capacity breach, re-evaluate the same chain against zero usage to decide
/// whether any callback could help.
pub(crate) fn preflight(inputs: &PreflightInputs) -> AdmissionVerdict {
    let live = evaluate_chain(inputs);
    match live {
        Ok(()) => AdmissionVerdict::Admit,
        Err(AdmissionFailure::Block(reason)) => AdmissionVerdict::Blocked { reason },
        Err(AdmissionFailure::Breach(live_breach)) => {
            let empty_classes: BTreeMap<MemoryClass, UsageBreakdown> = BTreeMap::new();
            let empty_owners: BTreeMap<OwnerKey, UsageBreakdown> = BTreeMap::new();
            let empty_contexts: BTreeMap<MemoryContextKind, UsageBreakdown> = BTreeMap::new();
            let empty_usage = UsageBreakdown::default();
            let zero_floors = inputs
                .policy
                .resource_groups
                .values()
                .fold(0u128, |sum, group| sum + u128::from(group.reserved_bytes));
            let zero = PreflightInputs {
                usage: UsageView {
                    class_usage: &empty_classes,
                    owner_usage: (&empty_owners).into(),
                    context_usage: &empty_contexts,
                    total_usage: &empty_usage,
                    credit: None,
                },
                non_headroom_bytes: 0,
                live_scratch_bytes: 0,
                group_current_usage: 0,
                group_current_non_headroom_usage: 0,
                unused_group_floors: zero_floors,
                rebuild_in_progress: false,
                ticket_id_space_exhausted: false,
                live_ticket_limit_reached: false,
                ..*inputs
            };
            match evaluate_chain(&zero) {
                Ok(()) => AdmissionVerdict::ReclaimCandidate { breach: live_breach },
                // A zero-state block is unreachable (empty maps cannot
                // overflow, unknown classes short-circuit live first), but
                // fail closed without a callback if it ever happens.
                Err(AdmissionFailure::Block(reason)) => AdmissionVerdict::Blocked { reason },
                Err(AdmissionFailure::Breach(_)) => {
                    // The zero-state probe decides terminality, not which live
                    // admission check failed first. Preserve the attempt's
                    // exact diagnostic even when the zero-state limiting axis
                    // differs from the live one.
                    AdmissionVerdict::TerminalDenial { breach: live_breach }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deficit_saturates_for_unrepresentable_postimages() {
        let breach = LimitBreach::new(
            LimitAxis::Global,
            100,
            None,
            MemoryError::capacity("global postimage exceeds u64"),
        );
        assert_eq!(breach.deficit, u64::MAX);
        assert_eq!(breach.into_error(), MemoryError::capacity("global postimage exceeds u64"));
    }

    // Self-audit: the pure engine must fail closed on ANY input, including
    // a usage view no live ledger can hold (kind slots summing past u64::MAX
    // is impossible via reserve, which checks total+bytes first). Drive the
    // chain directly with such a view: typed block, never a panic.
    #[test]
    fn unrepresentable_global_aggregate_blocks_without_panic() {
        use crate::domain::policy::{MemoryClassBudget, ReclaimPriority};
        let policy = ResidencyPolicy::new(u64::MAX).with_class_budget(
            MemoryClass::ArtifactCache,
            MemoryClassBudget {
                reserved_bytes: 0,
                soft_cap_bytes: u64::MAX,
                hard_cap_bytes: u64::MAX,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        );
        let total = UsageBreakdown {
            retained_payload_bytes: u64::MAX,
            retained_metadata_bytes: 1,
            live_scratch_bytes: 0,
        };
        let empty_classes = BTreeMap::new();
        let empty_owners = BTreeMap::new();
        let empty_contexts = BTreeMap::new();
        let usage = UsageView {
            class_usage: &empty_classes,
            owner_usage: (&empty_owners).into(),
            context_usage: &empty_contexts,
            total_usage: &total,
            credit: None,
        };
        let class = MemoryClass::ArtifactCache;
        let owner = "owner:x".to_string();
        let request = AdmissionRequest {
            class: &class,
            owner: &owner,
            context: MemoryContextKind::Request,
            kind: ChargeKind::RetainedPayload,
            bytes: 1,
        };
        assert_eq!(
            check_ledger_postimage(&policy, &usage, &request).map_err(|f| f.into_error()),
            Err(MemoryError::InvalidState("ledger usage breakdown exceeds u64".into()))
        );
    }

    // Self-audit P2-4: an owner mapped to an undeclared group can never be
    // fixed by reclaim, so the group check denies terminally with the exact
    // legacy scheduler message. Unreachable via public API (policy validation
    // rejects dangling owner->group mappings); this pins the defensive arm.
    #[test]
    fn unknown_group_denies_terminally_with_legacy_message() {
        use crate::domain::policy::ResourceGroup;
        let groups = BTreeMap::from([(
            "g".to_string(),
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 800,
                hard_cap_bytes: 900,
                burst_percent: 0,
                reclaim_priority: crate::domain::policy::ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )]);
        let shares = BTreeMap::from([("g".to_string(), 0u64)]);
        let decision = check_group_admission(&groups, 0, &shares, 0, "missing", 10);
        assert!(!decision.admitted(), "unknown group denies");
        let breach = decision.into_breach("missing", "owner:x", 10).expect("breach");
        assert_eq!(
            breach.into_error(),
            MemoryError::capacity(
                "resource group missing admission denied for owner owner:x reserve of 10 bytes"
            )
        );
    }
}
