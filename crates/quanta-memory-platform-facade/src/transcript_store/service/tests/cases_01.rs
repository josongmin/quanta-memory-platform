use super::*;

#[test]
fn reentrant_load_stats_and_close_under_read_lock_are_typed_busy() {
    let store = store();
    store.append(request("s", 0, "m", "body")).expect("seed row");
    let held = store.state.lock().expect("simulated client clone holds read lock");
    assert!(matches!(store.load(&"s".into()), Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
    drop(held);
    assert_eq!(store.load(&"s".into()).expect("complete row").len(), 1);
    store.close().expect("retry close");
}
#[test]
fn partial_snapshot_clone_and_drop_panics_cannot_abort() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    const CHILD: &str = "QMP_TRANSCRIPT_PARTIAL_SNAPSHOT_CHILD";
    struct HostileBody {
        bytes: Vec<u8>,
        calls: Arc<AtomicUsize>,
        drop_panics: Arc<AtomicBool>,
    }
    impl Clone for HostileBody {
        fn clone(&self) -> Self {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            assert_ne!(call, 2, "second body clone trap");
            Self {
                bytes: self.bytes.clone(),
                calls: Arc::clone(&self.calls),
                drop_panics: Arc::clone(&self.drop_panics),
            }
        }
    }
    impl ByteSized for HostileBody {
        fn byte_size(&self) -> u64 {
            self.bytes.len() as u64
        }
    }
    impl CanonicalKey for HostileBody {
        fn canonical_key(&self) -> Vec<u8> {
            self.bytes.clone()
        }
    }
    impl Drop for HostileBody {
        fn drop(&mut self) {
            assert!(!self.drop_panics.load(Ordering::SeqCst), "partial body drop trap");
        }
    }
    if std::env::var_os(CHILD).is_some() {
        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let store =
            SimpleSessionTranscriptStore::<String, String, HostileBody, Budgeted>::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "owner:partial-snapshot".into(),
                    None,
                ),
                TranscriptRetentionPolicyV2::default(),
                broker.clone(),
            );
        let calls = Arc::new(AtomicUsize::new(0));
        let drop_panics = Arc::new(AtomicBool::new(false));
        for sequence in 0..2 {
            store
                .append(TranscriptAppendRequestV1::new(
                    TranscriptAppendIdentityV1::new(
                        "s".to_string(),
                        sequence,
                        format!("m{sequence}"),
                    ),
                    HostileBody {
                        bytes: vec![sequence as u8],
                        calls: Arc::clone(&calls),
                        drop_panics: Arc::clone(&drop_panics),
                    },
                ))
                .expect("seed row");
        }
        drop_panics.store(true, Ordering::SeqCst);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.load_versioned_snapshot(&"s".to_string())
        }));
        assert!(caught.is_err(), "later Clone must panic");
        drop_panics.store(false, Ordering::SeqCst);
        assert_eq!(store.checked_stats().expect("usable after unwind").item_count, 2);
        store.close().expect("close");
        assert_eq!(broker.ledger_snapshot().expect("drained").live_ticket_count, 0);
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("transcript_store::service::tests::cases_01::partial_snapshot_clone_and_drop_panics_cannot_abort")
            .env(CHILD, "1")
            .require_one_test()
.output()
            .expect("isolated snapshot child");
    assert!(
        output.status.success(),
        "partial snapshot must not abort: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn partial_load_clone_and_drop_panics_cannot_abort() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    const CHILD: &str = "QMP_TRANSCRIPT_PARTIAL_LOAD_CHILD";
    struct HostileBody {
        bytes: Vec<u8>,
        calls: Arc<AtomicUsize>,
        drop_panics: Arc<AtomicBool>,
    }
    impl Clone for HostileBody {
        fn clone(&self) -> Self {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            assert_ne!(call, 2, "second body clone trap");
            Self {
                bytes: self.bytes.clone(),
                calls: Arc::clone(&self.calls),
                drop_panics: Arc::clone(&self.drop_panics),
            }
        }
    }
    impl ByteSized for HostileBody {
        fn byte_size(&self) -> u64 {
            self.bytes.len() as u64
        }
    }
    impl CanonicalKey for HostileBody {
        fn canonical_key(&self) -> Vec<u8> {
            self.bytes.clone()
        }
    }
    impl Drop for HostileBody {
        fn drop(&mut self) {
            assert!(!self.drop_panics.load(Ordering::SeqCst), "partial body drop trap");
        }
    }
    if std::env::var_os(CHILD).is_some() {
        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let store =
            SimpleSessionTranscriptStore::<String, String, HostileBody, Budgeted>::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "owner:partial-load".into(),
                    None,
                ),
                TranscriptRetentionPolicyV2::default(),
                broker.clone(),
            );
        let calls = Arc::new(AtomicUsize::new(0));
        let drop_panics = Arc::new(AtomicBool::new(false));
        for sequence in 0..2 {
            store
                .append(TranscriptAppendRequestV1::new(
                    TranscriptAppendIdentityV1::new(
                        "s".to_string(),
                        sequence,
                        format!("m{sequence}"),
                    ),
                    HostileBody {
                        bytes: vec![sequence as u8],
                        calls: Arc::clone(&calls),
                        drop_panics: Arc::clone(&drop_panics),
                    },
                ))
                .expect("seed row");
        }
        drop_panics.store(true, Ordering::SeqCst);
        let caught =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.load(&"s".to_string())));
        assert!(caught.is_err(), "later Clone must panic");
        drop_panics.store(false, Ordering::SeqCst);
        assert_eq!(store.checked_stats().expect("usable after unwind").item_count, 2);
        store.close().expect("close");
        assert_eq!(broker.ledger_snapshot().expect("drained").live_ticket_count, 0);
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("--exact")
        .arg(
            "transcript_store::service::tests::cases_01::partial_load_clone_and_drop_panics_cannot_abort",
        )
        .env(CHILD, "1")
        .require_one_test()
        .output()
        .expect("isolated load child");
    assert!(
        output.status.success(),
        "partial load must not abort: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn new_session_clone_and_drop_panics_cannot_abort() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    use std::sync::atomic::{AtomicBool, Ordering};
    const CHILD: &str = "QMP_TRANSCRIPT_NEW_SESSION_CHILD";
    #[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
    struct HostileSession(String);
    impl Clone for HostileSession {
        fn clone(&self) -> Self {
            panic!("new session key clone trap")
        }
    }
    impl ByteSized for HostileSession {
        fn byte_size(&self) -> u64 {
            self.0.len() as u64
        }
    }
    struct HostileBody {
        drop_panics: Arc<AtomicBool>,
    }
    impl Clone for HostileBody {
        fn clone(&self) -> Self {
            Self { drop_panics: Arc::clone(&self.drop_panics) }
        }
    }
    impl ByteSized for HostileBody {
        fn byte_size(&self) -> u64 {
            1
        }
    }
    impl CanonicalKey for HostileBody {
        fn canonical_key(&self) -> Vec<u8> {
            vec![1]
        }
    }
    impl Drop for HostileBody {
        fn drop(&mut self) {
            assert!(!self.drop_panics.load(Ordering::SeqCst), "body drop trap");
        }
    }
    if std::env::var_os(CHILD).is_some() {
        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let store = SimpleSessionTranscriptStore::<HostileSession, String, HostileBody, Budgeted>::new_budgeted(
            StoreBindingHandle::new(MemoryClass::SessionTranscript, "owner:new-session".into(), None),
            TranscriptRetentionPolicyV2::default(), broker.clone());
        let drop_panics = Arc::new(AtomicBool::new(true));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(HostileSession("s".into()), 0, "m0".into()),
                HostileBody { drop_panics: Arc::clone(&drop_panics) },
            ))
        }));
        assert!(caught.is_err(), "session Clone must panic");
        drop_panics.store(false, Ordering::SeqCst);
        assert_eq!(store.checked_stats().expect("usable after unwind").item_count, 0);
        store.close().expect("close");
        assert_eq!(broker.ledger_snapshot().expect("drained").live_ticket_count, 0);
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("--exact")
        .arg("transcript_store::service::tests::cases_01::new_session_clone_and_drop_panics_cannot_abort")
        .env(CHILD, "1")
        .require_one_test()
        .output()
        .expect("isolated append child");
    assert!(
        output.status.success(),
        "new session append must not abort: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn owned_session_key_order_and_drop_panics_cannot_abort() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    const CHILD: &str = "QMP_TRANSCRIPT_OWNED_SESSION_KEY_CHILD";
    #[derive(Clone)]
    struct HostileSession {
        name: &'static str,
        comparisons: Arc<AtomicUsize>,
        panic_at: Arc<AtomicUsize>,
        drop_panics: Arc<AtomicBool>,
    }
    impl ByteSized for HostileSession {
        fn byte_size(&self) -> u64 {
            self.name.len() as u64
        }
    }
    impl PartialEq for HostileSession {
        fn eq(&self, other: &Self) -> bool {
            self.name == other.name
        }
    }
    impl Eq for HostileSession {}
    impl PartialOrd for HostileSession {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for HostileSession {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            let call = self.comparisons.fetch_add(1, Ordering::SeqCst) + 1;
            assert_ne!(call, self.panic_at.load(Ordering::SeqCst), "owned key Ord trap");
            self.name.cmp(other.name)
        }
    }
    impl Drop for HostileSession {
        fn drop(&mut self) {
            assert!(!self.drop_panics.load(Ordering::SeqCst), "owned key Drop trap");
        }
    }
    if std::env::var_os(CHILD).is_some() {
        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let store =
            SimpleSessionTranscriptStore::<HostileSession, String, String, Budgeted>::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "owner:owned-session-key".into(),
                    None,
                ),
                TranscriptRetentionPolicyV2::default(),
                broker.clone(),
            );
        let comparisons = Arc::new(AtomicUsize::new(0));
        let panic_at = Arc::new(AtomicUsize::new(0));
        let drop_panics = Arc::new(AtomicBool::new(false));
        let key = |name| HostileSession {
            name,
            comparisons: Arc::clone(&comparisons),
            panic_at: Arc::clone(&panic_at),
            drop_panics: Arc::clone(&drop_panics),
        };
        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(key("old"), 0, "m0".into()),
                "old body".to_string(),
            ))
            .expect("seed old session");
        let before = broker.ledger_snapshot().expect("before");
        panic_at.store(comparisons.load(Ordering::SeqCst) + 3, Ordering::SeqCst);
        drop_panics.store(true, Ordering::SeqCst);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(key("new"), 0, "m1".into()),
                "new body".to_string(),
            ))
        }));
        assert!(caught.is_err(), "owned-key entry comparison must panic");
        drop_panics.store(false, Ordering::SeqCst);
        assert_eq!(store.checked_stats().expect("usable after unwind").item_count, 1);
        assert_eq!(broker.ledger_snapshot().expect("after"), before);
        store.close().expect("close");
        assert_eq!(broker.ledger_snapshot().expect("drained").live_ticket_count, 0);
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("transcript_store::service::tests::cases_01::owned_session_key_order_and_drop_panics_cannot_abort")
            .env(CHILD, "1").require_one_test()
.output().expect("isolated owned-key child");
    assert!(
        output.status.success(),
        "owned session key must not abort: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn partial_append_identity_clone_and_drop_panics_cannot_abort() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    const CHILD: &str = "QMP_TRANSCRIPT_PARTIAL_APPEND_CHILD";
    #[derive(Debug)]
    struct HostileId {
        name: String,
        calls: Arc<AtomicUsize>,
        drop_panics: Arc<AtomicBool>,
    }
    impl Clone for HostileId {
        fn clone(&self) -> Self {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            assert_ne!(call, 2, "second identity clone trap");
            Self {
                name: self.name.clone(),
                calls: Arc::clone(&self.calls),
                drop_panics: Arc::clone(&self.drop_panics),
            }
        }
    }
    impl ByteSized for HostileId {
        fn byte_size(&self) -> u64 {
            self.name.len() as u64
        }
    }
    impl PartialEq for HostileId {
        fn eq(&self, other: &Self) -> bool {
            self.name == other.name
        }
    }
    impl Eq for HostileId {}
    impl PartialOrd for HostileId {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for HostileId {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.name.cmp(&other.name)
        }
    }
    impl Drop for HostileId {
        fn drop(&mut self) {
            assert!(!self.drop_panics.load(Ordering::SeqCst), "partial identity drop trap");
        }
    }
    if std::env::var_os(CHILD).is_some() {
        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let store =
            SimpleSessionTranscriptStore::<String, HostileId, String, Budgeted>::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "owner:partial-append".into(),
                    None,
                ),
                TranscriptRetentionPolicyV2::default(),
                broker.clone(),
            );
        let calls = Arc::new(AtomicUsize::new(0));
        let drop_panics = Arc::new(AtomicBool::new(true));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(
                    "s".to_string(),
                    0,
                    HostileId {
                        name: "m0".into(),
                        calls: Arc::clone(&calls),
                        drop_panics: Arc::clone(&drop_panics),
                    },
                ),
                "body".to_string(),
            ))
        }));
        assert!(caught.is_err(), "later identity Clone must panic");
        drop_panics.store(false, Ordering::SeqCst);
        assert_eq!(store.checked_stats().expect("usable after unwind").item_count, 0);
        store.close().expect("close");
        assert_eq!(broker.ledger_snapshot().expect("drained").live_ticket_count, 0);
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("transcript_store::service::tests::cases_01::partial_append_identity_clone_and_drop_panics_cannot_abort")
            .env(CHILD, "1")
            .require_one_test()
.output()
            .expect("isolated append child");
    assert!(
        output.status.success(),
        "partial append must not abort: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn client_message_clone_during_load_reenters_as_typed_busy_without_ledger_drift() {
    use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{OnceLock, Weak};

    type ReentrantStore = SimpleSessionTranscriptStore<String, String, ReentrantBody, Budgeted>;

    struct ReentrantBody {
        bytes: Vec<u8>,
        armed: Arc<AtomicBool>,
        busy_callbacks: Arc<AtomicUsize>,
        owner: Arc<OnceLock<Weak<ReentrantStore>>>,
    }

    impl Clone for ReentrantBody {
        fn clone(&self) -> Self {
            if self.armed.load(Ordering::SeqCst) {
                let store = self.owner.get().expect("owner installed").upgrade().expect("live");
                assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
                assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
                self.busy_callbacks.fetch_add(1, Ordering::SeqCst);
            }
            Self {
                bytes: self.bytes.clone(),
                armed: Arc::clone(&self.armed),
                busy_callbacks: Arc::clone(&self.busy_callbacks),
                owner: Arc::clone(&self.owner),
            }
        }
    }

    impl ByteSized for ReentrantBody {
        fn byte_size(&self) -> u64 {
            self.bytes.len() as u64
        }
    }

    impl CanonicalKey for ReentrantBody {
        fn canonical_key(&self) -> Vec<u8> {
            self.bytes.clone()
        }
    }

    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("broker");
    let store = Arc::new(ReentrantStore::new_budgeted(
        StoreBindingHandle::new(MemoryClass::SessionTranscript, "clone-owner".into(), None),
        TranscriptRetentionPolicyV2::default(),
        broker.clone(),
    ));
    let armed = Arc::new(AtomicBool::new(false));
    let busy_callbacks = Arc::new(AtomicUsize::new(0));
    let owner = Arc::new(OnceLock::new());
    owner.set(Arc::downgrade(&store)).expect("one owner");
    store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new("s".into(), 0, "m".into()),
            ReentrantBody {
                bytes: vec![1, 2, 3],
                armed: Arc::clone(&armed),
                busy_callbacks: Arc::clone(&busy_callbacks),
                owner,
            },
        ))
        .expect("append");
    armed.store(true, Ordering::SeqCst);
    assert_eq!(store.load(&"s".into()).expect("outer load").len(), 1);
    armed.store(false, Ordering::SeqCst);
    assert_eq!(busy_callbacks.load(Ordering::SeqCst), 1);
    assert_eq!(store.stats().expect("postimage").estimated_bytes, cost("s", &[("m", "one", true)]));
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        cost("s", &[("m", "one", true)])
    );
    store.close().expect("close after callback");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn exact_request_retried_n_times_applies_once() {
    let store = store();
    let outcomes: Vec<_> =
        (0..8).map(|_| store.append(request("s", 0, "m0", "body")).expect("append")).collect();
    assert_eq!(
        outcomes.iter().filter(|d| **d == TranscriptAppendDispositionV1::Applied).count(),
        1
    );
    assert_eq!(
        outcomes.iter().filter(|d| **d == TranscriptAppendDispositionV1::Replayed).count(),
        7
    );
    assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
    assert_eq!(store.retained_bytes().expect("bytes"), cost("s", &[("m0", "body", true)]));
}
#[test]
fn append_truncate_cycles_cannot_grow_past_fail_closed_evidence_bound() {
    let store = store_with_policy(TranscriptRetentionPolicyV2::fail_closed(2, u64::MAX));
    for sequence in 0..2 {
        store
            .append(request("s", sequence, &format!("m{sequence}"), "body"))
            .expect("inside horizon");
        store.truncate_to(&"s".to_string(), 0).expect("truncate active body");
    }
    let before = store.retention_stats().expect("bounded stats");
    let error = store.append(request("s", 2, "m2", "body")).expect_err("saturated");
    assert!(matches!(
        error,
        TranscriptAppendErrorV1::RetentionSaturated {
            evidence_count: 2,
            max_evidence_count: 2,
            ..
        }
    ));
    assert_eq!(store.retention_stats().expect("unchanged stats"), before);
    assert!(store.load(&"s".to_string()).expect("no active rows").is_empty());
}
#[test]
fn exact_retry_and_conflicts_remain_deterministic_at_saturation() {
    let store = store_with_policy(TranscriptRetentionPolicyV2::fail_closed(1, u64::MAX));
    store.append(request("s", 7, "m0", "body")).expect("seed");
    store.truncate_to(&"s".to_string(), 0).expect("truncate");
    let before = store.retention_stats().expect("before retry");
    assert_eq!(
        store.append(request("s", 7, "m0", "body")).expect("exact replay"),
        TranscriptAppendDispositionV1::Replayed
    );
    assert!(matches!(
        store.append(request("s", 7, "m1", "body")),
        Err(TranscriptAppendErrorV1::SequenceConflict { sequence: 7 })
    ));
    assert!(matches!(
        store.append(request("s", 8, "m0", "body")),
        Err(TranscriptAppendErrorV1::MessageIdConflict)
    ));
    assert_eq!(store.retention_stats().expect("after retry"), before);
}
#[test]
fn tombstone_byte_limit_counts_large_message_ids() {
    let first_evidence = identity_evidence_cost(2).expect("oracle setup");
    let store =
        store_with_policy(TranscriptRetentionPolicyV2::fail_closed(usize::MAX, first_evidence));
    store.append(request("s", 0, "m0", "body")).expect("fits exact byte cap");
    let before = store.retention_stats().expect("before saturation");
    let error = store
        .append(request("s", 1, "message-id-that-exceeds-the-cap", "body"))
        .expect_err("large identity must exceed byte horizon");
    assert!(matches!(
        error,
        TranscriptAppendErrorV1::RetentionSaturated {
            evidence_count: 1,
            evidence_bytes,
            max_evidence_bytes,
            ..
        } if evidence_bytes == first_evidence && max_evidence_bytes == first_evidence
    ));
    assert_eq!(store.retention_stats().expect("unchanged stats"), before);
}
#[test]
fn retention_reconfigure_below_live_evidence_is_rejected() {
    let store = store();
    store.append(request("s", 0, "m0", "body")).expect("seed");
    let live = store.retention_stats().expect("live stats");
    let original = store.retention_policy().expect("original policy");
    let error = store
        .try_reconfigure_retention(TranscriptRetentionPolicyV2::fail_closed(0, live.evidence_bytes))
        .expect_err("count downgrade below live evidence");
    assert!(matches!(
        error,
        TranscriptRetentionReconfigureErrorV2::BelowLiveEvidence {
            live_evidence_count: 1,
            requested_max_evidence_count: 0,
            ..
        }
    ));
    assert_eq!(store.retention_policy().expect("policy unchanged"), original);

    let exact_live = TranscriptRetentionPolicyV2::fail_closed(1, live.evidence_bytes);
    store.try_reconfigure_retention(exact_live).expect("exact live bound is valid");
    assert_eq!(store.retention_policy().expect("updated policy"), exact_live);
    assert!(matches!(
        store.append(request("s", 1, "m1", "body")),
        Err(TranscriptAppendErrorV1::RetentionSaturated { .. })
    ));
}
#[test]
fn retention_stats_separate_active_evidence_and_session_costs() {
    let store = store();
    store.append(request("s", 0, "m0", "body")).expect("seed");
    let (expected_active, expected_evidence, expected_session, expected_total) =
        expected_partition_costs("s", "m0", "body");
    assert_eq!(
        (expected_active, expected_evidence, expected_session, expected_total),
        (46, 148, 41, 235)
    );
    let active = store.retention_stats().expect("active stats");
    assert_eq!(active.active_row_count, 1);
    assert_eq!(active.evidence_count, 1);
    assert_eq!(active.active_bytes, expected_active);
    assert_eq!(active.evidence_bytes, expected_evidence);
    assert_eq!(active.session_key_bytes, expected_session);
    assert_eq!(active.total_retained_bytes, expected_total);
    assert_eq!(
        active.total_retained_bytes,
        active.active_bytes + active.evidence_bytes + active.session_key_bytes
    );

    store.truncate_to(&"s".to_string(), 0).expect("truncate body");
    let truncated = store.retention_stats().expect("truncated stats");
    assert_eq!(truncated.active_row_count, 0);
    assert_eq!(truncated.active_bytes, 0);
    assert_eq!(truncated.evidence_count, 1);
    assert_eq!(truncated.evidence_bytes, expected_evidence);
    assert_eq!(truncated.evidence_bytes, 148);
    assert_eq!(truncated.session_key_bytes, expected_session);
    assert_eq!(truncated.session_key_bytes, 41);
    assert_eq!(truncated.total_retained_bytes, 189);
    assert_eq!(
        truncated.total_retained_bytes,
        truncated.active_bytes + truncated.evidence_bytes + truncated.session_key_bytes
    );
}
#[test]
fn same_identity_with_body_drift_is_payload_conflict() {
    let store = store();
    store.append(request("s", 0, "m0", "old")).expect("seed");
    let err = store.append(request("s", 0, "m0", "new")).expect_err("conflict");
    assert!(matches!(err, TranscriptAppendErrorV1::PayloadConflict { sequence: 0 }));
    assert_eq!(store.load(&"s".to_string()).expect("load")[0].body(), "old");
}
#[test]
fn reused_sequence_with_different_message_id_is_typed_conflict() {
    let store = store();
    store.append(request("s", 4, "m0", "body")).expect("seed");
    let err = store.append(request("s", 4, "m1", "body")).expect_err("conflict");
    assert!(matches!(err, TranscriptAppendErrorV1::SequenceConflict { sequence: 4 }));
}
#[test]
fn reused_message_id_at_different_sequence_is_typed_conflict() {
    let store = store();
    store.append(request("s", 4, "m0", "body")).expect("seed");
    let err = store.append(request("s", 5, "m0", "body")).expect_err("conflict");
    assert!(matches!(err, TranscriptAppendErrorV1::MessageIdConflict));
}
#[test]
fn append_identity_is_partitioned_by_session() {
    let store = store();
    assert_eq!(
        store.append(request("left", 4, "m0", "left-body")).expect("left"),
        TranscriptAppendDispositionV1::Applied
    );
    assert_eq!(
        store.append(request("right", 4, "m0", "right-body")).expect("right"),
        TranscriptAppendDispositionV1::Applied
    );
    assert_eq!(store.load(&"left".to_string()).expect("left load")[0].body(), "left-body");
    assert_eq!(store.load(&"right".to_string()).expect("right load")[0].body(), "right-body");
}
