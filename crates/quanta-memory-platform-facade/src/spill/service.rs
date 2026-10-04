use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use quanta_memory_platform_core::{MemoryError, MemoryResult};

use crate::spill::{
    DurableCommitFailure, DurableRecordId, DurableRecordResolution, DurableRestoreCandidate,
    DurableRestoreClaim, DurableRestoreController, DurableRestoreRecovery, DurableSpillController,
    DurableWriteFailure, InProcessSpillController, ReconciledDurableSpillController,
    SpillController,
};

/// Capability seals for accounting-bearing transfer paths. Generic
/// `SpillTarget` remains externally implementable for non-accounting use, but a
/// foreign implementation cannot claim that consuming a charge-carrying handle
/// is panic-safe or durably committed.
pub(crate) mod sealed {
    pub trait InProcessSpillTarget<Handle> {}
    pub trait DurableSpillTarget<Handle> {}
    pub trait InProcessSpillController<Handle> {}
    pub trait DurableSpillController<Handle> {}
}

/// Destination authority established by a successful spill commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use = "a successful spill receipt identifies the authoritative tier"]
pub enum SpillReceipt {
    /// The handle remains process-resident and must retain its memory charge.
    InProcess { inventory_after: u64 },
    /// The opaque bytes committed to durable storage under a stable sequence.
    Durable { sequence: u64, inventory_after: u64 },
}

impl SpillReceipt {
    #[must_use]
    pub fn inventory_after(self) -> u64 {
        match self {
            Self::InProcess { inventory_after } | Self::Durable { inventory_after, .. } => {
                inventory_after
            }
        }
    }
}

/// A failed spill returns the exact handle supplied by the caller. Losing the
/// handle on a pre-commit error would make source rollback impossible.
#[derive(Debug)]
#[must_use = "recover the handle or explicitly retire it"]
pub struct SpillFailure<Handle> {
    error: MemoryError,
    handle: Box<Handle>,
}

impl<Handle> SpillFailure<Handle> {
    pub fn new(error: MemoryError, handle: Handle) -> Self {
        Self { error, handle: Box::new(handle) }
    }

    pub fn error(&self) -> &MemoryError {
        &self.error
    }

    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    pub fn into_parts(self) -> (MemoryError, Handle) {
        (self.error, *self.handle)
    }
}

impl<Handle> std::fmt::Display for SpillFailure<Handle> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

/// A destination a spillable handle can be flushed to — and restored from.
///
/// Domain-neutral: the target only knows whether it can currently accept a spill,
/// how to accept it, and how to restore a previously spilled handle. Backing
/// integrations (`redb`) implement this inside the spill slice; they never become
/// cross-slice authority helpers.
///
/// `restore` makes spill a two-way tier instead of write-only: a target that
/// merely offloads-and-forgets (eviction) keeps the default `Ok(None)`, while a
/// retaining target ([`RetainingSpillTarget`]) returns spilled handles in order.
pub trait SpillInventory {
    /// Canonical current inventory, including persisted records from earlier
    /// process lifetimes.
    fn retained_count(&self) -> MemoryResult<u64>;
}

pub trait SpillTarget<Handle>: SpillInventory {
    /// Whether the target can currently accept a spill.
    fn is_available(&self) -> bool;

    /// Accept the spilled handle. Only called when [`Self::is_available`] is true.
    fn accept(&self, handle: Handle) -> Result<SpillReceipt, SpillFailure<Handle>>;

    /// Restore the next previously spilled handle (oldest first).
    ///
    /// Contract (uniform across all impls so the trait is Liskov-substitutable):
    /// - `Ok(Some(handle))` — a retained handle was brought back (FIFO).
    /// - `Ok(None)` — the target is readable but has nothing retained to restore.
    /// - `Err(MemoryError::Capacity(_))` — the target is OFFLINE: a vanished
    ///   backend cannot be read, which is distinct from "nothing to restore". An
    ///   retaining impl that can go offline MUST fail closed here rather than return `None`,
    ///   so a caller can tell "drained" from "unreachable".
    ///
    /// The default is a discard / eviction target: unavailable targets fail
    /// closed and available targets retain nothing. Retaining implementations
    /// must override this method and distinguish read availability themselves
    /// when it differs from write admission.
    fn restore(&self) -> MemoryResult<Option<Handle>> {
        if !self.is_available() {
            return Err(MemoryError::capacity("spill target unavailable during restore"));
        }
        Ok(None)
    }
}

/// Marker for targets that retain the exact handle in process and therefore
/// return [`SpillReceipt::InProcess`] on success.
pub trait InProcessSpillTarget<Handle>:
    SpillTarget<Handle> + sealed::InProcessSpillTarget<Handle>
{
}

/// Marker for targets that durably commit an encoded handle before returning
/// [`SpillReceipt::Durable`].
pub trait DurableSpillTarget<Handle>:
    SpillTarget<Handle> + sealed::DurableSpillTarget<Handle>
{
}

/// Durable byte targets expose a non-destructive prepare and exact commit so a
/// source can establish charge authority before deleting the durable record.
pub trait DurableRestoreTarget: DurableSpillTarget<Vec<u8>> {
    /// Claim and read the oldest record without removing it. Implementations
    /// must persist the claim before returning the candidate.
    fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>>;

    /// Remove exactly the claimed record and classify failure as known
    /// retained or commit-ambiguous. Only the active claimant may commit.
    fn commit_restore(&self, claim: &DurableRestoreClaim) -> Result<(), DurableCommitFailure>;
    fn finalize_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()>;
    fn abort_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()>;
    fn reconcile_restore(
        &self,
        claim: &DurableRestoreClaim,
    ) -> MemoryResult<DurableRecordResolution>;
    fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>>;
    fn recover_active_restore(&self) -> MemoryResult<Option<DurableRestoreRecovery>>;
}

/// A missing spill tier: fail closed and return the original handle.
#[derive(Clone, Debug, Default)]
pub struct NoopSpillController;

impl<Handle> SpillController<Handle> for NoopSpillController {
    fn spill(&self, handle: Handle) -> Result<SpillReceipt, SpillFailure<Handle>> {
        Err(SpillFailure::new(MemoryError::capacity("spill target not configured"), handle))
    }
}

impl<Handle> InProcessSpillController<Handle> for NoopSpillController {}
impl<Handle> sealed::InProcessSpillController<Handle> for NoopSpillController {}

/// A spill controller that routes handles to a [`SpillTarget`], failing closed when
/// the target is unavailable (negative path 4: spill target unavailable).
///
/// This helper never redefines quota semantics: it does not charge, reserve, or
/// release broker bytes (invariant 4). It only routes the handle and reports
/// success/failure.
#[derive(Clone, Debug, Default)]
pub struct RoutedSpillController<T> {
    target: T,
}

impl<T> RoutedSpillController<T> {
    pub fn new(target: T) -> Self {
        Self { target }
    }

    /// Current authoritative inventory, including records opened from a
    /// previous process lifetime.
    pub fn spilled_count(&self) -> MemoryResult<u64>
    where
        T: SpillInventory,
    {
        self.target.retained_count()
    }
}

impl<Handle, T> SpillController<Handle> for RoutedSpillController<T>
where
    T: SpillTarget<Handle>,
{
    fn spill(&self, handle: Handle) -> Result<SpillReceipt, SpillFailure<Handle>> {
        if !self.target.is_available() {
            return Err(SpillFailure::new(
                MemoryError::capacity("spill target unavailable"),
                handle,
            ));
        }
        self.target.accept(handle)
    }

    fn restore(&self) -> MemoryResult<Option<Handle>> {
        self.target.restore()
    }
}

impl<Handle, T> InProcessSpillController<Handle> for RoutedSpillController<T> where
    T: InProcessSpillTarget<Handle>
{
}
impl<Handle, T> sealed::InProcessSpillController<Handle> for RoutedSpillController<T> where
    T: InProcessSpillTarget<Handle>
{
}

impl<Handle, T> DurableSpillController<Handle> for RoutedSpillController<T> where
    T: DurableSpillTarget<Handle>
{
}
impl<Handle, T> sealed::DurableSpillController<Handle> for RoutedSpillController<T> where
    T: DurableSpillTarget<Handle>
{
}

impl<T> ReconciledDurableSpillController for RoutedSpillController<T>
where
    T: ReconciledDurableSpillTarget + DurableSpillTarget<Vec<u8>>,
{
    fn max_record_bytes(&self) -> Option<u64> {
        self.target.max_record_bytes()
    }

    fn reserve_record_id(&self) -> MemoryResult<DurableRecordId> {
        self.target.reserve_record_id()
    }

    fn commit_spill(
        &self,
        record_id: DurableRecordId,
        bytes: &[u8],
    ) -> Result<SpillReceipt, DurableWriteFailure> {
        self.target.commit_spill(record_id, bytes)
    }

    fn resolve_record(&self, record_id: DurableRecordId) -> MemoryResult<DurableRecordResolution> {
        self.target.resolve_record(record_id)
    }
}

/// Target half of [`ReconciledDurableSpillController`]. Implementations are
/// sealed because this trait carries accounting authority claims.
pub trait ReconciledDurableSpillTarget:
    SpillInventory + sealed::DurableSpillTarget<Vec<u8>>
{
    /// Return the cap actually enforced by `commit_spill`, if one exists.
    fn max_record_bytes(&self) -> Option<u64> {
        None
    }

    fn reserve_record_id(&self) -> MemoryResult<DurableRecordId>;
    fn commit_spill(
        &self,
        record_id: DurableRecordId,
        bytes: &[u8],
    ) -> Result<SpillReceipt, DurableWriteFailure>;
    fn resolve_record(&self, record_id: DurableRecordId) -> MemoryResult<DurableRecordResolution>;
}

impl<T> DurableRestoreController for RoutedSpillController<T>
where
    T: DurableRestoreTarget,
{
    fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>> {
        self.target.prepare_restore()
    }

    fn commit_restore(&self, claim: &DurableRestoreClaim) -> Result<(), DurableCommitFailure> {
        self.target.commit_restore(claim)
    }

    fn finalize_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
        self.target.finalize_restore(claim)
    }

    fn abort_restore(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
        self.target.abort_restore(claim)
    }

    fn reconcile_restore(
        &self,
        claim: &DurableRestoreClaim,
    ) -> MemoryResult<DurableRecordResolution> {
        self.target.reconcile_restore(claim)
    }

    fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>> {
        self.target.active_restore()
    }

    fn recover_active_restore(&self) -> MemoryResult<Option<DurableRestoreRecovery>> {
        self.target.recover_active_restore()
    }
}

/// An in-memory spill target whose availability can be toggled. This is the
/// portable, host-free target used for tests and for hosts that have no durable
/// spill backend configured.
///
/// It is a genuine two-way tier: `accept` RETAINS the spilled handle and
/// [`Self::restore`] brings it back (FIFO, oldest first). A restore on a drained
/// target yields `None`; a restore (or accept) while the target is offline fails
/// closed with a typed [`MemoryError::Capacity`]. `accepted_count` continues to
/// report the cumulative number of handles accepted so far.
#[derive(Debug, Default)]
pub struct InMemorySpillTarget<Handle> {
    available: AtomicBool,
    /// Cumulative count of handles accepted (never decremented on restore), kept
    /// for backward-compatible telemetry.
    accepted: AtomicU64,
    /// Retained spilled handles, restored oldest-first. The two-way (restorable)
    /// inverse of the write-only accept path.
    store: Mutex<VecDeque<Handle>>,
}

impl<Handle> InMemorySpillTarget<Handle> {
    /// Create an available in-memory spill target.
    pub fn available() -> Self {
        Self {
            available: AtomicBool::new(true),
            accepted: AtomicU64::new(0),
            store: Mutex::new(VecDeque::new()),
        }
    }

    /// Create an unavailable in-memory spill target.
    pub fn unavailable() -> Self {
        Self {
            available: AtomicBool::new(false),
            accepted: AtomicU64::new(0),
            store: Mutex::new(VecDeque::new()),
        }
    }

    /// Toggle availability (e.g. to simulate the backend going offline).
    pub fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::SeqCst);
    }

    /// Number of handles accepted so far (cumulative; not decremented on restore).
    pub fn accepted_count(&self) -> u64 {
        self.accepted.load(Ordering::SeqCst)
    }

    /// Number of spilled handles currently retained (restorable).
    pub fn retained_count(&self) -> MemoryResult<u64> {
        let len = self.store.lock().map_err(|_| MemoryError::poisoned("spill target store"))?.len();
        u64::try_from(len).map_err(|_| MemoryError::capacity("spill inventory exceeds u64"))
    }
}

impl<Handle> SpillTarget<Handle> for InMemorySpillTarget<Handle> {
    fn is_available(&self) -> bool {
        self.available.load(Ordering::SeqCst)
    }

    fn accept(&self, handle: Handle) -> Result<SpillReceipt, SpillFailure<Handle>> {
        if !self.available.load(Ordering::SeqCst) {
            return Err(SpillFailure::new(
                MemoryError::capacity("spill target unavailable at accept time"),
                handle,
            ));
        }
        // Retain the handle so it can be restored later (two-way tier). Pushed
        // BEFORE bumping the accepted counter so a poisoned-lock failure leaves the
        // counter untouched (fail-closed: a rejected accept counts nothing).
        let mut store = match self.store.lock() {
            Ok(store) => store,
            Err(_) => {
                return Err(SpillFailure::new(MemoryError::poisoned("spill target store"), handle));
            }
        };
        let inventory_after = match u64::try_from(store.len()).ok().and_then(|n| n.checked_add(1)) {
            Some(count) => count,
            None => {
                return Err(SpillFailure::new(
                    MemoryError::capacity("spill inventory exceeds u64"),
                    handle,
                ));
            }
        };
        store.push_back(handle);
        self.accepted.fetch_add(1, Ordering::SeqCst);
        Ok(SpillReceipt::InProcess { inventory_after })
    }

    fn restore(&self) -> MemoryResult<Option<Handle>> {
        // Restore fails closed while the target is offline: a backend that has gone
        // away cannot be read from, so this is a typed capacity error rather than a
        // silent `None` that a caller could mistake for "nothing spilled".
        if !self.available.load(Ordering::SeqCst) {
            return Err(MemoryError::capacity("spill target unavailable at restore time"));
        }
        Ok(self.store.lock().map_err(|_| MemoryError::poisoned("spill target store"))?.pop_front())
    }
}

impl<Handle> InProcessSpillTarget<Handle> for InMemorySpillTarget<Handle> {}
impl<Handle> sealed::InProcessSpillTarget<Handle> for InMemorySpillTarget<Handle> {}

impl<Handle> SpillInventory for InMemorySpillTarget<Handle> {
    fn retained_count(&self) -> MemoryResult<u64> {
        InMemorySpillTarget::retained_count(self)
    }
}

/// An in-memory spill target that RETAINS accepted handles so they can be
/// restored, making spill a genuine two-way tier (offload then bring back) rather
/// than write-only eviction. FIFO: the oldest spilled handle restores first.
///
/// Like every spill helper it stays accounting-neutral — it routes handles only.
/// The store that owns a spilled item holds its ledger ticket, so spill-out /
/// restore accounting is the owner's responsibility, not the controller's.
#[derive(Debug, Default)]
pub struct RetainingSpillTarget<Handle> {
    available: AtomicBool,
    store: Mutex<VecDeque<Handle>>,
}

impl<Handle> RetainingSpillTarget<Handle> {
    /// Create an available retaining target.
    #[must_use]
    pub fn available() -> Self {
        Self { available: AtomicBool::new(true), store: Mutex::new(VecDeque::new()) }
    }

    /// Create an unavailable retaining target.
    #[must_use]
    pub fn unavailable() -> Self {
        Self { available: AtomicBool::new(false), store: Mutex::new(VecDeque::new()) }
    }

    /// Toggle availability (e.g. to simulate the backend going offline).
    pub fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::SeqCst);
    }

    /// Number of spilled handles currently retained (restorable).
    pub fn retained_count(&self) -> MemoryResult<u64> {
        let len = self.store.lock().map_err(|_| MemoryError::poisoned("spill target store"))?.len();
        u64::try_from(len).map_err(|_| MemoryError::capacity("spill inventory exceeds u64"))
    }
}

impl<Handle> SpillTarget<Handle> for RetainingSpillTarget<Handle> {
    fn is_available(&self) -> bool {
        self.available.load(Ordering::SeqCst)
    }

    fn accept(&self, handle: Handle) -> Result<SpillReceipt, SpillFailure<Handle>> {
        if !self.available.load(Ordering::SeqCst) {
            return Err(SpillFailure::new(
                MemoryError::capacity("spill target unavailable at accept time"),
                handle,
            ));
        }
        let mut store = match self.store.lock() {
            Ok(store) => store,
            Err(_) => {
                return Err(SpillFailure::new(MemoryError::poisoned("spill target store"), handle));
            }
        };
        let inventory_after = match u64::try_from(store.len()).ok().and_then(|n| n.checked_add(1)) {
            Some(count) => count,
            None => {
                return Err(SpillFailure::new(
                    MemoryError::capacity("spill inventory exceeds u64"),
                    handle,
                ));
            }
        };
        store.push_back(handle);
        Ok(SpillReceipt::InProcess { inventory_after })
    }

    fn restore(&self) -> MemoryResult<Option<Handle>> {
        // Same offline contract as InMemorySpillTarget (Liskov-uniform): a restore
        // while the target is offline fails closed with a typed Capacity error, so
        // a caller can distinguish "drained" (Ok(None)) from "unreachable".
        if !self.available.load(Ordering::SeqCst) {
            return Err(MemoryError::capacity("spill target unavailable at restore time"));
        }
        Ok(self.store.lock().map_err(|_| MemoryError::poisoned("spill target store"))?.pop_front())
    }
}

impl<Handle> InProcessSpillTarget<Handle> for RetainingSpillTarget<Handle> {}
impl<Handle> sealed::InProcessSpillTarget<Handle> for RetainingSpillTarget<Handle> {}

impl<Handle> SpillInventory for RetainingSpillTarget<Handle> {
    fn retained_count(&self) -> MemoryResult<u64> {
        RetainingSpillTarget::retained_count(self)
    }
}

#[cfg(test)]
mod tests {
    mod restore_contract;
    use super::*;

    #[test]
    fn poisoned_spill_inventory_returns_error_not_zero() {
        let target = std::sync::Arc::new(InMemorySpillTarget::<u32>::available());
        let _receipt = SpillTarget::accept(target.as_ref(), 7).expect("seed retained inventory");
        let poisoner = std::sync::Arc::clone(&target);
        assert!(std::thread::spawn(move || {
            let _guard = poisoner.store.lock().expect("spill inventory lock");
            panic!("poison spill inventory");
        })
        .join()
        .is_err());

        assert!(matches!(target.retained_count(), Err(MemoryError::Poisoned(_))));
    }

    #[test]
    fn unavailable_spill_returns_the_only_handle_without_dropping_it() {
        #[derive(Debug)]
        struct DropProbe(std::sync::Arc<AtomicU64>);
        impl Drop for DropProbe {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drops = std::sync::Arc::new(AtomicU64::new(0));
        let controller = RoutedSpillController::new(InMemorySpillTarget::unavailable());
        let failure = controller.spill(DropProbe(drops.clone())).expect_err("offline target");
        assert_eq!(drops.load(Ordering::SeqCst), 0, "denial must return ownership");
        let (error, handle) = failure.into_parts();
        assert!(matches!(error, MemoryError::Capacity(_)));
        assert_eq!(controller.spilled_count(), Ok(0));
        drop(handle);
        assert_eq!(drops.load(Ordering::SeqCst), 1, "caller alone retires the handle");
    }

    #[test]
    fn noop_spill_rejects_and_returns_the_handle() {
        let failure = SpillController::spill(&NoopSpillController, "live".to_string())
            .expect_err("Noop must not certify persistence");
        let (error, handle) = failure.into_parts();
        assert!(matches!(error, MemoryError::Capacity(_)));
        assert_eq!(handle, "live");
    }

    #[test]
    fn routed_spill_to_available_target_succeeds_and_counts() {
        let controller = RoutedSpillController::new(InMemorySpillTarget::available());
        let first = controller.spill("handle-1".to_string()).expect("spill to available");
        let second = controller.spill("handle-2".to_string()).expect("spill again");
        assert_eq!(first, SpillReceipt::InProcess { inventory_after: 1 });
        assert_eq!(second, SpillReceipt::InProcess { inventory_after: 2 });
        assert_eq!(controller.spilled_count(), Ok(2));
    }

    /// A target that reports available but whose `accept` always fails — drives the
    /// accept-failure branch after the availability preflight.
    #[derive(Debug, Default)]
    struct AcceptFailingTarget;

    impl<Handle> SpillTarget<Handle> for AcceptFailingTarget {
        fn is_available(&self) -> bool {
            true // passes the controller's guard...
        }
        fn accept(&self, handle: Handle) -> Result<SpillReceipt, SpillFailure<Handle>> {
            Err(SpillFailure::new(MemoryError::capacity("accept always fails"), handle))
        }
    }

    impl SpillInventory for AcceptFailingTarget {
        fn retained_count(&self) -> MemoryResult<u64> {
            Ok(0)
        }
    }

    #[test]
    fn routed_spill_rolls_back_the_count_when_accept_fails() {
        let controller = RoutedSpillController::new(AcceptFailingTarget);
        // is_available() is true, but accept fails before target inventory moves.
        let r = controller.spill("h".to_string());
        assert!(
            matches!(&r, Err(failure) if matches!(failure.error(), MemoryError::Capacity(_))),
            "accept failure surfaces, got {r:?}"
        );
        assert_eq!(r.expect_err("accept denied").into_parts().1, "h");
        assert_eq!(controller.spilled_count(), Ok(0), "a failed spill counts nothing");
        // Repeated failures never drift the count or wrap it negative.
        let _ = controller.spill("h2".to_string());
        assert_eq!(controller.spilled_count(), Ok(0), "inventory is stable");
    }

    #[test]
    fn spill_to_unavailable_target_fails_closed_and_does_not_count() {
        // NEGATIVE-PATH 4: spill target unavailable must fail-closed.
        let controller = RoutedSpillController::new(InMemorySpillTarget::unavailable());
        let err = controller.spill("handle".to_string()).expect_err("must fail when unavailable");
        assert!(
            matches!(err.error(), MemoryError::Capacity(_)),
            "unavailable spill must be a Capacity error, got {err:?}"
        );
        assert_eq!(err.into_parts().1, "handle");
        assert_eq!(
            controller.spilled_count(),
            Ok(0),
            "a failed spill must not change target inventory"
        );
    }

    #[test]
    fn available_controller_succeeds_while_unavailable_controller_rejects() {
        // Positive / negative pair over the same controller type, differing only in
        // target availability (the one core condition).
        let online = RoutedSpillController::new(InMemorySpillTarget::available());
        let _receipt = online.spill(1u32).expect("online spill ok");
        assert_eq!(online.spilled_count(), Ok(1));

        let offline = RoutedSpillController::new(InMemorySpillTarget::unavailable());
        let err = offline.spill(2u32).expect_err("offline rejects");
        assert!(matches!(err.error(), MemoryError::Capacity(_)), "got {err:?}");
        assert_eq!(err.into_parts().1, 2);
        assert_eq!(offline.spilled_count(), Ok(0));
    }

    #[test]
    fn noop_controller_rejects_without_a_target_and_returns_handle() {
        let controller = NoopSpillController;
        let failure = SpillController::spill(&controller, "anything".to_string())
            .expect_err("Noop is not persistence");
        assert!(matches!(failure.error(), MemoryError::Capacity(_)));
        assert_eq!(failure.into_parts().1, "anything");
    }

    #[test]
    fn routed_spill_after_target_goes_offline_fails_closed_leaving_inventory_unchanged() {
        let controller = RoutedSpillController::new(InMemorySpillTarget::available());
        let _first = controller.spill("first".to_string()).expect("first spill ok");
        let _second = controller.spill("second".to_string()).expect("second spill ok");
        let count_before = controller.spilled_count().expect("seed inventory");
        assert_eq!(count_before, 2);

        controller.target.set_available(false);
        let denied = controller.spill("blocked".to_string()).expect_err("offline target");
        assert!(matches!(denied.error(), MemoryError::Capacity(_)));
        assert_eq!(denied.into_parts().1, "blocked");
        assert_eq!(controller.spilled_count(), Ok(count_before));

        controller.target.set_available(true);
        assert_eq!(controller.restore().expect("first survives denial"), Some("first".to_string()));
        assert_eq!(
            controller.restore().expect("second survives denial"),
            Some("second".to_string())
        );
        assert_eq!(controller.restore().expect("drained"), None);
        assert_eq!(controller.spilled_count(), Ok(0));
    }

    #[test]
    fn in_memory_target_accept_after_toggle_offline_is_rejected() {
        let target = InMemorySpillTarget::available();
        let _first = SpillTarget::accept(&target, 1u32).expect("accept first while available");
        let _second = SpillTarget::accept(&target, 2u32).expect("accept second while available");
        assert_eq!(target.accepted_count(), 2);
        assert_eq!(target.retained_count(), Ok(2));
        target.set_available(false);
        let err = SpillTarget::accept(&target, 3u32).expect_err("accept while offline must fail");
        assert!(matches!(err.error(), MemoryError::Capacity(_)), "got {err:?}");
        assert_eq!(err.into_parts().1, 3);
        assert_eq!(target.accepted_count(), 2, "rejected accept must not increment count");
        assert_eq!(target.retained_count(), Ok(2), "rejected accept must preserve inventory");
        target.set_available(true);
        assert_eq!(target.restore().expect("first survives denial"), Some(1));
        assert_eq!(target.restore().expect("second survives denial"), Some(2));
        assert_eq!(target.restore().expect("drained"), None);
        assert_eq!(target.retained_count(), Ok(0));
    }

    // ---- C: spill is a two-way tier — spilled handles can be restored ----

    #[test]
    fn in_memory_target_write_then_restore_round_trips_the_payload() {
        // JOB 2 (write -> restore round-trip): the in-memory target is a two-way
        // tier — a spilled handle is brought back EXACTLY by restore, in FIFO order,
        // then None once drained. accepted_count stays cumulative.
        let target = InMemorySpillTarget::available();
        let _first = SpillTarget::accept(&target, "first".to_string()).expect("accept 1");
        let _second = SpillTarget::accept(&target, "second".to_string()).expect("accept 2");
        assert_eq!(target.accepted_count(), 2, "accepted_count is cumulative");
        assert_eq!(target.retained_count(), Ok(2), "both handles are retained for restore");

        // Restore returns the spilled payloads oldest-first, byte-for-byte.
        assert_eq!(target.restore().expect("r1"), Some("first".to_string()));
        assert_eq!(target.restore().expect("r2"), Some("second".to_string()));
        assert_eq!(target.retained_count(), Ok(0), "restore drains the retained handles");
    }

    #[test]
    fn in_memory_target_restore_of_missing_handle_is_none_not_an_error() {
        // JOB 2 (missing handle): a restore on a drained-but-online target yields a
        // typed Ok(None) — "nothing spilled to bring back" — distinct from the
        // offline failure path below. Restoring past empty stays None.
        let target = InMemorySpillTarget::available();
        let _receipt = SpillTarget::accept(&target, 7u32).expect("accept");
        assert_eq!(target.restore().expect("drain the one handle"), Some(7u32));
        // The handle is now gone: a further restore finds nothing.
        assert_eq!(
            SpillTarget::<u32>::restore(&target).expect("restore empty"),
            None,
            "restoring an already-restored / unknown handle yields None, not garbage"
        );
    }

    #[test]
    fn in_memory_target_restore_after_offline_fails_closed_leaving_retained_state_unchanged() {
        // JOB 2 (offline restore fails closed): a target that retained a handle, then
        // went offline, must reject restore with a typed Capacity error and leave the
        // retained handle byte-for-byte intact (recoverable once back online).
        let target = InMemorySpillTarget::available();
        let _receipt =
            SpillTarget::accept(&target, "durable".to_string()).expect("accept while available");
        let retained_before = target.retained_count();
        assert_eq!(retained_before, Ok(1));

        // Backend goes offline.
        target.set_available(false);
        let err = SpillTarget::restore(&target).expect_err("restore while offline must fail");
        assert!(
            matches!(err, MemoryError::Capacity(_)),
            "an offline restore must be a typed capacity error, got {err:?}"
        );
        // Fail-closed: the retained handle is untouched by the denied restore.
        assert_eq!(
            target.retained_count(),
            retained_before,
            "a denied restore must not drop the retained handle"
        );

        // Once back online the handle restores intact (the denial was non-destructive).
        target.set_available(true);
        assert_eq!(
            SpillTarget::restore(&target).expect("restore after recovery"),
            Some("durable".to_string()),
            "the handle held through the offline window restores byte-for-byte"
        );
    }

    #[test]
    fn retaining_target_round_trips_handles_fifo() {
        let target = RetainingSpillTarget::available();
        let _first = SpillTarget::accept(&target, "first".to_string()).expect("accept 1");
        let _second = SpillTarget::accept(&target, "second".to_string()).expect("accept 2");
        assert_eq!(target.retained_count(), Ok(2));

        // Restore returns spilled handles oldest-first, then None when drained.
        assert_eq!(target.restore().expect("r1"), Some("first".to_string()));
        assert_eq!(target.restore().expect("r2"), Some("second".to_string()));
        assert_eq!(target.restore().expect("r3"), None);
        assert_eq!(target.retained_count(), Ok(0));
    }

    #[test]
    fn retaining_target_offline_rejects_accept_but_state_unchanged() {
        let target = RetainingSpillTarget::available();
        let _first = SpillTarget::accept(&target, "first".to_string()).expect("seed first");
        let _second = SpillTarget::accept(&target, "second".to_string()).expect("seed second");
        assert_eq!(target.retained_count(), Ok(2));
        target.set_available(false);
        let err = SpillTarget::accept(&target, "blocked".to_string())
            .expect_err("offline accept rejected");
        assert!(matches!(err.error(), MemoryError::Capacity(_)), "got {err:?}");
        assert_eq!(err.into_parts().1, "blocked");
        assert_eq!(target.retained_count(), Ok(2), "rejected accept preserves retained handles");
        target.set_available(true);
        assert_eq!(target.restore().expect("first survives denial"), Some("first".to_string()));
        assert_eq!(target.restore().expect("second survives denial"), Some("second".to_string()));
        assert_eq!(target.restore().expect("drained"), None);
        assert_eq!(target.retained_count(), Ok(0));
    }

    // ---- LSP: both SpillTarget impls honor the SAME offline-restore contract ----

    #[test]
    fn retaining_target_offline_restore_fails_closed_like_in_memory() {
        // The Liskov fix: RetainingSpillTarget, like InMemorySpillTarget, must fail
        // closed (Capacity) when restore is called offline — NOT return Ok(None),
        // which a caller would misread as "drained". State stays intact.
        let target = RetainingSpillTarget::available();
        let _receipt = SpillTarget::accept(&target, "durable".to_string()).expect("accept");
        target.set_available(false);
        let err = SpillTarget::restore(&target).expect_err("offline restore must fail closed");
        assert!(matches!(err, MemoryError::Capacity(_)), "got {err:?}");
        assert_eq!(target.retained_count(), Ok(1), "denied restore is non-destructive");
        target.set_available(true);
        assert_eq!(
            SpillTarget::restore(&target).expect("restore after recovery"),
            Some("durable".to_string())
        );
    }

    /// Generic over the target type: an offline restore is a typed Capacity error,
    /// an online-drained restore is Ok(None). Exercised for BOTH impls so the trait
    /// is substitutable — a consumer cannot tell them apart by offline behavior.
    fn assert_offline_restore_contract<T: SpillTarget<u32>>(target: T, set_offline: impl Fn(&T)) {
        let _receipt = SpillTarget::accept(&target, 9u32).expect("accept online");
        set_offline(&target);
        assert!(
            matches!(SpillTarget::restore(&target), Err(MemoryError::Capacity(_))),
            "offline restore must be a Capacity error for every SpillTarget impl"
        );
    }

    #[test]
    fn both_spill_targets_share_the_offline_restore_contract() {
        assert_offline_restore_contract(InMemorySpillTarget::available(), |t| {
            t.set_available(false)
        });
        assert_offline_restore_contract(RetainingSpillTarget::available(), |t| {
            t.set_available(false)
        });
    }

    #[test]
    fn routed_controller_spill_then_restore_balances_the_spilled_count() {
        let controller = RoutedSpillController::new(RetainingSpillTarget::available());
        let _first = controller.spill("h1".to_string()).expect("spill 1");
        let _second = controller.spill("h2".to_string()).expect("spill 2");
        assert_eq!(controller.spilled_count(), Ok(2));

        // Each restore pulls a handle back and changes canonical target inventory.
        assert_eq!(controller.restore().expect("restore 1"), Some("h1".to_string()));
        assert_eq!(controller.spilled_count(), Ok(1));
        assert_eq!(controller.restore().expect("restore 2"), Some("h2".to_string()));
        assert_eq!(controller.spilled_count(), Ok(0));
        // Draining past empty is inert and never underflows the count.
        assert_eq!(controller.restore().expect("restore empty"), None);
        assert_eq!(controller.spilled_count(), Ok(0));
    }

    #[test]
    fn concurrent_spills_keep_spilled_count_consistent_then_drain_to_zero() {
        use std::sync::Arc;
        // Many concurrent spillers stress the target's canonical queue; a full
        // drain proves inventory reaches exactly zero and every handle is recovered.
        let controller = Arc::new(RoutedSpillController::new(RetainingSpillTarget::available()));
        let threads = 8u32;
        let per_thread = 20u32;
        let mut handles = Vec::new();
        for t in 0..threads {
            let c = Arc::clone(&controller);
            handles.push(std::thread::spawn(move || {
                for i in 0..per_thread {
                    let _receipt = c.spill(format!("t{t}-{i}")).expect("concurrent spill");
                }
            }));
        }
        for h in handles {
            h.join().expect("thread panicked");
        }
        let total = u64::from(threads * per_thread);
        assert_eq!(controller.spilled_count(), Ok(total), "every concurrent spill counted once");

        let mut drained = 0u64;
        while controller.restore().expect("restore").is_some() {
            drained += 1;
        }
        assert_eq!(drained, total, "all spilled handles restored");
        assert_eq!(controller.spilled_count(), Ok(0), "count balances to 0 after a full drain");
    }

    #[test]
    fn noop_controller_restore_is_none() {
        let controller = NoopSpillController;
        assert_eq!(SpillController::<u32>::restore(&controller).expect("noop restore"), None);
    }

    #[test]
    fn default_restore_distinguishes_offline_from_available_empty_target() {
        struct EvictingTarget(bool);
        impl SpillInventory for EvictingTarget {
            fn retained_count(&self) -> MemoryResult<u64> {
                Ok(0)
            }
        }
        impl SpillTarget<u32> for EvictingTarget {
            fn is_available(&self) -> bool {
                self.0
            }
            fn accept(&self, handle: u32) -> Result<SpillReceipt, SpillFailure<u32>> {
                Err(SpillFailure::new(MemoryError::capacity("fixture rejects writes"), handle))
            }
        }
        let online = RoutedSpillController::new(EvictingTarget(true));
        let offline = RoutedSpillController::new(EvictingTarget(false));
        assert_eq!(online.restore().expect("available discard target"), None);
        assert!(matches!(offline.restore(), Err(MemoryError::Capacity(_))));
        assert_eq!(online.spilled_count().expect("online inventory"), 0);
        assert_eq!(offline.spilled_count().expect("offline inventory"), 0);
    }
}
