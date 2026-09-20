use std::fmt;

use crate::{CapacityError, ChargeKind, MemoryContextHandle, MemoryError, MemoryResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScratchRequest {
    pub label: String,
    pub requested_bytes: u64,
}

impl ScratchRequest {
    pub fn new(label: impl Into<String>, requested_bytes: u64) -> Self {
        Self { label: label.into(), requested_bytes }
    }
}

pub trait ScratchAllocator: Send + Sync {
    fn reserve_scratch_scope(
        &self,
        ctx: &MemoryContextHandle,
        request: ScratchRequest,
    ) -> Result<ScratchReservation, CapacityError>;
}

pub(crate) trait ScratchReservationAuthority: Send {
    fn release(&mut self) -> MemoryResult<()>;
}

/// Linear scratch-capacity authority. It cannot be cloned or forged.
///
/// ```compile_fail
/// # use quanta_memory_platform_core::ScratchReservation;
/// # fn duplicate(reservation: ScratchReservation) {
/// let second = reservation.clone();
/// # }
/// ```
pub struct ScratchReservation {
    context: MemoryContextHandle,
    label: String,
    reserved_bytes: u64,
    observed_bytes: u64,
    released: bool,
    authority: Option<Box<dyn ScratchReservationAuthority>>,
}

impl ScratchReservation {
    #[cfg(test)]
    fn unenforced(context: MemoryContextHandle, label: String, reserved_bytes: u64) -> Self {
        Self { context, label, reserved_bytes, observed_bytes: 0, released: false, authority: None }
    }

    #[cfg(any(feature = "live_control", test))]
    pub(crate) fn enforced(
        context: MemoryContextHandle,
        label: String,
        reserved_bytes: u64,
        authority: Box<dyn ScratchReservationAuthority>,
    ) -> Self {
        Self {
            context,
            label,
            reserved_bytes,
            observed_bytes: 0,
            released: false,
            authority: Some(authority),
        }
    }

    #[must_use]
    pub fn context(&self) -> &MemoryContextHandle {
        &self.context
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }

    #[must_use]
    pub fn observed_bytes(&self) -> u64 {
        self.observed_bytes
    }

    #[must_use]
    pub fn is_released(&self) -> bool {
        self.released
    }

    /// Whether this scope owns broker-backed accounting authority.
    #[must_use]
    pub fn is_enforced(&self) -> bool {
        self.authority.is_some() && !self.released
    }

    /// Scratch bytes are always accounted against the live-scratch charge kind
    /// while the reservation is outstanding. Releasing returns those bytes to
    /// the ledger; this accessor makes the mapping observable to callers and
    /// tests without exposing the ledger seam through the port.
    pub fn charge_kind(&self) -> ChargeKind {
        ChargeKind::LiveScratch
    }

    /// Record the peak observed scratch usage for this scope.
    ///
    /// Misuse-resistant: growing an already-released reservation is a typed
    /// `MemoryError::InvalidState` (the scope's bytes have already been returned
    /// to the ledger, so further growth would be silently lost). The check runs
    /// before any mutation, so a rejected call leaves `observed_bytes`
    /// byte-for-byte unchanged (fail-closed).
    pub fn grow_to_observed(&mut self, observed_bytes: u64) -> Result<(), CapacityError> {
        if self.released {
            return Err(MemoryError::InvalidState(
                "scratch reservation already released: grow_to_observed rejected".to_string(),
            ));
        }
        if observed_bytes > self.reserved_bytes {
            return Err(MemoryError::capacity("observed scratch exceeds reserved bytes"));
        }
        self.observed_bytes = observed_bytes;
        Ok(())
    }

    /// Release the reservation, returning its bytes to the ledger.
    ///
    /// Misuse-resistant: a double release is a typed `MemoryError::InvalidState`
    /// rather than a silent no-op, so accidental double-return of scratch bytes
    /// is caught. The flag is only set after the guard passes, so a rejected
    /// second release leaves the `released` state unchanged (it was already
    /// `true`) and never mutates anything further.
    pub fn release(&mut self) -> Result<(), CapacityError> {
        if self.released {
            return Err(MemoryError::InvalidState(
                "scratch reservation already released: double release rejected".to_string(),
            ));
        }
        if let Some(authority) = self.authority.as_mut() {
            authority.release()?;
        }
        self.authority = None;
        self.released = true;
        Ok(())
    }
}

impl fmt::Debug for ScratchReservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScratchReservation")
            .field("context", &self.context)
            .field("label", &self.label)
            .field("reserved_bytes", &self.reserved_bytes)
            .field("observed_bytes", &self.observed_bytes)
            .field("released", &self.released)
            .field("enforced", &self.authority.is_some())
            .finish()
    }
}

impl Drop for ScratchReservation {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryContextKind, OwnerKey, RuntimeId};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn ctx() -> MemoryContextHandle {
        MemoryContextHandle::new(
            RuntimeId::from("rt"),
            MemoryContextKind::Request,
            OwnerKey::from("owner"),
        )
    }

    fn reservation(reserved: u64) -> ScratchReservation {
        // Construct the port value directly; the concrete allocator adapter lives
        // in `adapters::scratch` and is exercised there. These tests cover the
        // ScratchReservation port logic (grow/release/charge-kind).
        ScratchReservation::unenforced(ctx(), "buf".to_string(), reserved)
    }

    // scenario N: a fresh reservation maps to the LiveScratch charge kind, the
    // only kind scratch bytes are ever accounted against.
    #[test]
    fn reservation_maps_to_live_scratch_charge_kind() {
        let res = reservation(1024);
        assert_eq!(
            res.charge_kind(),
            ChargeKind::LiveScratch,
            "outstanding scratch is accounted as LiveScratch"
        );
        assert_eq!(res.reserved_bytes(), 1024, "reserved bytes mirror the request");
        assert_eq!(res.observed_bytes(), 0, "fresh reservation observes nothing yet");
        assert!(!res.is_released(), "fresh reservation is live");
        assert!(!res.is_enforced(), "port-only fixture has no live authority");
    }

    // scenario N: grow_to_observed accepts values up to the reserved ceiling and
    // updates observed_bytes; the charge kind stays LiveScratch.
    #[test]
    fn grow_to_observed_updates_within_reserved_ceiling() {
        let mut res = reservation(1000);
        res.grow_to_observed(400).expect("grow within ceiling");
        assert_eq!(res.observed_bytes(), 400, "observed updated to the grown value");
        res.grow_to_observed(1000).expect("grow to exact ceiling");
        assert_eq!(res.observed_bytes(), 1000, "observed may reach the reserved ceiling");
        assert_eq!(res.charge_kind(), ChargeKind::LiveScratch, "still live scratch while held");
    }

    // scenario N: growth beyond the reserved ceiling is a typed Capacity error
    // and is fail-closed — observed_bytes is unchanged after the denial.
    #[test]
    fn grow_beyond_reserved_is_fail_closed_capacity_error() {
        let mut res = reservation(500);
        res.grow_to_observed(300).expect("seed observed");
        let before = res.observed_bytes();

        let denied = res.grow_to_observed(501);
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "over-reserved growth is a typed Capacity error, got {denied:?}"
        );
        assert_eq!(res.observed_bytes(), before, "denied growth leaves observed bytes unchanged");
    }

    // scenario N: operating on an already-released reservation is rejected.
    // grow_to_observed after release must be a typed InvalidState and must NOT
    // mutate observed_bytes (fail-closed misuse resistance).
    #[test]
    fn grow_after_release_is_rejected_and_fail_closed() {
        let mut res = reservation(1000);
        res.grow_to_observed(200).expect("seed observed before release");
        res.release().expect("first release succeeds");
        let after_release = res.observed_bytes();

        let denied = res.grow_to_observed(900);
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "grow on a released reservation is typed InvalidState, got {denied:?}"
        );
        assert_eq!(
            res.observed_bytes(),
            after_release,
            "rejected grow leaves observed bytes unchanged"
        );
    }

    // scenario N: release is NOT silently idempotent. A double release is a
    // typed InvalidState and leaves the released state unchanged (still true).
    #[test]
    fn double_release_is_rejected_not_silent() {
        let mut res = reservation(256);
        res.release().expect("first release succeeds");
        assert!(res.is_released(), "first release marks the reservation released");

        let denied = res.release();
        assert!(
            matches!(denied, Err(MemoryError::InvalidState(_))),
            "second release is typed InvalidState, got {denied:?}"
        );
        assert!(res.is_released(), "rejected double release remains released");
    }

    // scenario N: the happy-path lifecycle reserve -> grow -> release succeeds in
    // order, proving the guard only bites on misuse.
    #[test]
    fn lifecycle_reserve_grow_release_succeeds_in_order() {
        let mut res = reservation(2048);
        assert_eq!(res.charge_kind(), ChargeKind::LiveScratch);
        res.grow_to_observed(1024).expect("grow while live");
        assert_eq!(res.observed_bytes(), 1024);
        res.release().expect("release while live");
        assert!(res.is_released(), "lifecycle ends released");
    }

    struct CountingAuthority {
        calls: Arc<AtomicUsize>,
        fail_first: bool,
    }

    impl ScratchReservationAuthority for CountingAuthority {
        fn release(&mut self) -> MemoryResult<()> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_first && call == 0 {
                return Err(MemoryError::InvalidState("injected release failure".to_string()));
            }
            Ok(())
        }
    }

    #[test]
    fn failed_release_preserves_authority_for_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut reservation = ScratchReservation::enforced(
            ctx(),
            "retry".to_string(),
            64,
            Box::new(CountingAuthority { calls: Arc::clone(&calls), fail_first: true }),
        );

        assert!(matches!(reservation.release(), Err(MemoryError::InvalidState(_))));
        assert!(!reservation.is_released());
        assert!(reservation.is_enforced());
        reservation.release().expect("authority remains retryable");
        assert!(reservation.is_released());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn successful_release_prevents_drop_from_releasing_twice() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut reservation = ScratchReservation::enforced(
            ctx(),
            "once".to_string(),
            64,
            Box::new(CountingAuthority { calls: Arc::clone(&calls), fail_first: false }),
        );
        reservation.release().expect("release");
        drop(reservation);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
