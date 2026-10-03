impl UsageBreakdown {
    fn total(&self, extra: u64) -> u64 {
        // ruleid: qmp-exact-ledger-no-saturation
        self.bytes.saturating_add(extra)
    }
}
fn advisory_difference(a: u64, b: u64) -> u64 { a.saturating_sub(b) }
