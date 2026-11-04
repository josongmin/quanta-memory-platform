use super::*;
use crate::shared::lifecycle::RetainedMapReadRefusalV73;

#[test]
fn paid_read_observes_canonical_count_before_key_hash_and_keeps_entry() {
    let map = store();
    map.insert_checked("kept".to_string(), vec![4, 7]).expect("insert");
    let mut observed = None;
    let result = map.get_checked_with_admission_v73(
        &"kept".to_string(),
        |count| {
            observed = Some(count);
            Ok::<(), ()>(())
        },
        |_| panic!("readable map must not construct refusal"),
    );
    assert_eq!(observed, Some(1));
    assert_eq!(result, Ok(Ok(Some(vec![4, 7]))));
    assert_eq!(map.get_checked(&"kept".to_string()), Ok(Some(vec![4, 7])));
}

#[test]
fn budget_refusal_precedes_key_hash_and_keeps_map_unchanged() {
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "paid-read-map".to_string(), None);
    let map =
        SimpleRetainedMapStore::<PanickingHashKey, Vec<u8>, TestUnbudgeted>::new_unbudgeted_fixture(
            binding,
        );
    let armed = Arc::new(AtomicBool::new(true));
    let key = PanickingHashKey { id: 8, armed: Arc::clone(&armed) };
    let result = map.get_checked_with_admission_v73(
        &key,
        |count| {
            assert_eq!(count, 0);
            Err::<(), _>("work refused")
        },
        |_| panic!("work refusal is not lifecycle refusal"),
    );
    assert_eq!(result, Ok(Err("work refused")));
    armed.store(false, Ordering::SeqCst);
    assert_eq!(map.get_checked(&key), Ok(None));
}

#[test]
fn busy_and_poisoned_reads_move_prepared_memory_errors_without_lookup() {
    let map = Arc::new(store());
    let held = map.state.lock().expect("hold owner lock");
    let mut prepared = Some(MemoryError::operation_in_progress("prepared busy"));
    let busy = map.get_checked_with_admission_v73(
        &"key".to_string(),
        |_| -> Result<(), ()> { panic!("busy read must not inspect entries") },
        |condition| {
            assert_eq!(
                condition,
                RetainedMapReadRefusalV73::OperationInProgress { resource: "retained map" }
            );
            prepared.take().expect("one prepared busy cause")
        },
    );
    assert_eq!(busy, Err(MemoryError::operation_in_progress("prepared busy")));
    drop(held);

    let peer = Arc::clone(&map);
    assert!(std::thread::spawn(move || {
        let _guard = peer.state.lock().expect("poison fixture lock");
        panic!("poison fixture");
    })
    .join()
    .is_err());
    let mut prepared = Some(MemoryError::poisoned("prepared poison".to_string()));
    let poisoned = map.get_checked_with_admission_v73(
        &"key".to_string(),
        |_| -> Result<(), ()> { panic!("poisoned read must not inspect entries") },
        |condition| {
            assert_eq!(condition, RetainedMapReadRefusalV73::Poisoned { resource: "retained map" });
            prepared.take().expect("one prepared poison cause")
        },
    );
    assert_eq!(poisoned, Err(MemoryError::poisoned("prepared poison".to_string())));
}

#[test]
fn paid_callback_panic_releases_read_lock_before_resuming() {
    let map = store();
    map.insert_checked("kept".to_string(), vec![9]).expect("insert");
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = map.get_checked_with_admission_v73(
            &"kept".to_string(),
            |_| -> Result<(), ()> { panic!("paid callback panic") },
            |_| panic!("not a lifecycle refusal"),
        );
    }));
    assert!(panic.is_err());
    assert_eq!(map.get_checked(&"kept".to_string()), Ok(Some(vec![9])));
}
