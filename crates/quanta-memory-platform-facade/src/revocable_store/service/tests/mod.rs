mod cases_01;
#[cfg(feature = "spill")]
mod cases_02;
#[cfg(feature = "spill-redb")]
mod cases_03;
mod cases_04;

use super::*;
#[cfg(feature = "spill")]
use crate::spill::{
    DurableRestoreCandidate, DurableRestoreTarget, DurableSpillTarget, InMemorySpillTarget,
    InProcessSpillTarget, NoopSpillController, RoutedSpillController, SpillController,
    SpillInventory, SpillReceipt, SpillTarget,
};
use crate::test_support::SingleTestCommand;
#[cfg(feature = "spill")]
use quanta_memory_platform_core::MemoryOutcome;
use quanta_memory_platform_core::{
    ExecutionBudget, MemoryBudgetManifest, MemoryClass, MemoryClassBudget, MemoryEvent,
    MemoryEventSink, MemoryPreset, ProcessMemoryBrokerBuilder, ReclaimPriority, ResidencyPolicy,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Barrier;
use std::thread;

type Store = SimpleRevocableStore<String, Vec<u8>, Budgeted>;

#[derive(Default)]
struct RecordingEvents(Mutex<Vec<MemoryEvent>>);

impl MemoryEventSink for RecordingEvents {
    fn emit(&self, event: MemoryEvent) {
        self.0.lock().expect("event recorder").push(event);
    }
}

fn expected_entry_cost(key: &str, payload_bytes: u64) -> u64 {
    u64::try_from(key.len()).expect("test key fits") + payload_bytes + 16 + 16
}

fn binding() -> StoreBindingHandle {
    StoreBindingHandle::new(MemoryClass::ExecutionScratchRevocable, "owner-rev".to_string(), None)
}

fn live_broker(class_hard: u64) -> LiveProcessMemoryBroker {
    live_broker_with_events(class_hard, Arc::new(RecordingEvents::default()))
}

fn live_broker_with_events(
    class_hard: u64,
    events: Arc<dyn MemoryEventSink>,
) -> LiveProcessMemoryBroker {
    let residency = ResidencyPolicy::new(10_000_000).with_class_budget(
        MemoryClass::ExecutionScratchRevocable,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: class_hard,
            hard_cap_bytes: class_hard,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::First,
        },
    );
    ProcessMemoryBrokerBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            residency,
        ))
        .with_event_sink(events)
        .build()
        .expect("valid broker")
}

fn store_with_broker(broker: &LiveProcessMemoryBroker) -> Store {
    SimpleRevocableStore::new_budgeted(binding(), broker.clone())
}

fn store() -> Store {
    store_with_broker(&live_broker(1_000_000))
}

struct PanickingValue {
    bytes: Vec<u8>,
    drops: Arc<AtomicUsize>,
}

impl ByteSized for PanickingValue {
    fn byte_size(&self) -> u64 {
        self.bytes.len() as u64
    }
}

impl Drop for PanickingValue {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        panic!("client revocable destructor panic");
    }
}

// --- FUP-001: fenced source vs. exact disposition ------------------------
//
// A double-uncertain durable commit (lost ACK + lost probe) fences the
// source entry with its charge intact. A LATER record's finalize on the
// same backend must never retire that fenced source: the reserved-but-
// never-committed id is a hole (`Absent`), not a consumed record.

// --- FUP-002: single-owner recovery ----------------------------------
//
// Concurrent recoverers of one persisted restore each mint a distinct
// claim generation; exactly one publishes, and every loser retires its
// provisional copy without ever exposing it.

#[cfg(feature = "spill-redb")]
fn sep23_fup002_decode(bytes: &[u8]) -> MemoryResult<(String, Vec<u8>)> {
    let separator = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| MemoryError::report("missing durable record separator"))?;
    let key = String::from_utf8(bytes[..separator].to_vec())
        .map_err(|error| MemoryError::report(format!("invalid key: {error}")))?;
    Ok((key, bytes[separator + 1..].to_vec()))
}

/// Seed one record and leave it in the Committed recovery phase with a
/// persisted active claim, simulating a process that died after the
/// durable delete but before resident acknowledgement.
#[cfg(feature = "spill-redb")]
fn sep23_fup002_seed_committed_recovery(path: &std::path::Path) {
    use crate::spill::backings::redb::RedbSpillBacking;
    let controller = RoutedSpillController::new(RedbSpillBacking::create(path).expect("redb seed"));
    let record_id = controller.reserve_record_id().expect("reserve");
    let _receipt = controller.commit_spill(record_id, b"shared\0payload").expect("seed durable");
    let candidate = controller.prepare_restore().expect("prepare").expect("candidate");
    controller.commit_restore(&candidate.claim).expect("persist recovery before crash");
    assert_eq!(controller.active_restore(), Ok(Some(record_id)));
}

// Self-audit P2-3: a panicking decode must unwind WITHOUT stranding a
// claim, a fence, or a charge. Prepared-phase panic aborts the live claim
// so a plain retry works; Committed-phase panic preserves the durable
// recovery so a recovery retry publishes exactly once.

#[cfg(feature = "spill-redb")]
struct PanicOnceMeasuredValue {
    bytes: Vec<u8>,
    panic_once: Arc<AtomicBool>,
}

#[cfg(feature = "spill-redb")]
impl ByteSized for PanicOnceMeasuredValue {
    fn byte_size(&self) -> u64 {
        if self.panic_once.swap(false, Ordering::SeqCst) {
            panic!("injected restore measurement panic");
        }
        self.bytes.len() as u64
    }
}
