//! Library-only "hello world" for the memory platform.
//!
//! This is the canonical builder-first happy path for a consumer that wants the
//! portable memory platform WITHOUT the Semantica host adapter. It uses only the
//! recommended public surface:
//!
//! - [`PortableMemoryPresets`] to pick a budget manifest, and
//! - [`MemoryPlatformBuilder`] to build a broker.
//!
//! It deliberately does NOT touch any low-level broker backdoor
//! (`ProcessMemoryBrokerBuilder`, `LiveProcessMemoryBroker`, direct
//! `BasicProcessMemoryBroker` construction). The `memory-platform-doctor` rail
//! treats those as raw-constructor residue.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p quanta-memory-platform-facade --example hello_world
//! ```

use quanta_memory_platform_core::{ProcessMemoryBroker, RuntimeKind};
use quanta_memory_platform_facade::{MemoryPlatformBuilder, PortableMemoryPresets};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Pick a budget manifest from a named preset (no hand-rolled numbers).
    let manifest = PortableMemoryPresets::development();

    // 2. Build a broker with the builder-first public API.
    let broker = MemoryPlatformBuilder::new().with_manifest(manifest).build()?;

    // 3. Attach a runtime and read the effective budget report.
    let runtime = broker.attach_runtime("hello-world-runtime".to_string(), RuntimeKind::Runtime)?;
    let report = broker.effective_report();

    println!("preset={:?}", report.preset);
    println!("runtime_kind={:?}", runtime.runtime_kind());
    println!("global_hard_limit_bytes={}", report.residency.global_hard_limit_bytes);

    Ok(())
}
