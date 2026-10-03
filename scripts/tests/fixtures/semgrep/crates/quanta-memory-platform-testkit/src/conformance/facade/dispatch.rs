mod missing_spill {
    // ruleid: qmp-facade-suite-required-slices
    pub fn run_facade_store_suite(kit: &Kit) -> Vec<Verdict> {
        let mut verdicts = Vec::new();
        verdicts.extend(run_payload_store_slice(kit));
        verdicts.extend(run_retained_map_slice(kit));
        verdicts.extend(run_transcript_slice(kit));
        verdicts
    }
}
mod complete {
    pub fn run_facade_store_suite(kit: &Kit) -> Vec<Verdict> {
        let mut verdicts = Vec::new();
        verdicts.extend(run_payload_store_slice(kit));
        verdicts.extend(run_retained_map_slice(kit));
        verdicts.extend(run_transcript_slice(kit));
        verdicts.extend(run_spill_slice());
        verdicts
    }
}
