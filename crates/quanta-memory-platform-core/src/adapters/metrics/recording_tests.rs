use super::*;
use metrics::{
    Counter, CounterFn, Gauge, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder,
    SharedString, Unit,
};
use std::sync::{Arc, Mutex};

#[derive(Debug, PartialEq, Eq)]
enum Measurement {
    Increment(Key, u64),
    Record(Key, u64), // f64 bit pattern: exact recording without float equality.
}

type Measurements = Arc<Mutex<Vec<Measurement>>>;

#[derive(Default)]
struct CaptureRecorder(Measurements);

struct CaptureHandle {
    key: Key,
    measurements: Measurements,
}

impl CounterFn for CaptureHandle {
    fn increment(&self, value: u64) {
        self.measurements
            .lock()
            .expect("test recorder")
            .push(Measurement::Increment(self.key.clone(), value));
    }
    fn absolute(&self, _value: u64) {
        panic!("emitter must increment, not set an absolute counter");
    }
}

impl HistogramFn for CaptureHandle {
    fn record(&self, value: f64) {
        self.measurements
            .lock()
            .expect("test recorder")
            .push(Measurement::Record(self.key.clone(), value.to_bits()));
    }
}

impl Recorder for CaptureRecorder {
    fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
        Counter::from_arc(Arc::new(CaptureHandle {
            key: key.clone(),
            measurements: Arc::clone(&self.0),
        }))
    }
    fn register_gauge(&self, _: &Key, _: &Metadata<'_>) -> Gauge {
        panic!("emitter must not register a gauge");
    }
    fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
        Histogram::from_arc(Arc::new(CaptureHandle {
            key: key.clone(),
            measurements: Arc::clone(&self.0),
        }))
    }
}

fn key(name: &'static str, operation: &'static str, outcome: &'static str) -> Key {
    Key::from_parts(
        name,
        vec![metrics::Label::new("operation", operation), metrics::Label::new("outcome", outcome)],
    )
}

#[test]
fn emitter_records_exact_counter_labels_and_optional_byte_samples() {
    use crate::{MemoryOperation, MemoryOutcome};
    let recorder = CaptureRecorder::default();
    // Thread-local recorder avoids global registration and parallel-test interference.
    metrics::with_local_recorder(&recorder, || {
        for _ in 0..2 {
            MetricsEmitter.emit(MemoryEvent::new(
                MemoryOperation::Reserve,
                MemoryOutcome::Succeeded,
                Some(128),
            ));
        }
        MetricsEmitter.emit(MemoryEvent::new(
            MemoryOperation::Release,
            MemoryOutcome::CapacityDenied,
            None,
        ));
        MetricsEmitter.emit(MemoryEvent::new(
            MemoryOperation::Pressure,
            MemoryOutcome::Succeeded,
            Some(0),
        ));
    });
    let counter = "quanta_memory_operation_total";
    let histogram = "quanta_memory_operation_bytes";
    assert_eq!(
        *recorder.0.lock().expect("captured observations"),
        vec![
            Measurement::Increment(key(counter, "reserve", "succeeded"), 1),
            Measurement::Record(key(histogram, "reserve", "succeeded"), 128_f64.to_bits()),
            Measurement::Increment(key(counter, "reserve", "succeeded"), 1),
            Measurement::Record(key(histogram, "reserve", "succeeded"), 128_f64.to_bits()),
            Measurement::Increment(key(counter, "release", "capacity_denied"), 1),
            Measurement::Increment(key(counter, "pressure", "succeeded"), 1),
            Measurement::Record(key(histogram, "pressure", "succeeded"), 0_f64.to_bits()),
        ]
    );
}
