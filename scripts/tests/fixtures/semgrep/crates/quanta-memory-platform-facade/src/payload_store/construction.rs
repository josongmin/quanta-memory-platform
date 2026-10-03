impl Store {
    fn with_accounting() -> Self {
        // ruleid: qmp-store-digest-seed-construction
        Self { digest_seed: shared_seed(), entries: Vec::new() }
    }
    fn with_accounting() -> Self { Self { digest_seed: RandomState::new(), entries: Vec::new() } }
}
