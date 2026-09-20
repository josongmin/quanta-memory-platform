use quanta_memory_platform_facade::{MemoryPlatformBuilder, PortableMemoryPresets};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let report = MemoryPlatformBuilder::new()
        .with_manifest(PortableMemoryPresets::development())
        .build_report_only()?;

    println!("preset={:?}", report.preset);
    println!("global_hard_limit_bytes={}", report.residency.global_hard_limit_bytes);

    Ok(())
}
