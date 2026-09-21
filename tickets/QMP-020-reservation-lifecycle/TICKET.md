# QMP-020 — Reservation typestate, commit-once, and abandoned rollback

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-010  
Blocks: QMP-030, QMP-040, QMP-070, QMP-080, QMP-090

## Objective

현재 clone 가능한 ticket ID를 명시적 `Reserved -> Committed -> Released` ownership state machine으로 바꾸고, duplicate commit과 abandoned reservation capacity leak를 type/runtime 양쪽에서 차단한다.

## Confirmed evidence

- `ReservationTicket`은 clone 가능: `core/src/domain/ledger.rs:53-67`
- `live_tickets`는 issued/not-released ID만 저장: `ledger.rs:228-233`
- `commit`은 state mutation 없이 bytes만 반환: `ledger.rs:407-436`
- repeated commit/abandoned reservation test 없음.
- facade store entry drop에도 automatic release owner가 없음.

## Required state model

```text
reserve()
  -> PendingReservation        !Clone
       commit(self)
         -> CommittedCharge    single owner
       drop before commit
         -> rollback reservation

CommittedCharge
  -> transfer(self, destination)
  -> replace(self, new_cost)
  -> release(self)
  -> explicit close owner policy
```

구체 type 이름은 변경 가능하지만 다음 성질은 불변이다.

- commit은 최대 한 번만 성공한다.
- pending drop은 ledger/scheduler capacity를 반환한다.
- committed ownership은 중복 clone되지 않는다.
- release/replace/transfer는 old authority를 소비한다.
- failure는 old authority 또는 new authority 중 정확히 하나를 호출자에게 돌려준다.

## Canonical write set

- `crates/quanta-memory-platform-core/src/domain/ledger.rs`
- `crates/quanta-memory-platform-core/src/domain/recharge.rs`
- `crates/quanta-memory-platform-core/src/api/control.rs`
- `crates/quanta-memory-platform-core/src/lib.rs`
- focused core tests

Facade migration은 QMP-040/QMP-070이 소유한다. 이 ticket은 core primitive와 compatibility bridge까지만 소유한다.

Owner symbols:

- `ReservationTicket`
- `AccountingLedger::{reserve, commit, release, release_many, replace_committed}`
- `AccountingLedger::{validate_release_ticket, apply_release, restore_released_ticket_v1}`
- `LiveProcessMemoryBroker::{reserve, reserve_committed, commit, release, release_many, recharge}`
- `RechargeRecoveryV1`

## Required implementation

1. ledger 내부 ticket record에 explicit state를 둔다.
2. `commit`은 pending만 accepted하고 state를 committed로 전환한다.
3. `reserve_committed`는 중간 ticket을 외부에 노출하지 않고 committed owner를 반환한다.
4. pending rollback은 rebuild fence 때문에 실패해서는 안 된다. rollback은 신규 write가 아니라 미완료 reservation 취소다.
5. batch release는 duplicate/cross-ledger/stale receipt를 mutation 전에 전부 validate한다.
6. recharge/replace는 old consumed/new returned semantics를 type으로 표현한다.
7. ticket ID saturation은 wrap/reuse하지 않고 typed exhaustion을 반환한다.
8. temporary compatibility API가 필요하면 deprecated wrapper로 두고 raw clone authority는 노출하지 않는다.

## Failure handling

- commit-after-commit: typed invalid state, counters unchanged
- commit-after-release: typed invalid state, counters unchanged
- pending drop during rebuild: reservation rollback succeeds
- replace denial: old committed charge intact
- release batch에 duplicate: entire batch unchanged
- lock poison: fabricated release/zero state 금지

## RED tests to add

- `second_commit_of_same_reservation_is_rejected`
- `dropping_pending_reservation_rolls_back_all_axes`
- `dropping_pending_reservation_during_rebuild_does_not_strand_capacity`
- `committed_charge_cannot_be_cloned` compile-fail
- `release_consumes_committed_charge` compile-fail
- `replace_denial_returns_original_live_charge`
- `ticket_id_exhaustion_fails_without_reuse`
- `pending_drop_restores_resource_group_capacity`
- property sequence: reserve/commit/drop/release/replace 후 exact model equality

## Acceptance

- [x] state transition이 한 canonical owner에 구현된다.
- [x] duplicate commit 성공이 불가능하다.
- [x] abandoned pending reservation이 capacity를 남기지 않는다.
- [ ] class/owner/context/kind/group axes가 모든 transition에서 정확한 byte 총합을 유지한다. P1-D01 수정은 `59f0c4f`에 통합됐으나 통합 workspace/extraction·host compatibility 증거가 아직 없다.
- [x] public store가 raw clone ticket에 의존하지 않도록 migration path가 문서화된다.
- [x] existing recharge fail-closed semantics가 유지된다.

## Verification

```bash
cargo test -p quanta-memory-platform-core --all-features second_commit_of_same_reservation_is_rejected
cargo test -p quanta-memory-platform-core --all-features dropping_pending_reservation_rolls_back_all_axes
cargo test -p quanta-memory-platform-core --all-features
cargo test -p quanta-memory-platform-core --all-features --test wave1_policy_contracts
cargo clippy -p quanta-memory-platform-core --all-targets --all-features -- -D warnings
```

## SOTA constraint

DataFusion reservation과 Tokio owned permit처럼 capacity authority를 owner lifetime과 결합한다. 단, committed store charge의 fallible close를 무조건 `Drop` 하나로 숨기지 않는다.

## Non-goals

- facade store close policy: QMP-040
- spill transfer choreography: QMP-070
- runtime view integration: QMP-030

## Receipt

- Product implementation commit: `783d0bbfdf32efad97103b1dc6d8ac774abaa9ed`
- Receipt: [`receipts/2026-09-20T184809Z-783d0bb.md`](receipts/2026-09-20T184809Z-783d0bb.md)
- Historical receipt는 당시 reservation typestate 및 focused tests에만 유효하다. P1-D01의 RED로 ticket을 재개방했고, 격리 수정 `4533146`을 `59f0c4f`에 통합했다. QMP-040/050의 과거 receipt를 파기하지 않지만 QMP-020 및 의존 ticket의 terminal qualification은 fresh integrated proof 전 금지한다.
- P1-D01 scoped implementation receipt: [`receipts/2026-09-21T001438Z-59f0c4f.md`](receipts/2026-09-21T001438Z-59f0c4f.md). Isolated core 177+7+14+11 GREEN, main integration complete; clean integrated doctor/extraction and external host migration are separate gates. Status `IMPLEMENTED`, not `QUALIFIED`.

## Reopened P1-D01 — saturating admission loses a live ticket (2026-09-21)

Historical broken product source `47021d3271c3613c861ad4c06c4d3c2b4b1edb17`: `ledger.rs:344,359-371` used saturating prospective totals, `:156-159` saturated the persisted kind counter, `:184-236` saturated public snapshot totals/balance checks, and `:997-1014` asserted the broken acceptance as GREEN. A validated policy with global/class hard `u64::MAX` admitted `u64::MAX-10` followed by `100`. The latter created a second live ticket but the counters could not represent its cost; releasing the first left the second live while owner usage read `10` rather than `100`. This was semantic accounting corruption, not physical allocation of `u64::MAX` bytes: public reservation magnitudes and caller-provided `ByteSized` permit the path. The current `59f0c4f` contains a code repair; do not report these old line numbers as still reachable code.

Detached RED owner tests on `35bdd8de8c505d70b6559edc6b39743a1a0e4052` (same core source as current HEAD) both failed as expected: `overflow_denial_preserves_exact_live_ticket_accounting_red` observed `Ok(ticket id=2)` where typed capacity denial was required; `accepted_overflow_ticket_is_lost_after_first_release_red` observed owner total `10` with a live `100`-byte ticket. The same isolated fixture's `scratch_reserve_cap_overflow_is_denied_red` and `scratch_recharge_cap_overflow_is_denied_red` also failed: `control.rs:714,561` returned successful authority for a postimage beyond the `u64::MAX` scratch ceiling. `control.rs:943` group reconfigure overflow was inspected but its RED is **not run**. Main tree was not modified by this experiment; the uncommitted RED fixture remains at `/tmp/qmp-overflow-red-X2pSkd/worktree` and is not a product receipt.

Owner-level repair, in order:

1. In ledger admission, compute global/owner/class/kind/context prospective totals with `checked_add` before ticket ID allocation, counter writes, or scheduler mutation; overflow is typed Capacity and leaves the entire preimage untouched. Burst aggregation must also be checked.
2. Replace the saturating counter mutation with an exact, proven-in-range operation; release/rollback must never silently underflow. If public snapshot totals cannot be represented, expose typed checked observation rather than silently reporting saturated balance. Reconcile `control.rs:714,943` scratch/reconfigure and `application/scheduler.rs:47-56,80-96` group saturation with the same exact model; avoid a consumer-side cap shim.
3. Replace `saturating_add_near_u64_max_keeps_class_owner_balanced` with a denial oracle, then assert first ticket still exact after denial, first release drains all axes, failed second release is inert, and a later small reservation works. Add same-kind/different-kind, same-owner/different-owner, grouped owner, pending rollback, recharge replacement and `u64::MAX` boundary cases.
4. Record the observed isolated RED→GREEN owner proof and fresh integrated core/workspace/extraction delta receipt before restoring `IMPLEMENTED`; no original receipt is an overflow qualification. Public scheduler `record_usage`/`record_release` now return `MemoryResult<u64>`; QMP-140 owns SemVer notes and QMP-130 owns external host compile. The full-snapshot validation in hot-path release is a QMP-100 measured performance follow-up, not permission to reintroduce saturation.
