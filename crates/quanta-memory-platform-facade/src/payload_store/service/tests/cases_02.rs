use super::*;

#[test]
fn delete_drop_reentry_is_bounded_and_peer_publication_retains_exact_charge() {
    const CHILD: &str = "QMP_PAYLOAD_DELETE_DROP_REENTRY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("delete_drop_reentry_is_bounded_and_peer_publication_retains_exact_charge")
            .env(CHILD, "1")
            .require_one_test()
            .spawn()
            .expect("spawn bounded reentry test");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().expect("poll child") {
                assert!(status.success(), "reentry child failed: {status}");
                return;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().expect("terminate deadlocked child");
                child.wait().expect("reap child");
                panic!("reentry child exceeded bounded deadline");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    let broker = live_broker(1_000_000, 600_000);
    let home = Arc::new(
        DropReentryPayloadStore::new_budgeted(binding(), None, broker.clone())
            .expect("payload store nonce"),
    );
    let peer = Arc::new(
        DropReentryPayloadStore::new_budgeted(
            StoreBindingHandle::new(MemoryClass::ArtifactCache, "drop-peer".into(), None),
            None,
            broker.clone(),
        )
        .expect("payload store nonce"),
    );
    let home_slot = Arc::new(OnceLock::new());
    let peer_slot = Arc::new(OnceLock::new());
    home_slot.set(Arc::downgrade(&home)).expect("install home");
    peer_slot.set(Arc::downgrade(&peer)).expect("install peer");
    let hits = Arc::new(AtomicUsize::new(0));
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let published = home
        .put_canonical(RetainedEnvelope::new(DropReentryBody {
            id: 1,
            armed: Arc::clone(&armed),
            home: Arc::clone(&home_slot),
            peer: Arc::clone(&peer_slot),
            hits: Arc::clone(&hits),
        }))
        .expect("seed");
    assert_eq!(
        broker.ledger_snapshot().expect("seed ledger").total_owner_bytes(),
        retained_cost(1, 0, home.store_nonce)
    );
    armed.store(true, Ordering::SeqCst);
    home.delete_if_unpinned(&published.payload_ref).expect("delete and retire body");

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(home.stats().expect("home postimage").item_count, 0);
    assert_eq!(peer.stats().expect("peer postimage").item_count, 1);
    assert_eq!(
        broker.ledger_snapshot().expect("peer ledger").total_owner_bytes(),
        retained_cost(1, 0, peer.store_nonce)
    );
    home.close().expect("close home");
    peer.close().expect("close peer");
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
}
#[test]
fn delete_contains_two_panicking_client_destructors_after_exact_refund() {
    use std::sync::atomic::AtomicBool;

    static PANIC_REF: AtomicBool = AtomicBool::new(false);
    static PANIC_BODY: AtomicBool = AtomicBool::new(false);
    static PANIC_COUNT: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    struct Ref(String);

    impl Clone for Ref {
        fn clone(&self) -> Self {
            Self(self.0.clone())
        }
    }

    impl From<String> for Ref {
        fn from(value: String) -> Self {
            Self(value)
        }
    }

    impl PartialEq for Ref {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0
        }
    }

    impl Eq for Ref {}

    impl Hash for Ref {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.0.hash(state);
        }
    }

    impl ByteSized for Ref {
        fn byte_size(&self) -> u64 {
            self.0.len() as u64
        }
    }

    impl Drop for Ref {
        fn drop(&mut self) {
            if PANIC_REF.load(Ordering::SeqCst) {
                PANIC_COUNT.fetch_add(1, Ordering::SeqCst);
                panic!("reference drop trap");
            }
        }
    }

    #[derive(Clone, Debug)]
    struct Body(Vec<u8>);

    impl ByteSized for Body {
        fn byte_size(&self) -> u64 {
            self.0.len() as u64
        }
    }

    impl CanonicalKey for Body {
        fn canonical_key(&self) -> Vec<u8> {
            self.0.clone()
        }
    }

    impl Drop for Body {
        fn drop(&mut self) {
            if PANIC_BODY.load(Ordering::SeqCst) {
                PANIC_COUNT.fetch_add(1, Ordering::SeqCst);
                panic!("body drop trap");
            }
        }
    }

    const CHILD: &str = "QMP_PAYLOAD_DELETE_DROP_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let broker = live_broker(1_000_000, 600_000);
        let store = SimplePayloadStore::<Ref, Body, Budgeted>::new_budgeted(
            binding(),
            None,
            broker.clone(),
        )
        .expect("payload store nonce");
        let published =
            store.put_canonical(RetainedEnvelope::new(Body(vec![1, 2, 3]))).expect("seed");
        PANIC_REF.store(true, Ordering::SeqCst);
        PANIC_BODY.store(true, Ordering::SeqCst);
        store.delete_if_unpinned(&published.payload_ref).expect("delete");
        PANIC_REF.store(false, Ordering::SeqCst);
        PANIC_BODY.store(false, Ordering::SeqCst);
        assert_eq!(PANIC_COUNT.load(Ordering::SeqCst), 2, "both fields retired");
        assert_eq!(store.stats().expect("postimage").item_count, 0);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("delete_contains_two_panicking_client_destructors_after_exact_refund")
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
