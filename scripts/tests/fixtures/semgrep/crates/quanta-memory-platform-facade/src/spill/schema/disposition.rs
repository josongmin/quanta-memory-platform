impl RedbSpillBacking {
    fn resolve_record(&self, id: u64) -> Result<State, Error> {
        // ruleid: qmp-durable-no-watermark-resolution
        let watermark = self.meta.get(FINALIZED_THROUGH)?;
        self.state.get(id)
    }
    fn active_record(&self, id: u64) -> Result<State, Error> { self.state.get(id) }
}
