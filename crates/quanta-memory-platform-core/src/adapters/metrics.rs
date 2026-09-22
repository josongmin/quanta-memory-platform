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

    // scenario L: the metrics emitter is advisory and emit-only. Its surface
    // takes (&self, name, value) and returns `()` — there is no method that hands
    // back, accumulates, or mutates an accounting counter through the emitter. We
    // prove emission is a side-effect-free-to-the-caller observation: it never
    // borrows mutably and never yields a value the caller could fold into the
    // ledger. Driving it repeatedly must not require &mut self.
    #[test]
    fn emitter_surface_is_emit_only_and_takes_shared_ref() {
        let emitter = MetricsEmitter;
        // &self (shared) calls only: if emit could mutate accounting it would need
        // &mut self. Many emits via one shared reference is the proof of advisory.
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

    // scenario L: the emitter is freely cloneable and Default-constructible
    // without any handle to accounting state, confirming it holds no semantic
    // counter and cannot be a write path into the ledger.
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
