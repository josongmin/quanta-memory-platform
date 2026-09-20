# QMP-040 — Store typestate, explicit close, and lock-safe accounting

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-020, QMP-030  
Blocks: QMP-050, QMP-060, QMP-070, QMP-090, QMP-100, QMP-110

## Objective

모든 facade store에서 accounting mode를 construction 전에 고정하고, store 종료 시 live charge를 회수하며, public accounting callback을 container mutex 안에서 실행하지 않게 한다.

## Affected stores

- payload store
- retained map
- revocable store
- transcript store
- prepared publication service

## Confirmed evidence

- charged entry마다 ledger receipt가 있지만 facade에 `Drop`/`close` 구현이 없다.
- payload/revocable/transcript/prepared는 populated object에 `charged_against`를 적용할 수 있다.
- retained map만 builder-only charged construction을 사용한다.
- accounting callback이 store mutex 아래서 실행된다.
- payload/revocable의 saturating counters가 overflow를 숨긴다.
- payload/revocable/transcript 일부 stats는 poison을 zero로 반환한다.

## Canonical write set

- `facade/src/shared/{builder,accounting}.rs`
- `facade/src/payload_store/{mod,service}.rs`
- `facade/src/retained_map/{mod,service}.rs`
- `facade/src/revocable_store/{mod,service}.rs`
- `facade/src/transcript_store/{mod,service}.rs`
- `facade/src/publish_pipeline/prepared_publication.rs`
- facade/testkit store tests

QMP-050은 byte cost 산정만 소유하고, 이 ticket은 lifecycle/choreography를 소유한다.

Owner symbols and mutation call sites:

- `AccountingSink::{charge, refund, refund_many, recharge}`
- `SimplePayloadStore::{new, charged_against}` and `PayloadStore::{put_canonical, delete_if_unpinned, stats}`
- `SimpleRetainedMapStore::{new, new_charged}` and `RetainedMapStore::{insert_checked, remove, sweep, stats_checked}`
- `SimpleRevocableStore::{new, charged_against}` and `RevocableStore::{insert_checked, reclaim_one}`
- `SimpleSessionTranscriptStore::{new, charged_against}` and transcript append/retain/truncate/stats paths
- `PreparedPublicationService::{new, charged_against}` and prepare/abort/GC paths

## Required construction model

1. `Budgeted`와 `Unbudgeted`를 다른 builder state/type으로 표현한다.
2. sink는 materialization 전에 선택한다.
3. constructed store에는 public `charged_against(self, ...)`가 없다.
4. unbudgeted constructor는 `new_unbudgeted`처럼 이름으로 드러나고 production feature에서 금지/격리 가능해야 한다.
5. public `Simple*::new`가 builder-first를 우회하지 못한다.

## Required lifecycle model

1. `close/drain`은 live entries의 charge를 batch로 release하고 store를 closed state로 전환한다.
2. close 실패 시 store entry와 charge ownership이 함께 보존된다.
3. close 성공 후 mutation은 typed `Closed` error다.
4. `Drop` 정책을 명시한다.
   - preferred: already-closed assertion + best-effort emergency cleanup telemetry
   - silent ledger leak 또는 panic 금지
5. multiple logical clones/Arc ownership이 가능하면 마지막 owner만 close하는 구조를 명시한다.

## Lock choreography

외부 sink callback을 mutex 안에서 호출하지 않는다. 권장 protocol:

```text
lock -> read version + prepare immutable mutation plan -> unlock
accounting operation outside store lock
lock -> verify version/state -> apply
conflict -> compensate accounting -> retry or typed conflict
```

다른 허용 설계는 sink를 sealed/trusted non-reentrant authority로 좁히고 global lock order를 type/module boundary에서 강제하는 경우뿐이다.

## Counter/error requirements

- retained byte add/sub는 checked arithmetic을 사용한다.
- overflow/underflow는 mutation 전에 typed error다.
- `stats()`가 poison을 empty로 위장하지 않는다.
- infallible legacy stats가 필요하면 `Unknown/Unavailable`을 표현하는 별도 observation type을 사용한다.

## RED tests to add

Construction:

- 각 store별 `cannot_attach_sink_after_materialization` compile-fail
- `budgeted_builder_requires_sink_before_build`
- `unbudgeted_store_type_cannot_be_passed_as_budgeted_store`

Lifecycle:

- 각 store별 `close_refunds_every_live_entry_exactly_once`
- `close_refund_failure_preserves_store_and_ledger_ownership`
- `drop_without_explicit_close_does_not_strand_capacity`
- `mutation_after_close_is_rejected`
- `double_close_is_idempotent_or_typed_without_double_refund`

Concurrency/failure:

- `reentrant_accounting_sink_does_not_deadlock`
- `concurrent_mutation_and_close_has_one_linearized_outcome`
- `counter_overflow_is_rejected_without_mutation`
- `poisoned_stats_is_error_not_empty`

## Acceptance

- [ ] late attach API가 public surface에서 사라진다.
- [ ] charged store가 종료된 뒤 ledger/group usage가 정확히 zero다.
- [ ] close failure가 ticketless entry나 phantom refund를 만들지 않는다.
- [ ] public dyn callback이 store mutex 아래서 호출되지 않는다.
- [ ] all stores가 동일 lifecycle contract/conformance suite를 통과한다.
- [ ] checked overflow와 poison truth가 trait level에 반영된다.

## Verification

```bash
cargo test -p quanta-memory-platform-facade --all-features close_refunds_every_live_entry_exactly_once
cargo test -p quanta-memory-platform-facade --all-features reentrant_accounting_sink_does_not_deadlock
cargo test -p quanta-memory-platform-facade --all-features
cargo test -p quanta-memory-platform-testkit --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Non-goals

- key/metadata byte formula: QMP-050
- tombstone retention policy: QMP-060
- spill ownership transfer: QMP-070
- performance container rewrite: QMP-100
