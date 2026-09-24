# SMA-007 — Parser/PTA/trace의 추가 도입 선정

ARCHIVED DESIGN — 실행 티켓 아님. 아래 요구는 향후 해당 범위를 채택할 때만 참고한다.

Priority: CONDITIONAL audit/measurement. Status: CONDITIONAL — 활성 실행 목록 제외.
Admission gate: 활성 cache owner의 비용 조사/추가 도입이 필요하다고 결정한 경우만 시작.
Owner: 각 canonical cache owner. Depends on: SMA-004 metrics.
목적: 실제 제품 비용이 큰 owner만 추가한다.

## 대상과 시작 파일

| Candidate | Owner files | 선행 확인 |
| --- | --- | --- |
| Parser semantic modules / Python tree | `C/quanta-adapters-parser/src/language_host_adapter/adapter_impl/semantic_module_v3_cache.rs`, production parser ports | live feature/제품 caller, entry/descendant cap과 byte 분포 |
| PTA summaries | `C/quanta-adapters-in-memory/src/in_memory/core/pta_function_summary_cache.rs`, `C/quanta-contract-ports/src/pta_function_summary_cache_port.rs` | 활성 product feature, 8192-entry cap, infallible put와 capacity 거절 의미 |
| Trace indices | `C/quanta-runtime-invalidation-owner/src/invalidation/trace_invalidator/index/` 및 GC/invalidation owner | pin/graph authority, cardinality cap, 삭제 원자성 |

## 작업

1. 각 후보의 constructor→product consumer→read/write→eviction/invalidation 경로를 기록한다.
   upstream Salsa engine 도입은 별도 요구가 없으므로 포함하지 않는다.
2. 대표 workload에서 retained bytes, hit/rebuild 비용, contention, fanout,
   동시 query별 scratch·복제량을 측정한다. dormant feature/test-only candidate는 제외한다.
3. `ADOPT_QMP`, `KEEP_LOCAL_CAP`, `EXCLUDE_INACTIVE` 중 하나를 후보별로 선택한다.
   수치가 없으면 `MEASUREMENT_PENDING`이며 제품 구현 완료로 계산하지 않는다.
4. ADOPT할 때 기존 owner state에 charge를 결합한다. PTA infallible put을 변경해야 하면
   모든 caller의 cache-denial fallback을 breaking cutover로 함께 처리한다.
   trace는 index/GC/invalidator와 동일 mutation 권한을 보존한다.
5. class/owner taxonomy는 host에서 필요한 최소 수로 정한다. 새 cache마다 새 QMP enum이나
   별도 global broker를 만들어 해결하지 않는다.
   기존 host owner keys를 사용하고, 003의 retention seam은 단위·scope·lifetime이 맞는
   경우에만 재사용한다. owner별 sizing 공식은 한 곳에 두고 공통 프레임워크부터 만들지 않는다.
6. source buffers/snapshot authority를 reconstructible cache로 분류하지 않는다.
   fallback 근거가 없는 state를 memory pressure 때문에 삭제하지 않는다.

## 테스트·완료

채택 owner에만 insert/replace/evict/invalidate/cancel, exact cap/cap+1,
동시 generation drift, failed admission postimage, 결과 동등성 테스트를 추가한다.
기존 entry-count tests와 byte-cap tests의 의미를 구분한다.

각 package의 QBC owner rail과 채택된 제품 feature 한 개를 확인한다.
완료 산출물: 3개 후보의 결정표 + source/feature 증거 + raw measurement + ADOPT된
경우 변경/검증 receipt. 전체 코드베이스의 모든 cache를 다 QMP화하는 티켓이 아니다.
