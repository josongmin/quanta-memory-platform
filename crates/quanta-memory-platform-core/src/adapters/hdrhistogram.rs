//! `hdrhistogram` third-party binding.
//!
//! This is the ONLY place in the core crate that names `hdrhistogram` concrete
//! types. Higher layers see only the latency-recording API surface defined
//! here, never the third-party histogram itself.

#[cfg(feature = "telemetry")]
pub type LatencyHistogram = hdrhistogram::Histogram<u64>;

#[cfg(not(feature = "telemetry"))]
#[derive(Clone, Debug, Default)]
pub struct LatencyHistogram;

/// Records reclaim/admission latency samples behind a histogram. Advisory only:
/// it observes broker timing and never feeds semantic accounting.
#[cfg(feature = "telemetry")]
#[derive(Debug)]
pub struct LatencyRecorder {
    histogram: LatencyHistogram,
}

#[cfg(feature = "telemetry")]
impl LatencyRecorder {
    /// Create a recorder over the given value range. Returns a typed error
    /// instead of panicking if the range is invalid.
    pub fn new(low: u64, high: u64, sigfig: u8) -> Result<Self, String> {
        let histogram = hdrhistogram::Histogram::<u64>::new_with_bounds(low, high, sigfig)
            .map_err(|e| format!("invalid histogram bounds: {e}"))?;
        Ok(Self { histogram })
    }

    pub fn record(&mut self, value: u64) -> Result<(), String> {
        self.histogram.record(value).map_err(|e| format!("histogram record failed: {e}"))
    }

    #[must_use]
    pub fn percentile(&self, quantile: f64) -> u64 {
        self.histogram.value_at_quantile(quantile)
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        self.histogram.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.histogram.len() == 0
    }
}

#[cfg(not(feature = "telemetry"))]
#[derive(Clone, Debug, Default)]
pub struct LatencyRecorder;

#[cfg(not(feature = "telemetry"))]
impl LatencyRecorder {
    pub fn new(_low: u64, _high: u64, _sigfig: u8) -> Result<Self, String> {
        Ok(Self)
    }

    pub fn record(&mut self, _value: u64) -> Result<(), String> {
        Ok(())
    }

    #[must_use]
    pub fn percentile(&self, _quantile: f64) -> u64 {
        0
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        true
    }
}

#[cfg(all(test, feature = "telemetry"))]
mod tests {
    use super::*;

    // scenario L: a deterministic input set must read back correct counts and
    // percentiles. With 1..=100 recorded once each over a 3-sigfig histogram, the
    // p50 lands at the 50th value and p99 near the top of the range.
    #[test]
    fn records_deterministic_set_and_reads_back_count_and_percentiles() {
        let mut rec = LatencyRecorder::new(1, 1_000, 3).expect("1..1000 is a valid HDR range");
        assert!(rec.is_empty(), "a fresh recorder observes nothing");
        assert_eq!(rec.len(), 0);

        for v in 1..=100u64 {
            rec.record(v).expect("every value is inside the [1,1000] bounds");
        }

        assert_eq!(rec.len(), 100, "exactly the 100 samples we fed are counted");
        assert!(!rec.is_empty());

        // With 1..=100, the median value is 50 (3 sig figs => exact at this scale).
        let p50 = rec.percentile(0.50);
        assert!((49..=51).contains(&p50), "p50 of 1..=100 should be ~50, got {p50}");

        // The 99th percentile must sit at the high end of the observed range.
        let p99 = rec.percentile(0.99);
        assert!((98..=100).contains(&p99), "p99 of 1..=100 should be ~99, got {p99}");

        // The 100th percentile is the max observed sample.
        let p100 = rec.percentile(1.0);
        assert!((100..=101).contains(&p100), "p100 should be the max sample ~100, got {p100}");
    }

    // scenario L: recording is monotonic in count and never loses a sample; the
    // histogram only ever observes, so len() reflects exactly what was recorded.
    #[test]
    fn record_is_monotonic_and_lossless_in_count() {
        let mut rec = LatencyRecorder::new(1, 10_000, 3).expect("valid range");
        for (expected_len, v) in [(1u64, 5u64), (2, 5), (3, 9_999), (4, 1)] {
            rec.record(v).expect("value within bounds");
            assert_eq!(rec.len(), expected_len, "each record bumps the observed count by one");
        }
    }

    // scenario L: invalid histogram bounds surface a typed error string instead
    // of panicking; the recorder construction is fail-closed.
    #[test]
    fn invalid_bounds_return_error_not_panic() {
        // low must be >= 1 and < high for HDR; low > high is invalid.
        let err = LatencyRecorder::new(1_000, 1, 3);
        assert!(err.is_err(), "low > high must be rejected, not panic");
        let msg = err.expect_err("just asserted is_err");
        assert!(msg.contains("invalid histogram bounds"), "error preserves the source: {msg}");
    }

    // scenario L: recording a value outside the configured high bound is reported
    // as an error and does NOT silently corrupt the observed count.
    #[test]
    fn out_of_range_record_is_rejected_and_does_not_mutate_count() {
        let mut rec = LatencyRecorder::new(1, 100, 3).expect("valid range");
        rec.record(50).expect("in-range value records");
        let before = rec.len();
        // 5_000 exceeds the high bound of 100; hdrhistogram auto-resizing is off
        // for fixed bounds, so this must be reported.
        let result = rec.record(5_000);
        assert!(result.is_err(), "value above the high bound must be rejected");
        assert_eq!(rec.len(), before, "a rejected record must not change the observed count");
    }
}
