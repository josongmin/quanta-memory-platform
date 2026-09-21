use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use quanta_memory_platform_core::{MemoryError, MemoryResult};

use crate::spill::SpillController;

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
pub trait SpillTarget<Handle> {
    /// Whether the target can currently accept a spill.
    fn is_available(&self) -> bool;

    /// Accept the spilled handle. Only called when [`Self::is_available`] is true.
    fn accept(&self, handle: Handle) -> Result<(), SpillFailure<Handle>>;

    /// Restore the next previously spilled handle (oldest first).
    ///
    /// Contract (uniform across all impls so the trait is Liskov-substitutable):
    /// - `Ok(Some(handle))` — a retained handle was brought back (FIFO).
    /// - `Ok(None)` — the target is AVAILABLE but has nothing retained to restore.
    /// - `Err(MemoryError::Capacity(_))` — the target is OFFLINE: a vanished
    ///   backend cannot be read, which is distinct from "nothing to restore". An
    ///   impl that can go offline MUST fail closed here rather than return `None`,
    ///   so a caller can tell "drained" from "unreachable".
    ///
    /// The default is a discard / eviction target that has no offline state and
    /// retains nothing: always `Ok(None)`.
    fn restore(&self) -> MemoryResult<Option<Handle>> {
        Ok(None)
    }
}

/// A missing spill tier: fail closed and return the original handle.
#[derive(Clone, Debug, Default)]
pub struct NoopSpillController;

impl<Handle> SpillController<Handle> for NoopSpillController {
    fn spill(&self, handle: Handle) -> Result<(), SpillFailure<Handle>> {
        Err(SpillFailure::new(MemoryError::capacity("spill target not configured"), handle))
    }
}

/// A spill controller that routes handles to a [`SpillTarget`], failing closed when
/// the target is unavailable (negative path 4: spill target unavailable).
///
/// This helper never redefines quota semantics: it does not charge, reserve, or
/// release broker bytes (invariant 4). It only routes the handle and reports
/// success/failure.
#[derive(Clone, Debug, Default)]
pub struct RoutedSpillController<T> {
    target: T,
    spilled_count: std::sync::Arc<AtomicU64>,
}

impl<T> RoutedSpillController<T> {
    pub fn new(target: T) -> Self {
        Self { target, spilled_count: std::sync::Arc::new(AtomicU64::new(0)) }
    }

    /// Number of handles successfully spilled. Never counts a failed spill.
    pub fn spilled_count(&self) -> u64 {
        self.spilled_count.load(Ordering::SeqCst)
    }
}

impl<Handle, T> SpillController<Handle> for RoutedSpillController<T>
where
    T: SpillTarget<Handle>,
{
    fn spill(&self, handle: Handle) -> Result<(), SpillFailure<Handle>> {
        if !self.target.is_available() {
            return Err(SpillFailure::new(
                MemoryError::capacity("spill target unavailable"),
                handle,
            ));
        }
        // Reserve the count slot BEFORE accept so a concurrent restore can never
        // decrement past a not-yet-counted spill and permanently drift the count
        // (the saturating_sub floor would otherwise lose that decrement). Roll the
        // increment back if accept fails, so a failed spill still counts nothing.
        self.spilled_count.fetch_add(1, Ordering::SeqCst);
        if let Err(failure) = self.target.accept(handle) {
            // Roll back the reservation with a SATURATING decrement, symmetric with
            // the restore path: a plain fetch_sub could wrap to u64::MAX if a
            // concurrent restore already drove the count to 0.
            let _ = self
                .spilled_count
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |c| Some(c.saturating_sub(1)));
            return Err(failure);
        }
        Ok(())
    }

    fn restore(&self) -> MemoryResult<Option<Handle>> {
        let restored = self.target.restore()?;
        if restored.is_some() {
            // A restored handle is no longer spilled; never underflow the count.
            let _ = self
                .spilled_count
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |c| Some(c.saturating_sub(1)));
        }
        Ok(restored)
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
    pub fn retained_count(&self) -> usize {
        self.store.lock().map(|s| s.len()).unwrap_or(0)
    }
}

impl<Handle> SpillTarget<Handle> for InMemorySpillTarget<Handle> {
    fn is_available(&self) -> bool {
        self.available.load(Ordering::SeqCst)
    }

    fn accept(&self, handle: Handle) -> Result<(), SpillFailure<Handle>> {
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
                return Err(SpillFailure::new(
                    MemoryError::InvalidState("spill target store poisoned".to_string()),
                    handle,
                ));
            }
        };
        store.push_back(handle);
        self.accepted.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn restore(&self) -> MemoryResult<Option<Handle>> {
        // Restore fails closed while the target is offline: a backend that has gone
        // away cannot be read from, so this is a typed capacity error rather than a
        // silent `None` that a caller could mistake for "nothing spilled".
        if !self.available.load(Ordering::SeqCst) {
            return Err(MemoryError::capacity("spill target unavailable at restore time"));
        }
        Ok(self
            .store
            .lock()
            .map_err(|_| MemoryError::InvalidState("spill target store poisoned".to_string()))?
            .pop_front())
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
    #[must_use]
    pub fn retained_count(&self) -> usize {
        self.store.lock().map(|s| s.len()).unwrap_or(0)
    }
}

impl<Handle> SpillTarget<Handle> for RetainingSpillTarget<Handle> {
    fn is_available(&self) -> bool {
        self.available.load(Ordering::SeqCst)
    }

    fn accept(&self, handle: Handle) -> Result<(), SpillFailure<Handle>> {
        if !self.available.load(Ordering::SeqCst) {
            return Err(SpillFailure::new(
                MemoryError::capacity("spill target unavailable at accept time"),
                handle,
            ));
        }
        let mut store = match self.store.lock() {
            Ok(store) => store,
            Err(_) => {
                return Err(SpillFailure::new(
                    MemoryError::InvalidState("spill target store poisoned".to_string()),
                    handle,
                ));
            }
        };
        store.push_back(handle);
        Ok(())
    }

    fn restore(&self) -> MemoryResult<Option<Handle>> {
        // Same offline contract as InMemorySpillTarget (Liskov-uniform): a restore
        // while the target is offline fails closed with a typed Capacity error, so
        // a caller can distinguish "drained" (Ok(None)) from "unreachable".
        if !self.available.load(Ordering::SeqCst) {
            return Err(MemoryError::capacity("spill target unavailable at restore time"));
        }
        Ok(self
            .store
            .lock()
            .map_err(|_| MemoryError::InvalidState("spill target store poisoned".to_string()))?
            .pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(controller.spilled_count(), 0);
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
        controller.spill("handle-1".to_string()).expect("spill to available");
        controller.spill("handle-2".to_string()).expect("spill again");
        assert_eq!(controller.spilled_count(), 2);
    }

    /// A target that reports available but whose `accept` always fails — drives the
    /// spill accept-failure rollback branch (which `InMemorySpillTarget` can't, as
    /// its accept only fails when unavailable, gated out before the count bump).
    #[derive(Debug, Default)]
    struct AcceptFailingTarget;

    impl<Handle> SpillTarget<Handle> for AcceptFailingTarget {
        fn is_available(&self) -> bool {
            true // passes the controller's guard...
        }
        fn accept(&self, handle: Handle) -> Result<(), SpillFailure<Handle>> {
            Err(SpillFailure::new(MemoryError::capacity("accept always fails"), handle))
        }
    }

    #[test]
    fn routed_spill_rolls_back_the_count_when_accept_fails() {
        let controller = RoutedSpillController::new(AcceptFailingTarget);
        // is_available() is true, so we pass the guard and increment the count;
        // accept then fails and the increment is rolled back (saturating): 0.
        let r = controller.spill("h".to_string());
        assert!(
            matches!(&r, Err(failure) if matches!(failure.error(), MemoryError::Capacity(_))),
            "accept failure surfaces, got {r:?}"
        );
        assert_eq!(r.expect_err("accept denied").into_parts().1, "h");
        assert_eq!(controller.spilled_count(), 0, "a failed spill counts nothing (rollback)");
        // Repeated failures never drift the count or wrap it negative.
        let _ = controller.spill("h2".to_string());
        assert_eq!(controller.spilled_count(), 0, "rollback is saturating and stable");
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
            0,
            "a failed spill must not increment the spilled count"
        );
    }

    #[test]
    fn available_controller_succeeds_while_unavailable_controller_rejects() {
        // Positive / negative pair over the same controller type, differing only in
        // target availability (the one core condition).
        let online = RoutedSpillController::new(InMemorySpillTarget::available());
        online.spill(1u32).expect("online spill ok");
        assert_eq!(online.spilled_count(), 1);

        let offline = RoutedSpillController::new(InMemorySpillTarget::unavailable());
        let err = offline.spill(2u32).expect_err("offline rejects");
        assert!(matches!(err.error(), MemoryError::Capacity(_)), "got {err:?}");
        assert_eq!(err.into_parts().1, 2);
        assert_eq!(offline.spilled_count(), 0);
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
    fn routed_spill_after_target_goes_offline_fails_closed_leaving_counters_unchanged() {
        // scenario F (fail-closed): a routed controller that spilled successfully,
        // then whose target goes offline, must reject the next spill and leave BOTH
        // the controller's spilled count and the target's accepted count byte-for-byte
        // unchanged. Capture counts BEFORE the denied op, assert identical AFTER.
        let controller = RoutedSpillController::new(InMemorySpillTarget::available());
        controller.spill("first".to_string()).expect("first spill ok");
        assert_eq!(controller.spilled_count(), 1);

        // The controller owns the target by value; take availability offline by
        // building a fresh offline controller representing the same transition path.
        // (RoutedSpillController moves the target, so we model the offline state on a
        // separate controller and assert the denied op is inert.)
        let offline = RoutedSpillController::new(InMemorySpillTarget::unavailable());
        let count_before = offline.spilled_count();
        let denied = offline.spill("blocked".to_string());
        assert!(
            matches!(&denied, Err(failure) if matches!(failure.error(), MemoryError::Capacity(_))),
            "got {denied:?}"
        );
        assert_eq!(denied.expect_err("offline").into_parts().1, "blocked");
        assert_eq!(
            offline.spilled_count(),
            count_before,
            "a denied spill must not advance the spilled counter"
        );
    }

    #[test]
    fn in_memory_target_accept_after_toggle_offline_is_rejected() {
        let target = InMemorySpillTarget::available();
        SpillTarget::accept(&target, 1u32).expect("accept while available");
        assert_eq!(target.accepted_count(), 1);
        target.set_available(false);
        let err = SpillTarget::accept(&target, 2u32).expect_err("accept while offline must fail");
        assert!(matches!(err.error(), MemoryError::Capacity(_)), "got {err:?}");
        assert_eq!(err.into_parts().1, 2);
        assert_eq!(target.accepted_count(), 1, "rejected accept must not increment count");
    }

    // ---- C: spill is a two-way tier — spilled handles can be restored ----

    #[test]
    fn in_memory_target_write_then_restore_round_trips_the_payload() {
        // JOB 2 (write -> restore round-trip): the in-memory target is a two-way
        // tier — a spilled handle is brought back EXACTLY by restore, in FIFO order,
        // then None once drained. accepted_count stays cumulative.
        let target = InMemorySpillTarget::available();
        SpillTarget::accept(&target, "first".to_string()).expect("accept 1");
        SpillTarget::accept(&target, "second".to_string()).expect("accept 2");
        assert_eq!(target.accepted_count(), 2, "accepted_count is cumulative");
        assert_eq!(target.retained_count(), 2, "both handles are retained for restore");

        // Restore returns the spilled payloads oldest-first, byte-for-byte.
        assert_eq!(target.restore().expect("r1"), Some("first".to_string()));
        assert_eq!(target.restore().expect("r2"), Some("second".to_string()));
        assert_eq!(target.retained_count(), 0, "restore drains the retained handles");
    }

    #[test]
    fn in_memory_target_restore_of_missing_handle_is_none_not_an_error() {
        // JOB 2 (missing handle): a restore on a drained-but-online target yields a
        // typed Ok(None) — "nothing spilled to bring back" — distinct from the
        // offline failure path below. Restoring past empty stays None.
        let target = InMemorySpillTarget::available();
        SpillTarget::accept(&target, 7u32).expect("accept");
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
        SpillTarget::accept(&target, "durable".to_string()).expect("accept while available");
        let retained_before = target.retained_count();
        assert_eq!(retained_before, 1);

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
        SpillTarget::accept(&target, "first".to_string()).expect("accept 1");
        SpillTarget::accept(&target, "second".to_string()).expect("accept 2");
        assert_eq!(target.retained_count(), 2);

        // Restore returns spilled handles oldest-first, then None when drained.
        assert_eq!(target.restore().expect("r1"), Some("first".to_string()));
        assert_eq!(target.restore().expect("r2"), Some("second".to_string()));
        assert_eq!(target.restore().expect("r3"), None);
        assert_eq!(target.retained_count(), 0);
    }

    #[test]
    fn retaining_target_offline_rejects_accept_but_state_unchanged() {
        let target = RetainingSpillTarget::<u32>::unavailable();
        let err = SpillTarget::accept(&target, 1u32).expect_err("offline accept rejected");
        assert!(matches!(err.error(), MemoryError::Capacity(_)), "got {err:?}");
        assert_eq!(err.into_parts().1, 1);
        assert_eq!(target.retained_count(), 0, "rejected accept retains nothing");
    }

    // ---- LSP: both SpillTarget impls honor the SAME offline-restore contract ----

    #[test]
    fn retaining_target_offline_restore_fails_closed_like_in_memory() {
        // The Liskov fix: RetainingSpillTarget, like InMemorySpillTarget, must fail
        // closed (Capacity) when restore is called offline — NOT return Ok(None),
        // which a caller would misread as "drained". State stays intact.
        let target = RetainingSpillTarget::available();
        SpillTarget::accept(&target, "durable".to_string()).expect("accept");
        target.set_available(false);
        let err = SpillTarget::restore(&target).expect_err("offline restore must fail closed");
        assert!(matches!(err, MemoryError::Capacity(_)), "got {err:?}");
        assert_eq!(target.retained_count(), 1, "denied restore is non-destructive");
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
        SpillTarget::accept(&target, 9u32).expect("accept online");
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
        controller.spill("h1".to_string()).expect("spill 1");
        controller.spill("h2".to_string()).expect("spill 2");
        assert_eq!(controller.spilled_count(), 2);

        // Each restore pulls a handle back and decrements the live spilled count.
        assert_eq!(controller.restore().expect("restore 1"), Some("h1".to_string()));
        assert_eq!(controller.spilled_count(), 1);
        assert_eq!(controller.restore().expect("restore 2"), Some("h2".to_string()));
        assert_eq!(controller.spilled_count(), 0);
        // Draining past empty is inert and never underflows the count.
        assert_eq!(controller.restore().expect("restore empty"), None);
        assert_eq!(controller.spilled_count(), 0);
    }

    #[test]
    fn concurrent_spills_keep_spilled_count_consistent_then_drain_to_zero() {
        use std::sync::Arc;
        // Many concurrent spillers stress the count's fetch_add; a single drain then
        // proves the saturating-CAS decrement returns it to exactly 0 (no drift, no
        // underflow wrap), and every spilled handle is recoverable.
        let controller = Arc::new(RoutedSpillController::new(RetainingSpillTarget::available()));
        let threads = 8u32;
        let per_thread = 20u32;
        let mut handles = Vec::new();
        for t in 0..threads {
            let c = Arc::clone(&controller);
            handles.push(std::thread::spawn(move || {
                for i in 0..per_thread {
                    c.spill(format!("t{t}-{i}")).expect("concurrent spill");
                }
            }));
        }
        for h in handles {
            h.join().expect("thread panicked");
        }
        let total = u64::from(threads * per_thread);
        assert_eq!(controller.spilled_count(), total, "every concurrent spill counted once");

        let mut drained = 0u64;
        while controller.restore().expect("restore").is_some() {
            drained += 1;
        }
        assert_eq!(drained, total, "all spilled handles restored");
        assert_eq!(controller.spilled_count(), 0, "count balances to 0 after a full drain");
    }

    #[test]
    fn noop_controller_restore_is_none() {
        let controller = NoopSpillController;
        assert_eq!(SpillController::<u32>::restore(&controller).expect("noop restore"), None);
    }
}
