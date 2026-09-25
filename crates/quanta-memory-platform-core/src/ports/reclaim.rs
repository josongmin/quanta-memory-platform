//! Host-independent reclaim callback boundary.
//!
//! The broker prepares an immutable request while holding its internal lock,
//! drops that lock, and only then calls this port. Implementations may evict or
//! close stores that release broker-bound charges. The broker always remeasures
//! semantic usage before the final admission decision. A synchronous reserve
//! reentered on the same broker from this callback fails with
//! `OperationInProgress`; release and legal reconfiguration remain available.
//! The broker executes at most one reclaim callback concurrently per broker;
//! a competing reclaim plan skips the callback and re-evaluates admission.

use crate::{MemoryClass, MemoryResult, PressureState, ReclaimPriority, ReclaimRung};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReclaimTrigger {
    SoftThreshold,
    CapacityDenial,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReclaimCandidate {
    pub class: MemoryClass,
    pub priority: ReclaimPriority,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReclaimGroupCandidate {
    pub group: String,
    pub priority: ReclaimPriority,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReclaimRequest {
    pub trigger: ReclaimTrigger,
    pub requested_class: MemoryClass,
    pub requested_owner: String,
    pub resource_group: Option<String>,
    pub requested_bytes: u64,
    pub pressure: PressureState,
    pub rungs: Vec<ReclaimRung>,
    /// Deterministic victim order: `Last`, then `Normal`, then protected `First`.
    pub candidates: Vec<ReclaimCandidate>,
    /// Resource groups in the same deterministic victim order.
    pub group_candidates: Vec<ReclaimGroupCandidate>,
    /// One broker admission performs at most this many external attempts.
    pub max_attempts: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReclaimOutcome {
    pub reclaimed_bytes: u64,
}

/// Host-owned reclaim callback invoked by the broker's admission slow path.
///
/// Callback contract:
///
/// - **Synchronous.** The callback runs inline on the caller's admission
///   path. The broker executes at most one reclaim callback concurrently per
///   broker, but imposes no deadline: wall-time bounds are a host
///   responsibility, not a QMP guarantee.
/// - **Same-store reentry is bounded denial.** A callback that mutates the
///   same store whose admission triggered the reclaim may receive
///   `OperationInProgress` while that store's mutation lane is held; release
///   and legal reconfiguration remain available. Evicting from a *different*
///   store is the supported cross-store path.
/// - **Claimed bytes are advisory.** The broker remeasures actual ledger
///   usage after the callback returns and decides admission from the
///   remeasured number, never from [`ReclaimOutcome::reclaimed_bytes`].
/// - **No RSS guarantee.** Reclaim releases semantic charges; it does not
///   bound allocator growth, transient caller allocations, or process RSS.
pub trait MemoryReclaimer: Send + Sync {
    fn reclaim(&self, request: &ReclaimRequest) -> MemoryResult<ReclaimOutcome>;
}
