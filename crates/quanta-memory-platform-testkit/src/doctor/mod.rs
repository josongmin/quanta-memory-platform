#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryDoctorReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

impl MemoryDoctorReport {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}
