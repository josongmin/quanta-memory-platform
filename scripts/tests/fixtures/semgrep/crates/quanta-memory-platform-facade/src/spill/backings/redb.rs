impl RedbSpillBacking {
    fn resolve_record(&self, id: u64) -> Result<State, Error> {
        // ruleid: qmp-durable-no-watermark-resolution
        let watermark = self.meta.get(FINALIZED_THROUGH)?;
        self.state.get(id)
    }
}
#[cfg(test)]
mod tests {
    fn legacy_fixture(meta: &mut Table) {
        meta.insert(FINALIZED_THROUGH, 10).unwrap();
        assert!(meta.get(FINALIZED_THROUGH).unwrap().is_some());
    }
}
