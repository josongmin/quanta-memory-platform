# FUP-004 — 축별 kind 검증과 독립 oracle 확장

Status: `VERIFIED — defensive oracle closure + temporary-mutant RED`
Priority: `P2 — 방어 강화, reachable corruption 미입증`
Owner: `Lane C`
Depends on: source freeze와 `FUP-007`의 `ledger.rs` lease 반환
Blocks: `FUP-003`의 ledger 편집, `FUP-006`의 full-followup 후보. P1-only 후보에서는 OPEN으로 남을 수 있다.
Checklist: `B02`의 축별 kind 검증 oracle, `M01`의 해당 negative-fake 범위. 정상 API의 회계 corruption은 미재현.

## 문제와 근거

`domain/ledger.rs::LedgerSnapshot::validate_exact`는 class/owner/context 총량을 비교하고,
kind 합은 owner 값에서 다시 계산한다. class 또는 context의 payload/metadata 배분만
총량을 유지하며 바꾸면 이 검사로 검출하지 못한다.

정상 mutation은 같은 ticket kind/bytes를 모든 축에 반영한다. 외부 mutable snapshot
재설치 경로도 확인하지 못했으므로 현 API의 회계 corruption 버그로 격상하지 않는다.
기존 mixed-kind/fuzz, 128-case independent-model property test,
`suite_rejects_wrong_charge_kind_axis`는 보존하며 확장한다.

현재 B02의 첫 감사 `구현` 판정은 "모든 축의 kind 분할을 검증한다"는 의미로
읽히면 과했다. `ledger.rs:233-249`의 scalar 축 비교와 owner kind 재계산은
class/context 내부에서 payload↔metadata를 바꿔 총량만 보존한 snapshot을
검출하지 않는다. 그러나 public API로 그 snapshot을 다시 설치하는 경로는
확인되지 않았다. **따라서 P2 oracle 강화이며 P1 producer corruption 수정이 아니다.**

## Write set

- `crates/quanta-memory-platform-core/src/domain/ledger.rs`
- `crates/quanta-memory-platform-testkit/src/conformance/{target,invariants,suites}.rs`
- `crates/quanta-memory-platform-testkit/src/fake/{broker,views}.rs`의 필요한 kind 관측.
- `AccountingView`는 `target.rs`, `BrokerFault`와 independent model은 `suites.rs`가
  소유한다. 시작 시 모든 view struct literal과 re-export 영향을 확인한다.
- `api/control.rs`, provenance/report, scheduler는 편집하지 않는다.

## 구현 단계

1. snapshot test에서 class/context의 payload↔metadata만 변조하고 총량을 유지한다.
   올바른 expectation은 `validate_exact` 실패이며 현 구현 RED를 확보한다.
2. class/owner/context 각각에서 checked arithmetic으로
   `[retained_payload, retained_metadata, live_scratch]` 합계 벡터를 계산하고 비교한다.
   기존 scalar balance/overflow 검사를 유지하거나 같은 pass에 통합한다.
3. 이 검증은 기존 snapshot validation 경로에서 수행한다. 매 reserve/release에
   owner 전체 scan을 추가하거나 ticket 저장구조를 바꾸지 않는다.
4. testkit `AccountingView`에 축별·bucket별 kind 관측을 추가한다. owner-global kind
   결과를 class/context 값인 것처럼 복사하는 default 구현은 금지한다.
5. live adapter는 실제 snapshot에서, fake는 자신의 상태에서, independent model은
   operation trace에서 각각 계산한다. 같은 production helper를 oracle로 재사용하지 않는다.
6. **aggregate 일치와 bucket별 정답을 구분한다.** 모든 class의 kind 합까지 보존한
   서로 다른 bucket 간 swap은 snapshot aggregate 비교만으로는 잡을 수 없다.
   이 경우는 model 대 view의 bucket×kind 비교와 dedicated negative fake가 검출한다.

## 검증 계약

- [x] 총량 보존 class-kind/context-kind 변조를 snapshot validator가 거절.
- [x] aggregate kind까지 보존한 bucket 간 swap을 independent model/oracle가 거절.
- [x] mixed-kind reserve/commit/release/batch/recharge trace에서 모든 관측 축 일치.
- [x] zero-byte/overflow/rejected mutation/foreign ticket/replay 후 기존 state 불변.
- [x] 기존 WrongChargeKindAxis fake와 기존 property suite도 계속 유효.
- [x] 새 class/context 비교 제거 mutant에서 적절한 negative test RED.
- [x] single-release bounded-probe 계약과 `BTreeMap` ticket index 보존.

```bash
cargo test --locked -p quanta-memory-platform-core --all-features sep23_fup004 -- --nocapture
cargo test --locked -p quanta-memory-platform-testkit --all-features sep23_fup004 -- --nocapture
cargo test --locked -p quanta-memory-platform-testkit --all-features property_trace_matches_independent_model
cargo test --locked -p quanta-memory-platform-testkit --all-features suite_rejects_wrong_charge_kind_axis
```

`sep23_fup004`는 아직 없는 예정 selector다. 추가한 negative fake가 실제
축별 kind 오류를 만들고 기존 production helper를 oracle로 재사용하지 않는지
검토한 뒤, `--list`/selected count와 mutant RED를 기록한다.

## Handoff / stop 조건

`AccountingView`의 public trait/struct compatibility 영향을 기록한다. 필요하면 외부 fake
구현의 migration 예시를 I에 전달한다. target의 오류를 정상 zero 값으로 치환하지 않는다.
004 검증 후 ledger hash와 lease 반환을 B에 전달해야 003을 시작할 수 있다.
유효한 정상 state를 거절하는 새 invariant라면 요구사항을 재검토한다. 이 티켓은
class-owner의 존재하지 않는 1:1 관계를 새 제약으로 추가하지 않는다.
