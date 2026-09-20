# QMP-080 — Runtime semantics for policy, reconfigure, and reclaim

Priority: P2  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-010, QMP-020, QMP-030  
Blocks: QMP-110, QMP-130, QMP-140

## Objective

public policy field 각각에 명확한 runtime consumer와 event를 부여하고, reserve/reconfigure/reclaim이 동일 admission semantics를 사용하게 한다.

## Confirmed evidence

- class/group `soft_cap_bytes`는 validation 외 runtime consumer가 없다.
- class reserved는 global floor만 계산하고 해당 class 보호를 보장하지 않는다.
- group reserved/burst_percent/reclaim_priority는 scheduler admission에 미사용.
- scheduler는 hard cap, weight, shared pool만 사용.
- identity reconfigure가 legal borrowed usage 중 실패할 수 있다.
- 원본 감사의 public `record_group_usage` side channel은 current source에서 제거됐다. 이 항목을 잔여 결함으로 재구현하거나 중복 수정하지 않는다. 현재 남은 scheduler saturation과 ledger-derived usage 일치만 검증한다.
- reclaim ladder/controller는 broker reserve slow path에 연결되지 않는다.
- reconfigure는 report residency만 바꾸고 provenance는 stale다.

## Required semantic decision

Linux cgroup v2 기준으로 최소 다음을 분리한다.

| QMP field/concept | Required meaning |
| --- | --- |
| reserved | protected floor; 다른 consumer가 침범할 수 있는 조건 명시 |
| soft | pressure/reclaim 시작 threshold; 즉시 hard deny와 구분 |
| hard | 신규 admission ceiling |
| burst | finite shared allowance; percent와 weight의 결합 규칙 명시 |
| reclaim priority | deterministic victim ordering |
| headroom | non-headroom admission에서 보호되는 global band |

사용하지 않을 field는 deprecated 후 제거한다. inert public knob를 유지하지 않는다.

## Canonical write set

- `core/src/domain/policy.rs`
- `core/src/domain/config.rs`
- `core/src/application/{scheduler,controller}.rs`
- `core/src/api/control.rs`
- `core/src/domain/stats.rs`
- cross-cutting policy tests

Owner symbols:

- `WeightedResourceGroupScheduler::{record_usage, record_release, current_usage, fair_burst_share, admits}`
- `LadderPressureController::{classify_semantic, ladder_for, reclaimable_bytes}`
- `LiveProcessMemoryBroker::{reserve_on, release_on, try_live_reconfigure, install_policy}`
- `ResidencyPolicy`, `MemoryClassBudget`, `ResourceGroup`, `EffectiveMemoryBudgetReport`

## Required implementation

1. admission과 reconfigure가 같은 prospective-state predicate를 사용한다.
2. protected floor는 다른 class/group usage 계산에 반영한다.
3. soft threshold crossing은 bounded reclaim signal/event를 발생시킨다.
4. hard cap은 reclaim 이후에도 충족되지 않으면 typed deny다.
5. burst share는 overflow-safe `u128` math와 work-conserving remainder policy를 사용한다.
6. `burst_percent`와 `admission_weight`가 동시에 존재할 경우 정확한 공식/우선순위를 문서화한다.
7. current policy identity reconfigure는 legal borrow 중에도 성공한다.
8. 제거된 public `record_group_usage`가 다시 노출되지 않는 compile/API guard를 유지하고 usage는 ticket lifecycle에서만 유도한다.
9. reclaim port를 정의하되 broker mutex를 잡은 채 외부 callback을 호출하지 않는다.
10. reclaim loop는 bounded attempt, deterministic rung, post-reclaim remeasure, typed exhaustion을 가진다.
11. accepted reconfigure는 policy revision/source provenance를 atomic postimage에 포함한다.

## RED tests to add

- `reserved_floor_protects_class_from_peer_consumption`
- `soft_cap_crossing_triggers_reclaim_without_immediate_hard_deny`
- `hard_cap_denies_after_bounded_reclaim_exhaustion`
- `burst_percent_changes_admission_outcome`
- `reclaim_priority_orders_victims_deterministically`
- `identity_reconfigure_succeeds_during_legal_class_borrow`
- `identity_reconfigure_succeeds_during_legal_group_burst`
- `reconfigure_uses_same_postimage_predicate_as_reserve`
- `public_group_usage_side_channel_is_unavailable` compile-fail
- `reclaim_callback_runs_without_broker_lock_held`
- `accepted_reconfigure_updates_provenance_revision`
- `weighted_share_distributes_remainder_work_conservingly`
- `headroom_usage_does_not_consume_protected_non_headroom_band_twice`: Headroom 선점→일반 class reserve와 동일 policy identity reconfigure가 같은 legal-state predicate를 써야 한다. `ledger.rs:344-350`은 모든 class 사용량을 non-Headroom ceiling에 비교하지만 `control.rs:867-877`은 non-Headroom 사용량만 비교하는 현재 불일치의 semantic decision/RED.

## Acceptance

- [ ] every public policy field has validation + runtime consumer + tests.
- [ ] reserved/soft/hard가 서로 다른 observable behavior를 가진다.
- [ ] reserve와 reconfigure가 동일 legal-state definition을 공유한다.
- [ ] scheduler truth가 ledger ticket에서만 파생된다.
- [ ] reclaim path가 실제 broker denial slow path에서 reachable하다.
- [ ] no callback under broker mutex.
- [ ] provenance가 active values와 일치한다.

## Verification

```bash
cargo test -p quanta-memory-platform-core --all-features reserved_floor_protects_class_from_peer_consumption
cargo test -p quanta-memory-platform-core --all-features identity_reconfigure_succeeds_during_legal_class_borrow
cargo test -p quanta-memory-platform-core --all-features reclaim_callback_runs_without_broker_lock_held
cargo test -p quanta-memory-platform-core --all-features
cargo clippy -p quanta-memory-platform-core --all-targets --all-features -- -D warnings
```

## Non-goals

- host wall-time/subquery/output enforcement call sites: QMP-130
- metrics exporter wiring: QMP-110

## Current-source delta audit (2026-09-21, `47021d3`)

- `core/src/api/control.rs`에는 원본 감사의 `record_group_usage` 공개 API가 없다. P2-03의 제거 작업 자체는 현재 소스에 재적용하지 않는다. 그러나 제거를 증명하는 public compile/API guard와 scheduler truth 검증은 terminal 조건이다.
- `application/scheduler.rs:47-56,80-96`은 group usage/admission에 saturation을 사용한다. QMP-020의 P1-D01 checked ledger repair와 한 owner 계약으로 직렬화하고 그룹 예산이 장부와 어긋나지 않는 overflow RED를 먼저 만든다.
- `domain/ledger.rs:344-350`과 `api/control.rs:867-877`은 Headroom이 이미 사용 중일 때 일반 class의 legal state를 다르게 정의한다. 현상은 정적 코드 대조이며 실행 RED는 미실시다. protected band의 의미를 하나로 명시한 뒤 reserve/reconfigure 양쪽에 동일 prospective predicate를 적용한다. 보수적 거부인지 실제 의미 버그인지는 RED와 설계 결정까지 확정하지 않는다.
