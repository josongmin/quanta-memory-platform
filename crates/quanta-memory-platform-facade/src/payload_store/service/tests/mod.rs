mod cases_01;
mod cases_02;

use super::*;
use crate::test_support::SingleTestCommand;
use std::cell::{Cell, RefCell};
use std::hash::Hasher;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, OnceLock, Weak};

use quanta_memory_platform_core::{
    ExecutionBudget, MemoryBudgetManifest, MemoryClass, MemoryClassBudget, MemoryPreset,
    ReclaimPriority, ResidencyPolicy,
};

fn binding() -> StoreBindingHandle {
    StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-a".to_string(), None)
}

fn live_broker(global: u64, class_hard: u64) -> LiveProcessMemoryBroker {
    let residency = ResidencyPolicy::new(global).with_class_budget(
        MemoryClass::ArtifactCache,
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
        .build()
        .expect("valid broker")
}

fn store_with(broker: &LiveProcessMemoryBroker) -> SimplePayloadStore<String, Vec<u8>, Budgeted> {
    SimplePayloadStore::new_budgeted(binding(), None, broker.clone()).expect("payload store nonce")
}

fn store() -> (LiveProcessMemoryBroker, SimplePayloadStore<String, Vec<u8>, Budgeted>) {
    let broker = live_broker(1_000_000, 600_000);
    let store = store_with(&broker);
    (broker, store)
}

fn envelope(bytes: &[u8]) -> RetainedEnvelope<Vec<u8>> {
    RetainedEnvelope::new(bytes.to_vec())
}

fn retained_cost(payload_bytes: u64, sequence: u64, store_nonce: u64) -> u64 {
    // Independent oracle: only wire text length and public semantic components.
    payload_bytes + format!("payload-{store_nonce}-{sequence}").len() as u64 + 64 + 16
}

#[derive(Clone)]
struct PanickingBytes {
    bytes: Vec<u8>,
    drops: Arc<AtomicUsize>,
}

impl ByteSized for PanickingBytes {
    fn byte_size(&self) -> u64 {
        self.bytes.len() as u64
    }
}

impl CanonicalKey for PanickingBytes {
    fn canonical_key(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

#[derive(Clone)]
struct ZeroMeasuredHugeCanonicalKey;

impl ByteSized for ZeroMeasuredHugeCanonicalKey {
    fn byte_size(&self) -> u64 {
        0
    }
}

impl CanonicalKey for ZeroMeasuredHugeCanonicalKey {
    fn canonical_key(&self) -> Vec<u8> {
        vec![7; 1_000_000]
    }
}

#[derive(Clone)]
struct MaxMeasuredPayload;

impl ByteSized for MaxMeasuredPayload {
    fn byte_size(&self) -> u64 {
        u64::MAX
    }
}

impl CanonicalKey for MaxMeasuredPayload {
    fn canonical_key(&self) -> Vec<u8> {
        vec![9]
    }
}

#[derive(Clone)]
struct PanicMeasuredPayload;

impl ByteSized for PanicMeasuredPayload {
    fn byte_size(&self) -> u64 {
        panic!("injected ByteSized panic")
    }
}

impl CanonicalKey for PanicMeasuredPayload {
    fn canonical_key(&self) -> Vec<u8> {
        vec![10]
    }
}

impl Drop for PanickingBytes {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        panic!("client payload destructor panic");
    }
}

#[derive(Clone, Debug)]
struct ReentrantPayloadRef(String);

impl ByteSized for ReentrantPayloadRef {
    fn byte_size(&self) -> u64 {
        self.0.len() as u64
    }
}

type ReentrantRefStore = SimplePayloadStore<ReentrantPayloadRef, Vec<u8>, Budgeted>;

impl From<String> for ReentrantPayloadRef {
    fn from(value: String) -> Self {
        Self(value)
    }
}

thread_local! {
    static REF_REENTRY_STORE: RefCell<Option<Weak<ReentrantRefStore>>> = const { RefCell::new(None) };
    static REF_REENTRY_HITS: Cell<usize> = const { Cell::new(0) };
}

impl PartialEq for ReentrantPayloadRef {
    fn eq(&self, other: &Self) -> bool {
        REF_REENTRY_STORE.with(|slot| {
            if let Some(store) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                assert!(matches!(
                    store.stats(),
                    Err(MemoryError::OperationInProgress(resource)) if resource == "payload store"
                ));
                REF_REENTRY_HITS.with(|hits| hits.set(hits.get() + 1));
            }
        });
        self.0 == other.0
    }
}

impl Eq for ReentrantPayloadRef {}

impl Hash for ReentrantPayloadRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[derive(Clone)]
struct ReentrantCanonicalBytes {
    bytes: Vec<u8>,
    probe: Option<ReentrantByteProbe>,
    hits: Arc<AtomicUsize>,
}

type ReentrantByteStore = SimplePayloadStore<String, ReentrantCanonicalBytes, Budgeted>;
type ReentrantByteProbe = Arc<OnceLock<Weak<ReentrantByteStore>>>;

impl ByteSized for ReentrantCanonicalBytes {
    fn byte_size(&self) -> u64 {
        self.bytes.len() as u64
    }
}

impl CanonicalKey for ReentrantCanonicalBytes {
    fn canonical_key(&self) -> Vec<u8> {
        if let Some(store) =
            self.probe.as_ref().and_then(|probe| probe.get()).and_then(Weak::upgrade)
        {
            assert!(matches!(
                store.stats(),
                Err(MemoryError::OperationInProgress(resource)) if resource == "payload store"
            ));
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
        self.bytes.clone()
    }
}

fn context_handle(kind: MemoryContextKind) -> MemoryContextHandle {
    MemoryContextHandle::new("rt-test".to_string(), kind, "owner-a".to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ConstantRef;

impl From<String> for ConstantRef {
    fn from(_: String) -> Self {
        Self
    }
}

impl ByteSized for ConstantRef {
    fn byte_size(&self) -> u64 {
        0
    }
}

type DropReentryPayloadStore = SimplePayloadStore<String, DropReentryBody, Budgeted>;

#[derive(Clone)]
struct DropReentryBody {
    id: u8,
    armed: Arc<std::sync::atomic::AtomicBool>,
    home: Arc<OnceLock<Weak<DropReentryPayloadStore>>>,
    peer: Arc<OnceLock<Weak<DropReentryPayloadStore>>>,
    hits: Arc<AtomicUsize>,
}

impl ByteSized for DropReentryBody {
    fn byte_size(&self) -> u64 {
        1
    }
}

impl CanonicalKey for DropReentryBody {
    fn canonical_key(&self) -> Vec<u8> {
        vec![self.id]
    }
}

impl Drop for DropReentryBody {
    fn drop(&mut self) {
        if !self.armed.swap(false, Ordering::SeqCst) {
            return;
        }
        let home = self.home.get().expect("home installed").upgrade().expect("home live");
        assert!(matches!(
            home.stats(),
            Err(MemoryError::OperationInProgress(resource)) if resource == "payload store"
        ));
        let peer = self.peer.get().expect("peer installed").upgrade().expect("peer live");
        peer.put_canonical(RetainedEnvelope::new(Self {
            id: 2,
            armed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            home: Arc::clone(&self.home),
            peer: Arc::clone(&self.peer),
            hits: Arc::clone(&self.hits),
        }))
        .expect("other store can publish during retired body drop");
        self.hits.fetch_add(1, Ordering::SeqCst);
    }
}
