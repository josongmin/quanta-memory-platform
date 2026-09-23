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

pub trait MemoryReclaimer: Send + Sync {
    fn reclaim(&self, request: &ReclaimRequest) -> MemoryResult<ReclaimOutcome>;
}
