# QMP-030 — Enforced default builder and broker-bound runtime view

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-010, QMP-020  
Blocks: QMP-040, QMP-080, QMP-110, QMP-130, QMP-140

## Objective

권장/default public path가 실제 live accounting authority를 만들도록 하고, runtime scratch가 동일 ledger를 통과하게 한다. report-only/unbudgeted 경로는 이름과 type에서 위험이 드러나야 한다.

## Confirmed evidence

- facade default `build()`은 basic broker: `facade/src/shared/builder.rs:75-77`
- core default `build()`은 report-only: `core/src/api/builder.rs:39-55`
- `BasicProcessMemoryBroker`는 report만 저장: `core/src/api/broker.rs:78-100`
- shipped examples가 `.build()` 사용.
- live `attach_runtime`도 `BasicRuntimeMemoryView` 반환: `core/src/api/control.rs:555-562`
- basic scratch allocator는 ledger handle이 없음: `core/src/adapters/scratch.rs:9-27`
- public raw ledger/basic broker constructors가 builder-first claim을 우회.
- facade bumpalo arena는 unrestricted allocator를 public 노출.

## Canonical write set

- `crates/quanta-memory-platform-core/src/api/{builder,broker,control}.rs`
- `crates/quanta-memory-platform-core/src/adapters/scratch.rs`
- `crates/quanta-memory-platform-core/src/ports/{scratch,pin}.rs`
- `crates/quanta-memory-platform-core/src/lib.rs`
- `crates/quanta-memory-platform-facade/src/shared/builder.rs`
- `crates/quanta-memory-platform-facade/src/scratch/bumpalo.rs`
- facade examples and public API tests

Owner symbols and call sites:

- `ProcessMemoryBrokerBuilder::{with_override_layer, build, build_live}`
- `MemoryPlatformBuilder::{build, build_live}`
- `ProcessMemoryBroker::attach_runtime`
- `LiveProcessMemoryBroker::attach_runtime`
- `BasicProcessMemoryBroker::new`
- `BasicRuntimeMemoryView::new`
- `ScratchAllocator::reserve_scratch_scope`
- `BasicScratchAllocator::reserve_scratch_scope`
- `ScratchReservation::{grow_to_observed, release}`

## Required API outcome

1. `MemoryPlatformBuilder::build()`은 enforced/live platform을 반환한다.
2. report-only construction은 `build_report_only` 또는 `UnenforcedBudgetReport`처럼 명시적으로 분리한다.
3. override resolution은 report/live product에서 같은 canonical resolver를 사용한다.
4. live `attach_runtime`은 broker-bound runtime view를 반환한다.
5. runtime scratch reservation은 QMP-020 committed/pending authority를 소유한다.
6. scratch cap denial은 allocator object 생성 전에 발생한다.
7. release/close는 ledger와 resource group capacity를 정확히 반환한다.
8. `AccountingLedger`, unchecked policy swap, basic constructors는 crate-private 또는 explicit unsafe/advanced feature로 격리한다.
9. public bumpalo는 budget-aware scope 뒤로 숨기거나 public surface에서 제거한다.
10. `PayloadPinLease`/pin registry가 production authority가 아니라면 experimental/unbudgeted surface로 분리한다.

## Implemented design decision

- Runtime scratch는 allocation 이후의 retained object가 아니라 allocation
  전 capacity permit이다. 따라서 QMP-020 `PendingReservation`을 scope 전체의
  유일 authority로 소유하며 별도 scratch counter를 만들지 않는다.
- Rebuild fence는 reserve/commit/committed release를 막되, incomplete pending
  permit의 exact rollback은 허용한다. 그렇지 않으면 rebuild 중 Drop이
  class/owner/context/resource-group capacity를 영구 고착시킨다.
- Explicit release 실패는 authority를 보존한다. Poison 상태에서 최종 Drop은
  poisoned guard를 회수해 Pending 상태와 ticket identity를 검증한 뒤 exact
  rollback만 수행한다; committed charge는 이 경로로 반환할 수 없다.
- 기본 pin adapter는 production runtime view에서 제거했고, 명시적
  `unbudgeted_pin` feature의 `UnbudgetedPinRegistry`로 격리했다.
- facade dependency는 core default feature를 비활성화한다. 오직
  `core-live-accounting`이 core `live_control`을 켤 수 있으며 feature matrix가
  Cargo resolved graph에서 누수를 거부한다.

## SOTA constraints used

- Apache DataFusion의 memory consumer는 allocation 전에 shared pool에
  reserve하고 RAII `MemoryReservation`을 Drop 때 반환한다:
  <https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/trait.MemoryPool.html>,
  <https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/index.html>
- Tokio `OwnedSemaphorePermit`도 private owned permit이며 Drop이 capacity를
  반환한다: <https://docs.rs/tokio/latest/tokio/sync/struct.OwnedSemaphorePermit.html>
- Cargo feature는 additive이고 default feature는 dependency에서 명시적으로
  끄지 않으면 활성화될 수 있다:
  <https://doc.rust-lang.org/cargo/reference/features.html#the-default-feature>

## Compatibility plan

- 기존 `.build()` return type 변경은 breaking일 수 있다.
- QMP-140에서 SemVer 결정을 내리되, 여기서는 deprecated compatibility adapter를 최대 한 release window로 제한한다.
- compatibility adapter가 enforced path를 다시 basic broker로 downgrade하면 안 된다.

## RED tests to add

- `default_builder_over_cap_store_mutation_is_denied`
- `default_builder_applies_override_layers`
- `report_only_builder_name_is_explicit_and_has_no_store_authority`
- `live_runtime_scratch_charge_appears_in_ledger_snapshot`
- `live_runtime_scratch_over_cap_is_denied_without_mutation`
- `dropping_or_closing_runtime_scratch_returns_capacity`
- `runtime_scratch_and_direct_broker_charge_share_one_limit`
- `runtime_scratch_release_restores_resource_group_capacity`
- `poisoned_runtime_scratch_explicit_release_preserves_then_drop_recovers_authority`
- raw constructor compile-fail tests
- unrestricted bumpalo public construction compile-fail 또는 charged wrapper behavior test

## Example contract

`hello_world`는 최소 다음을 실행해야 한다.

1. small live budget build
2. charged store 또는 scratch allocation 성공
3. over-cap request의 typed denial
4. release/close
5. final ledger zero

report 출력만 하는 예제는 `effective_budget_report`로 분리한다.

## Acceptance

- [x] README 권장 happy path가 실제 enforcement를 증명한다.
- [x] live runtime view에서 uncharged scratch path가 없다.
- [x] report-only와 enforced product가 같은 type/name으로 혼동되지 않는다.
- [ ] raw authority가 external safe API에서 제거된다. `dfb58a6`에서 clone 가능한 `ReservationTicket`과 raw reserve/commit/release가 여전히 public이다.
- [x] override layer가 default build에서 무시되지 않는다.
- [x] all public API/compile-fail tests가 새 surface를 고정한다.

## Verification

```bash
cargo test -p quanta-memory-platform-core --all-features --test public_api_surface
cargo test -p quanta-memory-platform-core --all-features runtime_scratch
cargo test -p quanta-memory-platform-facade --all-features --test public_api_surface
cargo run -p quanta-memory-platform-facade --example hello_world --all-features
cargo run -p quanta-memory-platform-facade --example effective_budget_report --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Public/Cargo use path 변경이므로 QMP-120 actual extraction과 QMP-130 host compile/adoption receipt가 terminal 조건이다.

## Receipt

- Product checkpoint: `cde6e0a7f82893a0ad8b0a0db6aa73f4c980fb6c`
- Implementation receipt:
  [`receipts/2026-09-20T192334Z-cde6e0a.md`](receipts/2026-09-20T192334Z-cde6e0a.md)
- Lifecycle: `BLOCKED -> IMPLEMENTED`. QMP-120 actual extraction과 QMP-130
  host adoption 전에는 `QUALIFIED`/`DONE`으로 승격하지 않는다.

## Final-audit correction — builder-first is bypassable

The enforced default builder and broker-bound runtime scratch are present, but
the safe public surface still re-exports `ReservationTicket` and exposes raw
`reserve`, `reserve_committed`, `commit`, `release`, and `release_many`.
`core/tests/public_api_surface.rs` currently pins that export instead of rejecting
it. Therefore the previous checked acceptance claim was false. QMP-020 owns the
authority/lifecycle repair; QMP-030 owns removal from the recommended public
surface; QMP-140 owns the breaking-change/deprecation decision.
