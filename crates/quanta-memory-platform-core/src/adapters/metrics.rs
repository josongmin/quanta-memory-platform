use crate::ports::telemetry::{MemoryEvent, MemoryEventSink};

#[cfg(feature = "telemetry")]
#[derive(Clone, Debug, Default)]
pub struct MetricsEmitter;

#[cfg(feature = "telemetry")]
impl MemoryEventSink for MetricsEmitter {
    fn emit(&self, event: MemoryEvent) {
        metrics::counter!(
            "quanta_memory_operation_total",
            "operation" => event.operation.label(),
            "outcome" => event.outcome.label(),
        )
        .increment(1);
        if let Some(bytes) = event.bytes {
            metrics::histogram!(
                "quanta_memory_operation_bytes",
                "operation" => event.operation.label(),
                "outcome" => event.outcome.label(),
            )
            .record(bytes as f64);
        }
    }
}

#[cfg(not(feature = "telemetry"))]
#[derive(Clone, Debug, Default)]
pub struct MetricsEmitter;

#[cfg(not(feature = "telemetry"))]
impl MemoryEventSink for MetricsEmitter {
    fn emit(&self, _event: MemoryEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    // API smoke check; emission is observed by the recorder contract below.
    #[test]
    fn emitter_surface_is_emit_only_and_takes_shared_ref() {
        let emitter = MetricsEmitter;
        let by_ref: &MetricsEmitter = &emitter;
        let returned: () = by_ref.emit(MemoryEvent::new(
            crate::MemoryOperation::ObserveBroker,
            crate::MemoryOutcome::Succeeded,
            Some(1_024),
        ));
        assert_eq!(returned, (), "emit is observation-only: it yields nothing to fold back");
        by_ref.emit(MemoryEvent::new(
            crate::MemoryOperation::Pressure,
            crate::MemoryOutcome::Succeeded,
            Some(2_048),
        ));
    }

    // Construction does not require a broker or ledger handle.
    #[test]
    fn emitter_holds_no_accounting_state() {
        let emitter = MetricsEmitter;
        let cloned = emitter.clone();
        // Both can emit independently with shared refs; neither carries or shares
        // any mutable accounting counter.
        emitter.emit(MemoryEvent::new(
            crate::MemoryOperation::Reserve,
            crate::MemoryOutcome::Succeeded,
            Some(1),
        ));
        cloned.emit(MemoryEvent::new(
            crate::MemoryOperation::Release,
            crate::MemoryOutcome::Succeeded,
            Some(2),
        ));
    }
}

#[cfg(all(test, feature = "telemetry"))]
mod recording_tests;
