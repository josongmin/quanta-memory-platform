mod cases_01;
mod cases_02;

use super::*;
use quanta_memory_platform_core::{
    ExecutionBudget, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClassBudget,
    MemoryEventSink, MemoryPreset, NoopMemoryEventSink, ReclaimPriority, ResidencyPolicy,
};
use std::cmp::Ordering;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{OnceLock, Weak};

#[derive(Debug)]
struct ComparisonTrap {
    calls: AtomicUsize,
    panic_at: AtomicUsize,
}

impl ComparisonTrap {
    fn new() -> Arc<Self> {
        Arc::new(Self { calls: AtomicUsize::new(0), panic_at: AtomicUsize::new(usize::MAX) })
    }

    fn arm_after_one_more_comparison(&self) -> usize {
        let before = self.calls.load(AtomicOrdering::SeqCst);
        self.panic_at.store(before + 2, AtomicOrdering::SeqCst);
        before
    }

    fn disarm(&self) {
        self.panic_at.store(usize::MAX, AtomicOrdering::SeqCst);
    }
}

#[derive(Clone, Debug)]
struct OrderedSessionId {
    name: &'static str,
    trap: Arc<ComparisonTrap>,
}

impl ByteSized for OrderedSessionId {
    fn byte_size(&self) -> u64 {
        self.name.len() as u64
    }
}

impl PartialEq for OrderedSessionId {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for OrderedSessionId {}

impl PartialOrd for OrderedSessionId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedSessionId {
    fn cmp(&self, other: &Self) -> Ordering {
        let call = self.trap.calls.fetch_add(1, AtomicOrdering::SeqCst) + 1;
        assert_ne!(
            call,
            self.trap.panic_at.load(AtomicOrdering::SeqCst),
            "client SessionId::Ord must not run after retain planning",
        );
        self.name.cmp(other.name)
    }
}

#[derive(Debug)]
struct CallbackTrap {
    calls: AtomicUsize,
    panic_at: AtomicUsize,
}

impl CallbackTrap {
    fn new() -> Arc<Self> {
        Arc::new(Self { calls: AtomicUsize::new(0), panic_at: AtomicUsize::new(usize::MAX) })
    }

    fn arm_after(&self, additional_calls: usize) {
        self.panic_at.store(
            self.calls.load(AtomicOrdering::SeqCst) + additional_calls,
            AtomicOrdering::SeqCst,
        );
    }

    fn disarm(&self) {
        self.panic_at.store(usize::MAX, AtomicOrdering::SeqCst);
    }

    fn hit(&self) {
        let call = self.calls.fetch_add(1, AtomicOrdering::SeqCst) + 1;
        assert_ne!(call, self.panic_at.load(AtomicOrdering::SeqCst), "client callback panic");
    }
}

#[derive(Debug)]
struct CallbackMessageId {
    name: &'static str,
    clones: Arc<CallbackTrap>,
    comparisons: Arc<CallbackTrap>,
}

impl ByteSized for CallbackMessageId {
    fn byte_size(&self) -> u64 {
        self.name.len() as u64
    }
}

impl Clone for CallbackMessageId {
    fn clone(&self) -> Self {
        self.clones.hit();
        Self {
            name: self.name,
            clones: Arc::clone(&self.clones),
            comparisons: Arc::clone(&self.comparisons),
        }
    }
}

impl PartialEq for CallbackMessageId {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for CallbackMessageId {}

impl PartialOrd for CallbackMessageId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CallbackMessageId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.comparisons.hit();
        self.name.cmp(other.name)
    }
}

fn callback_id(
    name: &'static str,
    clones: &Arc<CallbackTrap>,
    comparisons: &Arc<CallbackTrap>,
) -> CallbackMessageId {
    CallbackMessageId { name, clones: Arc::clone(clones), comparisons: Arc::clone(comparisons) }
}

fn callback_store(
    broker: &LiveProcessMemoryBroker,
) -> SimpleSessionTranscriptStore<String, CallbackMessageId, String, Budgeted> {
    SimpleSessionTranscriptStore::new_budgeted(
        StoreBindingHandle::new(
            MemoryClass::SessionTranscript,
            "transcript-append-callback-test".to_string(),
            None,
        ),
        TranscriptRetentionPolicyV2::default(),
        broker.clone(),
    )
    .expect("transcript store nonce")
}

type ReentrantIdStore = SimpleSessionTranscriptStore<String, ReentrantMessageId, String, Budgeted>;

#[derive(Debug)]
struct ReentrantMessageId {
    name: &'static str,
    owner: Arc<OnceLock<Weak<ReentrantIdStore>>>,
    busy_callbacks: Arc<AtomicUsize>,
}

impl ByteSized for ReentrantMessageId {
    fn byte_size(&self) -> u64 {
        self.name.len() as u64
    }
}

impl ReentrantMessageId {
    fn callback(&self) {
        let store = self.owner.get().expect("owner installed").upgrade().expect("live");
        assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
        assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
        self.busy_callbacks.fetch_add(1, AtomicOrdering::SeqCst);
    }
}

impl Clone for ReentrantMessageId {
    fn clone(&self) -> Self {
        self.callback();
        Self {
            name: self.name,
            owner: Arc::clone(&self.owner),
            busy_callbacks: Arc::clone(&self.busy_callbacks),
        }
    }
}

impl PartialEq for ReentrantMessageId {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for ReentrantMessageId {}

impl PartialOrd for ReentrantMessageId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReentrantMessageId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.callback();
        self.name.cmp(other.name)
    }
}

fn assert_seed_only_after_append_panic(
    store: &SimpleSessionTranscriptStore<String, CallbackMessageId, String, Budgeted>,
    broker: &LiveProcessMemoryBroker,
) {
    let state = store.state.lock().unwrap_or_else(PoisonError::into_inner);
    let session = state.sessions.values().next().expect("seed session remains");
    assert_eq!(session.session_id, "s");
    assert_eq!(session.version, 1, "failed append advanced version");
    assert_eq!(session.rows.len(), 1, "failed append published a row");
    assert_eq!(session.sequence_evidence.len(), 1, "failed append published sequence evidence");
    assert_eq!(session.message_evidence.len(), 1, "failed append published message evidence");
    assert!(session
        .rows
        .get(&0)
        .expect("seed row")
        .active_charge
        .as_ref()
        .is_some_and(ManagedCharge::is_active));
    assert_eq!(
        state.retained_bytes,
        cost("s", &[("m0", "one", true)]),
        "failed append changed retained-byte counter"
    );
    drop(state);
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        cost("s", &[("m0", "one", true)])
    );
}

// P0 regression oracle (round-2 audit): a caller-trait panic in the
// FINAL message-evidence keyed step of publication must leave the
// store atomically intact — close still works, counters reconcile,
// and the rolled-back append retries as a clean Applied. The trap is
// armed at the last MessageId comparison of the append under the
// current publication shape (preflight probe, publication probe,
// keyed insert descent).

#[derive(Clone, Debug)]
struct PanicOnDropBody(Vec<u8>);

impl ByteSized for PanicOnDropBody {
    fn byte_size(&self) -> u64 {
        self.0.len() as u64
    }
}

impl CanonicalKey for PanicOnDropBody {
    fn canonical_key(&self) -> Vec<u8> {
        self.0.clone()
    }
}

impl Drop for PanicOnDropBody {
    fn drop(&mut self) {
        panic!("client body destructor panic");
    }
}

fn broker(class_hard: u64) -> LiveProcessMemoryBroker {
    broker_with_events(class_hard, Arc::new(NoopMemoryEventSink))
}

fn broker_with_events(
    class_hard: u64,
    events: Arc<dyn MemoryEventSink>,
) -> LiveProcessMemoryBroker {
    let residency = ResidencyPolicy::new(1_000_000).with_class_budget(
        MemoryClass::SessionTranscript,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: class_hard,
            hard_cap_bytes: class_hard,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        },
    );
    quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            residency,
        ))
        .with_event_sink(events)
        .build()
        .expect("valid broker")
}

fn charged(
    broker: &LiveProcessMemoryBroker,
) -> SimpleSessionTranscriptStore<String, String, String, Budgeted> {
    charged_with_policy(broker, TranscriptRetentionPolicyV2::default())
}

fn charged_with_policy(
    broker: &LiveProcessMemoryBroker,
    retention: TranscriptRetentionPolicyV2,
) -> SimpleSessionTranscriptStore<String, String, String, Budgeted> {
    SimpleSessionTranscriptStore::new_budgeted(
        StoreBindingHandle::new(
            MemoryClass::SessionTranscript,
            "transcript-idempotency-tests".to_string(),
            None,
        ),
        retention,
        broker.clone(),
    )
    .expect("transcript store nonce")
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SizedId {
    name: &'static str,
    bytes: u64,
}

impl ByteSized for SizedId {
    fn byte_size(&self) -> u64 {
        self.bytes
    }
}
