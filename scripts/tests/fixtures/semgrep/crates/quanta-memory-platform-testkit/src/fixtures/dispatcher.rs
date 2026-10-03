impl BrokenKit {
    pub fn run_conformance(&self) -> ConformanceVerdict {
        // ruleid: qmp-kit-no-constant-pass
        ConformanceVerdict::Passed
    }
}
impl HonestKit {
    pub fn run_conformance(&self) -> ConformanceVerdict { ConformanceSuite::new().run_broker_accounting(&mut self.fake_broker()) }
}
