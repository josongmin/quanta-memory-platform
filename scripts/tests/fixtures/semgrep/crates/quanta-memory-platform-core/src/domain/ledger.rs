struct UsageBreakdown { bytes: u64 }
impl UsageBreakdown {
    fn bad_add(&self, bytes: u64) -> u64 {
        // ruleid: qmp-exact-ledger-no-saturation
        self.bytes.saturating_add(bytes)
    }
    fn bad_sub(&self, bytes: u64) -> u64 {
        // ruleid: qmp-exact-ledger-no-saturation
        self.bytes.saturating_sub(bytes)
    }
    fn exact(&self, bytes: u64) -> Option<u64> { self.bytes.checked_add(bytes) }
}
impl LedgerSnapshot {
    fn bad_total(&self) -> u64 {
        // ruleid: qmp-exact-ledger-no-saturation
        self.bytes.iter().copied().fold(0, u64::saturating_add)
    }
}
impl AccountingLedger {
    fn preflight_release_batch(&self, tickets: &[Ticket]) -> Result<(), Error> {
        // ruleid: qmp-release-preflight-no-owner-scan
        let _owners = self.owner_usage.values();
        // ruleid: qmp-release-preflight-no-owner-scan
        let _snapshot = self.snapshot();
        // ruleid: qmp-release-preflight-no-owner-scan
        for owner in &self.owner_usage { consume(owner); }
        let _keyed = self.owner_usage.get(&tickets[0].owner);
        Ok(())
    }
    fn report(&self) { let _all = self.owner_usage.values(); }
}
fn advisory(a: u64, b: u64) -> u64 { a.saturating_sub(b) }
fn signed_magnitude(bytes: u64) -> i64 {
    // ruleid: qmp-magnitude-as-signed
    bytes as i64
}
fn saturated_ratio(a: u64, b: u64, c: u64) -> u64 {
    // ruleid: qmp-saturating-mul-then-divide
    a.saturating_mul(b) / c
}
fn wrapped_ratio(a: u64, b: u64, c: u64) -> u64 {
    // ruleid: qmp-wrapping-mul-then-divide
    a.wrapping_mul(b) / c
}
fn exact_ratio(a: u64, b: u64, c: u64) -> u128 { u128::from(a) * u128::from(b) / u128::from(c) }
fn random_seed(seed: u64) -> u64 { seed.wrapping_mul(97) }
fn underflow(counter: &AtomicU64) {
    // ruleid: qmp-atomic-fetch-sub
    counter.fetch_sub(1, Ordering::SeqCst);
}
fn saturated_counter(counter: &AtomicU64) {
    counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| Some(v.saturating_sub(1)));
}
