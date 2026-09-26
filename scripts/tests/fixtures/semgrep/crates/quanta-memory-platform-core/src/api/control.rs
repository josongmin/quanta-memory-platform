impl Broker {
    pub fn observe_allocator(&self, snapshot: &AllocatorSnapshot) -> Result<Advisory, Error> {
        // ruleid: qmp-allocator-observation-no-pressure-write
        self.controller.update_from_semantic(snapshot.resident_bytes.unwrap_or(0));
        self.emit(snapshot);
        Ok(Advisory::compare(snapshot, self.ledger.total()))
    }
    pub fn classify_pressure(&self) -> State { self.controller.update_from_semantic(self.ledger.total()) }
}
