# quanta-memory-platform remediation tickets — SSOT

이 디렉터리는 2026-09-21 시스템 감사에서 확인된 결함과 proof gap을 실제 코드 변경으로 닫기 위한 **유일한 작업 SSOT**다.

- 이 파일은 ticket inventory, 상태, 우선순위, 의존성, 실행 순서의 SSOT다.
- 각 `<ticket-id>/TICKET.md`는 해당 ticket의 코드 경계, 구현 계약, 실패 처리, 테스트, 완료 조건의 SSOT다.
- `AUDIT-BASELINE.md`는 동결된 감사 evidence다. 상태를 표현하지 않는다.
- `AUDIT-DELTA-2026-09-21.md`는 구현 중 재검증한 신규·수정 finding의 source-bound evidence다. 현재 상태의 권위는 이 inventory다.
- `FINAL-AUDIT-2026-09-21.md`는 `dfb58a6` 제품 source에 대한 최종 adversarial audit, 신규 finding, 실행 순서와 hellgate의 SSOT다.
- `REFERENCES.md`는 설계 기준과 외부 reference의 SSOT다.
- 루트 `AUDIT-2026-09-21.md`와 `SYSTEM-AUDIT-CHECKLIST.md`는 감사 산출물이며 ticket 상태 권위가 아니다.
- source가 바뀌면 과거 receipt는 해당 SHA에만 유효하다. 현재 상태로 자동 승격하지 않는다.

## 1. 동결 기준

| 항목 | 값 |
| --- | --- |
| Audit base | `e103771670da3143e2ffebb8950a5dfc7e3d4515` |
| Branch | `main` |
| Audit date | `2026-09-21` |
| Product source dirty at freeze | 없음 |
| Auditor-owned untracked at freeze | `AUDIT-2026-09-21.md`, `SYSTEM-AUDIT-CHECKLIST.md` |
| Final re-audit product source | `dfb58a68ff2a859e829e18e4649a8da80ddd262c` on `codex/qmp070-spill-contract`, clean before audit-doc edits |
| Overall verdict | `FAIL` |
| Frozen baseline severity | P0 0 / P1 10 / P2 15 |
| Current cumulative severity | P0 0 / P1 17 / P2 20 |

구현 시작 시 worker는 현재 `HEAD`, branch, dirty paths를 다시 기록해야 한다. 위 SHA는 발견 근거이지 미래 구현 base를 강제하지 않는다.

### Latest verified product checkpoint

```text
product source: 0c126a25aeab11f457e20c4a209fc7bb631ea859
branch/worktree: codex/qmp-remaining-hardening / isolated clean worktree
workspace tests: 620 passed
doctor output SHA-256: fe23223aa751c81c06bd2cfa7d43321e013b63606e1d54da80b901f8ecb52bd7
extraction output SHA-256: b568d1dc60e7572dd3b32d1958aa606350df3d56083b0d93f21722221267a4a3
extracted bootstrap: 381e25871cc2981ee10759dff38ebd8c2f6283f0 (clean)
scaling smoke output SHA-256: 87da472630e14467ecc0a5236058f63e9de94585db464b999d4c382dbc0e1a44
```

This checkpoint qualifies only the ticket scopes whose latest receipts say
`QUALIFIED`. It does not close QMP-050, QMP-090, QMP-100, QMP-110, remote QMP-120,
external-host QMP-130, or final SemVer/docs QMP-140. Inventory state below is
the lifecycle authority; this block is exact proof metadata, not a status
override.

## 2. 상태 vocabulary

| 상태 | 의미 |
| --- | --- |
| `READY` | 선행 계약과 compatible current-source receipt가 충분히 안정적이다. claim 후 구현 가능 |
| `BLOCKED` | `depends_on`의 현행 호환 receipt/계약이 없거나 재개방돼 불안정하다. read-only design은 가능하지만 owner mutation은 대기 |
| `IN_PROGRESS` | 한 owner가 claim했고 write set이 잠김 |
| `IMPLEMENTED` | 코드와 focused tests는 존재하지만 terminal rails 미완료 |
| `QUALIFIED` | ticket acceptance와 요구 receipt가 exact source에서 모두 통과 |
| `DONE` | 후속 consumer/adoption까지 닫혀 ticket 목적이 완결됨 |
| `REJECTED` | 설계 결정으로 폐기. 대체 ticket/결정 근거 필수 |

`IMPLEMENTED`나 focused GREEN을 `DONE`으로 해석하지 않는다.

## 3. Ticket inventory

| ID | 우선순위 | 상태 | 목적 | depends_on | 주요 write set |
| --- | --- | --- | --- | --- | --- |
| [QMP-000](QMP-000-campaign-admission/TICKET.md) | P1 | IMPLEMENTED | status/claim, receipt identity, committed-source digest, selected-count, and current-transition admission enforced; exact-head dynamic rerun deferred | — | `tickets/**`, `tools/tickets/**`, validator front door |
| [QMP-010](QMP-010-policy-totality/TICKET.md) | P1 | IMPLEMENTED | mandatory class totality와 validated policy authority | QMP-000 | core `policy.rs`, `config.rs`, builder/policy tests |
| [QMP-020](QMP-020-reservation-lifecycle/TICKET.md) | P1 | IMPLEMENTED | raw/discardable authority sealed behind managed RAII; poisoned commit retains rollback authority; runtime qualification deferred | QMP-010 | core `ledger.rs`, recharge/control seams |
| [QMP-030](QMP-030-enforced-builder-runtime-view/TICKET.md) | P1 | QUALIFIED | enforced builder/runtime scratch and managed RAII authority; current builder/control/public surface refreshed at `f9f6d15` | QMP-010, QMP-020 | core/facade builders, broker/control/scratch API, examples |
| [QMP-040](QMP-040-store-lifecycle/TICKET.md) | P1 | QUALIFIED | five-store lifecycle oracle, exact close/drop authority and hostile panic-payload containment pass at `0c126a2` | QMP-020, QMP-030 | facade shared lifecycle + all store services |
| [QMP-050](QMP-050-total-retained-cost/TICKET.md) | P1 | IMPLEMENTED | total retained-cost and independent underbilling oracles pass at `0c126a2`; public migration/spill-identity cost open | QMP-040 | measure/accounting + store entry cost paths |
| [QMP-060](QMP-060-transcript-tombstones/TICKET.md) | P1 | QUALIFIED | bounded fail-closed evidence horizon, typed stats/reconfigure, 620-test doctor and real extraction refreshed at `0c126a2` | QMP-050 | transcript types/service/builder |
| [QMP-070](QMP-070-spill-transfer-backings/TICKET.md) | P1 | QUALIFIED | recovery-payload/ack protocol preserved by VecDeque rewrite; 620-test doctor and extraction refreshed at `0c126a2` | QMP-020, QMP-040, QMP-050 | revocable/spill/backings/builders |
| [QMP-080](QMP-080-policy-runtime-semantics/TICKET.md) | P2 | QUALIFIED | canonical reserve/recharge/reconfigure postimages, protected floors, bounded out-of-lock reclaim, weighted burst and provenance pass at `f9f6d15` | QMP-010, QMP-020, QMP-030 | scheduler/controller/control/config |
| [QMP-090](QMP-090-conformance-model/TICKET.md) | P1 | IMPLEMENTED | 33 current broken/honest oracle tests pass at `0c126a2`; proptest/Loom/Miri 및 독립 context/group/live-ticket 축 open | QMP-020, QMP-040, QMP-070 | testkit conformance/fakes/faults |
| [QMP-100](QMP-100-scaling-benchmarks/TICKET.md) | P2 | IMPLEMENTED | payload indexed buckets/slot reuse and revocable VecDeque remove confirmed O(N²) paths; source-bound smoke exists, contention/memory/core threshold proof open | QMP-040, QMP-050, QMP-070 | payload/revocable containers, benches, CI scripts |
| [QMP-110](QMP-110-operability-observation/TICKET.md) | P2 | READY | QMP-030/040/080 prerequisites are qualified; typed observation and bounded broker/store telemetry remain unimplemented | QMP-030, QMP-040, QMP-080 | stats traits, metrics adapters, broker/store emitters |
| [QMP-120](QMP-120-standalone-extraction/TICKET.md) | P1 | IMPLEMENTED | local HEAD/tree/blob/mode-bound extraction and negative fixtures pass at `4fdac520`; remote workflow remains unobserved | QMP-000 | extraction manifest/scripts, lockfile, release CI |
| [QMP-130](QMP-130-host-adoption/TICKET.md) | P1 | BLOCKED | Semantica currently resolves adjacent shared-main QMP, not the audited/qualified revision; exact-root qualification absent | QMP-030..QMP-120 | external host repo only; QMP repo read-only except receipts |
| [QMP-140](QMP-140-semver-docs-closeout/TICKET.md) | P2 | BLOCKED | public API/SemVer, stale test selectors/docs, receipt schema and final closeout | QMP-010..QMP-130 | public exports, README/EXTRACTION/docs, Cargo metadata |

상태를 바꿀 때 이 표와 ticket receipt를 같은 change에서 갱신한다.

## 4. 감사 finding crosswalk

| Finding | Ticket | Closure proof |
| --- | --- | --- |
| P1-01 mandatory class 누락 fail-open | QMP-010 | build/reconfigure negative test + ledger deny |
| P1-02 live runtime scratch ledger 우회 | QMP-030 | runtime-view E2E charge/deny/refund |
| P1-03 default builder report-only | QMP-030 | canonical example over-cap denial |
| P1-04 public raw authority bypass | QMP-030, QMP-140 | compile-fail/public API diff + migration |
| P1-05 charged store drop charge leak | QMP-020, QMP-040 | abandoned guard rollback + store close/drop tests |
| P1-06 late sink attach mixed residency | QMP-040 | compile-fail typestate tests |
| P1-07 key/identity/index/tombstone undercount | QMP-050 | total-cost exact charge tests |
| P1-08 transcript tombstone unbounded/unaccounted | QMP-060 | count+byte saturation and reuse tests |
| P1-09 reclaim→in-memory spill loses ownership | QMP-070 | source/destination receipt transfer E2E |
| P1-10 actual extraction fails | QMP-120 | fresh actual extraction + clean git + locked build |
| P1-D01 saturated admission issues an unaccounted live ticket (2026-09-21 delta) | QMP-020 | `u64::MAX-10` 뒤 `100` 거부 + exact post-release accounting |
| P1-D02 generic store delete/remove can abort on panicking destructors (2026-09-21 delta; locally closed at `0c126a2`) | QMP-040 | two-field/outer-unwind/hostile-payload subprocess + exact refund, repeatable readable postimage |
| P1-F01 public cloneable raw ticket can be dropped while charge remains live | QMP-020, QMP-030, QMP-140 | raw ticket/root exports removed or made owned RAII; compile-fail bypass probe + drop/refund E2E |
| P1-F02 durable spill has ambiguous commit/retry outcomes, sequence ABA and panic ownership loss | QMP-070, QMP-090 | persisted intent/idempotency fence, monotonic identity, fault-injected reconcile/retry/panic tests |
| P1-F03 extraction is not bound to committed HEAD bytes or executable modes | QMP-120, QMP-140 | dirty tracked-byte/mode negative fixtures + HEAD/tree digest equality |
| P1-F04 host gate target and Cargo-resolved QMP revision can diverge | QMP-130 | canonical dependency metadata emits path+SHA; gate fails mismatch before tests |
| P1-F05 documented test selectors can exit 0 after selecting zero tests | QMP-000, QMP-140 | machine-readable command/count receipt; every named selector requires selected > 0 |
| P2-01 inert policy vocabulary | QMP-080 | every public policy field has runtime consumer tests |
| P2-02 identity reconfigure fails under legal borrow | QMP-080 | current-policy reconfigure under borrow passes |
| P2-03 public scheduler mutation side channel (baseline; current API removed) | QMP-080, QMP-140 | removal API guard + exact reconciliation test; do not re-add symbol |
| P2-04 commit has no state / abandoned ticket | QMP-020 | state-machine compile/runtime tests |
| P2-05 stale reconfigure provenance | QMP-080, QMP-110 | revision/source postimage assertion |
| P2-06 reclaim ladder disconnected | QMP-080 | broker slow-path reclaim E2E |
| P2-07 black-hole conformance target can pass | QMP-090 | negative fake rejected with localized verdict |
| P2-08 accounting callback under store mutex | QMP-040, QMP-090 | reentrant sink test + lock-order/model proof |
| P2-09 poison reported as zero | QMP-110 | typed observation failure tests |
| P2-10 moka/scc compile-only backings | QMP-070 | canonical backing port parity suite or feature removal |
| P2-11 spill count wrong after restart | QMP-070 | reopened redb controller reconciliation |
| P2-12 payload/revocable/core release O(N²), global mutex (facade paths closed; core/threshold work open) | QMP-100 | indexed payload + VecDeque receipt; affected-slot core preflight, contention and thresholds remain |
| P2-13 saturating counter hides live bytes | QMP-040, QMP-050 | checked overflow fail-closed tests |
| P2-14 noop spill discards and succeeds | QMP-070 | typed unsupported or explicit discard API test |
| P2-15 public bumpalo bypass | QMP-030, QMP-140 | budgeted wrapper or public removal compile test |
| P2-D01 client callback panic can poison a live store (2026-09-21 delta) | QMP-040 | callback panic then readable/retryable preimage + no ledger drift, or explicit supported-client contract |
| P2-F01 poisoned commit can consume its rollback ticket and strand accounting | QMP-020, QMP-110 | poisoned-lock commit fault test; rollback retains authority or reports recoverable owned error |
| P2-F02 conformance model omits group/context/live-ticket/overflow/rebuild/concurrency axes | QMP-090 | independent state expansion + shrinkable property traces + Loom/Miri rails |
| P2-F03 docs and ticket commands name stale semantics/nonexistent selectors | QMP-000, QMP-140 | doc-command execution audit with selected-count checks and generated current-source claims |
| P2-F04 in-process spill deep-clones uncharged binding strings per entry | QMP-050, QMP-070 | shared/bounded binding identity or exact charged cost; adversarial long-identity accounting test |
| Proof: telemetry unwired | QMP-110 | event emission/cardinality tests |
| Proof: no contention threshold/proptest/loom | QMP-090, QMP-100 | current smoke exists; generative/concurrency rails and enforced thresholds remain |
| Proof: feature warnings allowed | QMP-100, QMP-120 | `-D warnings` matrix |
| Proof: docs claim no limitations | QMP-140 | docs generated from current ticket state |
| Proof: host non-memory ExecutionBudget helpers unused | QMP-130 | host call-site census + behavior tests |

모든 confirmed finding은 정확히 하나 이상의 owning ticket에 배정됐다. `QMP-140`은 새로운 동작을 소유하지 않고 public compatibility/docs closure만 소유한다.

## 5. Dependency graph

```text
QMP-000 freeze/admission
  ├─ QMP-010 policy totality
  │    └─ QMP-020 ticket lifecycle
  │         └─ QMP-030 enforced builder/runtime view
  │              ├─ QMP-040 store lifecycle
  │              │    ├─ QMP-050 retained cost
  │              │    │    ├─ QMP-060 transcript bounds
  │              │    │    └─ QMP-070 spill transfer
  │              │    └─ QMP-100 performance
  │              ├─ QMP-080 policy runtime semantics
  │              └─ QMP-110 operability
  │
  └─ QMP-120 extraction reproducibility

QMP-090 conformance waits for QMP-020/QMP-040/QMP-070 stable contracts
QMP-130 host adoption waits for all QMP product/extraction tickets
QMP-140 SemVer/docs closeout waits for QMP-130
```

## 6. Safe execution lanes

초기에는 다음만 병렬 가능하다.

| Lane | Ticket | 제약 |
| --- | --- | --- |
| A | QMP-010 | core policy/config만 소유 |
| B | QMP-120 | scripts/self-hosting/lockfile만 소유 |
| C | QMP-090의 black-hole fixture 설계 | testkit-only; production API 변경 전 merge 금지 |

이후 serial owner barriers:

- `ledger.rs`: QMP-020 단독
- `api/{builder,broker,control}.rs`: QMP-030 후 QMP-080
- `facade/src/shared/**`와 store structs: QMP-040 후 QMP-050
- transcript: QMP-060 단독
- revocable/spill: QMP-070 단독
- public exports/manifests/docs: QMP-140 serial integration

같은 파일을 두 ticket이 동시에 수정하지 않는다. overlap이 생기면 상위 ticket을 split하지 말고 dependency 순서를 따른다.

## 7. Ticket execution contract

각 ticket worker는 다음 순서를 지킨다.

1. `git rev-parse HEAD`, branch, `git status --short` 기록.
2. foreign dirty path가 write set과 겹치면 구현 중단.
3. ticket의 current-source evidence를 재확인. line number는 힌트이며 symbol/behavior가 권위다.
4. 명시된 RED test를 먼저 추가하고 예상 실패 원인을 기록.
5. canonical owner를 수정. consumer shim이나 duplicate counter를 추가하지 않는다.
6. focused tests → owning crate all-features → workspace rails 순서로 실행.
7. Cargo/use path/public API 변경 시 actual extraction까지 실행.
8. receipt를 `tickets/<id>/receipts/<timestamp>-<sha>.md`에 남기고 status 변경을 제안.

Active claim은 [`CLAIM-TEMPLATE.md`](CLAIM-TEMPLATE.md)를 복사해
`tickets/claims/<ticket-id>-<owner>.md`에 두며, claim 종료 시 제거한다.
`scripts/verify-tickets.sh`는 inventory/dependency/finding/claim/receipt 체인을
negative fixture와 실제 tree 양쪽에서 검증하는 canonical admission command다.

## 8. Receipt evidence levels

| Level | 의미 | terminal 여부 |
| --- | --- | --- |
| `STATIC` | source/reachability/dependency census | 아니오 |
| `RED` | 결함을 재현하는 실패 test | 아니오 |
| `FOCUSED` | owner-local selector PASS | 아니오 |
| `CRATE` | owning crate all-features PASS | 아니오 |
| `WORKSPACE` | fmt/clippy/test/deny/feature matrix PASS | 조건부 |
| `EXTRACTION` | fresh standalone copy에서 locked rails PASS | standalone terminal 필수 |
| `HOST` | external adapter exact-head behavior PASS | host adoption terminal 필수 |
| `PERF` | frozen workload/toolchain threshold PASS | 성능 ticket terminal 필수 |

receipt가 없는 “구현됨”, stale SHA, source digest 변경, command 미실행은 closure가 아니다.

## 9. Campaign completion gate

전체 campaign은 다음이 모두 참일 때만 `DONE`이다. 원본 P1 10개, delta P1 2개, final audit P1 5개로 누적 P1은 17개다. 원본 P2 15개, delta P2 1개, final audit P2 4개로 누적 P2는 20개다. 원본 동결 판정/숫자는 변경하지 않는다. 신규 finding의 proof 수준은 delta/final audit에 명시한다.

- 누적 P1 17개가 owning ticket의 negative test로 재현되고 fixed behavior를 통과한다.
- 누적 P2 20개가 구현 또는 명시적 API 제거/정책 결정으로 닫힌다.
- public raw reservation ticket를 drop/clone해도 charge authority를 유실할 수 없다.
- redb spill의 commit 성공/실패/unknown outcome이 persisted intent와 idempotent reconcile로 수렴한다.
- default public example이 실제 cap denial과 ledger drain을 보인다.
- 모든 retained/scratch path가 하나의 accounting authority 또는 명시적 unbudgeted type에 속한다.
- conformance suite가 black-hole/commit-swallow/release-swallow/observation-error target을 거부한다.
- actual extraction이 `--locked`로 통과하고 extracted git status가 clean이다.
- extraction byte/mode manifest가 committed HEAD tree와 일치하고 dirty tracked source를 거부한다.
- Semantica host가 exact QMP revision으로 rewire되어 runtime/store behavior tests를 통과한다.
- host gate가 실제 Cargo-resolved QMP root와 SHA를 검증하고 다른 checkout을 검사할 수 없다.
- 모든 receipt의 named selector가 1개 이상의 test를 실행하며 command/result/count/source identity가 machine-check된다.
- README, EXTRACTION, verification docs의 claim이 receipt와 일치한다.
- 최종 source SHA에서 workspace, extraction, host, perf 요구 receipt가 모두 존재한다.

## 10. 금지되는 closeout 방식

- `unwrap_or_default`로 poison/observation error를 zero state로 바꾸기
- raw ledger/store constructor를 유지한 채 문서로만 “사용 금지” 선언
- store consumer에서 누락 byte를 임의 상수로 덧붙이는 shim
- in-memory spill을 RSS 감소로 간주
- test assertion을 약화해 기존 GREEN 유지
- dry-run extraction을 actual extraction 증거로 사용
- focused test를 host/product qualification으로 승격
- public API 제거를 SemVer/migration 계획 없이 수행
