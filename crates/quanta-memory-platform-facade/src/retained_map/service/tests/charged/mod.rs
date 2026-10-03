mod cases_01;
mod cases_02;

use super::*;
use quanta_memory_platform_core::{
    ExecutionBudget, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClassBudget,
    MemoryPreset, ReclaimPriority, ResidencyPolicy,
};

fn live_broker(class_hard: u64) -> LiveProcessMemoryBroker {
    let residency = ResidencyPolicy::new(10_000_000).with_class_budget(
        MemoryClass::MemoResidency,
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

fn charged(broker: &LiveProcessMemoryBroker) -> SimpleRetainedMapStore<String, Vec<u8>, Budgeted> {
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "owner-map".to_string(), None);
    SimpleRetainedMapStore::new_budgeted(binding, broker.clone())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PanicMeasuredKey(u8);

impl ByteSized for PanicMeasuredKey {
    fn byte_size(&self) -> u64 {
        panic!("injected key measurement panic")
    }
}

#[derive(Clone, Debug)]
struct AliasKey {
    id: u8,
    declared_bytes: u64,
}

impl PartialEq for AliasKey {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for AliasKey {}
impl Hash for AliasKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
impl ByteSized for AliasKey {
    fn byte_size(&self) -> u64 {
        self.declared_bytes
    }
}

#[derive(Clone, Debug)]
struct PanicOnRetire {
    bytes: u64,
    panic_once: Arc<AtomicBool>,
}

impl ByteSized for PanicOnRetire {
    fn byte_size(&self) -> u64 {
        self.bytes
    }
}

impl Drop for PanicOnRetire {
    fn drop(&mut self) {
        if self.panic_once.swap(false, Ordering::SeqCst) {
            panic!("client value destructor");
        }
    }
}

#[derive(Clone, Debug)]
struct PanicAfterSweepPlanKey {
    id: u64,
    hash_must_panic: Arc<AtomicBool>,
}

impl ByteSized for PanicAfterSweepPlanKey {
    fn byte_size(&self) -> u64 {
        8
    }
}

impl PartialEq for PanicAfterSweepPlanKey {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for PanicAfterSweepPlanKey {}

impl Hash for PanicAfterSweepPlanKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        assert!(!self.hash_must_panic.load(Ordering::SeqCst), "post-plan client hash callback");
        self.id.hash(state);
    }
}

#[derive(Clone, Debug)]
struct AssertChargedDuringHashKey {
    id: u64,
    broker: LiveProcessMemoryBroker,
    armed: Arc<AtomicBool>,
    callbacks: Arc<AtomicUsize>,
}

impl ByteSized for AssertChargedDuringHashKey {
    fn byte_size(&self) -> u64 {
        8
    }
}

impl PartialEq for AssertChargedDuringHashKey {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for AssertChargedDuringHashKey {}

impl Hash for AssertChargedDuringHashKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.armed.load(Ordering::SeqCst) {
            assert_eq!(
                self.broker.ledger_snapshot().expect("ledger in client Hash").total_owner_bytes(),
                retained_cost("x1234567", 5),
                "sweep must never call client Hash after refund"
            );
            self.callbacks.fetch_add(1, Ordering::SeqCst);
        }
        self.id.hash(state);
    }
}

#[derive(Clone, Debug)]
struct PanicAfterSweepDropKey {
    id: &'static str,
    panic_once: Arc<AtomicBool>,
}

impl ByteSized for PanicAfterSweepDropKey {
    fn byte_size(&self) -> u64 {
        self.id.len() as u64
    }
}

impl PartialEq for PanicAfterSweepDropKey {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for PanicAfterSweepDropKey {}

impl Hash for PanicAfterSweepDropKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl Drop for PanicAfterSweepDropKey {
    fn drop(&mut self) {
        if self.panic_once.swap(false, Ordering::SeqCst) {
            panic!("client key destructor");
        }
    }
}
