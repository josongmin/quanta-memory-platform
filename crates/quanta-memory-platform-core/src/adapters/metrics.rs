#[cfg(feature = "telemetry")]
#[derive(Clone, Debug, Default)]
pub struct MetricsEmitter;

#[cfg(feature = "telemetry")]
impl MetricsEmitter {
    pub fn emit_gauge(&self, name: &str, value: f64) {
        metrics::gauge!(name.to_owned()).set(value);
    }
}

#[cfg(not(feature = "telemetry"))]
#[derive(Clone, Debug, Default)]
pub struct MetricsEmitter;

#[cfg(not(feature = "telemetry"))]
impl MetricsEmitter {
    pub fn emit_gauge(&self, _name: &str, _value: f64) {}
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
        // The function returns the unit type — assigning to () compiles only if
        // emit_gauge yields no counter the caller could feed back into the ledger.
        let returned: () = by_ref.emit_gauge("memory.platform.resident_bytes", 1_024.0);
        assert_eq!(returned, (), "emit is observation-only: it yields nothing to fold back");
        by_ref.emit_gauge("memory.platform.resident_bytes", 2_048.0);
        by_ref.emit_gauge("memory.platform.pressure", 0.0);
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
        emitter.emit_gauge("a", 1.0);
        cloned.emit_gauge("b", 2.0);
    }
}
