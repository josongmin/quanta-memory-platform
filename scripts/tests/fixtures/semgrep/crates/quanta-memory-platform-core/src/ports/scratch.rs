impl ScratchReservation {
    pub fn grow_to_observed(&mut self, bytes: u64) -> Result<(), Error> {
        // ruleid: qmp-scratch-peak-monotonic
        self.observed_bytes = bytes;
        self.observed_bytes = self.observed_bytes.max(bytes);
        Ok(())
    }
    fn constructor(&mut self) { self.observed_bytes = 0; }
}
