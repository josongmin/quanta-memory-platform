# quanta-memory-platform 시스템 코드 감사 체크리스트

이 문서는 현재 구현이 저장소의 목적을 실제로 달성하는지 검증하기 위한 실행 계약이다.
문서에 적힌 의도, 테스트 이름, 과거 GREEN만으로 PASS하지 않는다. 모든 판정은 하나의
고정된 source snapshot과 재현 가능한 증거에 묶는다.

## 1. 목적과 성공 조건

### 1.1 이 저장소가 해결하는 문제

`quanta-memory-platform`은 프로세스 내 메모리를 class, owner, context,
charge-kind 축으로 예산화하고, 실제 retained store의 증감과 동일한 원장에 기록하며,
cap 초과·재구성·rebuild·reclaim 실패 때 상태를 망가뜨리지 않는 portable Rust library다.

| 책임 | canonical owner |
| --- | --- |
| 회계 SSOT와 ticket 수명주기 | `crates/quanta-memory-platform-core/src/domain/ledger.rs` |
| 정책, preset, override, validation | `core/src/domain/{policy,config}.rs` |
| live admission, reconfigure, concurrency | `core/src/api/control.rs` |
| pressure와 resource-group scheduling | `core/src/application/{controller,scheduler}.rs` |
| host-independent ports와 기본 adapters | `core/src/{ports,adapters}/` |
| builder-first 조립과 accounting seam | `facade/src/shared/{builder,accounting}.rs` |
| 실제 retained data path | `facade/src/{payload_store,retained_map,revocable_store,transcript_store,publish_pipeline,spill}/` |
| 타 구현에도 적용 가능한 계약 검증 | `testkit/src/{conformance,faults,fixtures}/` |
| 독립 저장소 추출과 자체 검증 | `scripts/`, `tools/ci/feature-matrix.sh`, `EXTRACTION.md` |

### 1.2 반드시 참이어야 하는 최상위 주장

- [ ] `core <- facade <- testkit` 의존 방향이 실제 manifest와 resolved graph에서 유지된다.
- [ ] `domain/`과 `ports/`는 host, filesystem, environment, wall-clock, adapter 구현에 의존하지 않는다.
- [ ] host-specific 명칭과 정책이 public portable API에 들어오지 않는다.
- [ ] budgeted production path의 모든 retained byte는 하나의 live ledger SSOT를 통과한다.
- [ ] deny된 mutation은 ledger, scheduler, store, index, tombstone을 부분 변경하지 않는다.
- [ ] class/owner/context 총계와 charge-kind partition이 모든 성공·실패·동시성 경로에서 보존된다.
- [ ] allocator 수치는 advisory이며 semantic counters를 변경하지 않는다.
- [ ] facade store와 pipeline은 replay, overwrite, reclaim, abort, GC에서 정확히 한 번 charge/refund한다.
- [ ] conformance suite는 정상 구현을 통과시키고 계약을 깨는 fake를 실패시킨다.
- [ ] feature 단독 조합과 추출된 standalone 저장소가 host 도움 없이 빌드·테스트된다.

### 1.3 명시적 비목적과 외부 책임

- wall time, subquery 수, trace edge 수, output byte의 metering point는 host execution engine에 있다.
  이 저장소는 cap과 `check_*` helper를 제공한다. host가 실제로 호출하는지는 별도 host audit 대상이다.
- Semantica의 `quanta-memory-host` 구현, 배포, 활성화는 이 저장소 밖의 책임이다.
- allocator RSS가 semantic usage와 같다는 보장은 없다. 차이는 telemetry/advisory 대상이다.
- static source audit와 standalone test GREEN은 host integration 또는 production workload 증거가 아니다.

## 2. 판정 규칙

| 판정 | 의미 |
| --- | --- |
| `PASS` | 현재 고정 SHA에서 positive와 필요한 negative 증거가 모두 재현됨 |
| `FAIL` | 목적/불변식 위반이 재현되거나 필수 gate가 RED |
| `PARTIAL` | 일부 경로만 증명됨. 전체 PASS로 합산 금지 |
| `NOT_RUN` | 실행하지 않음. 문서·과거 로그로 대체 금지 |
| `N/A` | 현재 제품 계약에 해당하지 않음. 근거와 승인자 기록 필수 |

심각도 기준:

- `P0`: 메모리/회계 손상, cap 우회, data loss, deadlock, 안전하지 않은 공개 계약.
- `P1`: 정상 사용 경로에서 fail-open, host 독립성/추출 실패, 원장과 실제 저장소 불일치.
- `P2`: 특정 feature/backing/경쟁 조건의 잘못된 동작, 운영상 큰 성능 또는 관측성 결함.
- `P3`: 유지보수성, 문서 진실성, 낮은 도달성의 API/성능 문제.

증거 우선순위:

1. 고정 SHA에서 실행한 adversarial test 또는 재현 프로그램.
2. 고정 SHA에서 실행한 owner-local/unit, cross-crate, extraction receipt.
3. 현재 source의 producer -> contract -> consumer 정적 경로.
4. 문서, 과거 감사, 과거 GREEN. 단독으로 완료 증거가 될 수 없다.

## 3. 감사 시작 전 source freeze

- [ ] 아래 결과를 감사 보고서 첫머리에 그대로 기록한다.

```bash
git rev-parse HEAD
git branch --show-current
git status --short
git diff --stat
git submodule status 2>/dev/null || true
cargo metadata --format-version 1 --locked --no-deps
```

- [ ] dirty file마다 `pre-existing`, `auditor-owned`, `unknown-owner`를 분류한다.
- [ ] 감사 중 HEAD 또는 dirty fingerprint가 변하면 기존 증거를 폐기하지 말고
  `STALE_FOR_NEW_SOURCE`로 표시한 뒤 delta를 재감사한다.
- [ ] 결과마다 SHA, command, exit code, selected test count, duration, log path를 남긴다.
- [ ] static, focused, workspace, extracted-repo, host-integration 증거를 별도 열로 유지한다.

## 4. 현재 HEAD의 사전 정찰 결과

기준 snapshot: `e103771670da3143e2ffebb8950a5dfc7e3d4515`, branch `main`, clean worktree.

현재 실행된 범위:

- `cargo test --workspace --all-features`: PASS. Rust unit/integration 399개와 compile-fail doctest 5개, 총 404개 통과.
- `python3 tools/pm/pm.py lint`: PASS.
- `scripts/regression-guards.sh`: PASS.
- `scripts/check-workspace.sh`: PASS. all-feature fmt/clippy/check/examples, feature matrix, cargo-deny 통과.
- extraction pre-flight `--dry-run`: PASS.

이 결과는 실제 extracted copy의 build, host adapter, 성능, 장시간 동시성,
production activation을 증명하지 않는다. 또한 single-feature matrix는 compile은
통과했지만 `-D warnings`를 적용하지 않아 일부 조합의 dead-code warning을 허용했다.

### 4.1 우선 검증할 위험 후보

아래는 확정 결함 목록이 아니라 현재 source에서 확인된 audit queue다.

| ID | 위험 후보 | 현재 근거 | 감사 질문 |
| --- | --- | --- | --- |
| R-01 | builder-first authority 우회 | `AccountingLedger`, `BasicProcessMemoryBroker::new`, `LiveProcessMemoryBroker::from_report`, 여러 `Simple*::new`가 public | 외부 소비자가 validation/accounting 조립을 우회해도 되는가? 아니면 visibility/문서/타입으로 막아야 하는가? |
| R-02 | 기본 store builder의 non-budgeted 모드 | 개별 store builder는 charger 없이 build 가능하고 `LiveMemoryPlatform` 경로만 자동 charge | production에서 accidental uncharged store가 타입 또는 config 단계에서 거부되는가? |
| R-03 | 정책 vocabulary의 runtime 의미 | `soft_cap_bytes`, class/group `reserved_bytes`, `reclaim_priority`, `burst_percent`는 주로 validation/report에 사용되고 admission 소비가 제한적 | reporting-only인지 enforcement 계약인지 명확한가? inert field가 운영자를 오도하지 않는가? |
| R-04 | live runtime view와 ledger 단절 가능성 | live broker의 `attach_runtime`도 `BasicRuntimeMemoryView`를 반환하며 basic scratch/pin registry를 보유 | runtime view의 scratch/pin/store binding이 live accounting과 일관되는가? |
| R-05 | scheduler 별도 mutation 표면 | `record_group_usage`는 public이며 repo 내부 production caller가 없음 | ledger와 분리된 phantom usage 또는 이중 계상이 가능한가? release 대칭이 필요한가? |
| R-06 | optional backing의 실제 대체 가능성 | redb는 `SpillTarget` 구현. moka/scc는 자체 API를 가지며 generic `RevocableStore`와 직접 교체 경로가 보이지 않음 | feature가 단순 compile이 아니라 실제 backing 선택 계약을 제공하는가? |
| R-07 | 성능 증거 부재 | bench/fuzz/loom/criterion/iai target이 없고 docs에는 benchmark 비중 목표가 있음 | O(1) 주장, allocation budget, contention, large-cardinality 성능이 측정됐는가? |
| R-08 | standalone과 host closure 분리 | dry-run은 copy/build를 실행하지 않고 host adapter는 외부 저장소에 있음 | 실제 추출본 및 Semantica consumer가 동일 source digest로 검증됐는가? |
| R-09 | 문서의 과도한 완료 주장 | `docs/verification-scenarios.md`는 “no known limitation”을 선언 | 현재 source/host/perf 증거 없이 closure를 주장하는 문구가 없는가? |
| R-10 | feature matrix의 warning 정책 | `--no-default-features`와 일부 leaf feature에서 ledger replacement helper dead-code warning이 발생하지만 matrix는 `-D warnings`가 아님 | 지원 feature 조합의 warning을 허용할지, cfg 경계를 고칠지, matrix를 엄격화할지 결정됐는가? |

## 5. 아키텍처와 경계 감사

### A. dependency direction / purity

- [ ] `cargo metadata` resolved graph에서 core가 facade/testkit을 참조하지 않고 facade가 testkit을 참조하지 않는다.
- [ ] dev/build/target-specific dependency까지 포함해 역방향 edge가 없다.
- [ ] `domain/`, `ports/`에서 `std::fs`, `std::env`, wall clock, thread sleep, process, network I/O가 없다.
- [ ] `domain/`, `ports/`에서 `crate::adapters`와 facade 타입을 import하지 않는다.
- [ ] host nouns가 crate public items, rustdoc, feature 이름, error variant에 노출되지 않는다.
- [ ] core 내부가 crate-root re-export를 통해 계층을 우회하지 않고 canonical inward path를 사용한다.
- [ ] optional dependency가 feature 없이 resolve/link되지 않는다.

필수 정적 증거:

```bash
cargo tree -e normal,build,dev --workspace --all-features
rg -n 'std::(fs|env)|SystemTime::now|Instant::now|crate::adapters' \
  crates/quanta-memory-platform-core/src/{domain,ports}
rg -n 'Semantica|quanta_runtime|codegraph' crates --glob '*.rs'
```

실패 조건: 역방향 edge, pure layer의 직접 I/O/clock/adapter 의존, portable public API의 host noun.

### B. public API와 authority

- [ ] 지원되는 생성 경로를 `builder`, `advanced/internal`, `test-only`로 분류한다.
- [ ] raw ledger/broker/store constructor마다 공개 필요성과 invariant 보존 책임을 문서화한다.
- [ ] public API test가 “builder가 존재함”뿐 아니라 금지된 우회가 불가능함을 검증한다.
- [ ] `BasicProcessMemoryBroker`와 live broker의 의미 차이가 타입/이름/rustdoc에서 오해되지 않는다.
- [ ] non-budgeted store는 명시적 opt-in이며 production preset에서 우발적으로 선택되지 않는다.
- [ ] `AccountingSink` 구현이 `charge`, `refund`, `refund_many`, `recharge`의 atomic contract를 모두 지킨다.
- [ ] `RuntimeMemoryView::bind_store`, `scratch`, `pins`가 live platform의 실제 authority와 연결되는지 검증한다.
- [ ] deprecated legacy publish surface에 제거 계획, consumer census, migration gate가 있다.

실패 조건: validation 또는 ledger SSOT를 public happy path에서 조용히 우회할 수 있음, 같은 이름의 broker/store가 budgeted 여부를 숨김.

## 6. 정책과 회계 kernel 감사

### C. policy / preset / override

- [ ] 모든 preset이 mandatory class 전체와 headroom을 결정적으로 생성한다.
- [ ] `reserved <= soft <= hard <= global`을 class/owner/group 각 축에서 검증한다.
- [ ] class reserved 합 + headroom의 global floor 계산이 overflow 시 fail-closed다.
- [ ] owner->group mapping은 선언된 group만 가리킨다.
- [ ] override precedence는 call order가 아니라 canonical authority order다.
- [ ] effective report가 최종 값과 last-writer provenance를 정확히 보존한다.
- [ ] `0`, `1`, `u64::MAX`, 작은 global, 큰 weight에서 underflow/overflow/0-division이 없다.
- [ ] reporting-only field와 enforced field를 표로 명시하고 둘을 같은 “cap”으로 광고하지 않는다.
- [ ] `ExecutionBudget`의 host-enforced 축은 host consumer call-site와 negative E2E가 별도로 존재한다.

적대 테스트:

- class/owner/group soft>hard, hard>global, unknown group, headroom>global.
- 동일 field를 manifest/env/CLI/test fixture 순서를 섞어 override.
- host가 `check_*` helper를 호출하지 않는 경로를 검출하는 integration test.

### D. ledger algebra

- [ ] `sum(class) == sum(owner) == sum(context)`.
- [ ] owner total이 payload + metadata + live scratch로 정확히 partition된다.
- [ ] global ceiling과 non-headroom ceiling이 모든 reserve에 적용된다.
- [ ] owner/class cap과 shared burst pool이 prospective postimage 기준으로 적용된다.
- [ ] ticket은 issuing ledger와 묶이고 release는 정확히 한 번만 가능하다.
- [ ] commit-after-release, cross-ledger, duplicate batch, forged/stale ticket이 mutation 전에 거부된다.
- [ ] zero-byte ticket이 축에 빈 entry를 남기지 않는다.
- [ ] batch release는 전부 성공하거나 전부 실패한다.
- [ ] recharge는 old charge와 new charge 사이에 transient double count나 uncharged window를 만들지 않는다.
- [ ] recharge deny/rollback failure의 recovery disposition이 모호하지 않다.
- [ ] rebuild 중 reserve/commit/release/recharge는 모두 동일한 fail-closed 규칙을 따른다.
- [ ] allocator observation은 ledger snapshot을 byte-for-byte 바꾸지 않는다.
- [ ] 모든 mutation 뒤 네 핵심 invariant와 live ticket set의 일관성을 검사한다.

필수 adversarial sequence:

1. reserve -> commit -> cloned release x2.
2. reserve A -> batch `[A, A]`.
3. ledger A ticket을 ledger B에서 commit/release.
4. old 100 -> replacement 200이 cap 초과하도록 recharge.
5. rebuild 시작 직전/직후 reserve, commit, recharge, release 경쟁.
6. random deterministic sequence 후 모든 live ticket drain, totals 0 확인.

실패 조건: 오류 반환 뒤 snapshot 차이, 어떤 축의 합 불일치, orphan live ticket, cap 초과 성공.

## 7. live control / concurrency 감사

### E. broker, scheduler, pressure, reconfigure

- [ ] broker lock이 ledger+scheduler+ticket-group+report를 하나의 atomic authority로 보호한다.
- [ ] group admission은 ledger mutation 전에 실행되고 성공 후에만 usage를 기록한다.
- [ ] release/recharge가 ticket 발급 당시 group에 정확히 대칭 반영된다.
- [ ] public `record_group_usage` 같은 side channel이 ledger와 scheduler truth를 분리하지 않는다.
- [ ] weighted share 계산이 overflow하지 않고 rounding 정책이 명시적이다.
- [ ] `burst_percent`, group reserved/soft, admission_weight의 실제 의미와 소비자가 모두 존재한다.
- [ ] live scratch cap은 scratch kind에만 적용되고 recharge로 우회할 수 없다.
- [ ] reconfigure는 global/class/owner/group cap 축소, live class drop, headroom 침범, live owner remap을 거부한다.
- [ ] accepted reconfigure가 ledger usage, scheduler usage, cached pressure, report를 동시에 갱신한다.
- [ ] poison 시 mutation은 fail-closed하고 report는 가짜 fallback 값을 만들지 않는다.
- [ ] pressure는 semantic usage만 권위로 사용하고 threshold boundary가 inclusive/exclusive 계약과 일치한다.
- [ ] reclaim ladder가 실제 broker/store slow path에서 도달 가능한지, 아니면 계산 helper에 불과한지 명시한다.

동시성 matrix:

| race | oracle |
| --- | --- |
| N개 동시 reserve | 성공 합이 global/class/owner/group cap 이하 |
| reserve vs reconfigure | 어느 선형화 순서에서도 cap truth 유지 |
| recharge vs rebuild | old 또는 new 중 정확히 하나만 유효 |
| delete/reclaim vs reconfigure | refund와 정책 교체 사이 partial state 없음 |
| 동일 key 동시 insert | documented winner 하나, charge 하나 |
| conflicting replay | typed conflict, 기존 payload/index/accounting 유지 |

추가 요구: 짧은 unit race만으로 장시간 contention safety를 주장하지 않는다. 필요 시 loom/model test 또는 반복 stress receipt를 별도로 둔다.

## 8. facade 실제 data path 감사

### F. 공통 store mutation protocol

- [ ] insert: measure -> admission/charge -> container publish. charge 실패 시 publish 없음.
- [ ] overwrite: existing ticket 확인 -> atomic recharge -> value/ticket 교체. deny 시 old 유지.
- [ ] delete/reclaim/truncate/abort/GC: candidate 고정 -> atomic refund -> container mutation.
- [ ] multi-item mutation은 `refund_many` 성공 전에 하나도 제거하지 않는다.
- [ ] local lock과 broker lock 획득 순서가 전 slice에서 일관되고 callback 아래 lock hold가 없다.
- [ ] overflow는 payload/index/accounting mutation 전에 검출된다.
- [ ] poisoned lock이 empty/zero/NotFound 성공으로 위장되지 않는다.
- [ ] store-local `charged_bytes`와 ledger ticket bytes가 일치한다.
- [ ] duplicate/replay path는 charge/refund를 발생시키지 않는다.
- [ ] uncharged mode와 charged mode의 behavior 차이는 이름/타입/telemetry로 식별 가능하다.

### G. payload store

- [ ] canonical identity가 payload와 안정적으로 대응하고 hash collision을 equality로 재확인한다.
- [ ] dedup hit는 기존 ref를 반환하고 second charge/entry를 만들지 않는다.
- [ ] pin count는 duplicate dependent를 중복 계산하지 않는다.
- [ ] pinned delete는 payload와 charge를 유지한 채 typed error다.
- [ ] unpin underflow, missing delete, double delete가 fail-closed다.
- [ ] generated payload ref가 host/domain identity 요구를 깨지 않는다.

### H. retained map

- [ ] key identity law는 `Eq + Hash`이며 point lookup/overwrite/remove가 full scan하지 않는다.
- [ ] overwrite가 insertion/reclaim order 계약을 의도대로 유지한다.
- [ ] atomic insert-if-absent는 occupied value를 덮어쓰거나 recharge하지 않는다.
- [ ] sweep은 target bytes를 만족시키되 zero-byte entry 때문에 과잉 eviction하지 않는다.
- [ ] ticketless/corrupt entry가 batch partial refund 또는 partial eviction을 만들지 않는다.
- [ ] deterministic enumeration 요구와 hash container의 순서를 분리 검증한다.

### I. revocable store

- [ ] revoke와 reclaim이 동일 entry를 이중 refund하지 않는다.
- [ ] FIFO/priority ordering이 capacity boundary와 arbitrary removal 뒤에도 결정적이다.
- [ ] `reclaim_until`이 목표 이상을 해제하되 불필요한 다음 entry를 제거하지 않는다.
- [ ] zero-byte entry만 있을 때 progress/termination 계약이 명확하다.
- [ ] moka/scc backing이 이 계약의 대체 구현인지 단순 helper인지 분류한다.

### J. transcript store

- [ ] identity `(session, sequence, message-id)` replay는 exact body에서만 inert다.
- [ ] payload/sequence/message-id 각 collision이 별도 typed error다.
- [ ] per-session isolation과 ordered sparse sequence load가 보장된다.
- [ ] truncate/selective retain이 tombstone을 유지해 late retry resurrection을 막는다.
- [ ] tombstone retention의 count/bytes/time bound와 process lifetime 비용을 평가한다.
- [ ] versioned snapshot retain은 concurrent append를 drop하지 않고 conflict를 반환한다.
- [ ] retention callback/user code가 internal lock 아래 실행되지 않는다.

### K. prepared publication / legacy publish

- [ ] prepare identity replay가 같은 token을 반환하고 charge는 하나다.
- [ ] identity는 같고 payload가 다르면 typed conflict이며 기존 state가 유지된다.
- [ ] resolve는 exact token만 허용한다.
- [ ] abort replay는 idempotent하고 refund는 한 번이다.
- [ ] orphan enumeration/GC는 caller의 stabilized complete root set을 전제로 한다.
- [ ] unknown/aborted root가 batch 전체를 fail-closed시키는 계약을 검증한다.
- [ ] tombstone saturation은 payload/charge를 바꾸기 전에 실패한다.
- [ ] legacy non-retaining pipeline이 실제 consumer에 남아 있는지 census하고 제거 조건을 기록한다.

### L. spill / scratch / pins

- [ ] spill accept 성공 후 restore가 FIFO/정의된 순서로 round-trip한다.
- [ ] unavailable/accept failure/restore failure가 counter와 retained backing state를 보존한다.
- [ ] controller count와 target retained count가 성공/실패/동시성 후 일치한다.
- [ ] redb 재시작, keyspace exhaustion, concurrent writers, corrupted/open error를 검증한다.
- [ ] scratch reservation의 grow/release/double-release가 live ledger contract와 일치한다.
- [ ] pin registry와 facade-local pin count가 별도 authority라면 경계와 reconciliation을 명시한다.

## 9. feature, backing, dependency 감사

- [ ] `--no-default-features`가 slice public surface와 optional dependency를 실제로 제거한다.
- [ ] leaf feature 각각이 단독으로 all-targets compile된다.
- [ ] `full`에는 in-memory 기능만, `full-backings`에 heavy backing만 포함된다는 계약이 Cargo graph와 일치한다.
- [ ] backing feature가 compile-only dead code가 아니라 선택/조립/trait contract로 연결된다.
- [ ] third-party error type이 portable public API로 누출되지 않는다.
- [ ] license/advisory/source 정책은 `cargo deny` config와 실제 lockfile에서 검증된다.
- [ ] MSRV, target triples, no-network build 필요 여부를 명시하고 해당 matrix를 실행한다.
- [ ] feature unification이 원치 않는 live-control/heavy dependency를 소비자에게 전파하지 않는다.

필수 명령:

```bash
tools/ci/feature-matrix.sh
cargo tree -e features --workspace --all-features
cargo tree -p quanta-memory-platform-facade --no-default-features
cargo tree -p quanta-memory-platform-facade --features full
cargo tree -p quanta-memory-platform-facade --features full-backings
cargo deny --all-features check
```

## 10. testkit 자체 감사

- [ ] suite가 empty assertion list를 PASS하지 않는다.
- [ ] 각 `ContractFamily`가 적어도 하나의 실제 실행 verdict를 가진다.
- [ ] honest fake와 real broker가 같은 portable suite를 통과한다.
- [ ] 각 broken fake가 의도한 family에서 반드시 실패한다.
- [ ] deny 시 mutate, under-report, class/owner divergence, allocator mutation, refund swallow, overwrite leak를 각각 잡는다.
- [ ] facade suite가 실제 live broker sink와 store를 조립한다.
- [ ] prepared-publication fake가 double-charge와 root-blind GC를 드러낸다.
- [ ] transcript fake가 non-idempotent replay를 드러낸다.
- [ ] fake가 real 구현의 버그를 그대로 복사해 동반 통과하지 않도록 독립 oracle을 사용한다.
- [ ] conformance target의 error-to-empty fallback이 corruption을 숨기지 않는다.

실패 조건: 깨진 fake 하나라도 PASS, real/fake가 서로 다른 계약을 검사, error를 zero snapshot으로 바꿔 invariant PASS.

## 11. 성능과 확장성 감사

정확성 GREEN을 성능 GREEN으로 간주하지 않는다.

- [ ] class/owner/context 수에 따른 `snapshot()` clone 비용을 측정한다.
- [ ] reserve/release/recharge의 lock hold time과 contention을 1/2/8/32 thread에서 측정한다.
- [ ] payload dedup, retained-map point lookup, transcript per-session load/retain의 cardinality scaling을 측정한다.
- [ ] tombstone과 ticket set의 장시간 churn 후 retained memory를 측정한다.
- [ ] redb reopen/restore throughput과 fsync/durability 정책을 명시한다.
- [ ] moka/scc가 실제 facade contract에 연결된다면 동일 workload로 semantic parity를 비교한다.
- [ ] allocation probe는 smoke가 아니라 명시적 budget threshold를 실패시킨다.
- [ ] benchmark baseline, variance policy, regression threshold, hardware metadata를 저장한다.
- [ ] benchmark가 correctness assertion을 대신하지 않는다.

현재 bench target이 없으면 이 섹션은 `NOT_RUN` 또는 `FAIL(요구 계약일 때)`이다.

## 12. 운영성, 오류, 관측성 감사

- [ ] 모든 외부 실패가 `Config`, `Capacity`, `InvalidState`, `Build`, `Report`, `Unsupported` 중 올바른 domain으로 매핑된다.
- [ ] error alias가 경계별 recovery를 구분해야 하는 곳에서 정보를 잃지 않는다.
- [ ] lock poison, backing unavailable, cap deny, replay conflict가 telemetry에서 구분된다.
- [ ] metrics/histogram adapter가 accounting authority를 소유하거나 변경하지 않는다.
- [ ] sensitive owner/runtime/key/payload가 error 또는 metric label에 무제한 노출되지 않는다.
- [ ] cardinality bound 없는 owner/class label을 production metric에 쓰지 않는다.
- [ ] rebuild 시작/종료, reconfigure accept/deny, spill/reclaim 결과를 운영자가 식별할 수 있다.
- [ ] report가 active policy와 동일하며 poison/reconfigure 뒤 stale/fabricated 값을 내지 않는다.

## 13. portability / extraction / host 감사

### M. standalone proof

- [ ] dry-run pre-flight가 member, direction, escaping path, metadata, copy set을 fail-closed 검증한다.
- [ ] 실제 임시 destination에 copy+git init+standalone checks를 수행한다.
- [ ] extracted tree에 monorepo gate가 복사되지 않고 필요한 PM/CI/script는 모두 존재한다.
- [ ] source와 extracted canonical file set의 digest가 일치한다.
- [ ] extracted repo에서 examples가 실제 public API만 사용해 실행된다.

```bash
audit_dest=$(mktemp -d /tmp/qmp-extracted-audit.XXXXXX)
scripts/extract-to-standalone-repo.sh "$audit_dest"
git -C "$audit_dest" status --short
```

### N. host integration proof

- [ ] host adapter가 extracted repo를 path/git/registry 중 명시된 방식으로 참조한다.
- [ ] host가 `LiveMemoryPlatform` 또는 동등한 charged path를 사용하고 raw/non-budgeted path를 선택하지 않는다.
- [ ] host execution engine이 non-memory `ExecutionBudget::check_*`를 실제 metering point에서 호출한다.
- [ ] host stores의 class/owner/context binding이 의도한 budget axis와 일치한다.
- [ ] static boundary guard, host compile/test, representative E2E를 같은 source digest로 실행한다.
- [ ] deploy/activation 여부는 standalone/host test와 별도 상태로 기록한다.

## 14. 문서와 test-truth 감사

- [ ] README의 지원 표면, feature, example, script가 현재 source와 일치한다.
- [ ] `docs/verification-scenarios.md`의 각 GREEN 항목이 존재하는 test와 정확히 매핑된다.
- [ ] test 이름 존재를 test 실행 성공으로 오인하지 않는다.
- [ ] historical `AUDIT-2026-06-07.md`의 finding을 `fixed`, `still-open`, `superseded`, `not-reproduced`로 재분류한다.
- [ ] “no known limitation”, “complete”, “standalone-ready” 같은 절대 표현은 host/perf/negative proof 범위를 함께 적는다.
- [ ] public API 변경 시 examples, re-export tests, README, extraction docs가 함께 바뀐다.
- [ ] generated agent docs는 `tools/pm/sources/**`만 수정하고 sync/lint receipt를 남긴다.

## 15. 표준 검증 레일

Owner-local/focused 증거 후 아래 순서로 확대한다.

```bash
python3 tools/pm/pm.py sync
python3 tools/pm/pm.py lint
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny --all-features check
scripts/check-workspace.sh
scripts/memory-platform-doctor
cargo run -p quanta-memory-platform-facade --features full --example hello_world
cargo run -p quanta-memory-platform-facade --features full --example effective_budget_report
```

주의:

- `cargo test --all-features`는 single-feature break를 증명하지 않는다.
- `scripts/check-workspace.sh`는 workspace test를 직접 실행하지 않는다.
- `memory-platform-doctor`가 cargo-deny 부재를 skip하면 dependency/license는 `PARTIAL`이다.
- dry-run extraction은 copied repo build를 실행하지 않는다.
- standalone GREEN은 host adapter GREEN이 아니다.

## 16. finding 기록 형식

각 finding은 아래 필드를 모두 가진다.

```text
ID / severity:
Status: CONFIRMED | PROOF_GAP | REJECTED
Claim violated:
Frozen SHA / dirty fingerprint:
Owner:
Producer -> contract -> consumer path:
Reachable trigger:
Observed result:
Expected result:
Minimal reproduction command/test:
Exact path:line evidence:
Blast radius:
Why existing tests/gates missed it:
Required owner fix:
Required negative test:
Closeout receipt:
```

`PROOF_GAP`을 `CONFIRMED defect`로 부풀리지 않는다. 반대로 public reachable fail-open을
“테스트 없음”으로 축소하지 않는다.

## 17. 최종 감사 보고서 matrix

| 영역 | Static | Owner tests | Adversarial | Workspace | Extracted | Host | Perf | 판정 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Dependency/purity |  |  | N/A |  |  |  | N/A |  |
| Policy/config |  |  |  |  |  |  | N/A |  |
| Ledger |  |  |  |  |  |  |  |  |
| Live control |  |  |  |  |  |  |  |  |
| Facade stores |  |  |  |  |  |  |  |  |
| Backings/features |  |  |  |  |  |  |  |  |
| Testkit bite |  |  |  |  |  | N/A | N/A |  |
| Portability |  |  |  |  |  |  | N/A |  |
| Host integration |  |  |  |  |  |  |  |  |

전체 PASS 조건:

- P0/P1 finding 0개.
- 모든 최상위 주장에 current-SHA positive + 필요한 negative 증거 존재.
- workspace, feature matrix, deny, actual extraction이 동일 source snapshot에서 GREEN.
- host 통합을 제품 완료 범위에 넣는 경우 host receipt도 동일 digest에서 GREEN.
- performance SLO를 제품 계약으로 주장하는 경우 측정 결과와 threshold가 존재.
- 남은 P2/P3, N/A, NOT_RUN은 owner와 수용 근거가 명시됨.

그 외에는 최종 판정을 `PARTIAL` 또는 `FAIL`로 유지한다.
