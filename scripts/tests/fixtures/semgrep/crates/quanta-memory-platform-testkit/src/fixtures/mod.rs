impl BrokenKit {
    pub fn run_conformance(&self) -> ConformanceVerdict {
        // ruleid: qmp-kit-no-constant-pass
        ConformanceVerdict::Passed
    }
}
impl HonestKit {
    pub fn run_conformance(&self) -> ConformanceVerdict {
        let mut broker = self.fake_broker();
        ConformanceSuite::new().run_broker_accounting(&mut broker)
    }
}
#[cfg(test)]
mod tests { fn fixture() -> ConformanceVerdict { ConformanceVerdict::Passed } }
