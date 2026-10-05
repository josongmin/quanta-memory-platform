//! A retained-map overwrite admitted after a different store releases its lease.
//!
//! The callback holds only a `Weak` reference to the revocable victim, so
//! platform -> callback -> store ownership does not form a strong cycle. The
//! callback consumes the typed reclaim unit/scope and applies a finite work
//! budget if the broker cannot represent an exact reduction.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p quanta-memory-platform-facade --features core-live-accounting,retained_map,revocable_store --example shared_budget_reclaim
//! ```

use std::sync::{Arc, Mutex, OnceLock, Weak};

use quanta_memory_platform_core::{
    MemoryClass, MemoryClassBudget, MemoryError, MemoryReclaimer, MemoryResult, ReclaimOutcome,
    ReclaimPriority, ReclaimRequest, ReclaimScope, ReclaimUnit, ResidencyPolicy,
    StoreBindingHandle, SweepStats,
};
use quanta_memory_platform_facade::{
    Budgeted, LiveMemoryPlatform, MemoryPlatformBuilder, PortableMemoryPresets, RetainedMapStore,
    RevocableStore, SimpleRevocableStore,
};

type Revocable = SimpleRevocableStore<String, Vec<u8>, Budgeted>;

const MAP_OWNER: &str = "example:retained-map";
const VICTIM_OWNER: &str = "example:revocable-victim";
const RECLAIM_WORK_BUDGET_BYTES: u64 = 64 * 1024;

struct CrossStoreReclaimer {
    victim: OnceLock<Weak<Revocable>>,
    last_sweep: Mutex<Option<SweepStats>>,
}

impl MemoryReclaimer for CrossStoreReclaimer {
    fn reclaim(&self, request: &ReclaimRequest) -> MemoryResult<ReclaimOutcome> {
        if request.target.unit != ReclaimUnit::Bytes
            || !matches!(&request.target.scope, ReclaimScope::Global | ReclaimScope::NonHeadroom)
        {
            return Err(MemoryError::config(
                "example victim is eligible only for global byte pressure",
            ));
        }

        let victim = self.victim.get().and_then(Weak::upgrade).ok_or_else(|| {
            MemoryError::InvalidState("revocable victim is no longer live".into())
        })?;
        let target = request
            .target
            .minimum_reduction
            .unwrap_or(RECLAIM_WORK_BUDGET_BYTES)
            .min(RECLAIM_WORK_BUDGET_BYTES);
        let swept = victim.reclaim_until(target)?;
        *self.last_sweep.lock().expect("sweep result mutex") = Some(swept);

        // Deliberately under-report. The broker must remeasure the shared
        // ledger after callback return; this number is advisory, not authority.
        Ok(ReclaimOutcome { reclaimed_bytes: 0 })
    }
}

fn class_budget(cap: u64) -> MemoryClassBudget {
    MemoryClassBudget {
        reserved_bytes: 0,
        soft_cap_bytes: cap,
        hard_cap_bytes: cap,
        borrow_from_shared_pool: false,
        reclaim_priority: ReclaimPriority::Normal,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The portable development preset supplies the rest of the manifest; this
    // example narrows the global limit to make the cross-store interaction
    // deterministic while leaving both per-class limits non-binding.
    let mut manifest = PortableMemoryPresets::development();
    manifest.residency = ResidencyPolicy::new(100)
        .with_class_budget(MemoryClass::MemoResidency, class_budget(1_000))
        .with_class_budget(MemoryClass::ExecutionScratchRevocable, class_budget(1_000));

    let reclaimer =
        Arc::new(CrossStoreReclaimer { victim: OnceLock::new(), last_sweep: Mutex::new(None) });
    let platform: LiveMemoryPlatform = MemoryPlatformBuilder::new()
        .with_manifest(manifest)
        .with_reclaimer(Arc::clone(&reclaimer) as Arc<dyn MemoryReclaimer>)
        .build()?;

    let map = platform
        .retained_map_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::MemoResidency, MAP_OWNER.into(), None))
        .build();
    let victim = Arc::new(
        platform
            .revocable_store_builder::<String, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ExecutionScratchRevocable,
                VICTIM_OWNER.into(),
                None,
            ))
            .build()?,
    );
    assert!(reclaimer.victim.set(Arc::downgrade(&victim)).is_ok());

    // Independent semantic-cost expectations: map entry = 4 + 1 + 24 + 16
    // = 45 bytes; revocable entry = 4 + 1 + 16 + 16 = 37 bytes.
    map.insert_checked("m".into(), vec![1; 4])?;
    victim.insert_checked("v".into(), vec![9; 4])?;
    let before = platform.broker().ledger_snapshot()?;
    assert_eq!(before.total_owner_bytes(), 82);
    assert_eq!(before.live_ticket_count, 2);

    // Replacing 45 bytes with 85 would make the global postimage 122 bytes.
    // The broker asks for a 22-byte reduction; the other store releases its
    // 37-byte lease, then the broker remeasures and admits the 85-byte image.
    map.insert_checked("m".into(), vec![7; 44])?;
    assert_eq!(map.get_checked(&"m".into())?, Some(vec![7; 44]));
    assert_eq!(map.committed_bytes()?, 85);
    assert_eq!(victim.committed_bytes()?, 0);
    let sweep_guard = reclaimer.last_sweep.lock().expect("sweep result mutex");
    let sweep = sweep_guard.as_ref().expect("reclaim callback ran");
    assert_eq!(sweep.reclaimed_items, 1);
    assert_eq!(sweep.reclaimed_bytes, 37);
    drop(sweep_guard);

    let admitted = platform.broker().ledger_snapshot()?;
    assert!(admitted.class_owner_balanced());
    assert!(admitted.context_owner_balanced());
    assert!(admitted.owner_partition_balanced());
    assert_eq!(admitted.total_owner_bytes(), 85);
    assert_eq!(admitted.total_class_bytes(), 85);
    assert_eq!(admitted.total_context_bytes(), 85);
    assert_eq!(admitted.live_ticket_count, 1);
    assert_eq!(admitted.class_usage[&MemoryClass::MemoResidency].retained_metadata_bytes, 85);
    assert_eq!(admitted.owner_usage[MAP_OWNER].retained_metadata_bytes, 85);
    assert!(!admitted.owner_usage.contains_key(VICTIM_OWNER));

    map.close()?;
    victim.close()?;
    let drained = platform.broker().ledger_snapshot()?;
    assert!(drained.class_owner_balanced());
    assert!(drained.context_owner_balanced());
    assert!(drained.owner_partition_balanced());
    assert_eq!(drained.total_owner_bytes(), 0);
    assert_eq!(drained.live_ticket_count, 0);
    assert!(drained.class_usage.is_empty());
    assert!(drained.owner_usage.is_empty());
    assert!(drained.context_usage.is_empty());

    let weak_victim = Arc::downgrade(&victim);
    drop(victim);
    assert!(weak_victim.upgrade().is_none(), "callback must not pin victim store");

    println!("shared_budget_reclaim=ok reclaimed_bytes=37 final_bytes=0 live_tickets=0");
    Ok(())
}
