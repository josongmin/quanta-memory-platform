impl AccountingLedger {
    fn preflight_release_batch(&self, tickets: &[Ticket]) -> Result<(), Error> {
        // ruleid: qmp-release-preflight-no-owner-scan
        let _ = self.owner_usage.iter();
        let _ = self.owner_usage.get(&tickets[0].owner);
        Ok(())
    }
    fn report(&self) { let _ = self.owner_usage.iter(); }
}
