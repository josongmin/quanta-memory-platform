//! Public consumer example for a broker-bound custom charge lifecycle.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p quanta-memory-platform-facade --features core-live-accounting --example managed_charge_lifecycle
//! ```

use quanta_memory_platform_core::{ChargeKind, MemoryClass, MemoryContextKind};
use quanta_memory_platform_facade::{MemoryPlatformBuilder, PortableMemoryPresets};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let platform =
        MemoryPlatformBuilder::new().with_manifest(PortableMemoryPresets::development()).build()?;
    let broker = platform.broker();

    let mut explicit = broker
        .reserve_owned(
            MemoryClass::ArtifactCache,
            "managed-charge-example".to_owned(),
            MemoryContextKind::Runtime,
            ChargeKind::RetainedPayload,
            64,
        )?
        .commit()?;
    assert_eq!(explicit.bytes(), Some(64));

    assert!(
        explicit
            .recharge(
                MemoryClass::ArtifactCache,
                "managed-charge-example".to_owned(),
                MemoryContextKind::Runtime,
                ChargeKind::RetainedPayload,
                96,
            )
            .is_ok(),
        "initial recharge should fit the development budget"
    );
    assert_eq!(explicit.bytes(), Some(96));

    let before_denial = broker.observe()?.ledger;
    let denied = explicit.recharge(
        MemoryClass::ArtifactCache,
        "managed-charge-example".to_owned(),
        MemoryContextKind::Runtime,
        ChargeKind::RetainedPayload,
        u64::MAX,
    );
    assert!(denied.is_err(), "an unrepresentable request must be denied");
    assert!(explicit.is_active(), "denial must preserve the old lease");
    assert_eq!(explicit.bytes(), Some(96), "denial must preserve old bytes");
    assert_eq!(broker.observe()?.ledger, before_denial, "denial must preserve ledger state");

    explicit.release()?;
    assert!(!explicit.is_active());
    drop(explicit);

    {
        let drop_released = broker
            .reserve_owned(
                MemoryClass::ArtifactCache,
                "managed-charge-example-drop".to_owned(),
                MemoryContextKind::Runtime,
                ChargeKind::RetainedPayload,
                32,
            )?
            .commit()?;
        assert_eq!(drop_released.bytes(), Some(32));
    }

    let final_ledger = broker.observe()?.ledger;
    assert_eq!(final_ledger.checked_total_owner_bytes()?, 0);
    assert_eq!(final_ledger.live_ticket_count, 0);
    println!("managed_charge_lifecycle=ok final_bytes=0 live_tickets=0");
    Ok(())
}
