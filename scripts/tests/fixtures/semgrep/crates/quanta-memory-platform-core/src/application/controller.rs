mod public_bypass {
    // ruleid: qmp-threshold-fields-private
    pub struct PressureThresholds { pub soft_percent: u8, hard_percent: u8, fatal_percent: u8 }
}
mod crate_bypass {
    // ruleid: qmp-threshold-fields-private
    pub struct PressureThresholds { soft_percent: u8, pub(crate) hard_percent: u8, fatal_percent: u8 }
}
mod validated {
    pub struct PressureThresholds { soft_percent: u8, hard_percent: u8, fatal_percent: u8 }
}
mod parent_bypass {
    // ruleid: qmp-threshold-fields-private
    pub struct PressureThresholds { pub(super) soft_percent: u8 }
}
mod path_bypass {
    // ruleid: qmp-threshold-fields-private
    pub struct PressureThresholds { pub(in crate::application) soft_percent: u8 }
}
