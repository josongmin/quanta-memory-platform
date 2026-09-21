//! Non-blocking lifecycle gate shared by accounting-backed facade stores.
//!
//! The gate serializes externally visible accounting transitions without holding
//! a store/container mutex while an accounting callback runs. Reentrant callers
//! receive a typed error instead of waiting on the in-flight operation.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};

use quanta_memory_platform_core::{MemoryError, MemoryResult};

const OPEN: u8 = 0;
const OPERATION_IN_PROGRESS: u8 = 1;
const CLOSING: u8 = 2;
const CLOSED: u8 = 3;

/// Never let an arbitrary client's destructor unwind from a store destructor.
/// Call once per client-owned field, after releasing its managed charge. Catching
/// a whole collection is insufficient: two field panics during collection
/// unwinding would abort the process before the second could be contained.
pub(crate) fn retire_client_value<T>(value: T) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(value))) {
        // Dropping a caught panic payload is user-controlled too: `panic_any`
        // can carry a value whose destructor panics. During an outer unwind that
        // second panic aborts the process. This exceptional path intentionally
        // leaks only the panic payload so store cleanup can continue retiring
        // the remaining fields and release every accounting authority.
        std::mem::forget(payload);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoreLifecyclePhase {
    Open,
    OperationInProgress,
    Closing,
    Closed,
}

impl StoreLifecyclePhase {
    fn from_raw(raw: u8) -> Self {
        match raw {
            OPEN => Self::Open,
            OPERATION_IN_PROGRESS => Self::OperationInProgress,
            CLOSING => Self::Closing,
            CLOSED => Self::Closed,
            _ => unreachable!("StoreLifecycle only writes known phase values"),
        }
    }
}

/// Per-store lifecycle authority.
///
/// This value is intentionally not `Clone`: one store has one mutation lane and
/// one terminal close transition.
#[derive(Debug)]
pub(crate) struct StoreLifecycle {
    resource: &'static str,
    phase: AtomicU8,
}

impl StoreLifecycle {
    pub(crate) const fn new(resource: &'static str) -> Self {
        Self { resource, phase: AtomicU8::new(OPEN) }
    }

    pub(crate) fn phase(&self) -> StoreLifecyclePhase {
        StoreLifecyclePhase::from_raw(self.phase.load(Ordering::Acquire))
    }

    /// A client-supplied Clone/Hash/Eq/Ord/CanonicalKey may execute while a
    /// container mutex is held. A callback that reenters the same store must
    /// fail with typed contention, never block on its own mutex. This applies
    /// to both reads and mutations; the lifecycle phase alone cannot exclude
    /// callbacks reached by a read-only operation.
    pub(crate) fn try_lock_state<'a, T>(
        &self,
        state: &'a Mutex<T>,
    ) -> MemoryResult<MutexGuard<'a, T>> {
        match state.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::WouldBlock) => Err(MemoryError::operation_in_progress(self.resource)),
            Err(TryLockError::Poisoned(_)) => {
                Err(MemoryError::InvalidState(format!("{} lock poisoned", self.resource)))
            }
        }
    }

    /// Read either a complete open-state image or the terminal empty image.
    /// A store calls this before and after acquiring its container lock, so a
    /// temporarily detached accounting plan is never mistaken for empty data.
    pub(crate) fn ensure_readable(&self) -> MemoryResult<()> {
        match self.phase() {
            StoreLifecyclePhase::Open | StoreLifecyclePhase::Closed => Ok(()),
            StoreLifecyclePhase::OperationInProgress | StoreLifecyclePhase::Closing => {
                Err(MemoryError::operation_in_progress(self.resource))
            }
        }
    }

    /// Acquire the sole mutation lane without waiting.
    ///
    /// Dropping the returned permit reopens the lane, including during unwind.
    pub(crate) fn begin_mutation(&self) -> MemoryResult<MutationPermit<'_>> {
        match self.phase.compare_exchange(
            OPEN,
            OPERATION_IN_PROGRESS,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(MutationPermit { lifecycle: self, active: true }),
            Err(CLOSED) => Err(MemoryError::closed(self.resource)),
            Err(_) => Err(MemoryError::operation_in_progress(self.resource)),
        }
    }

    /// Begin the terminal close transition without waiting.
    ///
    /// An already-closed store is an idempotent outcome. An acquired close permit
    /// reopens the store if it is dropped before [`ClosePermit::commit`].
    pub(crate) fn begin_close(&self) -> MemoryResult<BeginClose<'_>> {
        match self.phase.compare_exchange(OPEN, CLOSING, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => Ok(BeginClose::Acquired(ClosePermit { lifecycle: self, active: true })),
            Err(CLOSED) => Ok(BeginClose::AlreadyClosed),
            Err(_) => Err(MemoryError::operation_in_progress(self.resource)),
        }
    }

    /// Force the terminal state from a store's `Drop` implementation.
    ///
    /// `&mut self` proves normal permits cannot still borrow the gate. The swap
    /// also recovers safely from a deliberately forgotten permit: Drop cleanup is
    /// still required for every prior non-closed phase.
    pub(crate) fn transition_to_closed_for_drop(&mut self) -> DropTransition {
        if *self.phase.get_mut() == CLOSED {
            DropTransition::AlreadyClosed
        } else {
            *self.phase.get_mut() = CLOSED;
            DropTransition::CleanupRequired
        }
    }

    fn reopen(&self, owner: u8) {
        let result = self.phase.compare_exchange(owner, OPEN, Ordering::AcqRel, Ordering::Acquire);
        debug_assert!(result.is_ok(), "lifecycle permit lost ownership of its phase");
    }
}

#[derive(Debug)]
pub(crate) enum BeginClose<'a> {
    Acquired(ClosePermit<'a>),
    AlreadyClosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DropTransition {
    CleanupRequired,
    AlreadyClosed,
}

/// RAII ownership of the store mutation lane.
#[derive(Debug)]
pub(crate) struct MutationPermit<'a> {
    lifecycle: &'a StoreLifecycle,
    active: bool,
}

impl Drop for MutationPermit<'_> {
    fn drop(&mut self) {
        if self.active {
            self.lifecycle.reopen(OPERATION_IN_PROGRESS);
            self.active = false;
        }
    }
}

/// RAII ownership of an in-flight close attempt.
#[derive(Debug)]
pub(crate) struct ClosePermit<'a> {
    lifecycle: &'a StoreLifecycle,
    active: bool,
}

impl ClosePermit<'_> {
    /// Commit the terminal transition. Every later mutation observes `Closed`.
    pub(crate) fn commit(mut self) {
        let result = self.lifecycle.phase.compare_exchange(
            CLOSING,
            CLOSED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        debug_assert!(result.is_ok(), "close permit lost ownership of the closing phase");
        self.active = false;
    }

    /// Explicit failure path. Dropping without commit has the same behavior.
    #[cfg(any(
        test,
        feature = "payload_store",
        feature = "retained_map",
        feature = "publish_pipeline"
    ))]
    pub(crate) fn rollback(mut self) {
        self.lifecycle.reopen(CLOSING);
        self.active = false;
    }
}

impl Drop for ClosePermit<'_> {
    fn drop(&mut self) {
        if self.active {
            self.lifecycle.reopen(CLOSING);
            self.active = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PanicOnPayloadDrop;

    impl Drop for PanicOnPayloadDrop {
        fn drop(&mut self) {
            panic!("panic payload destructor");
        }
    }

    struct PanicWithHostilePayload;

    impl Drop for PanicWithHostilePayload {
        fn drop(&mut self) {
            std::panic::panic_any(PanicOnPayloadDrop);
        }
    }

    struct RetireDuringOuterUnwind;

    impl Drop for RetireDuringOuterUnwind {
        fn drop(&mut self) {
            retire_client_value(PanicWithHostilePayload);
        }
    }

    #[test]
    fn hostile_panic_payload_drop_cannot_abort_outer_unwind() {
        const CHILD: &str = "QMP_HOSTILE_PANIC_PAYLOAD_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let outer = std::panic::catch_unwind(|| {
                let _retire = RetireDuringOuterUnwind;
                panic!("outer unwind");
            })
            .expect_err("outer panic propagates after client retirement");
            assert_eq!(outer.downcast_ref::<&str>(), Some(&"outer unwind"));
            return;
        }

        let status = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("shared::lifecycle::tests::hostile_panic_payload_drop_cannot_abort_outer_unwind")
            .arg("--nocapture")
            .env(CHILD, "1")
            .status()
            .expect("run isolated destructor-panic child");
        assert!(status.success(), "hostile panic payload must not abort the child: {status}");
    }

    #[test]
    fn client_callback_reentry_on_held_container_lock_is_typed_busy() {
        let lifecycle = StoreLifecycle::new("callback owner");
        let state = Mutex::new(7u64);
        let held = state.lock().expect("outer read holds state");
        assert!(matches!(
            lifecycle.try_lock_state(&state),
            Err(MemoryError::OperationInProgress(resource)) if resource == "callback owner"
        ));
        drop(held);
        assert_eq!(*lifecycle.try_lock_state(&state).expect("reentrant call ended"), 7);
    }

    #[test]
    fn reentry_returns_typed_busy_without_blocking() {
        let lifecycle = StoreLifecycle::new("payload store");
        let permit = lifecycle.begin_mutation().expect("first mutation owns lane");

        let reentrant = lifecycle.begin_mutation();
        assert!(matches!(
            reentrant,
            Err(MemoryError::OperationInProgress(resource)) if resource == "payload store"
        ));
        assert!(matches!(
            lifecycle.ensure_readable(),
            Err(MemoryError::OperationInProgress(resource)) if resource == "payload store"
        ));
        assert!(matches!(
            lifecycle.begin_close(),
            Err(MemoryError::OperationInProgress(resource)) if resource == "payload store"
        ));

        drop(permit);
        assert!(lifecycle.begin_mutation().is_ok(), "permit drop must reopen the lane");
    }

    #[test]
    fn close_excludes_mutations_and_other_close_attempts() {
        let lifecycle = StoreLifecycle::new("retained map");
        let BeginClose::Acquired(close) = lifecycle.begin_close().expect("begin close") else {
            panic!("fresh lifecycle must acquire close");
        };

        assert_eq!(lifecycle.phase(), StoreLifecyclePhase::Closing);
        assert!(matches!(
            lifecycle.begin_mutation(),
            Err(MemoryError::OperationInProgress(resource)) if resource == "retained map"
        ));
        assert!(matches!(
            lifecycle.begin_close(),
            Err(MemoryError::OperationInProgress(resource)) if resource == "retained map"
        ));

        close.rollback();
        assert_eq!(lifecycle.phase(), StoreLifecyclePhase::Open);
    }

    #[test]
    fn failed_close_reopens_for_mutation_and_retry() {
        let lifecycle = StoreLifecycle::new("transcript store");
        let BeginClose::Acquired(failed) = lifecycle.begin_close().expect("begin close") else {
            panic!("fresh lifecycle must acquire close");
        };
        drop(failed);

        assert_eq!(lifecycle.phase(), StoreLifecyclePhase::Open);
        drop(lifecycle.begin_mutation().expect("mutation resumes after failed close"));
        assert!(matches!(lifecycle.begin_close(), Ok(BeginClose::Acquired(_))));
    }

    #[test]
    fn committed_close_stays_closed_and_double_close_is_idempotent() {
        let lifecycle = StoreLifecycle::new("prepared publication");
        let BeginClose::Acquired(close) = lifecycle.begin_close().expect("begin close") else {
            panic!("fresh lifecycle must acquire close");
        };
        close.commit();

        assert_eq!(lifecycle.phase(), StoreLifecyclePhase::Closed);
        lifecycle.ensure_readable().expect("terminal empty image is readable");
        assert!(matches!(
            lifecycle.begin_mutation(),
            Err(MemoryError::Closed(resource)) if resource == "prepared publication"
        ));
        assert!(matches!(lifecycle.begin_close(), Ok(BeginClose::AlreadyClosed)));
        assert_eq!(lifecycle.phase(), StoreLifecyclePhase::Closed);
    }

    #[test]
    fn drop_transition_is_terminal_and_reports_cleanup_once() {
        let mut open = StoreLifecycle::new("revocable store");
        assert_eq!(open.transition_to_closed_for_drop(), DropTransition::CleanupRequired);
        assert_eq!(open.phase(), StoreLifecyclePhase::Closed);
        assert_eq!(open.transition_to_closed_for_drop(), DropTransition::AlreadyClosed);

        let mut closed = StoreLifecycle::new("payload store");
        let BeginClose::Acquired(close) = closed.begin_close().expect("begin close") else {
            panic!("fresh lifecycle must acquire close");
        };
        close.commit();
        assert_eq!(closed.transition_to_closed_for_drop(), DropTransition::AlreadyClosed);
    }
}
