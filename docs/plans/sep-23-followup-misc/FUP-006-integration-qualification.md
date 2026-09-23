# FUP-006 — 통합과 current-source qualification

Status: `VERIFIED — full-followup exact-source local, extraction, benchmark, and QBC host-pair GREEN; hosted CI/deploy/activation NOT RUN`
Priority: `SCOPE-SPECIFIC RELEASE GATE` (구현 결함 티켓이 아님)
Owner: `Lane I — serial integrator`
Depends on: core-enabled 후보는 `FUP-007`; durable-enabled 후보는 추가로 `FUP-001`, `FUP-002`; full-followup 후보는 추가로 `FUP-003`, `FUP-004`, `FUP-005`
Checklist: source-bound `M09/M10`, durable restart 주장 시 `M05`의 강제 종료/재시작 범위. 이 티켓 자체가 100항목 전체를 자동으로 닫지 않는다.

## 목적

owner-local GREEN을 제품 전체 qualification으로 승격하지 않는다. 하나의 고정 source에서
API/feature/extraction/host 영향과 회귀를 검증하고, 남은 경계를 명시한다.
이 티켓은 다른 lane의 누락 증거를 추정하여 채우는 작업이 아니다.

## 후보 scope를 먼저 고정

| 후보 | 필수 선행 티켓 | 허용되는 완료 표현 | 금지되는 확대 주장 |
| --- | --- | --- | --- |
| `core/no-redb` | 007 | core 권한 수명과 **Redb 비활성 graph가 증명된** profile의 source-bound 검증 | 001/002가 OPEN인데 durable 또는 `full-backings` PASS라고 말하기 |
| `durable-enabled` | 001→002, 007 | Redb state/claim/migration과 필요한 실제 restart·host gate까지 통과한 exact source | drop/reopen 단위 테스트만으로 forced kill/power loss/host activation GREEN 주장 |
| `full-followup` | 001→002, 007, 004/005→003 | 위 P1 계약과 P2 admission/oracle/provenance까지 동일 source에서 검증 | 개별 focused GREEN을 합쳐 전체 repo/host GREEN으로 승격하기 |

시작 전에 실제 Cargo feature graph와 host adapter가 `spill-redb`, reclaimer,
custom event sink를 사용하는지 읽기 전용으로 확인한다. 사용하지 않음을 증명하지
못한 capability는 `UNVERIFIED`다. F06/L04의 callback 재진입이 host에 설치돼 있으면
P2로 기계적으로 제외하지 말고 도달성과 우선순위를 다시 판정한다. 미사용 feature를
비활성으로 분리해도 FUP-007의 core 결함은 남는다.

## Write set

- 이 디렉터리의 README/티켓 상태와 새 `RECEIPT.md`(실행 후 작성).
- `README.md`, `README-detailed.md`, 필요 시 `MIGRATION.md`, `EXTRACTION.md`,
  `docs/verification-scenarios.md`, public examples의 실제 변경 계약.
- root re-export/공통 fixture 충돌의 최소 통합 수정은 owner 동의 후 수행한다.
- host repository 수정, version/tag 발행, commit/push, 배포는 별도 승인 대상이다.
  host read-only inspection/로컬 검증은 실제 작업 권한과 freeze 범위를 먼저 확인한다.

## 1. Writer admission 종료와 source freeze

1. 선택한 후보에 필요한 각 lane의 pre/postimage, 변경 파일, RED→GREEN, 실제 selected count, API/DB migration,
   lease 반환을 수집한다. proof가 없으면 해당 티켓은 IMPLEMENTED까지만 인정한다.
2. baseline 이후 외부 커밋과 dirty 소유권을 다시 확인한다. 기존 untracked 문서를
   검증 편의를 위해 삭제하거나 전체 stage에 포함하지 않는다.
3. 승인된 통합 후보를 clean commit 또는 격리된 clean snapshot으로 고정한다.
   도구가 commit을 요구하면 필요한 승인을 받는다. 계획 생성 자체는 commit 권한이 아니다.
4. HEAD/tree, owner hashes, features, toolchain/machine, dependency identity를 기록한다.
   정확한 소스 경로를 Cargo metadata로 확인하고 sibling checkout을 혼동하지 않는다.

## 2. 통합 검증

- durable-enabled/full-followup 후보에서는 실제 Redb의 record state와 recovery lease를 함께 검증: ID hole, reopen, 두 store,
  foreign/stale claimant, unknown ACK/probe, exact one publication, 최종 charge 반환.
- 같은 후보에서 FUP-001의 낮은 hole→높은 finalized ID가 `Absent`이고, FUP-002의 두 새 store
  barrier race가 정확히 한 `Restored`만 내는 owner-local negative oracle를
  **같은 후보 소스에서** 다시 실행한다. 기존 임시 clone의 exit 101은 RED preimage다.
- FUP-007의 committed explicit release/Drop 및 pending rollback/Drop을 group
  제거 재설정과 rebuild fence 아래 재검증한다. 재설정 reject 또는 retiring group
  계약이 일관되며 최종 live ticket/deferred backlog가 0이어야 한다.
- durable-enabled/full-followup 후보에서는 정상 persistent fixture와 legacy fixture 각각에서 안전한 migration 또는 명시적
  unresolved error를 확인. destructive DB 초기화 fallback은 금지.
- full-followup 후보에서는 reclaim callback 중 reconfigure와 store release가 결합돼도
  initial/final revision, decision receipt, ledger/group usage가 일치하는지 검증.
- public examples, re-export, external consumer compile-fail, disabled-feature build를
  확인. 새 API가 broker/raw authority 우회 경로를 만들지 않아야 한다.
- core/testkit dependency 방향과 pure domain/ports를 유지한다. stale snapshot/receipt를
  테스트 expected value에 복사하여 GREEN을 만들지 않는다.
- durable restart recovery를 출하 계약에 포함하면 child process를 실제 종료하고
  새 process가 같은 Redb 파일을 reopen하는 rail을 둔다. write/commit/ACK/source
  release/restore/delete/finalize 중 **어떤 경계가 시험됐는지** 각 행에 기록한다.
  drop/reopen simulation, fault-injected ACK, forced process termination, power-loss
  durability를 서로 다른 증거로 취급한다. power-loss는 실행하지 않았다면 미검증.

## 3. 검증 명령과 중복 제거

먼저 선택한 scope의 owner-local 001/002/007 및 해당 P2 티켓의 신규 selector가 실제로
선택되는지 확인한다. `0 passed / 0 selected`는 증거가 아니다. 이후 canonical
rail을 한 번 실행하며, 이미 그 rail이 실행한 동일 suite를 근거 없이 반복하지 않는다.

```bash
scripts/check-workspace.sh
cargo run --locked -p quanta-memory-platform-facade --example hello_world
cargo run --locked -p quanta-memory-platform-facade --example effective_budget_report
```

- `tools/pm/sources/**`를 변경한 경우에만 sync 후 lint한다. generated AGENTS/CLAUDE는 직접 수정하지 않는다.
- `core/no-redb`는 `--no-default-features --features full` 및 실제 host dependency
  graph에서 Redb 비활성임을 확인한다. `--all-features` GREEN을 durable 결함 해결의
  대체 근거로 쓰지 않는다. `durable-enabled`는 `full-backings` graph와 owner-local
  Redb 계약을 별도로 명시한다.
- Cargo manifest 또는 cross-crate use path 변경 시 clean committed candidate에서 extraction
  preflight를 실행한다. 기존 receipt가 약속한 full standalone qualification을 갱신하려면
  dry-run뿐 아니라 실제 추출물의 canonical rail까지 실행한다.
- extraction destination은 **새 비어 있는 절대 경로**로 정하고 receipt에 기록한다.
  기존 `/tmp` 디렉터리를 덮어쓰거나 broad cleanup하지 않는다.
- 003/004가 hot path 구조를 바꾸면 기존 scaling harness의 해당 cardinality/throughput
  계약을 source-bound로 확인한다. `--smoke`만으로 complexity나 contention을 증명하지 않는다.
- 모든 전체 검증의 시작/종료 source hash가 달라지면 그 실행은 새 후보의 qualification이
  아니다. 마지막 성공 stage를 새 소스의 PASS로 합성하지 말고 영향 범위를 재실행한다.

## 4. Host / qualification 경계

- 001/002의 durable API, 007의 live reconfiguration, 003의 reclaim API를 실제
  host가 사용하는지 확인한다. 사용하면
  caller migration과 정확한 host SHA + QMP SHA 조합의 compile/contract 검증이 필요하다.
- 사용하지 않는다면 그 근거를 남긴다. 사용 여부 미확인은 PASS가 아니라 `UNVERIFIED`다.
- 공유 sibling dependency를 사용하는 host QBC 중에는 코드뿐 아니라 문서 변경도
  source digest를 바꿀 수 있다. 요청된 freeze/release를 지키고 실제 tuple receipt를 확인한다.
- 로컬 저장소 GREEN, standalone GREEN, host-pair GREEN, hosted CI, deploy, activation을
  별도 열로 기록한다. receipt/tag 생성과 외부 발행은 별도 승인이 있을 때만 수행한다.

### Exact-source host-pair result (2026-09-23)

Dirty, active source checkouts were not used as build inputs. A clean sibling
bundle at `/private/tmp/qmp-v5-host-pair-bundle-20260923` pinned the QMP,
Semantica, Quanta Index, GQLang, and Taskmesh commits listed in `RECEIPT.md`.
Both Semantica Cargo metadata roots resolved QMP dependencies to that bundle.
With expected snapshot digest pinned, the compose target passed 45/45 and
`quanta-memory-host --tests --all-features` passed 126/126. Raw receipt paths,
run IDs, and the common before/after digest are in `RECEIPT.md`.

This closes the exact-source local host-pair gate only. Hosted CI, deployment,
activation, power-loss durability, and instruction-point kill during finalize
remain outside this proof.

## 완료 기준

- [x] 선택한 scope의 001/002/007 및 포함된 003/004/005의 regression과
  negative proof가 같은 최종 소스에서 유효.
- [x] canonical rail 성공, 예제/API/feature 계약 성공.
- [x] `core/no-redb`이면 Redb 비활성 graph 증거. `durable-enabled`이면 legacy
  migration, 배타적 recovery, 실제 process 종료/재시작의 적용 경계와 미시험
  power-loss 범위가 명시됨.
- [x] 필요한 extraction와 host-pair 검증 성공 또는 미검증 범위가 명시적으로 남음.
- [x] 저장 포맷 호환성, opaque identity/lease, 오류/receipt API의 migration 안내 완성.
- [x] source-before/source-after 일치; 모든 실패·미실행·잔여 위험이 receipt에 포함.
- [x] 코드만 VERIFIED인 상태를 deployment/activation 완료로 표기하지 않음.

`B07`의 0-byte metadata 개수 상한, `J06` backlog 상한, `I03/I05/I06/I09`의
durability/physical-resource/format 계약, custom callback 재진입과 host 의미 경계는
해당 scope에서 따로 검증·명시하지 않으면 OPEN이다. **P1 세 root를 고쳤다 =
100점 체크리스트 완료**라고 기록하지 않는다. M09 host E2E와 M10 receipt도 실제
host와 raw 결과가 없으면 미검증이다.

`RECEIPT.md`에는 command/exit/selected count, source SHA/tree/digest, dependency tuple,
artifact 경로를 기록한다. 최소 하나의 필수 rail이 미실행이면 전체 `QUALIFIED` 대신
정확한 부분 상태를 사용한다. plan 문서 작성만으로 이 체크박스를 채우지 않는다.

## Handoff 실제 기록 (Lane I, 2026-09-23)

Scope: `full-followup`. Candidate lineage: `0284489` (plan base) →
`beba437` (first clean implementation candidate) → `c7706d0` (peer
review + self-audit delta, committed) → then-uncommitted workdir
lazy-breach delta (`admission.rs` + `scheduler.rs`) at Lane I handoff.

Changed (Lane I owned): reclaim×reconfigure coupling tests (2),
SIGKILL/reopen rail (integration test + crash-child example + manifest
entry), self-audit oracles (`sep23_audit_` 10 core + 4 facade),
randomized preflight/attempt agreement, lazy `GroupAdmission` breach
(zero-alloc `admits` restoration), docs (`RECEIPT.md` completion,
`MIGRATION.md`, `CHANGELOG.md`, verification rows, all handoffs).

Historical v3/v4 source verification: full mutant battery RED (15
mutants incl. KILL2 identity re-mint and G1 deny-vanish); per-ticket
selectors GREEN (001:6, 002:8, 003:7, 004:4+4, 005:2, 006:2+3, 007:8);
`scripts/check-workspace.sh` exit 0; both public examples run;
no-default/spill-only/full feature checks GREEN with `redb` absent from
the `full` graph; scaling verifier OK (170 samples).

Historical extraction: real extraction + in-extract rail GREEN on `c7706d0`
(bootstrap `276893f`, tree `8e10fc78…`). The lazy-breach delta is
intra-crate (no manifest/use-path change); the v2 annotated tag owns the
repeated exact-source extraction receipt.

Historical host-pair: the Semantica-owned default-provenance projection was
repaired; QBC compose 45/45 and host 114+5+7 are GREEN on the v3/v4 source
digest. This does not qualify the two-test v5 source delta. Details and reopen conditions are in
`tickets/HOST-PAIR-RUNTIME-DEFAULT-PROVENANCE.md`.

Not run: hosted CI; deploy/activation; power loss; mid-finalize
instruction-point kill.

Promotion authority: predecessor tags `qmp-final-qualified-sep23-followup`,
`qmp-final-qualified-sep23-followup-v2`, v3, and v4 remain immutable. The
updated candidate is qualified only when all repository, extraction,
benchmark, and host-pair rails pass for its exact source digest and a new
annotated v5 tag targets the clean candidate.
