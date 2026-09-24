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

Earlier G8 finding (before Semantica `10413dfe6c8`): the public
`sql-family-product-runtime` builder created unbudgeted in-memory
artifact/evidence/memo stores. `quanta-cli-ingress-runtime`의
`sql-family-runtime-activation` (`dist-full` 포함)과 별도 프로세스
`quanta-sql-rulepack-worker`가 이 경로를 실제로 사용했다. 이 발견은 아래
구현으로 소스상 수정됐지만 실행 증거가 아직 없다. SQL 경로를 SDK G6
통과로 합산하지 않는다.

2026-09-25 implementation update: Semantica main `95fe5446aed`에는 CLI와
rulepack worker가 각각 프로세스 로컬 singleton `RuntimeCutoverMemoryDeps`를
통해 SQL 전용 QMP production preset domain을 설치하고, 매 baseline/candidate
leg마다 분리된 artifact/evidence store와 broker-backed memo table을 발급하는
변경이 들어갔다. SQL domain은 SDK installed domain과 **별개**이며, QMP
production preset의 4 GiB global hard limit을 각 프로세스에 별도로 적용한다.
따라서 두 domain의 합산 프로세스 RSS를 4 GiB로 보장하지 않는다. Composer에는
두 leg의 memo charge 합산·개별 환급과 shared-pool peer denial→release→retry
oracle이 작성됐다. 이 단락은 구현·테스트 소스 상태이지 G8 GREEN이 아니다.
QBC compiler/owner, CLI `dist-full`, worker process 제품 실행과 실제
거절·해제 관측을 고정된 최종 소스에서 별도로 판정한다. CLI의
`bootstrap_scope_direct_with_paired_rulepack_sources_v1`는 현재 정의만 있고
호출자가 확인되지 않았다. 따라서 CLI `dist-full` 컴파일·E2E GREEN은 feature
closure 증거이지 SQL paired 실행 증거가 아니다. 현재 도달 가능한 SQL paired
제품 경로는 별도 rulepack worker이고, 그 프로세스 테스트와 composer의
admission/refund oracle을 각각 확인해야 한다.

2026-09-25 실행 판정: Semantica 고정 스냅샷
`ffc285adef9316b7a62598b02d202a3b10f8ae02+ext-3932ca34c00dcfc40ba8`에서
`quanta-runtime-memory-compose --lib --all-features` owner rail은 55/55 GREEN이다.
두 SQL shadow-leg 합산·격리 및 peer denial→release→retry 테스트가 실제로 실행됐다.
같은 스냅샷의 `quanta-cli-ingress-runtime --lib --all-features`는 runtime
retrieval/targeted-recompute 경로의 37개 compile error로 RED였다. 같은
스냅샷의 worker `--tests --all-features`는 실행 도중 Semantica main에
candidate-failure baseline drain 변경(`9c0b9181fe6`)과 다른 통합 변경이
들어와 최종 소스와 달라졌고, 중단되어 GREEN receipt가 없다. 이후 main
`73b24ff9cb5`에 다른 경로의 변경도 들어왔으므로 G7/G8은 여전히 OPEN이다.
새 최종 소스에서 composer·CLI feature closure·worker process 증거를
다시 묶어 판정한다.

후속 고정 스냅샷
`73b24ff9cb56129211c8299b1cf81400cc0de5c5+ext-fcff8df2cdf05ac707a9`의
CLI `--lib --all-features`도 compile RED였다. 이번 실패는
`quanta-sdk-session-backend-owner`의 `ReadIndexRepositoryWorkItemV1` 초기화에서
`binary_content_digest_v1` 누락(E0063)이다. 이후 Semantica main의 해당
초기화에는 필드가 추가됐으므로 이 RED를 현 HEAD 실패로 일반화하지 않는다.
최종 메인에 대한 CLI GREEN과 worker 제품 GREEN은 아직 확보되지 않았다.

별도 `semantica-shadow-budget-final` 체크아웃의 worker owner rail은
`worker_process_canary_owns_timer_runtime_v1`를 실제 실행했으나 RED였다.
baseline product runtime 구성 중 `RUNTIME.QUERY_ARGS_SOURCE_CLOSURE_MISMATCH`
(`sql_canary.py`, `QueryId(82)`)가 발생했다. 해당 체크아웃은 이 경로를
수정 중인 dirty 상태이므로, 이 결과는 최종 main verdict가 아니다. 수정 후
고정 소스 worker process 재실행과 성공 receipt가 필요하다.

2026-09-25 후속 판정: 전용 Semantica 브랜치
`codex/shadow-budget-closure`의 `dac08123e51`에는 제품 source authority가
코드 source member와 `pyproject.toml` 같은 workspace input을 별도로 고정하고,
SemanticModule 하위 질의에는 발급된 workspace union을 전달하며,
no-runtime-persist 질의에는 같은 실행의 pre-digest artifact를 재사용하는
수정이 커밋·푸시됐다. 해당 수정으로 worker 통합 테스트
`worker_process_canary_owns_timer_runtime_v1`는 컴파일 후 실제 프로세스를
실행했고, 기존 `QueryId(82)` closure mismatch를 통과했다. 그러나 baseline
`PTA_PRECISE`가 `LOWERING_CAPABILITY_BOUNDARY` /
`pta_external_boundary_occurrence_missing_or_ambiguous_external_edge`를 반환해
`dual_run` 단계에서 여전히 RED였다. 이는 제품 PASS나 main 통합 증거가 아니다.
외부 경계 packet과 call-graph edge의 caller/callsite/origin/callee 일치 조건을
독립 진단하고, 소스 또는 fixture의 결함을 수정한 뒤 같은 process E2E를
최종 고정 소스에서 GREEN으로 만들어야 G8을 닫을 수 있다. 브랜치의
owner-local 신규 테스트는 아직 실행되지 않았다. RED 브랜치를 main으로
병합하거나 예전 composer GREEN을 최종 qualification으로 재사용하지 않는다.
임시 계수 진단을 넣은 같은 worker 테스트의 재실행에서는
`sql_canary.product_sql_execute` → `sqlite3.connect` packet 행에 대해
해당 caller edge 0개였고, 그래프 전체에서도 같은 callsite ID 또는 origin의
edge가 각각 0개였다. 이는 PTA의 이름 비교만 느슨하게 해서 해결할 문제가
아니라 call-graph packet/edge 생성·전달의 불일치다. 임시 진단 코드는
출하 브랜치에 남기지 않았다.
추가 계수 진단에서 해당 call graph는 edge 2개·function 6개였고
`direct_call_target_not_found_in_canonical_interproc_authority` 및
`call_graph_trace_diff_unavailable` partial reason을 보유했다. budget
exhaustion 증거는 없었다. `sqlite3.connect`의 canonical direct-call target
해결 또는 제품 fixture의 source/import authority를 먼저 교정하고,
partial graph를 promotable baseline으로 취급하지 않는지 재검증해야 한다.

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
| G8 SQL shadow 제품 경계 | CLI `dist-full` feature closure + 실제 rulepack worker baseline/candidate + composer admission oracle | worker의 프로세스별 bounded domain 설치, 두 leg의 합산 한도·거절·해제·재시도 검증. CLI helper는 호출자가 생기기 전까지 제품 실행으로 합산하지 않음. SDK broker와 다른 프로세스의 broker를 하나라고 주장하지 않음 |

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
