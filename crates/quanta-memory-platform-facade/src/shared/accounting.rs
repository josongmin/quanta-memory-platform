//! The facade-level accounting seam.
//!
//! Portable stores must charge their bytes against the broker's ledger (the
//! accounting SSOT) rather than tracking a private counter that no cap can see —
//! the latter is two parallel accounting universes where global/owner/class caps
//! never gate real store bytes. [`AccountingSink`] is the seam that closes that
//! gap: a store holds an optional sink and, when present, every retained byte is
//! reserved+committed through it on write and released on delete, so the broker's
//! caps actually control store residency.
//!
//! When `core-live-accounting` is enabled, the blanket impl wires the live
//! broker in as a sink. A store with no sink stays a self-accounting in-memory
//! structure (explicit non-budgeted mode), which keeps the builder-first happy
//! path and unit fixtures dependency-free.

use std::sync::Arc;

#[cfg(feature = "core-live-accounting")]
use quanta_memory_platform_core::LiveProcessMemoryBroker;
use quanta_memory_platform_core::{
    ChargeKind, MemoryClass, MemoryContextKind, MemoryError, MemoryResult, OwnerKey,
    ReservationTicket,
};

/// Outcome of a denied [`AccountingSink::recharge`]: the new charge was rejected.
/// `restored` carries the old charge under a fresh ticket the caller must adopt,
/// or `None` when the old charge was never released (the original ticket stays
/// valid).
pub type RechargeDenied = (MemoryError, Option<ReservationTicket>);

/// A charge/refund seam against the broker's accounting ledger.
///
/// Implementors route store bytes through the real ledger. The receipt returned
/// by [`Self::charge`] is the ledger's own [`ReservationTicket`], so refunds are
/// idempotent and double-refund is rejected by the ledger (invariant 7).
///
/// `recharge` is a REQUIRED atomic contract: the facade no longer provides a
/// refund-old-then-charge-new fallback because that shape cannot fail closed for
/// a generic sink. Implementors must either preserve the old charge or restore
/// it under a fresh ticket when the new charge is denied.
pub trait AccountingSink: std::fmt::Debug + Send + Sync {
    /// Reserve and commit `bytes` against (class, owner, context, kind). Fails
    /// closed: a cap denial returns `Err` and charges nothing.
    fn charge(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket>;

    /// Return previously charged bytes to the ledger.
    fn refund(&self, ticket: &ReservationTicket) -> MemoryResult<()>;

    /// Return a batch of previously charged tickets to the ledger atomically.
    ///
    /// A charged store uses this for multi-item mutations (truncate / sweep) so a
    /// rebuild cannot partially refund the ledger before the store mutation is
    /// known to succeed. On error, no ticket in the batch is refunded.
    fn refund_many(&self, tickets: &[ReservationTicket]) -> MemoryResult<()>;

    /// Atomically swap `old`'s charge for a new charge of `bytes` (an in-place
    /// overwrite). On success the old receipt is consumed and the new receipt is
    /// returned. On denial the old charge is preserved or restored under a fresh
    /// receipt — see [`RechargeDenied`].
    ///
    fn recharge(
        &self,
        old: &ReservationTicket,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> Result<ReservationTicket, RechargeDenied>;
}

/// The shared, cloneable handle a store holds to charge against the ledger.
pub type SharedAccountingSink = Arc<dyn AccountingSink>;

#[cfg(feature = "core-live-accounting")]
impl AccountingSink for LiveProcessMemoryBroker {
    fn charge(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        // A retained store byte is a committed charge. Use the broker's atomic
        // reserve+commit primitive so the reservation and its commit happen under
        // ONE lock: a rebuild can never interleave between them and strand the
        // reserved bytes behind a discarded ticket (fail-closed on a cap breach).
        self.reserve_committed(class, owner, context, kind, bytes)
    }

    fn refund(&self, ticket: &ReservationTicket) -> MemoryResult<()> {
        self.release(ticket)
    }

    fn refund_many(&self, tickets: &[ReservationTicket]) -> MemoryResult<()> {
        self.release_many(tickets)
    }

    fn recharge(
        &self,
        old: &ReservationTicket,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> Result<ReservationTicket, RechargeDenied> {
        // Single-lock atomic swap: no transient double-count, no mid-rebuild strand.
        LiveProcessMemoryBroker::recharge(self, old, class, owner, context, kind, bytes)
            .map_err(|e| (e.error, e.restored))
    }
}
