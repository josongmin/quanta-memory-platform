impl Store {
    fn with_accounting() -> Self {
        // ruleid: qmp-store-digest-seed-construction
        Self { digest_seed: shared_seed(), entries: Vec::new() }
    }
    fn with_accounting() -> Self {
        // ruleid: qmp-store-digest-seed-construction
        Self { digest_seed: BuildHasherDefault::default(), entries: Vec::new() }
    }
    fn with_accounting(entries: Entries) -> Self {
        // ruleid: qmp-store-digest-seed-construction
        Self { entries, digest_seed: shared_seed(), mode: Mode }
    }
    fn with_accounting() -> Self { Self { digest_seed: RandomState::new(), entries: Vec::new() } }
}
fn erased_cleanup(lease: Lease, controller: Controller, claim: Claim) {
    // ruleid: qmp-cleanup-result-not-erased
    lease.release().ok();
    // ruleid: qmp-cleanup-result-not-erased
    controller.abort_restore(&claim).ok();
}
fn observed_cleanup(lease: Lease) -> Result<(), Error> { lease.release()?; Ok(()) }
