//! Hell-gate end-to-end verification.
//!
//! Drives the FULL public stack — live broker + charged facade stores + spill —
//! through realistic, adversarial, and concurrent sequences, asserting the ledger
//! invariants hold end-to-end (not just that calls returned `Ok`). These are the
//! emergent contracts that no single unit test can see: that a charged store's
//! bytes actually flow through the broker ledger (F#3), that caps gate REAL store
//! bytes fail-closed, that overwrite/rebuild/reconfigure/concurrency keep the SSOT
//! balanced.
//!
//! Gated on the live-accounting + store features so it only compiles where the
//! charged path exists (e.g. `--all-features` / `--features full`).
#![cfg(all(feature = "core-live-accounting", feature = "retained_map"))]

use std::sync::Arc;

use quanta_memory_platform_core::{
    ChargeKind, EffectiveMemoryBudgetReport, ExecutionBudget, LedgerSnapshot,
    LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClass, MemoryClassBudget,
    MemoryContextKind, MemoryError, MemoryPreset, ReclaimPriority, ResidencyPolicy,
    StoreBindingHandle,
};
use quanta_memory_platform_facade::{
    RetainedMapStore, SharedAccountingSink, SimpleRetainedMapStore,
};

const OWNER: &str = "e2e-owner";

/// A broker whose `MemoResidency` class is capped at `class_hard` and whose global
/// limit is `global`. Roomy elsewhere so the test controls exactly which cap bites.
fn broker(global: u64, class_hard: u64) -> LiveProcessMemoryBroker {
    let residency = ResidencyPolicy::new(global).with_class_budget(
        MemoryClass::MemoResidency,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: class_hard,
            hard_cap_bytes: class_hard,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        },
    );
    let report = EffectiveMemoryBudgetReport::from_manifest(
        MemoryBudgetManifest::new(MemoryPreset::Production, ExecutionBudget::default(), residency),
        vec!["hellgate-e2e".to_string()],
    );
    LiveProcessMemoryBroker::from_report(report).expect("valid broker")
}

fn charged_map(broker: &LiveProcessMemoryBroker) -> SimpleRetainedMapStore<String, Vec<u8>> {
    let sink: SharedAccountingSink = Arc::new(broker.clone());
    let binding = StoreBindingHandle::new(MemoryClass::MemoResidency, OWNER.to_string(), None);
    SimpleRetainedMapStore::new(binding).charged_against(sink)
}

/// Assert the three accounting axes balance — the SSOT is internally consistent.
fn assert_balanced(snap: &LedgerSnapshot) {
    assert!(snap.class_owner_balanced(), "class total must equal owner total: {snap:?}");
    assert!(snap.owner_partition_balanced(), "owner usage must partition into kinds: {snap:?}");
    assert!(snap.context_owner_balanced(), "context total must equal owner total: {snap:?}");
}

// 1) F#3 end-to-end: a charged store's bytes flow through the broker ledger, and
//    a full lifecycle returns the SSOT to zero, balanced at every observation.
#[test]
fn charged_store_bytes_flow_through_the_ledger_and_drain_to_zero() {
    let b = broker(10_000_000, 1_000_000);
    let store = charged_map(&b);
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);

    store.insert_checked("a".to_string(), vec![0u8; 100]).expect("insert a");
    store.insert_checked("b".to_string(), vec![0u8; 250]).expect("insert b");
    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(snap.total_owner_bytes(), 350, "store bytes are visible in the broker ledger");
    assert_eq!(
        store.committed_bytes().expect("committed"),
        350,
        "store counter agrees with ledger"
    );
    assert_balanced(&snap);

    // Overwrite 'a' smaller: the ledger re-charges to the new size, never doubles.
    store.insert_checked("a".to_string(), vec![0u8; 40]).expect("overwrite a");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 290);

    store.remove(&"a".to_string()).expect("remove a");
    store.remove(&"b".to_string()).expect("remove b");
    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(snap.total_owner_bytes(), 0, "draining the store returns the ledger to zero");
    assert_balanced(&snap);
}

// E2E: the spill tier (claimed in this file's header) actually round-trips end to
// end AND is accounting-neutral — spilling/restoring handles never charges or
// refunds broker bytes (ledger invariant: the spill controller routes only).
#[cfg(feature = "spill")]
#[test]
fn e2e_spill_tier_round_trips_fifo_and_never_touches_the_ledger() {
    use quanta_memory_platform_facade::{
        RetainingSpillTarget, RoutedSpillController, SpillController,
    };

    // A loaded broker: the ledger holds real charged store bytes throughout.
    let b = broker(10_000_000, 1_000_000);
    let store = charged_map(&b);
    store.insert_checked("resident".to_string(), vec![0u8; 500]).expect("insert");
    let ledger_before = b.ledger_snapshot().expect("snap").total_owner_bytes();
    assert_eq!(ledger_before, 500);

    let controller = RoutedSpillController::new(RetainingSpillTarget::available());
    controller.spill("h1".to_string()).expect("spill 1");
    controller.spill("h2".to_string()).expect("spill 2");
    assert_eq!(controller.spilled_count(), 2);
    // Accounting-neutral: routing handles to the spill tier moves no broker bytes.
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), ledger_before);

    // FIFO restore brings handles back oldest-first; the count balances to zero.
    assert_eq!(controller.restore().expect("r1"), Some("h1".to_string()));
    assert_eq!(controller.restore().expect("r2"), Some("h2".to_string()));
    assert_eq!(controller.restore().expect("drained"), None);
    assert_eq!(controller.spilled_count(), 0);
    assert_eq!(
        b.ledger_snapshot().expect("snap").total_owner_bytes(),
        ledger_before,
        "the spill round-trip left the ledger untouched"
    );
}

// 2) Caps gate REAL store bytes, fail-closed: an over-cap insert is denied and
//    leaves BOTH the store and the ledger byte-for-byte unchanged.
#[test]
fn class_cap_gates_store_bytes_fail_closed() {
    let b = broker(10_000_000, 1_000); // class cap 1_000
    let store = charged_map(&b);
    store.insert_checked("survivor".to_string(), vec![0u8; 800]).expect("under cap");
    let before = b.ledger_snapshot().expect("snap");
    let store_before = store.committed_bytes().expect("committed");

    // 800 + 400 = 1_200 > 1_000 class cap -> denied.
    let denied = store.insert_checked("overflow".to_string(), vec![0u8; 400]);
    assert!(
        matches!(denied, Err(MemoryError::Capacity(_))),
        "over-cap charged insert must be a typed capacity error, got {denied:?}"
    );
    assert_eq!(
        b.ledger_snapshot().expect("snap"),
        before,
        "denied insert must not move the ledger"
    );
    assert_eq!(
        store.committed_bytes().expect("c"),
        store_before,
        "denied insert must not move the store"
    );
    assert_eq!(store.get(&"survivor".to_string()), Some(vec![0u8; 800]), "survivor intact");
    assert!(store.get(&"overflow".to_string()).is_none(), "no phantom entry created");
}

// 3) Overwrite uses the atomic recharge: an in-place overwrite that NETS within
//    the cap succeeds even when (old + new) would transiently exceed it.
#[test]
fn overwrite_recharge_admits_net_fitting_replacement() {
    let b = broker(10_000_000, 1_000); // class cap 1_000
    let store = charged_map(&b);
    store.insert_checked("k".to_string(), vec![0u8; 900]).expect("900 of 1000");
    // Replace with 1_000: net is +100 (fits), but old+new = 1_900 > cap. A
    // naive charge-then-refund would deny; the atomic recharge admits it.
    store
        .insert_checked("k".to_string(), vec![0u8; 1_000])
        .expect("net-fitting overwrite via recharge");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 1_000);
    assert_eq!(store.committed_bytes().expect("c"), 1_000);
}

// 4) A broker rebuild fences charged-store writes (fail-closed), then resumes.
#[test]
fn rebuild_fences_charged_store_writes_then_resumes() {
    let b = broker(10_000_000, 1_000_000);
    let store = charged_map(&b);
    store.insert_checked("pre".to_string(), vec![0u8; 500]).expect("pre-rebuild insert");
    b.begin_rebuild().expect("begin rebuild");

    let during = store.insert_checked("during".to_string(), vec![0u8; 500]);
    assert!(
        matches!(during, Err(MemoryError::InvalidState(_))),
        "charged insert during rebuild must be rejected, got {during:?}"
    );
    // Fail-closed: nothing stranded, the pre-rebuild charge is intact.
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 500);

    b.finish_rebuild().expect("finish rebuild");
    store.insert_checked("post".to_string(), vec![0u8; 300]).expect("writes resume after rebuild");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 800);
    assert_balanced(&b.ledger_snapshot().expect("snap"));
}

// 5) A legal live reconfigure (raising caps) preserves charged usage and the
//    balanced SSOT across the swap.
#[test]
fn reconfigure_raise_preserves_charged_usage() {
    let b = broker(2_000_000, 1_000_000);
    let store = charged_map(&b);
    store.insert_checked("x".to_string(), vec![0u8; 600_000]).expect("600k under 1M class cap");

    let raised = ResidencyPolicy::new(8_000_000).with_class_budget(
        MemoryClass::MemoResidency,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: 3_000_000,
            hard_cap_bytes: 4_000_000,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        },
    );
    b.try_live_reconfigure(raised).expect("raising caps is legal");
    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(snap.total_owner_bytes(), 600_000, "charged usage survives a legal reconfigure");
    assert_balanced(&snap);
    // The larger cap now admits what the old one would have denied.
    store
        .insert_checked("y".to_string(), vec![0u8; 3_000_000])
        .expect("admitted under the raised cap");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 3_600_000);
}

// 6) Concurrency: many threads hammering one charged store (shared broker) never
//    push the ledger past the global limit, and the store counter stays exactly
//    consistent with the ledger (no torn updates / no TOCTOU).
#[test]
fn concurrent_charged_inserts_never_exceed_global_and_stay_consistent() {
    let global = 100_000u64;
    let b = broker(global, global); // class cap == global, so the global limit gates
    let store = Arc::new(charged_map(&b));

    let mut handles = Vec::new();
    for t in 0..8 {
        let store = Arc::clone(&store);
        handles.push(std::thread::spawn(move || {
            for i in 0..50 {
                // Distinct keys so every attempt is a fresh 1_000-byte insert.
                let _ = store.insert_checked(format!("k{t}-{i}"), vec![0u8; 1_000]);
            }
        }));
    }
    for h in handles {
        h.join().expect("thread panicked");
    }

    let snap = b.ledger_snapshot().expect("snap");
    assert!(
        snap.total_owner_bytes() <= global,
        "global limit must hold under concurrent charged inserts: {}",
        snap.total_owner_bytes()
    );
    assert_eq!(
        snap.total_owner_bytes(),
        store.committed_bytes().expect("committed"),
        "store counter and ledger must agree exactly after concurrent inserts (no torn updates)"
    );
    // 8*50*1000 = 400k attempts against a 100k ceiling -> saturates exactly.
    assert_eq!(snap.total_owner_bytes(), global, "the ceiling is saturated, not under/over-shot");
    assert_balanced(&snap);
}

// 7) Idempotency e2e: removing the same key twice does not double-refund the
//    ledger (the second remove is a no-op against accounting).
#[test]
fn double_remove_does_not_double_refund_the_ledger() {
    let b = broker(10_000_000, 1_000_000);
    let store = charged_map(&b);
    store.insert_checked("k".to_string(), vec![0u8; 1_000]).expect("insert");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 1_000);

    assert_eq!(
        store.remove(&"k".to_string()).expect("first remove"),
        Some(vec![0u8; 1_000]),
        "first remove returns value"
    );
    assert_eq!(
        b.ledger_snapshot().expect("snap").total_owner_bytes(),
        0,
        "first remove refunds once"
    );
    assert!(
        store.remove(&"k".to_string()).expect("second remove").is_none(),
        "second remove finds nothing"
    );
    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(snap.total_owner_bytes(), 0, "second remove cannot drive the ledger negative");
    assert_balanced(&snap);
}

// 8) Direct broker recharge restores the old charge under a fresh ticket when the
//    new charge is denied — exercised at the public surface alongside the stores.
#[test]
fn broker_recharge_denied_restores_old_charge() {
    let b = broker(10_000_000, 1_000);
    let old = b
        .reserve_committed(
            MemoryClass::MemoResidency,
            OWNER.to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata,
            800,
        )
        .expect("charge 800 of 1000");
    let err = b
        .recharge(
            &old,
            MemoryClass::MemoResidency,
            OWNER.to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata,
            2_000, // > class cap 1_000
        )
        .expect_err("over-cap recharge denied");
    assert!(matches!(err.error, MemoryError::Capacity(_)));
    let restored = err.restored.expect("old restored under a fresh ticket");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 800, "old charge preserved");
    b.release(&restored).expect("restored ticket releases the old bytes");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
}

// --- batch release (release_many) atomicity + idempotency at the public surface ---
// (the charged stores' sweep/truncate refund through this; audit findings [1]/[16])

fn committed(
    b: &LiveProcessMemoryBroker,
    bytes: u64,
) -> quanta_memory_platform_core::ReservationTicket {
    b.reserve_committed(
        MemoryClass::MemoResidency,
        OWNER.to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedMetadata,
        bytes,
    )
    .expect("committed charge")
}

#[test]
fn release_many_drains_a_batch_atomically() {
    let b = broker(10_000_000, 1_000_000);
    let t1 = committed(&b, 100);
    let t2 = committed(&b, 200);
    let t3 = committed(&b, 300);
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 600);

    b.release_many(&[t1, t2, t3]).expect("batch release drains all");
    let snap = b.ledger_snapshot().expect("snap");
    assert_eq!(snap.total_owner_bytes(), 0, "batch release returns every ticket's bytes");
    assert_balanced(&snap);
}

#[test]
fn release_many_with_a_duplicate_is_fail_closed() {
    let b = broker(10_000_000, 1_000_000);
    let t1 = committed(&b, 100);
    let t2 = committed(&b, 200);
    let before = b.ledger_snapshot().expect("snap");

    // A duplicate ticket in one batch is rejected BEFORE any counter moves.
    let denied = b.release_many(&[t1.clone(), t1.clone(), t2.clone()]);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "a duplicate in the batch must be rejected, got {denied:?}"
    );
    assert_eq!(
        b.ledger_snapshot().expect("snap"),
        before,
        "a rejected batch must leave every ticket charged (all-or-none)"
    );
    // The batch is still releasable cleanly once de-duplicated.
    b.release_many(&[t1, t2]).expect("de-duplicated batch releases");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
}

#[test]
fn release_many_with_an_already_released_ticket_is_fail_closed() {
    let b = broker(10_000_000, 1_000_000);
    let t1 = committed(&b, 100);
    let t2 = committed(&b, 200);
    b.release(&t1).expect("release t1 individually");
    let before = b.ledger_snapshot().expect("snap");
    assert_eq!(before.total_owner_bytes(), 200, "only t2 remains charged");

    // Batch containing the already-released t1 must fail closed, NOT partially
    // refund t2 (which would strand/lose accounting).
    let denied = b.release_many(&[t1, t2.clone()]);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "a batch with an already-released ticket must be rejected, got {denied:?}"
    );
    assert_eq!(
        b.ledger_snapshot().expect("snap"),
        before,
        "the still-live ticket must not be partially refunded by a rejected batch"
    );
    b.release(&t2).expect("t2 still releasable on its own");
    assert_eq!(b.ledger_snapshot().expect("snap").total_owner_bytes(), 0);
}

#[test]
fn release_many_during_rebuild_is_fail_closed() {
    let b = broker(10_000_000, 1_000_000);
    let t1 = committed(&b, 100);
    let t2 = committed(&b, 200);
    b.begin_rebuild().expect("begin rebuild");
    let before = b.ledger_snapshot().expect("snap");
    let denied = b.release_many(&[t1, t2]);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "batch release during rebuild is rejected, got {denied:?}"
    );
    assert_eq!(
        b.ledger_snapshot().expect("snap"),
        before,
        "a rebuild-rejected batch strands nothing",
    );
}

// --- charged payload store full-stack: dedup, pin discipline, refund (F#3) ---
#[cfg(feature = "payload_store")]
mod payload_e2e {
    use super::{broker, OWNER};
    use std::sync::Arc;

    use quanta_memory_platform_core::{
        MemoryClass, MemoryError, RetainedEnvelope, StoreBindingHandle,
    };
    use quanta_memory_platform_facade::{PayloadStore, SharedAccountingSink, SimplePayloadStore};

    fn charged_payload(
        b: &quanta_memory_platform_core::LiveProcessMemoryBroker,
    ) -> SimplePayloadStore<String, Vec<u8>> {
        let sink: SharedAccountingSink = Arc::new(b.clone());
        let binding = StoreBindingHandle::new(MemoryClass::MemoResidency, OWNER.to_string(), None);
        SimplePayloadStore::new(binding, None).charged_against(sink)
    }

    fn ledger(b: &quanta_memory_platform_core::LiveProcessMemoryBroker) -> u64 {
        b.ledger_snapshot().expect("snap").total_owner_bytes()
    }

    #[test]
    fn charged_payload_dedup_pin_delete_flow_through_the_ledger() {
        let b = broker(10_000_000, 1_000_000);
        let store = charged_payload(&b);

        let first = store
            .put_canonical(RetainedEnvelope::new(vec![7u8; 32]))
            .expect("first put charges the ledger");
        assert_eq!(ledger(&b), 32, "payload bytes flow through the broker ledger");

        // Canonical dedup: the identical payload re-uses the entry, one live charge.
        let second = store.put_canonical(RetainedEnvelope::new(vec![7u8; 32])).expect("dedup put");
        assert_eq!(second.payload_ref(), first.payload_ref(), "dedup returns the same ref");
        assert_eq!(ledger(&b), 32, "dedup does not double-charge the ledger");

        // Pin discipline: a pinned payload cannot be deleted; the ledger is intact.
        store.pin(first.payload_ref()).expect("pin");
        let pinned_delete = store.delete_if_unpinned(first.payload_ref());
        assert!(
            matches!(pinned_delete, Err(MemoryError::InvalidState(_))),
            "deleting a pinned payload must be rejected, got {pinned_delete:?}"
        );
        assert_eq!(ledger(&b), 32, "a refused pinned delete leaves the ledger charged");

        // Unpin, then delete refunds the ledger to zero.
        store.unpin(first.payload_ref()).expect("unpin");
        store.delete_if_unpinned(first.payload_ref()).expect("delete after unpin");
        assert_eq!(ledger(&b), 0, "delete refunds the payload bytes to the ledger");
    }

    #[test]
    fn charged_payload_over_cap_put_is_fail_closed() {
        let b = broker(10_000_000, 1_000); // class cap 1_000
        let store = charged_payload(&b);
        store.put_canonical(RetainedEnvelope::new(vec![0u8; 800])).expect("under cap");
        let before = ledger(&b);
        let denied = store.put_canonical(RetainedEnvelope::new(vec![1u8; 400])); // 1_200 > 1_000
        assert!(
            matches!(denied, Err(MemoryError::Capacity(_))),
            "over-cap payload put must fail closed, got {denied:?}"
        );
        assert_eq!(ledger(&b), before, "denied payload put must not move the ledger");
    }
}
