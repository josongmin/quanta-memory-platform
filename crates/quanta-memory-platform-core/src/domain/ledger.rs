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

/// Why a charge is held: one semantic charge bucket per owned lifetime
/// component. Invariant 2 partitions all owner usage across these kinds.
///
/// A bucket classifies the *whole* semantic cost of one owned lifetime
/// component — the retained object together with the key/index/entry
/// overhead held under the same lease — not a literal per-field byte
/// measurement. Each store lifetime component carries exactly one kind, so
/// a facade's `RetainedCostV1`-style component formulas calculate one
/// component's total while this enum classifies which lifetime owns it.
/// Kind totals are semantic budgeting buckets, not allocator-accurate
/// field-byte telemetry or an RSS bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChargeKind {
    /// Payload-lifetime component: the retained payload object plus the
    /// key/index/entry overhead held with it until delete/close/Drop.
    RetainedPayload,
    /// Metadata-lifetime component: independently retained identity, index,
    /// tombstone, or bookkeeping state, which may outlive the payload
    /// component of the same entry.
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

    /// Checked per-kind totals of one accounting axis:
    /// `[payload, metadata, scratch]`.
    #[cfg(feature = "live_control")]
    fn checked_kind_totals<K: Ord>(axis: &BTreeMap<K, UsageBreakdown>) -> MemoryResult<[u64; 3]> {
        axis.values().try_fold([0u64; 3], |sums, usage| {
            Ok([
                sums[0].checked_add(usage.retained_payload_bytes).ok_or_else(|| {
                    MemoryError::InvalidState("ledger payload kind total exceeds u64".into())
                })?,
                sums[1].checked_add(usage.retained_metadata_bytes).ok_or_else(|| {
                    MemoryError::InvalidState("ledger metadata kind total exceeds u64".into())
                })?,
                sums[2].checked_add(usage.live_scratch_bytes).ok_or_else(|| {
                    MemoryError::InvalidState("ledger scratch kind total exceeds u64".into())
                })?,
            ])
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
        // FUP-004: scalar totals are not enough — the per-kind split must
        // agree on EVERY axis. A payload↔metadata swap confined to one axis
        // preserves all scalar totals but corrupts the kind accounting.
        let class_kinds = Self::checked_kind_totals(&self.class_usage)?;
        let owner_kinds = Self::checked_kind_totals(&self.owner_usage)?;
        let context_kinds = Self::checked_kind_totals(&self.context_usage)?;
        if class_kinds != owner_kinds || owner_kinds != context_kinds {
            return Err(MemoryError::InvalidState(format!(
                "ledger kind split diverges across axes: class={class_kinds:?} \
                 owner={owner_kinds:?} context={context_kinds:?}"
            )));
        }
        Ok(())
    }

    /// Total bytes on the class axis.
    ///
    /// # Panics
    /// Panics when the axis total exceeds `u64`. Broker-issued snapshots can
    /// never overflow (every mutation preflights with checked arithmetic),
    /// but the all-public fields make an externally assembled snapshot
    /// forgeable; use [`Self::checked_total_class_bytes`] on untrusted
    /// snapshots.
    #[must_use]
    pub fn total_class_bytes(&self) -> u64 {
        self.checked_total_class_bytes().expect("ledger class total exceeds u64")
    }

    /// Total bytes on the owner axis.
    ///
    /// # Panics
    /// Panics when the axis total exceeds `u64`; see [`Self::total_class_bytes`]
    /// and use [`Self::checked_total_owner_bytes`] on untrusted snapshots.
    #[must_use]
    pub fn total_owner_bytes(&self) -> u64 {
        self.checked_total_owner_bytes().expect("ledger owner total exceeds u64")
    }

    /// Total bytes on the context axis. The context axis is the third accounting
    /// dimension (alongside class and owner); every reserve/release touches all
    /// three, so this total must equal the owner total at every point.
    ///
    /// # Panics
    /// Panics when the axis total exceeds `u64`; see [`Self::total_class_bytes`]
    /// and use [`Self::checked_total_context_bytes`] on untrusted snapshots.
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

    /// Total retained payload bytes across owners.
    ///
    /// # Panics
    /// Panics when the kind total exceeds `u64`; see
    /// [`Self::total_class_bytes`] and use
    /// [`Self::checked_total_retained_payload_bytes`] on untrusted snapshots.
    #[must_use]
    pub fn total_retained_payload_bytes(&self) -> u64 {
        self.checked_total_retained_payload_bytes().expect("ledger payload total exceeds u64")
    }

    /// Total retained metadata bytes across owners.
    ///
    /// # Panics
    /// Panics when the kind total exceeds `u64`; see
    /// [`Self::total_class_bytes`] and use
    /// [`Self::checked_total_retained_metadata_bytes`] on untrusted snapshots.
    #[must_use]
    pub fn total_retained_metadata_bytes(&self) -> u64 {
        self.checked_total_retained_metadata_bytes().expect("ledger metadata total exceeds u64")
    }

    /// Total live scratch bytes across owners.
    ///
    /// # Panics
    /// Panics when the kind total exceeds `u64`; see
    /// [`Self::total_class_bytes`] and use
    /// [`Self::checked_total_live_scratch_bytes`] on untrusted snapshots.
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

/// Test-only interception of owner-key reads. Production keeps a plain map.
#[cfg(all(test, feature = "live_control"))]
#[derive(Debug, Default)]
struct MeteredOwnerUsage {
    entries: BTreeMap<OwnerKey, UsageBreakdown>,
    probes_active: std::sync::atomic::AtomicBool,
    key_visits: std::sync::atomic::AtomicUsize,
}

#[cfg(all(test, feature = "live_control"))]
impl MeteredOwnerUsage {
    fn visit_key(&self) {
        if self.probes_active.load(Ordering::Relaxed) {
            self.key_visits.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn start_probes(&self) -> OwnerUsageProbeGuard<'_> {
        self.key_visits.store(0, Ordering::Relaxed);
        self.probes_active.store(true, Ordering::Relaxed);
        OwnerUsageProbeGuard(self)
    }

    fn get(&self, owner: &OwnerKey) -> Option<&UsageBreakdown> {
        self.visit_key();
        self.entries.get(owner)
    }

    fn get_mut(&mut self, owner: &OwnerKey) -> Option<&mut UsageBreakdown> {
        self.entries.get_mut(owner)
    }

    fn entry(
        &mut self,
        owner: OwnerKey,
    ) -> std::collections::btree_map::Entry<'_, OwnerKey, UsageBreakdown> {
        self.entries.entry(owner)
    }

    fn remove(&mut self, owner: &OwnerKey) -> Option<UsageBreakdown> {
        self.entries.remove(owner)
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn iter(&self) -> MeteredOwnerIter<'_> {
        MeteredOwnerIter { entries: self.entries.iter(), meter: self }
    }

    fn values(&self) -> impl Iterator<Item = &UsageBreakdown> {
        self.iter().map(|(_, usage)| usage)
    }
}

#[cfg(all(test, feature = "live_control"))]
struct MeteredOwnerIter<'a> {
    entries: std::collections::btree_map::Iter<'a, OwnerKey, UsageBreakdown>,
    meter: &'a MeteredOwnerUsage,
}

#[cfg(all(test, feature = "live_control"))]
impl<'a> Iterator for MeteredOwnerIter<'a> {
    type Item = (&'a OwnerKey, &'a UsageBreakdown);

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.next()?;
        self.meter.visit_key();
        Some(entry)
    }
}

#[cfg(all(test, feature = "live_control"))]
impl<'a> IntoIterator for &'a MeteredOwnerUsage {
    type Item = (&'a OwnerKey, &'a UsageBreakdown);
    type IntoIter = MeteredOwnerIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(all(test, feature = "live_control"))]
struct OwnerUsageProbeGuard<'a>(&'a MeteredOwnerUsage);

#[cfg(all(test, feature = "live_control"))]
impl OwnerUsageProbeGuard<'_> {
    fn key_visits(&self) -> usize {
        self.0.key_visits.load(Ordering::Relaxed)
    }
}

#[cfg(all(test, feature = "live_control"))]
impl Drop for OwnerUsageProbeGuard<'_> {
    fn drop(&mut self) {
        self.0.probes_active.store(false, Ordering::Relaxed);
    }
}

/// Keyed owner accounting view; consumers cannot scan or clone the backing map.
#[cfg(feature = "live_control")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct OwnerUsageRead<'a>(OwnerUsageReadData<'a>);

#[cfg(feature = "live_control")]
#[derive(Clone, Copy, Debug)]
enum OwnerUsageReadData<'a> {
    Plain(&'a BTreeMap<OwnerKey, UsageBreakdown>),
    #[cfg(all(test, feature = "live_control"))]
    Metered(&'a MeteredOwnerUsage),
}

#[cfg(feature = "live_control")]
impl<'a> From<&'a BTreeMap<OwnerKey, UsageBreakdown>> for OwnerUsageRead<'a> {
    fn from(usage: &'a BTreeMap<OwnerKey, UsageBreakdown>) -> Self {
        Self(OwnerUsageReadData::Plain(usage))
    }
}

#[cfg(feature = "live_control")]
impl<'a> OwnerUsageRead<'a> {
    pub(crate) fn get(&self, owner: &OwnerKey) -> Option<&'a UsageBreakdown> {
        match self.0 {
            OwnerUsageReadData::Plain(usage) => usage.get(owner),
            #[cfg(all(test, feature = "live_control"))]
            OwnerUsageReadData::Metered(usage) => usage.get(owner),
        }
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
    #[cfg(not(test))]
    owner_usage: BTreeMap<OwnerKey, UsageBreakdown>,
    #[cfg(test)]
    owner_usage: MeteredOwnerUsage,
    context_usage: BTreeMap<MemoryContextKind, UsageBreakdown>,
    /// Exact O(1) aggregate used by admission. The three public accounting
    /// axes remain the reporting authority; every mutation updates all four
    /// representations in the same fail-closed transaction.
    total_usage: UsageBreakdown,
    /// Exact subset of `total_usage` excluding the reserved Headroom class.
    non_headroom_bytes: u64,
    /// This ledger's process-unique instance tag, stamped on every issued ticket.
    instance_id: u64,
    /// Canonical reservation state machine. An id exists exactly while its
    /// charge is live, and can transition only Pending -> Committed -> absent.
    ticket_states: BTreeMap<u64, TicketState>,
    /// `None` means the u64 id space was exhausted. IDs never wrap or reuse.
    next_ticket_id: Option<u64>,
    rebuild_in_progress: bool,
    #[cfg(test)]
    last_release_preflight_axis_probes: usize,
}

#[cfg(feature = "live_control")]
mod operations;

#[cfg(all(test, feature = "live_control"))]
mod tests;
