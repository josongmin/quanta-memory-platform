//! Builder-first public paid-read surface and actual retained value identity.

#[cfg(feature = "retained_map")]
#[test]
fn retained_map_paid_read_public_surface_uses_the_built_store() {
    use std::sync::Arc;

    use quanta_memory_platform_core::StoreBindingHandle;
    use quanta_memory_platform_facade::RetainedMapReadRefusalV73;

    use quanta_memory_platform_core::{MemoryClass, MemoryPreset};
    use quanta_memory_platform_facade::{
        Budgeted, MemoryPlatformBuilder, RetainedMapStore, SimpleRetainedMapStore,
    };

    let platform = MemoryPlatformBuilder::new()
        .with_preset(MemoryPreset::Production)
        .build()
        .expect("valid platform");
    let binding =
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-paid-read".to_string(), None);
    let store: SimpleRetainedMapStore<String, Arc<Vec<u8>>, Budgeted> =
        platform.retained_map_store_builder::<String, Arc<Vec<u8>>>().bind(binding).build();
    let retained = Arc::new(vec![4, 1]);
    store.insert_checked("k".to_string(), Arc::clone(&retained)).expect("insert");
    let selected = store
        .get_checked_with_admission_v73(
            &"k".to_string(),
            |entries| {
                assert_eq!(entries, 1);
                Ok::<(), ()>(())
            },
            |_: RetainedMapReadRefusalV73| panic!("open store is readable"),
        )
        .expect("lifecycle read")
        .expect("paid lookup")
        .expect("stored entry");
    assert!(Arc::ptr_eq(&selected, &retained));
}
