mod cases_01;
mod cases_02;

use super::*;
use crate::domain::config::ValidatedResidencyPolicy;
use crate::domain::policy::{MemoryClassBudget, OwnerQuota, ReclaimPriority, ResidencyPolicy};

fn class_budget(soft: u64, hard: u64, borrow: bool) -> MemoryClassBudget {
    MemoryClassBudget {
        reserved_bytes: 0,
        soft_cap_bytes: soft,
        hard_cap_bytes: hard,
        borrow_from_shared_pool: borrow,
        reclaim_priority: ReclaimPriority::Normal,
    }
}

/// Ledger unit tests that are not testing class admission use an explicitly
/// permissive class map. Production `ResidencyPolicy::new` is fail-closed
/// with zero budgets and never receives this test-only expansion.
fn ledger_from_policy(mut policy: ResidencyPolicy) -> AccountingLedger {
    let global = policy.global_hard_limit_bytes;
    for class in MemoryClass::mandatory() {
        let budget = policy.classes.get_mut(&class).expect("mandatory class materialized");
        if budget.hard_cap_bytes == 0 {
            budget.soft_cap_bytes = global;
            budget.hard_cap_bytes = global;
        }
    }
    let validated = ValidatedResidencyPolicy::try_from(policy).expect("valid test policy");
    AccountingLedger::new(validated)
}

fn reserve(
    ledger: &mut AccountingLedger,
    class: MemoryClass,
    owner: &str,
    bytes: u64,
) -> MemoryResult<ReservationTicket> {
    ledger.reserve(
        class,
        owner.to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        bytes,
    )
}

// ---- #1: the global hard limit is enforced on reserve (was advisory) ----

// ---- #4: reservation tickets are idempotent (no double-release corruption) ----

// ---- #6: borrowing over a class cap is bounded by the shared burst pool ----

// sanity: an owner quota still trips independently of the new ceilings.

// An EMPTY batch release is a true no-op and must succeed even during rebuild
// (a charged store's no-op sweep/truncate that reclaimed nothing must not be
// wrongly rejected mid-rebuild), while a NON-empty batch stays rebuild-fenced.

// ===================================================================
// Scenario group A: ledger accounting invariants (extras).
// Shared assertion: after every op the global invariants must hold.
// ===================================================================

/// Assert invariants 1, 2 and the third-axis (context==owner) balance on a
/// snapshot. Used as the post-condition of every accounting op below.
fn assert_axes_balanced(snap: &LedgerSnapshot) {
    assert!(snap.class_owner_balanced(), "invariant 1: class==owner must hold: {snap:?}");
    assert!(
        snap.owner_partition_balanced(),
        "invariant 2: owner partitions into payload+metadata+scratch: {snap:?}"
    );
    assert!(
        snap.context_owner_balanced(),
        "third axis: context total must equal owner total: {snap:?}"
    );
}

fn reserve_kind(
    ledger: &mut AccountingLedger,
    class: MemoryClass,
    owner: &str,
    context: MemoryContextKind,
    kind: ChargeKind,
    bytes: u64,
) -> MemoryResult<ReservationTicket> {
    ledger.reserve(class, owner.to_string(), context, kind, bytes)
}

// ---- scenario A: zero-byte reserve reserves/commits/releases coherently ----

// ---- scenario A: saturating_add near u64::MAX preserves invariant 1 ----

fn assert_single_release_probe_bound(owner_count: usize) {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new((owner_count as u64) + 1));
    let mut tickets = Vec::with_capacity(owner_count);
    for owner in 0..owner_count {
        tickets.push(
            reserve(&mut ledger, MemoryClass::ArtifactCache, &format!("owner-{owner}"), 1)
                .expect("reserve one byte per owner"),
        );
    }
    assert_eq!(ledger.total_semantic_bytes(), owner_count as u64);
    assert_eq!(ledger.non_headroom_bytes(), owner_count as u64);

    let released = tickets.pop().expect("one ticket");
    ledger.release(&released).expect("single owner release");

    assert_eq!(
        ledger.last_release_preflight_axis_probes, 3,
        "one release probes exactly one class/kind, owner/kind and context/kind slot"
    );
    assert_eq!(
        ledger.owner_usage.len(),
        owner_count - 1,
        "unaffected owner cardinality is preserved"
    );
    assert_eq!(ledger.total_semantic_bytes(), (owner_count - 1) as u64);
    assert_eq!(ledger.non_headroom_bytes(), (owner_count - 1) as u64);
    assert_axes_balanced(&ledger.snapshot());

    ledger.release_batch(&tickets).expect("drain remaining tickets");
    assert_eq!(ledger.total_semantic_bytes(), 0);
    assert_eq!(ledger.non_headroom_bytes(), 0);
    assert_axes_balanced(&ledger.snapshot());
}

// The all-public snapshot fields make an externally assembled snapshot
// forgeable: the panicking total accessors document that contract, and
// the checked companions are the safe path for untrusted snapshots.

// ---- scenario A: bytes > i64::MAX must not wrap to a negative delta ----

// ---- scenario A: cross-ledger ticket misuse is rejected by the idempotency guard ----

// ---- scenario A: next_ticket_id is monotonic and unique across N reserves ----

// ---- scenario A: context axis balances against owner axis in a mixed sequence ----

// ---- scenario A: wrong-kind/mixed-kind release keeps the partition (invariant 2) ----

// ---- scenario A: deterministic property-style fuzz keeps invariants at every step ----

// --- FUP-004: per-axis kind validation ---------------------------------
//
// Scalar axis totals can balance while the kind split diverges on one
// axis. The snapshot validator must compare kind vectors per axis, not
// just scalar totals plus an owner-side re-sum.

fn sep23_mixed_kind_ledger() -> AccountingLedger {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
    for (class, owner, context, kind, bytes) in [
        (
            MemoryClass::ArtifactCache,
            "owner:a",
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1_000,
        ),
        (
            MemoryClass::EvidenceCache,
            "owner:a",
            MemoryContextKind::Session,
            ChargeKind::RetainedMetadata,
            250,
        ),
        (
            MemoryClass::ArtifactCache,
            "owner:b",
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            500,
        ),
    ] {
        let ticket = ledger
            .reserve(class, owner.to_string(), context, kind, bytes)
            .expect("mixed-kind reserve");
        ledger.commit(&ticket).expect("commit");
    }
    ledger
}

// Self-audit P2-5: recharge and batch release move kind bytes on every
// axis exactly — pinned with full kind vectors, not just scalar totals.
fn sep23_audit_mixed_kind_ledger_with_tickets() -> (AccountingLedger, Vec<ReservationTicket>) {
    let mut ledger = ledger_from_policy(ResidencyPolicy::new(1_000_000));
    let mut tickets = Vec::new();
    for (class, owner, context, kind, bytes) in [
        (
            MemoryClass::ArtifactCache,
            "owner:a",
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1_000,
        ),
        (
            MemoryClass::EvidenceCache,
            "owner:a",
            MemoryContextKind::Session,
            ChargeKind::RetainedMetadata,
            250,
        ),
        (
            MemoryClass::ArtifactCache,
            "owner:b",
            MemoryContextKind::Request,
            ChargeKind::LiveScratch,
            500,
        ),
    ] {
        let ticket = ledger
            .reserve(class, owner.to_string(), context, kind, bytes)
            .expect("mixed-kind reserve");
        ledger.commit(&ticket).expect("commit");
        tickets.push(ticket);
    }
    (ledger, tickets)
}

fn sep23_audit_kind_vectors(snap: &LedgerSnapshot) -> ([u64; 3], [u64; 3], [u64; 3]) {
    (
        LedgerSnapshot::checked_kind_totals(&snap.class_usage).expect("class kinds"),
        LedgerSnapshot::checked_kind_totals(&snap.owner_usage).expect("owner kinds"),
        LedgerSnapshot::checked_kind_totals(&snap.context_usage).expect("context kinds"),
    )
}
