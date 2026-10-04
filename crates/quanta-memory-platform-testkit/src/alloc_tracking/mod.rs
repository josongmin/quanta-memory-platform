//! Dev/test-only allocation probe.
//!
//! `alloc_tracking/*` is the ONLY place the `tracking-allocator` crate may be
//! integrated, and it is gated behind the `alloc_tracking` feature so it can
//! never enter a production dependency graph. The probe accumulates allocation
//! signals (bytes allocated/deallocated, allocation count) and feeds them into
//! deterministic assertions.
//!
//! Honesty contract: this crate does NOT install a `#[global_allocator]`. The
//! probe therefore reports the signals it is given — either directly, via
//! [`AllocationProbe::record_alloc`] / [`AllocationProbe::record_dealloc`]
//! (deterministic, the path the testkit's own assertions use), or via the
//! `tracking-allocator` [`AllocationTracker`] callbacks if a host has installed a
//! global tracking allocator. In neither case does it panic or fabricate values:
//! with no signals it reports zeros, and its net-live figure saturates at zero
//! rather than underflowing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tracking_allocator::{AllocationGroupId, AllocationTracker};

/// Shared accumulator of allocation signals. Cheap to clone (it shares the same
/// atomics), so a probe can be both registered as a tracker and read for
/// assertions.
#[derive(Clone, Debug, Default)]
pub struct AllocationProbe {
    inner: Arc<ProbeCounters>,
}

#[derive(Debug, Default)]
struct ProbeCounters {
    alloc_calls: AtomicU64,
    dealloc_calls: AtomicU64,
    bytes_allocated: AtomicU64,
    bytes_deallocated: AtomicU64,
}

/// An immutable, deterministic snapshot of the probe's signals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocationSignals {
    pub alloc_calls: u64,
    pub dealloc_calls: u64,
    pub bytes_allocated: u64,
    pub bytes_deallocated: u64,
}

impl AllocationSignals {
    /// Net live bytes: allocated minus deallocated, saturating at zero. Never
    /// underflows into a nonsense huge value.
    #[must_use]
    pub fn net_live_bytes(&self) -> u64 {
        self.bytes_allocated.saturating_sub(self.bytes_deallocated)
    }
}

impl AllocationProbe {
    /// Create a fresh probe with zeroed counters. Installs nothing global.
    #[must_use]
    pub fn install() -> Self {
        Self::default()
    }

    /// Record an allocation signal deterministically (the path the testkit's own
    /// assertions use, independent of any global allocator).
    pub fn record_alloc(&self, bytes: u64) {
        self.inner.alloc_calls.fetch_add(1, Ordering::Relaxed);
        // Byte accumulators saturate rather than wrap (defence-in-depth on a byte
        // counter, per the workspace cheatsheet — a wrapped probe would under-report).
        let _ =
            self.inner.bytes_allocated.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_add(bytes))
            });
    }

    /// Record a deallocation signal deterministically.
    pub fn record_dealloc(&self, bytes: u64) {
        self.inner.dealloc_calls.fetch_add(1, Ordering::Relaxed);
        let _ =
            self.inner.bytes_deallocated.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_add(bytes))
            });
    }

    /// A deterministic snapshot of the accumulated signals.
    #[must_use]
    pub fn signals(&self) -> AllocationSignals {
        AllocationSignals {
            alloc_calls: self.inner.alloc_calls.load(Ordering::Relaxed),
            dealloc_calls: self.inner.dealloc_calls.load(Ordering::Relaxed),
            bytes_allocated: self.inner.bytes_allocated.load(Ordering::Relaxed),
            bytes_deallocated: self.inner.bytes_deallocated.load(Ordering::Relaxed),
        }
    }
}

/// Live wiring to `tracking-allocator`: when a host installs a global tracking
/// allocator and registers this probe, the allocator's callbacks accumulate into
/// the same atomics the deterministic API uses. The callbacks never allocate and
/// never panic.
impl AllocationTracker for AllocationProbe {
    fn allocated(
        &self,
        _addr: usize,
        object_size: usize,
        _wrapped_size: usize,
        _group_id: AllocationGroupId,
    ) {
        self.record_alloc(object_size as u64);
    }

    fn deallocated(
        &self,
        _addr: usize,
        object_size: usize,
        _wrapped_size: usize,
        _source_group_id: AllocationGroupId,
        _current_group_id: AllocationGroupId,
    ) {
        self.record_dealloc(object_size as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_probe_reports_zero_signals() {
        let probe = AllocationProbe::install();
        let signals = probe.signals();
        assert_eq!(signals, AllocationSignals::default());
        // NEGATIVE / nonsense guard: net live must be 0, not a huge underflow.
        assert_eq!(signals.net_live_bytes(), 0);
    }

    #[test]
    fn recorded_signals_accumulate_and_net_live_is_correct() {
        let probe = AllocationProbe::install();
        probe.record_alloc(1_000);
        probe.record_alloc(500);
        probe.record_dealloc(400);
        let s = probe.signals();
        assert_eq!(s.alloc_calls, 2);
        assert_eq!(s.dealloc_calls, 1);
        assert_eq!(s.bytes_allocated, 1_500);
        assert_eq!(s.bytes_deallocated, 400);
        assert_eq!(s.net_live_bytes(), 1_100);
    }

    #[test]
    fn dealloc_exceeding_alloc_saturates_at_zero_not_nonsense() {
        // NEGATIVE PATH 5: a probe that would underflow must not return nonsense.
        let probe = AllocationProbe::install();
        probe.record_alloc(100);
        probe.record_dealloc(999);
        let s = probe.signals();
        assert_eq!(s.net_live_bytes(), 0, "net live must saturate, never underflow to ~u64::MAX");
        assert_ne!(s.net_live_bytes(), u64::MAX - 898);
    }

    #[cfg(feature = "conformance")]
    #[test]
    fn signals_feed_an_invariant_assertion() {
        use crate::conformance::{ContractFamily, InvariantAssertion};

        // The probe must feed dev/test signals INTO assertions, not just exist.
        let probe = AllocationProbe::install();
        probe.record_alloc(2_048);
        probe.record_dealloc(2_048);
        let s = probe.signals();
        let assertion =
            InvariantAssertion::new(ContractFamily::AllocatorAdvisory, "balanced_alloc_dealloc");
        let verdict =
            assertion.assert_eq_bytes("alloc_vs_dealloc", s.bytes_allocated, s.bytes_deallocated);
        assert!(verdict.held(), "balanced alloc/dealloc must hold: {verdict}");

        // Negative pair: an imbalance must be VIOLATED with drift evidence.
        let probe2 = AllocationProbe::install();
        probe2.record_alloc(2_048);
        let s2 = probe2.signals();
        let v2 =
            assertion.assert_eq_bytes("alloc_vs_dealloc", s2.bytes_allocated, s2.bytes_deallocated);
        assert!(!v2.held());
        assert!(v2.evidence.contains("drift=2048"));
    }

    #[test]
    fn callback_and_direct_byte_totals_saturate_identically() {
        let direct = AllocationProbe::install();
        let callback = AllocationProbe::install();
        direct.record_alloc(u64::MAX);
        direct.record_dealloc(u64::MAX);
        callback.record_alloc(u64::MAX);
        callback.record_dealloc(u64::MAX);
        direct.record_alloc(1);
        direct.record_dealloc(1);
        callback.allocated(0, 1, 1, AllocationGroupId::ROOT);
        callback.deallocated(0, 1, 1, AllocationGroupId::ROOT, AllocationGroupId::ROOT);
        assert_eq!(direct.signals(), callback.signals());
        assert_eq!(
            callback.signals(),
            AllocationSignals {
                alloc_calls: 2,
                dealloc_calls: 2,
                bytes_allocated: u64::MAX,
                bytes_deallocated: u64::MAX,
            }
        );
    }

    #[test]
    fn tracker_callbacks_route_into_the_same_counters() {
        let probe = AllocationProbe::install();
        // Drive the AllocationTracker surface directly (as the global allocator
        // would) and prove it accumulates into the deterministic counters.
        probe.allocated(0xDEAD, 256, 320, AllocationGroupId::ROOT);
        probe.deallocated(0xDEAD, 256, 320, AllocationGroupId::ROOT, AllocationGroupId::ROOT);
        let s = probe.signals();
        assert_eq!(s.alloc_calls, 1);
        assert_eq!(s.dealloc_calls, 1);
        assert_eq!(s.bytes_allocated, 256);
        assert_eq!(s.bytes_deallocated, 256);
        assert_eq!(s.net_live_bytes(), 0);
    }
}
