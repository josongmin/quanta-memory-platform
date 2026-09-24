# SMA-001 — 기존 overlay 제거 변경 검토·완결

Priority: P1. Status: MAIN_INTEGRATED_OWNER_PROOF_PENDING.
`0d04d66410a`는 001/002 부분 후보의 [이전 통합 기록](INTEGRATION-2026-09-24.md)이다.
현재 001/002/003 구현은 Semantica main `31a07f43e1f`에 통합됐다.
composer owner lib/all-features 51/51은 통과했지만 이후 후보가 변경됐으므로 최종
source receipt와 public/product 검증은 남아 있다. 원본 diff를 재작성하지 않는다.
Owner: Semantica SDK/composition. Depends on: final-source QBC capacity. Blocks: 009.
기준 경로·HEAD: [README](README.md). 근거: [최종 재감사](FINAL-AUDIT.md).

## 이미 있는 것 — 신규 구현에서 제외

- main: 기본 report의 live policy readback, stats의 단일 `observe()`.
- candidate: `Code::with_memory_budget*`, handle-local `MemoryBudget`,
  overlay/selection 타입과 session/kernel 전파 제거.
- candidate: report의 설치 정책 projection, host owner-key reexport,
  defaults overflow/invalid-policy·live reconfigure·public SDK 테스트 소스.

candidate의 `packages/ crates/` Rust literal census에서 제거 대상 API/type hit는 없었다.
이는 저장 소스 검사이며 컴파일 성공이나 전체 외부 consumer 호환 증거는 아니다.

## 남은 작업

1. 기존 diff를 인수한다. 새 제거 구현을 작성하지 않는다. 현재 main의 session/source/cancel
   변경과 비교해 API 제거에 필요한 hunk만 통합 가능하게 정리한다.
2. producer→port→kernel/mock→session→SDK export 전체 signature cutover를 검토한다.
   stale caller가 확인된 경우만 수정한다. no-op alias/호환 overlay를 다시 만들지 않는다.
3. ingress validator 제거가 defaults 검증을 약화시키지 않았는지 기존 composer lowering과
   candidate 회귀로 확인한다. global/headroom/class 및 overflow/fail-closed 계약을 보존한다.
4. candidate가 추가한 report scope/revision/byte fields는 이미 있는 diff의 소비자 closure를
   검토한다. 새 domain 모델·resolver·generic wrapper를 추가하거나 같은 필드를 재구현하지 않는다.
5. 아래 테스트를 기존 QBC owner에서 실행하고 실제 실패만 수정한다. 통합 proof는 009로 넘긴다.

## 검토 대상 파일

- `C/quanta-runtime/src/sdk/{memory_budget.rs,code/mod.rs,code/session_selector.rs,analysis_session.rs,runtime_kernel_core.rs,public_exports_head.rs}`
- `C/quanta-runtime/src/{orchestration/ports/orchestration.rs,boundary_gqlang_runtime_v1/code.rs}`
- `C/quanta-runtime-ingress-core/src/{lib.rs,memory_budget_v1.rs}`
- `C/quanta-sdk-runtime-composition-owner/src/port_impls.rs`
- `C/quanta-runtime-memory-compose/src/{lib.rs,lib_tests.rs}`
- `C/quanta-sdk/src/{lib.rs,memory_budget.rs}`, SDK smoke 및 기존 session/kernel mocks.
  파일 목록은 검토 범위이며 모든 파일을 다시 수정하라는 지시가 아니다.

## 기존 테스트 재사용

- `runtime_budget_defaults_reject_mb_overflow_without_partial_manifest_mutation`
- `runtime_budget_defaults_reject_invalid_policy_without_partial_manifest_mutation`
- `runtime_budget_soft_cap_keeps_three_quarters_near_u64_limit`
- `runtime_cutover_memory_budget_report_reads_installed_policy`
- `runtime_cutover_memory_budget_report_tracks_live_reconfigure_revision`
- `runtime_cutover_memory_usage_stats_project_live_runtime_identity`
- `public_memory_budget_defaults_install_into_the_reported_platform` (SDK smoke)

위 이름은 main 소스에 존재한다. 새 동명/동목적 테스트를 만들지 않는다.
기존 platform-domain 테스트도 삭제하지 않되 별도 domain 구현 티켓으로 확대하지 않는다.
checked poison/busy/closed와 observation 원자성은 기존 FIN-20/QMP owner proof를 재사용한다.

완료: 제거 API/caller 잔존 없음 + defaults/live readback 보존 + public consumer compile 및
owner 테스트의 source-bound GREEN. main 통합만으로 이 티켓을 제품 완료로 표시하지 않는다.
