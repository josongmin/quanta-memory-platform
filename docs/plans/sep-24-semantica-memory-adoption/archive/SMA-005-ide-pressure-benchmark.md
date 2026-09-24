# SMA-005 — 실제 IDE cache 합산 peak와 pressure benchmark

ARCHIVED DESIGN — 실행 티켓 아님. 아래 요구는 향후 해당 범위를 채택할 때만 참고한다.

Priority: CONDITIONAL measurement. Status: CONDITIONAL — 활성 실행 목록 제외.
Admission gate: IDE 합산 pressure/성능 판정을 별도 제품 요구로 채택한 경우만 시작.
Owner: Semantica runtime-search/product benchmark. Depends on: SMA-004.
Evidence: [E06](../AUDIT.md). 기존 owner-local cap/실패 테스트를 보존한다.

## 소유 파일

- `C/quanta-runtime-search/src/production_read_query_host/ide_result_cache.rs`
- `C/quanta-runtime-search/src/production_read_query_host/workspace_symbol_index.rs`
- 동일 owner의 host construction/production read dispatch 및 기존 tests
- `S/crates/quanta-cli-server/src/production_analysis_host.rs`는 E2E 진입 증거;
  instrumentation을 위해 불필요한 product handler 변경을 만들지 않는다.
- 004가 선정한 benchmark target/registration.

## 작업

1. 고정 snapshot·profile·source generation으로 실제 product host를 구성한다.
   4 slot은 각각 64 MiB/256 entries/200k items; union은 aggregate 128 MiB다.
   이 값들의 합을 RSS hard limit로 주장하지 않는다.
2. ready bytes, build scratch, returned handles, retained source snapshots를 분리 계측한다.
   workspace union의 per-file symbol 복사와 normalized-name 비용을 함께 기록한다.
   각 owner counter가 admission SSOT다. aggregate는 projection이며 두 번째 mutable
   admission counter를 추가하지 않는다. shared allocation과 deep clone의 비용/수명을 구분한다.
3. 기본 matrix는 cold/warm/edit/reclaim-after-use의 lifecycle과 concurrency 1/8이다.
   slot의 16-flight/256-waiter 한계·cap±1은 기존 owner-local 테스트로 확인하고,
   제품 stress에서는 실제 그 branch를 밟은 counter가 있을 때만 해당 시나리오로 계산한다.
4. 4 slot+union 동시 사용, 같은 key storm, 여러 key 경쟁, cancel/failed build,
   generation 변경 중 build, caller가 Arc를 유지한 eviction, 재호출/recompute를 선택한다.
   모든 축의 Cartesian product 대신 각 독립 transition을 한 번 확실히 친다.
5. 큰/small source와 편중된 symbol 분포를 고정 corpus로 사용한다.
   모델 수/캐시 entry 수가 아닌 실제 retained size·hit/miss·eviction·build count를 기록한다.
6. cold/warm/evicted 경로 결과를 독립 canonical digest로 비교한다.
   stale generation이 새 응답으로 publication되지 않아야 한다.
   benchmark에 production policy를 복제해 정답으로 삼지 않는다. 관측은 실제 owner 경로에서 얻는다.
7. p50/p95, throughput, actual peak RSS, semantic peak, denial/rebuild 비용을 보고한다.
   비교 실행 전 반복 횟수·warmup·허용 회귀를 baseline 계획에 고정한다.
   부하에 따른 실패 artifact도 보존하고 threshold를 결과에 맞춰 완화하지 않는다.

## 결정과 완료 조건

- `KEEP_LOCAL_CAPS`: 측정 범위에서 cap과 latency가 목적을 충족; 자료/적용 범위를 기록.
- `ADOPT_SHARED_ACCOUNTING`: 합산 pressure의 구체적 문제와 대상 owner를 006에 넘김.
- `OPTIMIZE_DUPLICATION`: 깊은 복사가 지배적일 때 immutable 공유 표현을 검토.
  generation/ownership 변경을 포함하므로 별도 owner test와 전후 수치가 필요.

결정은 이 중 하나 이상과 raw artifact/hash가 있어야 완료다.
QMP 미연결이라는 이유만으로 실패/무제한이라고 판정하지 않는다.
기존 cap을 없애고 새로운 미검증 cap으로 바꾸지 않는다.
