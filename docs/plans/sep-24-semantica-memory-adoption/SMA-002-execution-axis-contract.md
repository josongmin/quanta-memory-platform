# SMA-002 — 기존 미집행 output/scratch 제거 변경 검토·완결

Priority: P1. Status: MAIN_INTEGRATED_OWNER_PROOF_PENDING.
`0d04d66410a`는 [이전 001/002 부분 후보](INTEGRATION-2026-09-24.md)다.
현재 구현은 Semantica main `31a07f43e1f`에 통합됐으며 최종
source와 public/product 검증이 남아 있다.
Owner: SMA-001과 동일한 SDK/ingress/composer owner.
Depends on: 001의 공유 diff 검토. Blocks: 009.
기준 소스: [README](README.md). 근거: [최종 재감사](FINAL-AUDIT.md).

## 확정 범위

**제거-only**: Semantica의 미연결 `output_mb/scratch_mb` public setters, DTO,
lowering을 제거한 기존 candidate를 완결한다.
main에는 설정/report 경로가 남아 있고 QMP execution/scratch 제품 호출은 조사 범위에서
없었다. candidate에서는 관련 Rust literal이 이미 제거됐다. 새 집행 엔진은 만들지 않는다.

기존 execution rail의 output/materialization/scratch counter, permit, typed errors,
QMP 자체의 portable execution API는 보존한다. QMP에 같은 기능이 없다는 판정이 아니다.

## 남은 작업

1. 001과 같은 diff를 사용한다. SDK defaults/exports→ingress→composer→report 소비자에서
   한 번에 제거됐는지 확인하고 잔여 consumer만 수정한다.
2. 명시 설정뿐 아니라 preset/projection으로 지원되지 않는 축을 계속 노출하는 경로가
   없는지 확인한다. 숨은 입력, no-op setter, “None=disabled/truncated” 잔존 문구를 제거한다.
3. 기존 default residency·artifact reserve가 바뀌지 않았는지 candidate의 production
   composition test를 실행한다. QMP preset/실행 제한을 삭제하는 방식으로 해결하지 않는다.
4. 기존 execution rail 소스가 변경되지 않았는지 비교한다. ingress/public compile과
   관련 기존 rail boundary 회귀를 재사용하며 실패한 경계만 수정한다.

## 대상과 기존 proof

- `C/quanta-runtime/src/sdk/memory_budget.rs`, `C/quanta-sdk/src/{lib.rs,memory_budget.rs}`
- `C/quanta-runtime-ingress-core/src/memory_budget_v1.rs`
- `C/quanta-runtime-memory-compose/src/{lib.rs,lib_tests.rs}`
- `C/quanta-runtime-memory-compose/tests/production_memory_budget_contract.rs`의
  `production_composition_retains_sdk_artifact_cache_reservation_v1`.
- 기존 `quanta-runtime-execution/src/gateway_envelope/gateway_boundary_context_v1.rs`
  boundary 테스트는 보존 대상이지 신규 metering 구현 대상이 아니다.

완료: public 축/숨은 입력/가상 report 제거 + 지원 defaults 보존 + public consumer compile.
새 dual authority, scratch allocator, cancellation matrix, output E2E는 이 제거 작업에 추가하지 않는다.
실제 consumer가 새 집행을 요구하면 별도 scope 결정 후 설계한다; 이번 티켓의 대안 분기로 남기지 않는다.
