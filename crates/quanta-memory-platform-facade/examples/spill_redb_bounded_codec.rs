//! Public bounded-codec round trip. Run with a fresh database path:
//! `cargo run -p quanta-memory-platform-facade --features revocable_store,spill-redb --example spill_redb_bounded_codec -- /tmp/qmp-bounded-example.redb`

#[cfg(all(feature = "revocable_store", feature = "spill-redb"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use quanta_memory_platform_core::{MemoryClass, MemoryPreset, StoreBindingHandle};
    use quanta_memory_platform_facade::{
        MemoryPlatformBuilder, RedbSpillBacking, RevocableStore, RoutedSpillController,
    };

    let path = std::env::args().nth(1).ok_or("pass a fresh Redb database path")?;
    if std::path::Path::new(&path).exists() {
        return Err("bounded-codec example requires a fresh database path".into());
    }
    let platform = MemoryPlatformBuilder::new().with_preset(MemoryPreset::Development).build()?;
    let store = platform
        .revocable_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "example".into(), None))
        .build()?;
    store.insert_checked("key".into(), b"payload".to_vec())?;
    {
        let controller = RoutedSpillController::new(RedbSpillBacking::create(&path)?);
        assert!(store.spill_one_durable_bounded(&controller)?.is_some());
    }
    let controller = RoutedSpillController::new(RedbSpillBacking::create(&path)?);
    let _outcome = store.restore_one_durable_bounded(&controller)?;
    assert_eq!(store.reclaim_one()?, Some(("key".into(), b"payload".to_vec())));
    assert_eq!(controller.spilled_count()?, 0);
    Ok(())
}

#[cfg(not(all(feature = "revocable_store", feature = "spill-redb")))]
fn main() {
    eprintln!("enable revocable_store,spill-redb to run this example");
}
