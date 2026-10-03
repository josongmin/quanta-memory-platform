use super::*;

#[test]
fn charged_sweep_during_rebuild_is_fail_closed() {
    let broker = live_broker(1_000_000);
    let store = charged(&broker);
    store.insert_checked("a".to_string(), vec![0u8; 5]).expect("a");
    store.insert_checked("b".to_string(), vec![0u8; 5]).expect("b");
    store.insert_checked("c".to_string(), vec![0u8; 5]).expect("c");
    let rebuild_fence4 = broker.begin_rebuild().expect("begin rebuild");

    let denied = store.sweep(SweepHint { target_bytes: retained_cost("a", 5) + 1 });
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "sweep during rebuild must fail closed, got {denied:?}"
    );
    assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    assert_eq!(store.committed_bytes().expect("committed"), 3 * retained_cost("a", 5));
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        3 * retained_cost("a", 5)
    );

    // A failed refund_many must also leave internal insertion order intact.
    // Once the accounting gate reopens, the original newest entry is still
    // the first one reclaimed.
    assert!(rebuild_fence4.finish().expect("finish rebuild").drain_error.is_none());
    let swept =
        store.sweep(SweepHint { target_bytes: retained_cost("c", 5) }).expect("sweep after retry");
    assert_eq!(swept, SweepStats { reclaimed_items: 1, reclaimed_bytes: retained_cost("c", 5) });
    assert_eq!(store.get(&"c".to_string()), None, "c remains newest after denial");
    assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string()]);
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        2 * retained_cost("a", 5)
    );
}
#[test]
fn charged_sweep_rejects_a_ticketless_candidate_without_partial_refund_or_eviction() {
    let broker = live_broker(1_000_000);
    let store = charged(&broker);
    for key in ["a", "b", "c"] {
        store.insert_checked(key.to_string(), vec![0u8; 5]).expect("seed");
    }
    let held = store
        .state
        .lock()
        .expect("state")
        .entries
        .get_mut("b")
        .expect("entry")
        .charge
        .take()
        .expect("budgeted charge");
    // c is visited first and has a ticket, then b exposes corruption.
    // No refund_many call or keyed removal may happen before all selected
    // entries have proven their ticket invariant.
    let denied = store.sweep(SweepHint { target_bytes: retained_cost("a", 5) + 1 });
    assert!(matches!(denied, Err(MemoryError::InvalidState(_))), "got {denied:?}");
    let insertion_order =
        store.state.lock().expect("state").entries.keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        insertion_order,
        vec!["a".to_string(), "b".to_string(), "c".to_string()],
        "ticket validation failure must preserve canonical linked order"
    );
    assert_eq!(store.keys(), vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    assert_eq!(store.committed_bytes().expect("committed"), 3 * retained_cost("a", 5));
    assert_eq!(
        broker.ledger_snapshot().expect("snap").total_owner_bytes(),
        3 * retained_cost("a", 5)
    );
    store.state.lock().expect("state").entries.get_mut("b").expect("entry").charge = Some(held);
    store.close().expect("cleanup recovered charge");
    assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
}
#[test]
fn uncharged_store_is_explicit_non_budgeted_mode() {
    // (c) no sink: the store self-accounts and never touches a ledger.
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-map".to_string(), None);
    let store: UnbudgetedTestStore<String, Vec<u8>> =
        SimpleRetainedMapStore::new_unbudgeted_fixture(binding);
    store.insert_checked("k".to_string(), vec![0u8; 7]).expect("insert");
    assert_eq!(store.committed_bytes().expect("committed"), retained_cost("k", 7));
}
#[test]
fn remove_contains_panicking_key_and_returns_value_after_exact_refund() {
    use std::hash::{Hash, Hasher};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static PANIC_COUNT: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    struct Key {
        text: String,
        panic_on_drop: bool,
    }

    impl Clone for Key {
        fn clone(&self) -> Self {
            Self { text: self.text.clone(), panic_on_drop: false }
        }
    }

    impl PartialEq for Key {
        fn eq(&self, other: &Self) -> bool {
            self.text == other.text
        }
    }

    impl Eq for Key {}

    impl Hash for Key {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.text.hash(state);
        }
    }

    impl ByteSized for Key {
        fn byte_size(&self) -> u64 {
            self.text.len() as u64
        }
    }

    impl Drop for Key {
        fn drop(&mut self) {
            if self.panic_on_drop {
                PANIC_COUNT.fetch_add(1, Ordering::SeqCst);
                panic!("retired key drop trap");
            }
        }
    }

    #[derive(Clone, Debug)]
    struct Value;

    impl ByteSized for Value {
        fn byte_size(&self) -> u64 {
            1
        }
    }

    impl Drop for Value {
        fn drop(&mut self) {
            PANIC_COUNT.fetch_add(1, Ordering::SeqCst);
            panic!("returned value drop trap");
        }
    }

    const CHILD: &str = "QMP_RETAINED_REMOVE_DROP_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let broker = live_broker(1_000_000);
        let binding =
            StoreBindingHandle::new(MemoryClass::MemoResidency, "remove-drop-test".into(), None);
        let store =
            SimpleRetainedMapStore::<Key, Value, Budgeted>::new_budgeted(binding, broker.clone());
        store.insert_checked(Key { text: "key".into(), panic_on_drop: true }, Value).expect("seed");
        let query = Key { text: "key".into(), panic_on_drop: false };
        let returned = store.remove(&query).expect("remove").expect("returned value");
        assert_eq!(store.stats_checked().expect("postimage").item_count, 0);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
        retire_client_value(returned);
        assert_eq!(PANIC_COUNT.load(Ordering::SeqCst), 2, "both fields retired");
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("remove_contains_panicking_key_and_returns_value_after_exact_refund")
        .arg("--nocapture")
        .env(CHILD, "1")
        .require_one_test()
        .output()
        .expect("spawn child");
    assert!(
        output.status.success(),
        "child {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}
