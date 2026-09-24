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

2026-09-24 current-source note: Semantica main `1d17286`의 SDK composition
owner-local test에 설치된 `memo_residency=0` 정책을 통한 실제 query 거절,
typed budget reason, rollback postimage, 같은 세션의 후속 no-persist 요청
oracle을 추가했다. 이는 **작성된 테스트**이며 QBC 미실행이다. 현재 내부
볼륨은 persona preflight의 16 GiB floor를 충족하지 못했다. 직접
`invalidate_scope`를 호출하는 기존 SDK test도 coordinated invalidation/GC,
cancel/close/deferred drain 증거로 확대하지 않는다.

The denial test now compares the complete installed class/owner usage vectors
and retained-payload/metadata/scratch counters before and after failure, in
addition to total bytes, memo row absence, and artifact inventory. Once run,
these assertions can detect a same-total charge transfer in that fixture; they do not prove
ticket-count closure or a high-level `Code::query()` product route. On the
preceding `ff634f7` main with unrelated dirty paths, both the direct persona check and canonical
bootstrap recovery returned `quota_preflight_denied` (roughly 1 GiB free,
16 GiB floor). No QBC test executed after this assertion change.

The composer `installed_artifact_cap_rejects_product_publication_without_changing_ledger`
test now exercises a real host-backed artifact row at the independent
`32 MiB - 72 byte` payload boundary: exact shared-pool charge, checked
inventory, deletion/refund, then a one-byte-over request with unchanged
ledger. The 72-byte expectation is written as key/index/entry-state literals,
not read back from the production cost helper. The test is committed in
Semantica `1d17286`. A subsequent owner-local extension, committed in
Semantica `7d6333b`, sets the
artifact and evidence class caps to zero, verifies that one real artifact row
exhausts their shared pool, rejects a peer evidence publication without a
ledger/store postimage, then admits and refunds the evidence owner after the
artifact is removed. It also verifies that the same artifact owner can publish
and refund a smaller row after a cap+1 denial. The extension passed formatting
and diff checks on the working-tree precursor; it is not an executed QBC result or a public
`Code::query()` proof.

2026-09-25 verification update: Semantica `ea9b9e9` retains the committed
test unchanged. The direct persona preflight still returned
`quota_preflight_denied`: roughly 8 GiB free against a 16 GiB floor. No
owner QBC or product E2E was launched under that failed preflight. This is an
environment block, not a test failure or a current-source GREEN receipt.

2026-09-25 later source update: Semantica `e5d3ef11a7e` committed a public
`Code::query().builder::<FileTextQuery>()` miss/hit and installed memo-charge
oracle. Semantica `4515ce74c98` extends the zero-cap test to require a
typed public `Code` denial with unchanged installed ledger. Both passed
`rustfmt --check` and `git diff --check`; neither was compiled or executed.
A later working-tree extension checks public memo hit→invalidation/refund→miss;
it is likewise uncompiled and is not coordinated GC or close/drain proof.
An exact owner selector for the public test resolved, but `owner run` stopped
before compilation: the QBC lane reported 7.4 GiB free against its 50 GiB and
4.75% activation thresholds. The earlier 16 GiB persona floor is therefore
not sufficient for this QBC lane. Do not infer GREEN from selector resolution.

Read-only QBC owner resolution on that checkout identified the exact lib-test
selectors below. Resolution is not compilation or test discovery; run them
only after the quota preflight passes, and require one executed test each:

```sh
scripts/quanta-build-cli owner resolve --package quanta-runtime-memory-compose --target-kind lib --selector-mode exact --selector tests::installed_artifact_cap_rejects_product_publication_without_changing_ledger --compile-policy package-all-features --json
scripts/quanta-build-cli owner resolve --package quanta-sdk-runtime-composition-owner --target-kind lib --selector-mode exact --selector types::no_persist_composition_tests::sdk_memo_capacity_denial_is_typed_and_leaves_session_usable_v1 --compile-policy package-all-features --json
```

`quanta-runtime/src/orchestration/observation_feedback_runtime/product_bound_runtime.rs`
의 SQL-family shadow leg는 별도 in-memory artifact/evidence/memo store를 만들고,
SDK installed broker 밖이라고 명시한다. 해당 함수는
`sql-family-product-runtime` 공개 surface로 re-export된다. 이는 test/probe
전용이 아니다. `quanta-cli-ingress-runtime`의 `sql-family-runtime-activation`
(`dist-full` 포함)과 별도 프로세스 `quanta-sql-rulepack-worker`가 baseline 및
candidate leg를 실제로 생성한다. 현재 두 production caller 모두 packet에
예산 authority를 전달하지 않는다. 따라서 각 프로세스의 설치된 예산과
shadow leg의 실사용량 사이에 연결이 없다. worker와 CLI는 프로세스가
달라 하나의 in-process broker를 공유할 수 없으며, 각 프로세스에서
baseline/candidate가 공유하는 bounded domain을 설치하거나 별도 domain의
명시적 한도 및 제품 범위 계약을 정의해야 한다. 이 경로를 SDK G6 통과로
합산하지 않는다.

## 최소 proof matrix

| Gate | 재사용할 경계 | 통과 조건 |
| --- | --- | --- |
| G1 API cutover | SDK exports/session/kernel/mock 및 SDK smoke | 제거 API/type/caller 잔존 없음, 실제 public consumer compile 성공 |
| G2 Defaults/readback | composer 001에 열거한 기존 테스트 | global/headroom/class/overflow·fail-closed 보존, 설치된 report/stats |
| G3 Product policy | 기존 production_memory_budget_contract target | SDK artifact reservation 1024 MiB 보존 |
| G4 실제 admission 연결 | 기존 composer live-broker bridge/cap 테스트 | 실제 publication의 typed denial와 기존 값/원장 postimage 보존 |
| G5 Canonical memo 수명 | adapter+host+composer | row/index semantic cost, receipt/prepared/GC 최종 lease, 동일 broker snapshot |
| G6 SDK/query 제품 E2E | 실제 SDK ingress의 broker-backed memo table | query miss→publish→hit→invalidate/GC, capacity denial typed, 후속 정상 요청, close/drain |
| G7 Source integration | 현재 main | 001/002/003 계약 보존, 뒤늦은 caller/feature drift 없음, 최종 소스에서 G1~G6 및 G8 재검증 |
| G8 SQL shadow 제품 경계 | CLI `dist-full` + 별도 rulepack worker의 baseline/candidate | 프로세스별 bounded domain 설치, 두 leg의 합산 한도·거절·해제·재시도 검증. SDK broker와 다른 프로세스의 broker를 하나라고 주장하지 않음 |

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

완료: main에 포함된 001/002/003 변경의 G1~G8 evidence. owner GREEN, 제품
GREEN, main 통합, 최종 검증은 분리 기록한다. benchmark/디스크/프로세스 RSS
보장은 포함하지 않는다.
