# SMA-009 — 고정 소스 제품 검증·직렬 통합

Priority: P1 release evidence. Status: PRODUCT_PROOF_PENDING.
Owner: serial integrator. Depends on: SMA-001/002/003 owner handoff.
Source/worktree: [README](README.md). 001/002/003은 Semantica main
`31a07f43e1f`에 통합됐다. [통합 기록](INTEGRATION-2026-09-24.md)의 이전
부분 후보와 과거 owner GREEN은 최종-main proof가 아니다.

## 실제 남은 작업

1. 현재 main HEAD·dirty fingerprint·QMP dependency를 고정한다.
   다른 owner의 dirty work는 건드리지 않는다.
2. source-bound owner/public/product proof를 확보한다. 이미 있는 테스트를 먼저
   실행하고, 실제 경계에 없는 oracle만 owner-local 또는 최소 제품 E2E로 보완한다.
3. 최종 main에서 001/002/003의 API cutover와 memo admission/lease/runtime
   error closure를 검증한다. Rust/QBC는 용량 gate를 충족한 뒤 실행한다.

## 최소 proof matrix

| Gate | 재사용할 경계 | 통과 조건 |
| --- | --- | --- |
| G1 API cutover | SDK exports/session/kernel/mock 및 SDK smoke | 제거 API/type/caller 잔존 없음, 실제 public consumer compile 성공 |
| G2 Defaults/readback | composer 001에 열거한 기존 테스트 | global/headroom/class/overflow·fail-closed 보존, 설치된 report/stats |
| G3 Product policy | 기존 production_memory_budget_contract target | SDK artifact reservation 1024 MiB 보존 |
| G4 실제 admission 연결 | 기존 composer live-broker bridge/cap 테스트 | 실제 publication의 typed denial와 기존 값/원장 postimage 보존 |
| G5 Canonical memo 수명 | adapter+host+composer | row/index semantic cost, receipt/prepared/GC 최종 lease, 동일 broker snapshot |
| G6 SDK/query 제품 E2E | 실제 SDK ingress의 broker-backed memo table | query miss→publish→hit→invalidate/GC, capacity denial typed, 후속 정상 요청, close/drain |
| G7 Source integration | 현재 main | 001/002/003 계약 보존, 뒤늦은 caller/feature drift 없음, 최종 소스에서 G1~G6 재검증 |

main에는 `production_cutover_memory_deps_charge_runtime_contract_bridges_against_live_broker`
등 실제 bridge 테스트가 이미 있다. G4를 위해 같은 목적의 새 root E2E를 만들지 않는다.
G5/G6은 기존 artifact bridge 테스트로 대체할 수 없다. 새 OS process 경계를
바꾸지 않으므로 daemon restart, RSS benchmark, multi-cache reclaim, Redb
qualification은 이번 gate가 아니다.

## 실행과 증거

- Semantica QBC 등록 owner/profile 사용. ports, adapter, memory-host,
  ingress-core, runtime-memory-compose, runtime, SDK composition/public smoke 및
  변경된 public consumer closure가 대상이다.
- target/features/filter, nonzero 실행 수, exit, 로그 위치, 양 repo HEAD/dirty digest,
  실제 Cargo dependency source를 기록한다. 테스트 소스 존재는 PASS가 아니다.
- defaults regression을 없애거나 errors를 empty success로 바꿔 통과시키지 않는다.
- FIN-20/host-pair 증거는 실제 source/features 적합성 확인 후 재사용한다.
  환경변수로 선택한 검증 QMP와 Cargo가 참조한 QMP를 혼동하지 않는다.
- QMP runtime/API/Cargo는 변경 범위가 아니므로 새 QMP 구현이나 전체 qualification을
  무조건 선행시키지 않는다. 실제 dependency/feature 변화가 있으면 해당 proof만 추가한다.
- 큰 root Rust rail은 owner/public boundary로 커버되지 않는 이유가 확인된 경우만 실행한다.

완료: main에 포함된 001/002/003 변경의 G1~G7 evidence. owner GREEN, 제품
GREEN, main 통합, 최종 검증은 분리 기록한다. benchmark/디스크/프로세스 RSS
보장은 포함하지 않는다.
