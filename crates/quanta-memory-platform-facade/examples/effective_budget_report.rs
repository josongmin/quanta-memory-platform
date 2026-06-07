use quanta_memory_platform_core::{ProcessMemoryBroker, RuntimeKind};
use quanta_memory_platform_facade::{MemoryPlatformBuilder, PortableMemoryPresets};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let broker =
        MemoryPlatformBuilder::new().with_manifest(PortableMemoryPresets::development()).build()?;
    let runtime = broker.attach_runtime("example-runtime".to_string(), RuntimeKind::Runtime)?;
    let report = broker.effective_report();

    println!("preset={:?}", report.preset);
    println!("runtime_kind={:?}", runtime.runtime_kind());
    println!("global_hard_limit_bytes={}", report.residency.global_hard_limit_bytes);

    Ok(())
}
