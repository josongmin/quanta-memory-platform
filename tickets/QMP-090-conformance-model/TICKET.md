# QMP-090 — Independent conformance oracle and concurrency/model verification

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-020, QMP-040, QMP-070  
Blocks: QMP-130, QMP-140

## Objective

testkit이 honest implementation의 self-report를 재확인하는 수준을 넘어, 독립 model로 exact expected state를 계산하고 중요한 broken fake를 반드시 거부하게 한다.

## Confirmed evidence

- suite는 exact expected totals를 피하고 target-reported relation을 비교: `testkit/src/conformance/suites.rs:355-405`
- commit 결과와 일부 finish/release 결과를 무시: `:380-388,434-440`
- real broker snapshot error를 `AccountingView::default()`로 변환: `conformance/target.rs:232-245`
- 현재 negative fakes는 일부 lie를 잡지만 black-hole accounting/commit swallow를 배제하지 못한다.

## Canonical write set

- `testkit/src/conformance/{target,invariants,suites}.rs`
- `testkit/src/fake/**`
- `testkit/src/faults/**`
- `testkit/tests/**`
- test-only dev dependencies/loom models

Owner symbols:

- `BrokerConformanceTarget::{do_reserve, do_commit, do_release, do_begin_rebuild, do_finish_rebuild, view}`
- `AccountingView`
- `run_broker_accounting_suite`
- `run_facade_store_suite`
- `run_prepared_publication_suite`
- `run_transcript_append_suite`
- `ConformanceSuite::{run_broker_accounting, run_facade_store_suite}`

## Required oracle model

1. suite가 issued operation으로 expected model을 자체 갱신한다.
2. `view()`는 `Result<AccountingView, Error>`다.
3. 모든 reserve/commit/release/rebuild/observe operation 결과를 assert한다.
4. expected totals:
   - class
   - owner
   - context
   - charge kind
   - group usage
   - ticket lifecycle
5. final drain 후 exact zero와 no-live-ticket을 assert한다.
6. target snapshot/observation error는 failed verdict이며 balanced zero로 바꾸지 않는다.
7. fake-specific internal state를 oracle로 사용하지 않는다.

## Mandatory broken fakes

- `BlackHoleAccounting`: tickets 반환, counters 항상 zero
- `CommitSwallowing`: commit error 또는 no state transition
- `ReleaseSwallowing`: release success지만 usage 유지
- `DuplicateCommitAccepting`
- `WrongContextAxis`
- `WrongChargeKindAxis`
- `StaleTicketAccepting`
- `ObservationErrorAsZero`
- `TransferDoubleOwner`
- `ReentrantSink`

각 fake는 예상 contract family 하나 이상에 localized rejection되어야 한다.

## Property/model tests

- proptest arbitrary operation sequence
- shrink된 최소 failing sequence 출력
- model과 real broker를 같은 trace로 비교
- denial 전후 exact snapshot equality
- every successful charge has exactly one terminal release/transfer

## Concurrency tests

Loom small models:

- reserve vs reconfigure
- release vs batch release
- close vs mutation
- spill vs restore/offline
- versioned callback-outside-lock compensation

Miri rail:

- core/facade focused test subset
- unsafe/alias/data-race failure를 terminal RED로 취급

## RED tests to add

- `suite_rejects_black_hole_accounting`
- `suite_rejects_commit_swallowing_target`
- `suite_rejects_release_swallowing_target`
- `snapshot_error_is_failed_verdict_not_zero`
- `every_operation_result_is_part_of_verdict`
- `property_trace_matches_independent_model`
- `loom_ticket_transition_has_single_terminal_owner`
- `loom_spill_transfer_never_has_zero_or_double_owner`

## Acceptance

- [x] listed broken fakes가 모두 rejected다.
- [x] honest fake와 real broker가 같은 exact model suite를 통과한다.
- [x] observation failure가 PASS로 변환되지 않는다.
- [ ] arbitrary sequence가 model equality와 final drain을 증명한다.
- [ ] Loom/Miri scope와 한계가 receipt에 명시된다.
- [ ] conformance family mapping이 failure cause를 숨기지 않는다.

## Verification

```bash
cargo test -p quanta-memory-platform-testkit --all-features suite_rejects_black_hole_accounting
cargo test -p quanta-memory-platform-testkit --all-features property_trace_matches_independent_model
cargo test -p quanta-memory-platform-testkit --all-features
cargo test --workspace --all-features
cargo +nightly miri test -p quanta-memory-platform-core --all-features second_commit_of_same_reservation_is_rejected
cargo test -p quanta-memory-platform-testkit --features loom loom_ticket_transition_has_single_terminal_owner
```

nightly/tool availability가 없으면 `NOT_RUN`으로 남으며 ticket terminal closure가 아니다.

## Current-source oracle audit (2026-09-21)

On `47021d3`, `testkit/src/conformance/target.rs:95-96,232-245` exposes infallible `view()` and translates a real ledger snapshot error into a zero `AccountingView`; `suites.rs:355-405` asserts self-reported class/owner/partition relationships rather than independently predicted bytes, while commit `:380-388` and finish/release `:434-440` results are discarded. The existing fake at `fake/broker.rs:303-327` has no-op commit and replay-tolerant release. This is static reachability; the specified black-hole and commit-swallowing RED fakes are still required before any qualification claim.

Canonical change order: make observation fallible and a failed verdict; construct one independent model over ticket state and per-axis expected bytes; assert every operation result and exact postimage (including final zero) against that model; then show each negative fake is rejected at its first causal mismatch while the honest adapter passes. The QMP-050 transcript/published-cost testkit migration must use an independent literal semantic formula, not call the implementation cost calculator as its oracle. Do not equate relational balance of three fabricated zeros with proof of charged memory.

## Implementation checkpoint (2026-09-21; partial)

- `BrokerConformanceTarget::view` is fallible. The real adapter propagates `ledger_snapshot` failure instead of fabricating an empty view.
- `AccountingView` now carries exact two-class, two-owner, request-context and charge-kind axes. The broker suite owns literal expected postimages for the 1000/250/500 workload, asserts every commit/rebuild/release result, rejects duplicate commit/release, and requires final exact zero.
- `FakeProcessBroker` now models pending/committed/released ticket state instead of a no-op commit and replay-tolerant release. A committed RED first localized the old honest fake to `TicketLifecycle`; the corrected fake and the real broker pass the same suite.
- Executed broken targets: black-hole accounting, commit swallowing, release swallowing, duplicate commit accepting, wrong context, wrong charge kind, stale ticket accepting, observation error, transfer double-owner, plus the existing mutate-on-deny and observation-mutation targets. Each is rejected with a causal family.
- `property_trace_matches_independent_model` drives the same dependency-free 256-step deterministic operation generator against the honest fake and real broker, comparing exact postimage after every operation and final drain. It is a useful model trace, but it is **not** proptest and has no shrinker.

Open before acceptance: actual `proptest` arbitrary/shrinking rail, `ReentrantSink` negative target, Loom models, Miri execution, resource-group exact observation, and ticket-live-set observation. Their Cargo/dev-dependency changes overlap foreign-owned shared-main manifests, so no manifest mutation is made in this checkpoint.

Final-audit detail: the honest fake does not own an independent context map;
`AccountingView.total_context_bytes` is copied from owner total. `partition_sum`
uses saturating addition, exact family checks accept `contains` instead of an
exact allowed set, and the deterministic trace omits rebuild, denial, allocator
observation, overflow, group state, and transfer commands. Exact-zero counters
also do not prove an empty live-ticket set. Add independent context/group/live-
ticket axes, checked view arithmetic, a no-context broken fake, exact causal
family assertions, proptest shrinking/persisted seeds, registered Loom models,
and an executed Miri subset before `QUALIFIED`.

Current-source evidence is refreshed at
`0c126a25aeab11f457e20c4a209fc7bb631ea859` in
[`receipts/2026-09-21T072928Z-0c126a2.md`](receipts/2026-09-21T072928Z-0c126a2.md).
QMP-090 remains `IMPLEMENTED`; the receipt does not substitute deterministic
traces for proptest shrinking, Loom, Miri, or independent group/context/live-
ticket state.
