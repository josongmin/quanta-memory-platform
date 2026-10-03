use super::*;

#[test]
fn get_missing_key_is_none() {
    let store = store();
    assert_eq!(store.get(&"absent".to_string()), None);
}
#[test]
fn keys_lists_inserted_keys_in_deterministic_sorted_order() {
    let store = store();
    // Insert OUT of sorted order to prove enumeration sorts, not echoes
    // insertion order.
    store.insert_checked("c".to_string(), vec![0u8; 1]).expect("c");
    store.insert_checked("a".to_string(), vec![0u8; 1]).expect("a");
    store.insert_checked("b".to_string(), vec![0u8; 1]).expect("b");
    assert_eq!(
        store.keys(),
        vec!["a".to_string(), "b".to_string(), "c".to_string()],
        "keys must be returned in deterministic key-sorted order"
    );
}
#[test]
fn entries_lists_pairs_in_same_sorted_order_as_keys() {
    let store = store();
    store.insert_checked("z".to_string(), vec![9u8]).expect("z");
    store.insert_checked("m".to_string(), vec![7u8, 7]).expect("m");
    let entries = store.entries();
    assert_eq!(
        entries,
        vec![("m".to_string(), vec![7u8, 7]), ("z".to_string(), vec![9u8])],
        "entries must be key-sorted and carry the stored value"
    );
    // The entry key order matches keys().
    let entry_keys: Vec<String> = entries.into_iter().map(|(k, _)| k).collect();
    assert_eq!(entry_keys, store.keys());
}
#[test]
fn keys_after_remove_drops_the_removed_key() {
    let store = store();
    store.insert_checked("a".to_string(), vec![0u8]).expect("a");
    store.insert_checked("b".to_string(), vec![0u8]).expect("b");
    assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string()]);
    store.remove(&"a".to_string()).expect("remove a");
    // NEGATIVE-PAIR: a removed key must no longer enumerate.
    assert_eq!(store.keys(), vec!["b".to_string()]);
}
#[test]
fn keys_on_empty_store_is_empty() {
    let store = store();
    assert!(store.keys().is_empty(), "empty store must enumerate no keys");
    assert!(store.entries().is_empty(), "empty store must enumerate no entries");
}
#[cfg(feature = "core-live-accounting")]
#[test]
fn uncharged_retained_map_bytes_are_not_reflected_in_a_broker_ledger_snapshot() {
    // scenario F#3 (explicit non-budgeted mode): a SimpleRetainedMapStore built
    // WITHOUT `charged_against` self-accounts; its committed bytes live only in
    // the store's own counter and are invisible to any broker ledger snapshot.
    // This is the intentional non-budgeted path, kept distinct from the charged
    // path below.
    use quanta_memory_platform_core::{
        ExecutionBudget, MemoryBudgetManifest, MemoryPreset, ResidencyPolicy,
    };

    let broker = quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            ResidencyPolicy::new(1_000_000),
        ))
        .build()
        .expect("valid broker");

    // No `charged_against`: explicit non-budgeted mode.
    let store = store();
    store.insert_checked("k".to_string(), vec![0u8; 4096]).expect("insert");

    // The store sees its own bytes...
    assert_eq!(store.committed_bytes().expect("store bytes"), retained_cost("k", 4096));
    // ...but the broker ledger (the SSOT) sees none of them: an uncharged store
    // never wires its bytes into the ledger.
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        0,
        "an uncharged retained map must not route bytes through the broker ledger"
    );
}
#[test]
fn typed_value_round_trips_and_accounts_its_own_byte_size() {
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-typed".to_string(), None);
    let store: UnbudgetedTestStore<String, TypedRecord> =
        SimpleRetainedMapStore::new_unbudgeted_fixture(binding);
    let record = TypedRecord { tag: 7, payload: vec![1u8, 2, 3, 4, 5] };
    store.insert_checked("rec".to_string(), record.clone()).expect("insert typed");
    // get returns the exact typed value (no codec needed).
    assert_eq!(store.get(&"rec".to_string()), Some(record));
    // The value's declared bytes and retained key/index are both charged.
    assert_eq!(store.committed_bytes().expect("committed"), retained_cost("rec", 9));
    assert_eq!(
        store.stats(),
        StoreStats { item_count: 1, estimated_bytes: retained_cost("rec", 9) }
    );
}
#[test]
fn measured_any_handle_round_trips_and_accounts_caller_size() {
    use crate::shared::measure::MeasuredValue;
    use std::any::Any;
    use std::sync::Arc;

    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-handle".to_string(), None);
    let store: UnbudgetedTestStore<String, MeasuredValue<Arc<dyn Any + Send + Sync>>> =
        SimpleRetainedMapStore::new_unbudgeted_fixture(binding);

    let handle: Arc<dyn Any + Send + Sync> = Arc::new(vec![10u32, 20, 30]);
    // Caller computes the size accounting explicitly (4096 bytes here).
    store
        .insert_checked("h".to_string(), MeasuredValue::new(handle.clone(), 4096))
        .expect("insert handle");

    // get returns the same Arc; downcast proves identity of the carried value.
    let got = store.get(&"h".to_string()).expect("handle present");
    assert_eq!(got.size(), 4096, "wrapper carries the caller size");
    // Same Arc allocation (resume-state / whole-program-authority identity):
    // the retrieved erased Arc points at the original allocation.
    assert!(Arc::ptr_eq(&handle, got.value()), "retrieved handle must be the same Arc");
    let downcast = got.value().clone().downcast::<Vec<u32>>().expect("downcast to original");
    assert_eq!(*downcast, vec![10u32, 20, 30]);

    // Broker accounting reflects the caller-supplied size, not Arc::size_of.
    assert_eq!(store.committed_bytes().expect("committed"), retained_cost("h", 4096));
    assert_eq!(
        store.stats(),
        StoreStats { item_count: 1, estimated_bytes: retained_cost("h", 4096) }
    );

    // NEGATIVE-PAIR: removing the handle releases the caller-supplied charge.
    let removed = store.remove(&"h".to_string()).expect("remove handle").expect("handle present");
    assert_eq!(removed.size(), 4096);
    assert_eq!(store.committed_bytes().expect("released"), 0);
    assert!(store.get(&"h".to_string()).is_none(), "removed handle must not be retrievable");
}
