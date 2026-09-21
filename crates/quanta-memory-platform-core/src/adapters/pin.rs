//! Explicitly unbudgeted in-memory adapter for the [`PinRegistry`] port.
//!
//! Lives in `adapters/` (not `ports/`) so the port layer stays a pure trait
//! surface: the `Arc<Mutex<HashMap>>` machinery and `std::sync` dependency belong
//! to the adapter, per the hexagonal rule (trait in `ports/`, impl in `adapters/`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::ports::pin::{InvalidationOutcome, PayloadPinLease, PinRegistry};
use crate::{MemoryError, MemoryResult, PinError};

#[derive(Clone, Default)]
pub struct UnbudgetedPinRegistry {
    state: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

impl std::fmt::Debug for UnbudgetedPinRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnbudgetedPinRegistry").finish_non_exhaustive()
    }
}

impl UnbudgetedPinRegistry {
    /// Acquire the state lock fail-closed: a poisoned lock surfaces as a typed
    /// `MemoryError::InvalidState` instead of panicking, so a mutating op can
    /// abort cleanly without taking the process down. Mirrors the broker
    /// poisoned-lock handling in `api/control.rs`.
    fn locked(&self) -> MemoryResult<MutexGuard<'_, HashMap<String, Vec<String>>>> {
        self.state.lock().map_err(|e| MemoryError::poisoned(format!("pin registry lock: {e}")))
    }

    /// Acquire the state lock for read-only accounting, recovering the guard if
    /// the lock is poisoned. The registry is consistent on read (a poisoned
    /// lock only means a writer panicked mid-update, which cannot happen here
    /// because the mutating ops are all-or-nothing), so recovering and reporting
    /// the current count is the documented safe default for the infallible
    /// `dependent_count` signature — it never panics.
    fn locked_recovering(&self) -> MutexGuard<'_, HashMap<String, Vec<String>>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PinRegistry<String, String> for UnbudgetedPinRegistry {
    fn pin(
        &self,
        payload: String,
        dependent: String,
    ) -> MemoryResult<PayloadPinLease<String, String>> {
        let mut guard = self.locked()?;
        let dependents = guard.entry(payload.clone()).or_default();
        // De-dup: a repeated (payload, dependent) pin must not over-count the
        // dependent fan-out. The lease is still handed back (idempotent pin).
        if !dependents.iter().any(|existing| existing == &dependent) {
            dependents.push(dependent.clone());
        }
        Ok(PayloadPinLease { payload, dependent })
    }

    fn dependent_count(&self, payload: &String) -> u64 {
        self.locked_recovering()
            .get(payload)
            .map_or(0, |dependents| u64::try_from(dependents.len()).unwrap_or(u64::MAX))
    }

    fn invalidate_dependents_atomically(
        &self,
        payload: &String,
    ) -> Result<InvalidationOutcome<String>, PinError> {
        let invalidated_dependents = self.locked()?.remove(payload).unwrap_or_default();
        Ok(InvalidationOutcome { invalidated_dependents })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> UnbudgetedPinRegistry {
        UnbudgetedPinRegistry::default()
    }

    // scenario M: pin -> invalidate -> unpin (re-pin) round-trip leaves the
    // registry in a consistent state with the dependent count tracking the
    // observable fan-out at each step.
    #[test]
    fn pin_invalidate_round_trip_tracks_dependent_count() {
        let reg = registry();
        assert_eq!(reg.dependent_count(&"p".to_string()), 0, "empty registry has no dependents");

        let lease = reg.pin("p".to_string(), "d1".to_string()).expect("pin must succeed");
        assert_eq!(lease.payload, "p");
        assert_eq!(lease.dependent, "d1");
        assert_eq!(reg.dependent_count(&"p".to_string()), 1, "one dependent after first pin");

        reg.pin("p".to_string(), "d2".to_string()).expect("second pin must succeed");
        assert_eq!(reg.dependent_count(&"p".to_string()), 2, "two distinct dependents pinned");

        let outcome = reg
            .invalidate_dependents_atomically(&"p".to_string())
            .expect("invalidate must succeed");
        assert_eq!(
            reg.dependent_count(&"p".to_string()),
            0,
            "invalidation drains all dependents to zero"
        );
        assert_eq!(outcome.invalidated_dependents.len(), 2, "both dependents reported invalidated");

        // Re-pin (the "unpin then pin fresh" leg of the round-trip) starts clean.
        reg.pin("p".to_string(), "d1".to_string()).expect("re-pin must succeed");
        assert_eq!(reg.dependent_count(&"p".to_string()), 1, "re-pin restores a single dependent");
    }

    // scenario M: InvalidationOutcome must report exactly the dependents that
    // were pinned to the payload, and an empty outcome for an unknown payload.
    #[test]
    fn invalidation_outcome_reports_exact_dependents() {
        let reg = registry();
        reg.pin("alpha".to_string(), "x".to_string()).expect("pin x");
        reg.pin("alpha".to_string(), "y".to_string()).expect("pin y");
        reg.pin("beta".to_string(), "z".to_string()).expect("pin z");

        let outcome =
            reg.invalidate_dependents_atomically(&"alpha".to_string()).expect("invalidate alpha");
        assert_eq!(
            outcome.invalidated_dependents,
            vec!["x".to_string(), "y".to_string()],
            "outcome lists alpha's dependents in pin order, not beta's"
        );
        // beta is untouched by alpha's invalidation.
        assert_eq!(reg.dependent_count(&"beta".to_string()), 1, "unrelated payload untouched");

        // Invalidating an unknown payload yields an empty, defaulted outcome.
        let empty = reg
            .invalidate_dependents_atomically(&"missing".to_string())
            .expect("invalidate missing");
        assert_eq!(empty, InvalidationOutcome::default(), "unknown payload -> empty outcome");
    }

    // scenario M: a duplicate (payload, dependent) pin must NOT over-count the
    // dependent fan-out; it is idempotent and still returns a lease.
    #[test]
    fn duplicate_pin_does_not_over_count_dependent() {
        let reg = registry();
        reg.pin("p".to_string(), "d".to_string()).expect("first pin");
        reg.pin("p".to_string(), "d".to_string()).expect("duplicate pin");
        reg.pin("p".to_string(), "d".to_string()).expect("triplicate pin");

        assert_eq!(
            reg.dependent_count(&"p".to_string()),
            1,
            "repeated identical pins collapse to a single dependent"
        );

        // The duplicate dependent is invalidated exactly once.
        let outcome = reg.invalidate_dependents_atomically(&"p".to_string()).expect("invalidate");
        assert_eq!(
            outcome.invalidated_dependents,
            vec!["d".to_string()],
            "deduped dependent invalidated once, not three times"
        );
    }

    // scenario M: distinct dependents on the same payload all count.
    #[test]
    fn distinct_dependents_each_increment_count() {
        let reg = registry();
        for d in ["d1", "d2", "d3"] {
            reg.pin("p".to_string(), d.to_string()).expect("pin distinct dependent");
        }
        assert_eq!(
            reg.dependent_count(&"p".to_string()),
            3,
            "three distinct dependents counted independently"
        );
    }

    // scenario M: a poisoned lock is fail-closed for mutating ops (typed
    // Poisoned, no panic) and recovers a safe count for the read path,
    // never taking the process down.
    #[test]
    fn poisoned_lock_is_fail_closed_not_panicking() {
        use std::sync::Arc as StdArc;

        let reg = registry();
        reg.pin("p".to_string(), "d".to_string()).expect("seed pin");

        // Poison the inner lock by panicking while holding the guard.
        let inner = StdArc::clone(&reg.state);
        let _ = std::thread::spawn(move || {
            let _g = inner.lock().expect("acquire to poison");
            panic!("intentional poison for test");
        })
        .join();

        // Read path recovers and reports the count that was present pre-poison.
        assert_eq!(
            reg.dependent_count(&"p".to_string()),
            1,
            "read path recovers a poisoned lock instead of panicking"
        );

        // Mutating path is fail-closed with a typed error, no panic.
        let pin_result = reg.pin("p".to_string(), "d2".to_string());
        assert!(
            matches!(pin_result, Err(MemoryError::Poisoned(_))),
            "pin on a poisoned lock returns typed Poisoned, got {pin_result:?}"
        );
        let inv_result = reg.invalidate_dependents_atomically(&"p".to_string());
        assert!(
            matches!(inv_result, Err(MemoryError::Poisoned(_))),
            "invalidate on a poisoned lock returns typed Poisoned, got {inv_result:?}"
        );
    }
}
