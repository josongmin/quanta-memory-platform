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
        // nosemgrep: qmp-durable-no-watermark-resolution -- reason: obsolete fixture metadata proves recovery ignores the legacy watermark.
        meta.insert(FINALIZED_THROUGH, 10).unwrap();
        // ruleid: qmp-durable-no-watermark-resolution
        let watermark = meta.get(FINALIZED_THROUGH).unwrap();
    }
}
mod production_named_tests {
    mod tests {
        fn resolve(meta: &Table) {
            // ruleid: qmp-durable-no-watermark-resolution
            meta.get(FINALIZED_THROUGH);
        }
    }
}
