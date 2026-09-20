//! The facade-store conformance must BITE.
//!
//! CLAUDE.md non-negotiable: "the testkit suite must REJECT a broken fake, not
//! just accept honest ones." The facade-store contract families (ReclaimAccounting,
//! PinDiscipline) are otherwise only ever exercised against an honest live broker,
//! so a regression that swallowed or double-applied a refund would still report
//! `passed()`. These tests feed deliberately broken [`AccountingSink`] targets
//! through a charged store and assert the contract is OBSERVABLY violated (the
//! broker ledger diverges from the store's own counter), with an honest positive
//! pole proving the check does not false-positive.
// The `conformance` testkit feature forwards the facade's `core-live-accounting`
// + `retained_map` features, so the charged store path exists under it.
#![cfg(feature = "conformance")]

use std::sync::Arc;

use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, MemoryClass, MemoryContextKind, MemoryPreset,
    MemoryResult, OwnerKey, ProcessMemoryBrokerBuilder, RechargeError, RechargeRecoveryV1,
    ReservationTicket, StoreBindingHandle,
};
use quanta_memory_platform_facade::shared::accounting::RechargeDenied;
use quanta_memory_platform_facade::{
    AccountingSink, RetainedMapStore, RetainedMapStoreBuilder, SharedAccountingSink,
    SimpleRetainedMapStore,
};

fn broker() -> LiveProcessMemoryBroker {
    ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Production)
        .build()
        .expect("valid broker")
}

fn charged(
    broker: &LiveProcessMemoryBroker,
    sink: SharedAccountingSink,
) -> SimpleRetainedMapStore<String, Vec<u8>> {
    let _ = broker;
    let binding =
        StoreBindingHandle::new(MemoryClass::MemoResidency, "bite-owner".to_string(), None);
    RetainedMapStoreBuilder::new().bind(binding).charged_against(sink).build()
}

/// Honest sink: routes everything to the broker (the contract-correct behavior).
#[derive(Debug)]
struct HonestSink(LiveProcessMemoryBroker);

impl AccountingSink for HonestSink {
    fn charge(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        self.0.reserve_committed(class, owner, context, kind, bytes)
    }
    fn refund(&self, ticket: &ReservationTicket) -> MemoryResult<()> {
        self.0.release(ticket)
    }
    fn refund_many(&self, tickets: &[ReservationTicket]) -> MemoryResult<()> {
        self.0.release_many(tickets)
    }
    fn recharge(
        &self,
        old: &ReservationTicket,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> Result<ReservationTicket, RechargeDenied> {
        self.0.recharge(old, class, owner, context, kind, bytes)
    }
}

/// Broken sink: charges honestly but SWALLOWS every refund (a no-op). A store
/// built on it under-refunds the ledger — exactly the ReclaimAccounting fault.
#[derive(Debug)]
struct RefundSwallowingSink(LiveProcessMemoryBroker);

impl AccountingSink for RefundSwallowingSink {
    fn charge(
        &self,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> MemoryResult<ReservationTicket> {
        self.0.reserve_committed(class, owner, context, kind, bytes)
    }
    fn refund(&self, _ticket: &ReservationTicket) -> MemoryResult<()> {
        Ok(()) // BUG under test: pretends to refund, leaves the ledger charged.
    }
    fn refund_many(&self, _tickets: &[ReservationTicket]) -> MemoryResult<()> {
        Ok(()) // BUG under test.
    }
    fn recharge(
        &self,
        _old: &ReservationTicket,
        class: MemoryClass,
        owner: OwnerKey,
        context: MemoryContextKind,
        kind: ChargeKind,
        bytes: u64,
    ) -> Result<ReservationTicket, RechargeDenied> {
        // Charge the new value but never release the old: a leaking overwrite.
        self.charge(class, owner, context, kind, bytes).map_err(|error| RechargeError {
            error,
            recovery: RechargeRecoveryV1::OldChargePreserved,
        })
    }
}

fn ledger_bytes(b: &LiveProcessMemoryBroker) -> u64 {
    b.ledger_snapshot().expect("snap").total_owner_bytes()
}

// Positive pole: the honest sink keeps the broker ledger in lockstep with the
// store, draining to zero on remove. (Proves the check below does not false-positive.)
#[test]
fn reclaim_accounting_holds_for_an_honest_sink() {
    let b = broker();
    let sink: SharedAccountingSink = Arc::new(HonestSink(b.clone()));
    let store = charged(&b, sink);

    store.insert_checked("k".to_string(), vec![0u8; 1_000]).expect("insert");
    assert_eq!(ledger_bytes(&b), 1_000, "honest charge reaches the ledger");
    assert_eq!(store.committed_bytes().expect("c"), 1_000);

    store.remove(&"k".to_string()).expect("remove");
    assert_eq!(ledger_bytes(&b), 0, "honest refund drains the ledger");
    assert_eq!(store.committed_bytes().expect("c"), 0, "store and ledger agree at zero");
}

// BITE: a refund-swallowing sink is CAUGHT — after remove, the store's own counter
// says 0 but the broker ledger is still charged, so the ReclaimAccounting contract
// (ledger reflects store residency and drains on remove) is observably violated.
#[test]
fn reclaim_accounting_bites_a_refund_swallowing_sink() {
    let b = broker();
    let sink: SharedAccountingSink = Arc::new(RefundSwallowingSink(b.clone()));
    let store = charged(&b, sink);

    store.insert_checked("k".to_string(), vec![0u8; 1_000]).expect("insert");
    store.remove(&"k".to_string()).expect("store removes its entry");

    let store_bytes = store.committed_bytes().expect("c");
    let ledger = ledger_bytes(&b);
    assert_eq!(store_bytes, 0, "the store believes it released the entry");
    assert_eq!(ledger, 1_000, "but the swallowed refund left the ledger charged");
    assert_ne!(
        store_bytes, ledger,
        "the ReclaimAccounting contract MUST detect the divergence: a check that \
         passed this broken sink would be a rubber stamp"
    );
}

// BITE: a leaking-overwrite (recharge that never refunds the old) is caught — two
// inserts under one key leave the ledger holding BOTH charges while the store
// holds one, again diverging.
#[test]
fn reclaim_accounting_bites_a_leaking_overwrite_sink() {
    let b = broker();
    let sink: SharedAccountingSink = Arc::new(RefundSwallowingSink(b.clone()));
    let store = charged(&b, sink);

    store.insert_checked("k".to_string(), vec![0u8; 400]).expect("insert");
    store.insert_checked("k".to_string(), vec![0u8; 400]).expect("overwrite");

    assert_eq!(store.committed_bytes().expect("c"), 400, "store keeps one live charge");
    assert_eq!(ledger_bytes(&b), 800, "leaking overwrite double-charged the ledger");
    assert_ne!(
        store.committed_bytes().expect("c"),
        ledger_bytes(&b),
        "a leaking overwrite must be detectable as a store/ledger divergence"
    );
}

// The bite expressed as a localized conformance VERDICT: the same
// ReclaimAccounting [`InvariantAssertion`] the facade-store suite uses must HOLD
// for an honest sink and VIOLATE for a broken one. This guards against the
// suite's own assertion being silently weakened — a verdict that can never flip
// to violated is a rubber stamp.
#[test]
fn reclaim_accounting_verdict_holds_for_honest_and_violates_for_broken_sink() {
    use quanta_memory_platform_testkit::{ContractFamily, InvariantAssertion};

    let assertion =
        InvariantAssertion::new(ContractFamily::ReclaimAccounting, "store_drains_ledger");

    // Honest: insert then remove drains the ledger to 0 -> the verdict HOLDS.
    let hb = broker();
    let honest = charged(&hb, Arc::new(HonestSink(hb.clone())));
    honest.insert_checked("k".to_string(), vec![0u8; 1_000]).expect("insert");
    honest.remove(&"k".to_string()).expect("remove");
    let honest_verdict = assertion.assert_eq_bytes("ledger_after_remove", ledger_bytes(&hb), 0);
    assert!(honest_verdict.held(), "honest sink: ReclaimAccounting must hold");

    // Broken (refund swallowed): the ledger stays charged after remove -> the SAME
    // assertion VIOLATES, localized to ReclaimAccounting. Proves the suite bites.
    let bb = broker();
    let broken = charged(&bb, Arc::new(RefundSwallowingSink(bb.clone())));
    broken.insert_checked("k".to_string(), vec![0u8; 1_000]).expect("insert");
    broken.remove(&"k".to_string()).expect("remove");
    let broken_verdict = assertion.assert_eq_bytes("ledger_after_remove", ledger_bytes(&bb), 0);
    assert!(
        !broken_verdict.held(),
        "a refund-swallowing sink MUST make the ReclaimAccounting verdict violate"
    );
}
