//! Library-only "hello world" for the memory platform.
//!
//! This is the canonical builder-first happy path for a consumer that wants the
//! portable memory platform WITHOUT the Semantica host adapter. It uses only the
//! recommended public surface:
//!
//! - [`PortableMemoryPresets`] to pick a budget manifest, and
//! - [`MemoryPlatformBuilder`] to build an enforced platform.
//!
//! It deliberately does NOT touch any low-level broker backdoor
//! (`ProcessMemoryBrokerBuilder`, `LiveProcessMemoryBroker`, or raw ledger
//! construction). The regression guard treats those as raw-constructor residue.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p quanta-memory-platform-facade --example hello_world
//! ```

use quanta_memory_platform_core::{
    MemoryClass, MemoryContextKind, MemoryError, ProcessMemoryBroker, RuntimeKind, ScratchRequest,
};
use quanta_memory_platform_facade::{MemoryPlatformBuilder, PortableMemoryPresets};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Pick a budget manifest from a named preset (no hand-rolled numbers).
    let manifest = PortableMemoryPresets::development();

    let mut manifest = manifest;
    manifest.execution.max_execution_scratch_bytes = Some(64);
    let scratch_budget = manifest
        .residency
        .classes
        .get_mut(&MemoryClass::ExecutionScratch)
        .expect("preset defines the mandatory scratch class");
    scratch_budget.soft_cap_bytes = 64;
    scratch_budget.hard_cap_bytes = 64;

    // 2. The default builder returns the live accounting authority.
    let platform = MemoryPlatformBuilder::new().with_manifest(manifest).build()?;

    // 3. Runtime scratch shares the platform ledger and cap.
    let runtime =
        platform.attach_runtime("hello-world-runtime".to_string(), RuntimeKind::Runtime)?;
    let context = runtime.context(MemoryContextKind::Request, "hello-world-owner".to_string());
    let mut reservation = runtime
        .scratch()
        .reserve_scratch_scope(&context, ScratchRequest::new("working-set", 48))?;
    assert_eq!(platform.broker().ledger_snapshot()?.total_live_scratch_bytes(), 48);

    let denied =
        runtime.scratch().reserve_scratch_scope(&context, ScratchRequest::new("over-cap", 17));
    assert!(matches!(denied, Err(MemoryError::Capacity(_))));
    assert_eq!(platform.broker().ledger_snapshot()?.total_live_scratch_bytes(), 48);

    reservation.release()?;
    assert_eq!(platform.broker().ledger_snapshot()?.total_owner_bytes(), 0);
    let report = platform.effective_report();

    println!("preset={:?}", report.preset);
    println!("runtime_kind={:?}", runtime.runtime_kind());
    println!("global_hard_limit_bytes={}", report.residency.global_hard_limit_bytes);
    println!("scratch_cap_enforced=true");

    Ok(())
}
