impl ScratchReservation {
    pub fn grow_to_observed(&mut self, bytes: u64) -> Result<(), Error> {
        // ruleid: qmp-scratch-peak-monotonic
        self.observed_bytes = bytes;
        Ok(())
    }
    fn reset(&mut self) { self.observed_bytes = 0; }
}
