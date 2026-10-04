mod borrowed_zero_cap_reclaim;
mod cases_01;
mod cases_02;
mod cases_03;
mod cases_04;
mod cases_05;
mod cases_06;
mod cases_07;
mod cases_08;
mod live_class_authority;

use super::*;
use crate::api::broker::ProcessMemoryBroker;
use crate::domain::context::RuntimeKind;
use crate::domain::policy::{
    MemoryClassBudget, OwnerQuota, ReclaimPriority, ResidencyPolicy, ResourceGroup,
};
use crate::domain::stats::PressureState;
use crate::ports::scratch::ScratchRequest;

fn report_with(mut residency: ResidencyPolicy) -> EffectiveMemoryBudgetReport {
    // Most control-plane tests isolate global/owner/group behavior. Expand
    // only the constructor's explicit zero defaults into permissive test
    // caps; a removed class stays removed so totality tests still bite.
    let global = residency.global_hard_limit_bytes;
    for budget in residency.classes.values_mut() {
        if budget.reserved_bytes == 0 && budget.soft_cap_bytes == 0 && budget.hard_cap_bytes == 0 {
            budget.soft_cap_bytes = global;
            budget.hard_cap_bytes = global;
        }
    }
    EffectiveMemoryBudgetReport {
        preset: crate::domain::policy::MemoryPreset::Production,
        execution: crate::domain::policy::ExecutionBudget::default(),
        residency,
        override_sources: vec!["test".to_string()],
        override_provenance: Vec::new(),
        policy_revision: 1,
        active_policy_source: "test".to_string(),
    }
}

fn class_budget(reserved: u64, soft: u64, hard: u64, borrow: bool) -> MemoryClassBudget {
    MemoryClassBudget {
        reserved_bytes: reserved,
        soft_cap_bytes: soft,
        hard_cap_bytes: hard,
        borrow_from_shared_pool: borrow,
        reclaim_priority: ReclaimPriority::Normal,
    }
}

fn base_residency() -> ResidencyPolicy {
    ResidencyPolicy::new(1_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 400_000, 600_000, false))
        .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 400_000, 600_000, false))
        .with_owner_quota(
            "owner:a",
            OwnerQuota { soft_cap_bytes: 300_000, hard_cap_bytes: 500_000 },
        )
}

fn broker() -> LiveProcessMemoryBroker {
    LiveProcessMemoryBroker::from_report(report_with(base_residency()))
        .expect("base residency is valid")
}

fn managed_charge(broker: &LiveProcessMemoryBroker, bytes: u64) -> ManagedCharge {
    broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            "owner:a".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            bytes,
        )
        .expect("reserve managed charge")
}

// --- Invariant 1 & 2: class total == owner total == payload+metadata+scratch

// --- Negative path 1: reserve denied BEFORE mutation (owner hard cap)

// --- Negative path 3: class hard cap exceeded when class may not borrow

// --- #6 at the broker surface: borrowing past the burst pool is denied

// --- Invariant: release subtracts and keeps balance

// --- Negative path 6: rebuild-in-progress rejects writes

// --- Invariant 5: denied live-reconfig leaves old active policy untouched

// --- #5: a class holding live bytes cannot be silently dropped (fail-open)

// --- #5: an owner quota below that owner's live usage is denied

// --- audit fix: raising headroom over live non-Headroom usage is denied
// (invariant 5 band must keep holding for the live state, not just future
// reserves). Without the check, the reconfigure would silently strand 800k of
// non-Headroom bytes inside the newly reserved headroom band.

// --- audit fix: lowering a resource-group hard cap below the group's live
// usage is denied (symmetric with the class/owner/headroom live-usage checks);
// without it the reconfigure would install a group cap the group already breaks.

// --- positive pair: a legal reconfig (raising caps) is accepted

// --- Invariant 6: allocator telemetry mismatch does not mutate accounting

// --- pressure classification is semantic, not allocator-driven

// === Scenario group C: control-plane extras ===========================

// scenario C: reconfigure atomicity. A DENIED try_live_reconfigure must
// leave scheduler + controller + report ALL unchanged. We capture the full
// observable state (effective_report residency, ledger snapshot, pressure
// classification, scheduler admission) BEFORE the denied op and assert each
// is identical AFTER -- fail-closed, all-or-nothing.

// scenario C: an ACCEPTED legal raise rebuilds derived state. Usage that was
// Soft under the old global limit must become Normal under a larger one,
// proving the controller was rebuilt from the new policy (not stale).

// scenario C: pressure threshold boundaries. Default thresholds are
// soft=75%, hard=90%, fatal=98% and the classifier uses `>=` (inclusive
// lower bound) on `used*100/limit`. We drive usage to exactly the boundary
// fractions of the 1_000_000 global limit and pin the precise transitions,
// documenting the >= (not >) behavior: 750k (75%) is Soft, 749_999 (<75%)
// is Normal; 900k (90%) is Hard; 980k (98%) is Fatal.

// scenario C (new fence contract): the fence is a single-owner authority.
// A second live begin is rejected; the owning permit finishes exactly
// once (a second finish has no permit to consume); a dropped permit
// abandons the fence closed and observably, and an explicit re-begin
// recovers it. Supersedes the old boolean idempotence contract.

// scenario C (authority hardening): the rebuild fence must have ONE owner.
// A second broker clone must not be able to begin over a live fence, and
// the tokenless finish path is structurally gone: no public API lets a
// clone open another coordinator's fence.
// An exhausted epoch must fail closed WITHOUT mutating fence state: an
// abandoned fence stays observably abandoned and a fresh fence is never
// opened, so no Err path can strand a fence with no permit.

// scenario C (fence gates the slow path): a capacity breach detected
// before the rebuild fence must NOT run the host reclaim callback or be
// reported as a terminal Capacity denial while the fence is active. The
// fence is a non-capacity refusal no callback can cure, so it wins the
// preflight chain.

// Live reconfigure must not silently uncap an owner: removing an owner's
// quota while that owner holds live bytes or tickets is denied, exactly
// like dropping a live class from the policy.

/// [`managed_charge`](self::managed_charge) with a caller-selected owner.
fn managed_charge_owner(
    broker: &LiveProcessMemoryBroker,
    owner: &str,
    bytes: u64,
) -> MemoryResult<ManagedCharge> {
    broker.reserve_managed_charge(
        MemoryClass::ArtifactCache,
        owner.to_string(),
        MemoryContextKind::Request,
        ChargeKind::RetainedPayload,
        bytes,
    )
}

// The port contract bounds reentrant mutation from this broker's own
// reclaimer callback: a synchronous reserve fails with
// OperationInProgress. A reentrant recharge is the same synchronous
// release+reserve+commit swap and must be bounded identically.

// scenario C (IMPL): the ProcessMemoryBroker::effective_report trait impl
// must return the SAME real report as the inherent method on the healthy
// path, and must NOT fabricate a fictitious Production budget. We assert the
// two are byte-for-byte equal and that the report is the one we built from
// (a 1M global limit), proving the fabricated 4GiB Production fallback is
// gone.

// --- D: resource-group admission gates the reserve, fail-closed ---

// --- ExecutionBudget: the memory-relevant scratch cap is enforced ---

fn runtime_scratch_broker(cap: usize) -> LiveProcessMemoryBroker {
    let report = EffectiveMemoryBudgetReport {
        execution: crate::domain::policy::ExecutionBudget {
            max_execution_scratch_bytes: Some(cap),
            ..crate::domain::policy::ExecutionBudget::unlimited()
        },
        ..report_with(ResidencyPolicy::new(10_000_000))
    };
    LiveProcessMemoryBroker::from_report(report).expect("valid scratch broker")
}

// A recharge whose NEW live-scratch charge exceeds the execution scratch budget
// (not a class/owner cap) must also be denied AND restore the old scratch charge
// — proving the restore path is exercised for the scratch-budget denial source,
// not just cap denials.

// --- J: concurrency (broker is Arc<Mutex>, Clone) ---

// --- audit fix [2]: an accepted live reconfigure must NOT reset scheduler
// group usage to zero (phantom-empty groups would re-open admission).

// --- audit fix [0]: the atomic charge primitive fails closed during rebuild
// (no inter-lock window can strand reserved bytes behind a discarded ticket).

// reserve_committed on the happy path charges exactly once (reserve+commit).

// --- audit fix [3]/[8]/[28]: atomic recharge (in-place overwrite) ---

// A DENIED recharge whose NEW charge kind differs from the OLD must restore on
// the OLD axis (kind), not the new one. A wrong-axis restore would keep owner
// totals balanced (so the existing test passes) yet move bytes to the wrong
// partition — this pins the partition, not just the grand total.

// A DENIED recharge on a grouped owner must preserve the old charge and keep
// scheduler group usage unchanged — no release/restore drift.

fn poison_broker_lock(broker: &LiveProcessMemoryBroker) {
    let clone = broker.clone();
    let panicked = std::thread::spawn(move || {
        let _guard = clone.inner.lock().expect("lock");
        panic!("poison broker lock for test");
    })
    .join();
    assert!(panicked.is_err(), "poison helper must panic while holding the lock");
}

#[derive(Default)]
struct RecordingEventSink {
    events: Mutex<Vec<MemoryEvent>>,
    broker: Mutex<Option<LiveProcessMemoryBroker>>,
}

impl MemoryEventSink for RecordingEventSink {
    fn emit(&self, event: MemoryEvent) {
        if let Some(broker) = self.broker.lock().expect("recorder broker lock").as_ref() {
            // This would deadlock if the broker invoked the sink while
            // holding its mutex.
            broker.effective_report().expect("event callback runs outside broker lock");
        }
        self.events.lock().expect("recorder events lock").push(event);
    }
}

struct PanickingEventSink;

impl MemoryEventSink for PanickingEventSink {
    fn emit(&self, _event: MemoryEvent) {
        panic!("telemetry failure");
    }
}

struct CountedPanicPayload {
    drops: Arc<std::sync::atomic::AtomicUsize>,
    panic_on_drop: bool,
}

impl Drop for CountedPanicPayload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, AtomicOrdering::SeqCst);
        if self.panic_on_drop {
            panic!("host panic payload destructor");
        }
    }
}

struct PayloadPanickingSink {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    drops: Arc<std::sync::atomic::AtomicUsize>,
    panic_on_drop: bool,
}

impl MemoryEventSink for PayloadPanickingSink {
    fn emit(&self, _event: MemoryEvent) {
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        std::panic::panic_any(CountedPanicPayload {
            drops: self.drops.clone(),
            panic_on_drop: self.panic_on_drop,
        });
    }
}

struct PayloadPanickingReclaimer {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    drops: Arc<std::sync::atomic::AtomicUsize>,
    panic_on_drop: bool,
}

impl MemoryReclaimer for PayloadPanickingReclaimer {
    fn reclaim(
        &self,
        _request: &ReclaimRequest,
    ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        std::panic::panic_any(CountedPanicPayload {
            drops: self.drops.clone(),
            panic_on_drop: self.panic_on_drop,
        });
    }
}

struct RecursiveObserveEventSink {
    broker: Mutex<Option<LiveProcessMemoryBroker>>,
    calls: std::sync::atomic::AtomicUsize,
}

impl MemoryEventSink for RecursiveObserveEventSink {
    fn emit(&self, _event: MemoryEvent) {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let broker = self.broker.lock().expect("broker slot").clone();
        if let Some(broker) = broker {
            std::thread::spawn(move || {
                broker.observe().expect("read observation remains available in event callback");
            })
            .join()
            .expect("nested event observation thread");
        }
    }
}

struct RecursiveReserveReclaimer {
    broker: Mutex<Option<LiveProcessMemoryBroker>>,
    calls: std::sync::atomic::AtomicUsize,
    nested_error: Mutex<Option<MemoryError>>,
}

impl MemoryReclaimer for RecursiveReserveReclaimer {
    fn reclaim(
        &self,
        _request: &ReclaimRequest,
    ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let broker = self.broker.lock().expect("broker slot").clone();
        if let Some(broker) = broker {
            let nested = broker.reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "nested-reclaim".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1,
            );
            *self.nested_error.lock().expect("nested error") = nested.err();
        }
        Ok(crate::ports::reclaim::ReclaimOutcome::default())
    }
}

struct ThreadSpawnReclaimer {
    broker: Mutex<Option<LiveProcessMemoryBroker>>,
    calls: std::sync::atomic::AtomicUsize,
    nested_error: Mutex<Option<MemoryError>>,
}

impl MemoryReclaimer for ThreadSpawnReclaimer {
    fn reclaim(
        &self,
        _request: &ReclaimRequest,
    ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let broker = self.broker.lock().expect("broker slot").clone();
        if let Some(broker) = broker {
            let nested = std::thread::spawn(move || {
                broker.reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    "thread-reentry".into(),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    1,
                )
            })
            .join()
            .expect("nested reserve thread");
            *self.nested_error.lock().expect("nested error") = nested.err();
        }
        Ok(crate::ports::reclaim::ReclaimOutcome::default())
    }
}

#[derive(Default)]
struct RecordingReclaimer {
    requests: Mutex<Vec<ReclaimRequest>>,
    charge_to_release: Mutex<Option<ManagedCharge>>,
    broker_for_reentry: Mutex<Option<LiveProcessMemoryBroker>>,
    reconfigure_inside: Mutex<Option<ResidencyPolicy>>,
    reconfigure_attempts: Mutex<Vec<MemoryResult<()>>>,
    forged_reported_bytes: Mutex<Option<u64>>,
}

impl MemoryReclaimer for RecordingReclaimer {
    fn reclaim(
        &self,
        request: &ReclaimRequest,
    ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
        self.requests.lock().expect("requests").push(request.clone());
        if let Some(broker) = self.broker_for_reentry.lock().expect("broker").as_ref() {
            broker.effective_report().expect("callback must run outside broker lock");
            if let Some(policy) = self.reconfigure_inside.lock().expect("policy").take() {
                let outcome = broker.try_live_reconfigure(policy);
                self.reconfigure_attempts.lock().expect("attempts").push(outcome);
            }
        }
        // A forged report lies about reclaimed bytes while freeing nothing:
        // telemetry records the lie, but admission must remeasure the
        // ledger and never trust it.
        if let Some(forged) = self.forged_reported_bytes.lock().expect("forged").take() {
            return Ok(crate::ports::reclaim::ReclaimOutcome { reclaimed_bytes: forged });
        }
        let mut reclaimed = 0;
        if let Some(mut charge) = self.charge_to_release.lock().expect("charge").take() {
            reclaimed = charge.bytes().expect("live charge");
            charge.release().expect("callback release");
        }
        Ok(crate::ports::reclaim::ReclaimOutcome { reclaimed_bytes: reclaimed })
    }
}

struct PanickingReclaimer {
    calls: Mutex<u32>,
}

impl MemoryReclaimer for PanickingReclaimer {
    fn reclaim(
        &self,
        _request: &ReclaimRequest,
    ) -> MemoryResult<crate::ports::reclaim::ReclaimOutcome> {
        *self.calls.lock().expect("calls") += 1;
        panic!("reclaimer panic");
    }
}

// --- FUP-007: zero-byte ticket scope lifetime ---------------------------
//
// A zero-byte ticket holds live release/rollback authority without any
// positive usage entry. Live reconfigure must treat "zero usage" as
// authority-unknown and consult the exact live-ticket owner index instead.

fn sep23_grouped_residency() -> ResidencyPolicy {
    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 50_000,
        hard_cap_bytes: 100_000,
        burst_percent: 100,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
        .with_resource_group("g", group)
        .with_owner_group("owner:g", "g")
}

fn sep23_ungrouped_residency() -> ResidencyPolicy {
    ResidencyPolicy::new(10_000_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 200_000, 300_000, false))
}

fn sep23_grouped_broker() -> LiveProcessMemoryBroker {
    LiveProcessMemoryBroker::from_report(report_with(sep23_grouped_residency())).expect("valid")
}

fn sep23_zero_committed(broker: &LiveProcessMemoryBroker, owner: &str) -> ManagedCharge {
    broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            owner.to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            0,
        )
        .expect("zero-byte committed charge")
}

fn sep23_assert_fully_drained(broker: &LiveProcessMemoryBroker) {
    let snapshot = broker.ledger_snapshot().expect("snapshot");
    snapshot.validate_exact().expect("axes balance after drain");
    assert_eq!(snapshot.total_owner_bytes(), 0, "semantic counters restored");
    assert_eq!(snapshot.total_class_bytes(), 0);
    assert_eq!(snapshot.total_context_bytes(), 0);
    assert_eq!(snapshot.live_ticket_count, 0, "no live ticket remains");
    let core = broker.inner.lock().expect("broker lock");
    assert!(core.ticket_attribution.is_empty(), "no issuing-scope refs remain");
    assert!(core.deferred_committed_releases.is_empty(), "no deferred committed backlog");
    assert!(core.deferred_pending_rollbacks.is_empty(), "no deferred pending backlog");
}

// --- FUP-005: bounded reconfigure provenance ---------------------------
//
// The active-residency provenance is one CURRENT record, not an
// ever-growing journal: N accepted reconfigures add at most one entry on
// top of the initial resolver provenance.

// --- FUP-003: admission preflight + reclaim plans ------------------------
//
// The preflight decides BEFORE any callback: terminally impossible and
// blocked requests fail without touching the host, a predicted failure
// goes straight to the hard plan, and only an admissible request that
// crosses a soft threshold gets a proactive soft request.

fn sep23_fup003_matrix_broker(reclaimer: &Arc<RecordingReclaimer>) -> LiveProcessMemoryBroker {
    let policy = ResidencyPolicy::new(1_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800, 1_000, false))
        .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false))
        .with_owner_quota("owner:quota", OwnerQuota { soft_cap_bytes: 800, hard_cap_bytes: 900 })
        .with_resource_group(
            "g",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 800,
                hard_cap_bytes: 900,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("owner:grouped", "g");
    let mut report = report_with(policy);
    report.execution.max_execution_scratch_bytes = Some(50);
    LiveProcessMemoryBroker::from_report_with_reclaimer(report, Some(reclaimer.clone()))
        .expect("matrix broker")
}

// --- FUP-006: reclaim x reconfigure x release coupling -------------------
//
// The full-followup candidate must hold together when a reclaim callback
// reconfigures the live policy AND releases a charge in one callback: the
// remeasure admits under exactly one policy generation, and the initial /
// final revision, decision receipt, ledger axes and scheduler group usage
// all agree afterwards.

fn sep23_fup006_grouped_policy(class_hard: u64) -> ResidencyPolicy {
    ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, class_hard, false))
        .with_resource_group(
            "g",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 90,
                hard_cap_bytes: 100,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("owner:r", "g")
}

// --- Self-audit: max-scale exact fill + 1 byte denies clean -------------
//
// At u64::MAX live usage the next byte is a ReclaimCandidate (the byte
// alone fits everywhere), but the hard plan cannot even be constructed:
// the prospective aggregate is unrepresentable. Plan construction fails
// BEFORE any callback fires, the broker stays usable, and the exact fill
// still drains to zero.

// --- Self-audit P2-2: deferred drains never discard authority -----------
//
// The deferred queues only fill on an internal invariant break (or a
// rebuild race), so no public flow seeds them — but if a drain retry
// still fails, the exact ticket must stay queued for a later retry, never
// silently dropped. A foreign ticket (wrong ledger identity) is the
// deterministic stand-in for an unfailable rollback.

fn sep23_audit_seed_deferred_foreign_ticket() -> ReservationTicket {
    let foreign = LiveProcessMemoryBroker::from_report(report_with(base_residency()))
        .expect("foreign broker");
    foreign
        .reserve(
            MemoryClass::ArtifactCache,
            "owner:foreign".to_string(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            64,
        )
        .expect("foreign ticket")
}

// --- Self-audit P2-4: FUP-003 contract gaps ------------------------------
//
// The audit's group-ladder probe, wrong-victim release, forged telemetry,
// callback panic, and 0-byte silence each get a pinned regression, plus
// the randomized preflight/attempt agreement test the admission module
// docstring promises.

fn sep23_audit_group_ladder_policy(group_soft: u64) -> ResidencyPolicy {
    ResidencyPolicy::new(10_000)
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 9_000, 10_000, false))
        .with_resource_group(
            "g",
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: group_soft,
                hard_cap_bytes: 50,
                burst_percent: 0,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        )
        .with_owner_group("owner:r", "g")
}

// --- Randomized preflight/attempt agreement (FUP-003) --------------------
//
// The admission module promises this test: the preflight verdict and the
// no-callback attempt must agree on outcome AND byte-identical error for
// every request, and every TerminalDenial must also deny on a fresh
// broker (proving no callback could help). Deterministic xorshift stream,
// no threads, no timing.

struct Sep23XorShift64(u64);

impl Sep23XorShift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn sep23_agreement_policy(shape: u64) -> ResidencyPolicy {
    let group = ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: 800,
        hard_cap_bytes: 900,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    };
    match shape {
        0 => ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800, 1_000, false))
            .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false)),
        1 => ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800, 1_000, false))
            .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false))
            .with_resource_group("g", group)
            .with_owner_group("owner:r", "g"),
        2 => ResidencyPolicy::new(1_000)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 800, 1_000, false))
            .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false))
            .with_owner_quota("owner:q", OwnerQuota { soft_cap_bytes: 800, hard_cap_bytes: 900 }),
        _ => ResidencyPolicy::new(10_000)
            .with_shared_burst_pool_bytes(50)
            .with_class_budget(MemoryClass::ArtifactCache, class_budget(0, 90, 100, true))
            .with_class_budget(MemoryClass::EvidenceCache, class_budget(0, 100, 100, false)),
    }
}
