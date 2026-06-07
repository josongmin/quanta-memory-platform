use quanta_memory_platform_core::{MemoryBudgetManifest, MemoryPreset};

#[derive(Clone, Debug, Default)]
pub struct PortableMemoryPresets;

impl PortableMemoryPresets {
    pub fn development() -> MemoryBudgetManifest {
        MemoryBudgetManifest::for_preset(MemoryPreset::Development)
    }

    pub fn ci() -> MemoryBudgetManifest {
        MemoryBudgetManifest::for_preset(MemoryPreset::Ci)
    }

    pub fn production() -> MemoryBudgetManifest {
        MemoryBudgetManifest::for_preset(MemoryPreset::Production)
    }

    pub fn interactive() -> MemoryBudgetManifest {
        MemoryBudgetManifest::for_preset(MemoryPreset::Interactive)
    }
}
