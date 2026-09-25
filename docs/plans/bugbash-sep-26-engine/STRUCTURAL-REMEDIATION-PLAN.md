# Sep-26 structural remediation plan

Status: **IMPLEMENTED (STR-01–05, STR-07) at a working tree over base
`438b869`; STR-06 measurement and conditional 0.3 work remain open. Red/GREEN
and rail receipts: [implementation receipt](README.md#implementation-receipt--2026-09-26-working-tree)**.

Base: QMP `438b869a36369021ee81560fccdb9692e3f39e35`, initially clean `main`.
Product source is unchanged from the original `96d0a25` audit; the intervening
commit added only plan documents. Scope is the current in-memory library.
The [finding register](README.md) retains severity and historical evidence;
[this pass's evidence](STRUCTURAL-AUDIT-EVIDENCE.md) records four public-API
reproductions. No new P0/P1 was established.
The [file-by-file implementation runbook](IMPLEMENTATION-RUNBOOK.md) supplies
specific edit points, regression cases, commands and worker handoffs.

## 1. Final audit decisions

수정 단위는 finding 개수가 아니라 **불변식을 소유한 경계**로 잡는다.
아래 결정은 구현에 적용할 권고안이다. 현재 API가 이미 이 계약을 보장한다는
뜻은 아니다. 기존 0.3 [RFC-0001](../../rfcs/0001-sdk-interface-contract.md)의
미구현 요구사항을 이번 0.2 보수로 완료 처리하지 않는다.

| Decision | Selected direction | Reason / constraint |
| --- | --- | --- |
| D1 — store read boundary | 공개 read는 guard를 반환받아 직접 caller 코드를 실행하지 않고, lifecycle이 소유하는 closure 기반 read 경계를 통과한다. | ENG-01의 원인은 각 call site가 panic helper를 기억해야 하는 구조다. `try_lock`, 읽기 전후 lifecycle 검사, 정상 guard 해제 후 panic 전파를 한 경계에서 수행한다. |
| D2 — charge-kind semantics | `ChargeKind`는 **소유 수명별 semantic charge bucket**으로 정의한다. 독립 metadata 수명은 metadata로 분리한다. | `RetainedCostV1::total()`과 단일-kind ticket이라는 현재 모델에 부합한다. payload 객체의 key/index overhead까지 필드별로 분할한 telemetry라고 주장하지 않는다. |
| D3 — scratch peak | 유효한 관측은 `peak = max(peak, observed)`이다. | 감소 관측은 허용하되 peak는 유지한다. ceiling 초과·해제 후 호출은 기존 typed error와 불변 postimage를 유지한다. |
| D4 — thresholds | `PressureThresholds` 필드를 private으로 만들고 검증된 생성자·유효한 Default·read-only getter만 제공한다. | unchecked 생성 경로를 남겨서는 닫히지 않는다. 작은 API 변경이므로 전체 SDK 재설계 완료를 기다릴 이유도 없다. |
| D5 — input work | 입력 크기·권한 검증 → bounded normalization → store/broker transition 순서를 고정한다. | 무효 batch를 거절하기 전에 크기에 비례하는 ticket vector를 만들지 않는다. GC roots의 중복 제거는 mutex와 mutation permit을 얻기 전에 수행하고, 현재 state 검증은 그 뒤에 다시 한다. |
| D6 — same-store reclaim | 현재 계약에서는 같은 mutation lane 재진입을 `OperationInProgress`로 거절한다. 다른 store의 reclaim은 지원한다. | 현재 host에 reclaimer wiring이 없고 기존 구조가 안전한 bounded denial을 제공한다. permit을 임시 해제하면 detached state와 accounting 전이가 노출된다. |
| D7 — operating scope | 현재는 trusted host의 semantic budgeting이다. callback 실행시간·임의 caller allocation·RSS를 제한한다고 선언하지 않는다. | synchronous callback을 timeout thread로 감싸도 작업 중단이나 authority 회수가 보장되지 않는다. host가 bounded callback/입력/프로세스 한계를 소유한다. |
| D8 — CI scope | 현 main 직접 작업 방식과 비용 제약을 유지한다. main의 기존 extraction job에서 public examples도 실행한다. | 자동 premerge qualification을 약속하지 않는다. PR 방식으로 전환할 때 candidate gate를 별도로 선택한다. 새 유료 job/provider는 기본 요구가 아니다. |

D2는 기존 문구를 몰래 바꿔 PASS로 만드는 조치가 아니다. core rustdoc,
store별 비용 표, facade 설명, host에 전달할 metric 의미/호환성 노트를 함께
수정해야 한다. 현재 공개 필드명은 호환성을 위해 유지하되 overhead를 포함한
charge bucket임을 명시한다. field-byte 분할이 필요한 실제 consumer가 생기면
단일 ticket의 원자적 charge vector를 별도 설계한다. payload/metadata별 lease를
임의로 추가하면 ticket cap·부분 실패·rollback 의미까지 달라지므로 이번에는 하지 않는다.

### Store별 규범적 charge table

| Lifetime component | Bucket | Release point |
| --- | --- | --- |
| Payload store의 payload + 함께 보유하는 key/index/entry overhead | RetainedPayload | delete/close/Drop |
| Retained map의 value + key/index/entry 전체 | RetainedMetadata | remove/replace/close/Drop |
| Revocable store의 resident entry 전체 | LiveScratch | reclaim/spill retirement/close/Drop |
| Transcript active row | RetainedPayload | retention/truncate/close/Drop |
| Transcript session key + persistent identity evidence | RetainedMetadata | 해당 수명 종료/close/Drop |
| Prepared publication active payload component | RetainedPayload | abort/GC/close/Drop |
| Prepared publication persistent identity/index/tombstone component | RetainedMetadata | close/Drop |

단위는 기존 versioned semantic cost다. allocator size/RSS도 아니고, reclaimability를
`ChargeKind`에서 추론하는 새 정책도 아니다. ledger의 class/owner/context 각 축은
동일한 bucket 의미를 가져야 한다. admission 총량과 formula-v1 상수는 바꾸지 않는다.

## 2. Work packages and owners

모든 패키지는 현재 **PLANNED**다. P2/P3는 finding의 심각도이며 작업 순서와
동일하지 않다. 작은 확정 결함을 조건부 SDK/성능 작업 때문에 지연시키지 않는다.
아래 owner path의 `core/`와 `facade/`는 각각 repository root 아래
`crates/quanta-memory-platform-core/`, `crates/quanta-memory-platform-facade/`를 뜻한다.

### STR-00 — contract and oracle inventory (integrator)

- D1–D8을 구현 기준으로 반영한다. ENG 상태표는 구현 완료 전까지 열린 상태다.
- 공개 store operation × 실행하는 caller trait (`Clone/Ord/Hash/Eq/CanonicalKey/ByteSized/Drop`)
  × 잠금/permit 소유 위치를 owning test 옆의 짧은 표로 남긴다.
- API 이름 나열을 계약 검증으로 인정하지 않는다. 각 불변식마다 정상·음성·실패 후
  재사용/cleanup oracle을 지정하고 기존 test가 있으면 확장한다.
- B07/J06 상태는 family별로 구분한다: live ticket/identifier/deferred batch는
  구현됨; policy cardinality와 미정의 lifetime metadata 전체까지 닫힌 것은 아니다.
- 완료: 아래 패키지의 요구사항·test 위치·공유 파일 owner가 하나로 정해져 있다.

### STR-01 — facade read ownership boundary (facade lifecycle owner; ENG-01)

Owner paths: `facade/src/shared/lifecycle.rs`와 payload/retained-map/transcript/
prepared-publication의 read entry points. 이 패키지가 네 service 파일을 수정하는
동안 STR-03은 동일 파일을 수정하지 않는다.

1. 공통 read 함수가 state lock, lifecycle 검사, immutable closure 호출,
   guard 해제 및 unwind 전파를 모두 소유하게 한다. 공개 read에 raw guard를
   반환하는 기존 helper는 제거/전환한다. mutable transition과 rollback의
   state ownership은 기존 permit/plan에 남긴다.
2. `session_bytes`를 포함한 read inventory 전체를 같은 경계로 옮긴다.
   mutation 중의 read-only preflight는 permit 소유 경로로 별도 검토한다.
3. poison을 전역 무시하거나, panic 후 mutable postimage를 자동 복구하지 않는다.
   caller의 interior side effect와 `panic=abort`는 unwind containment의 보장이 아니다.

Owner-local oracles:

- 각 해당 read 경로에서 caller trait panic → unwind 포착 → 같은 store의 정상
  load/stats/write/close 재사용 → 모든 charge와 ticket 정리.
- read callback이 같은 store를 다시 호출하면 typed Busy로 끝나고 데이터·ledger는 보존.
- mutation plan이 state를 잠시 분리한 동안 read는 Busy이며 빈 데이터 성공을 반환하지 않음.
- 기존 close 실패 복원, Drop panic containment, concurrent read/mutation 검증 유지.
- wrapper를 우회한 임시 mutant에서 해당 API 반례가 반드시 실패해야 함.

D1은 0.2의 panic/reentry 계약을 소유 경계에 모은다. **0.3의 모든 caller callback을
mutex 밖으로 옮기는 계약은 여전히 별도 작업**이다. 그 이동에서 `Arc<Value>`만
복사하고 charge를 먼저 해제하면 살아 있는 read가 무회계 상태가 될 수 있다.
따라서 0.3은 read가 값과 charge 수명을 함께 보호하는 pin/lease와 close의 Busy 또는
drain 정책부터 설계해야 한다. 임의 `Ord`를 byte key로 대체하면서 equality 의미를
바꾸는 migration도 금지한다. RFC 구현 시 slow-callback barrier와 close/reclaim
경합으로 이 수명을 먼저 증명한다.

### STR-02 — observation and validated construction (core scalar owner; ENG-11/12)

Owner paths: `core/src/ports/scratch.rs`, `application/controller.rs`, 관련
public API tests/re-exports/docs. STR-04의 `api/control.rs` 수정은 integrator가 순차 적용한다.

- Scratch는 D3을 적용한다. `0 → 800 → 200 → 800 → 1000`의 peak,
  ceiling 초과/해제 후 unchanged, explicit release/Drop 양쪽의 실제 broker drain을 검증한다.
  port fixture만 통과하고 broker authority 검증이 빠져서는 안 된다.
- Thresholds는 private fields와 getter로 단일 유효성 경계를 만든다. inverted/>100
  값은 typed Config error, 공개 literal 생성은 compile-fail로 검증한다.
- 75/90/98의 경계 전후, 0 hard limit, `u64::MAX`, 허용된 equal thresholds를 보존한다.
  monotonic 조건을 임의로 strict ordering으로 강화하지 않는다.
- private fields는 source-breaking이므로 release/migration 문서와 실제 call-site
  inventory를 포함한다. 호환성이 필수인 0.2 patch에는 이 API 변경만 다음 API release로
  분리하며, public validation gap을 완료 처리하지 않는다. 권고는 다음 versioned API
  변경에 즉시 포함하는 것이다; RFC-0001 전체 구현과 묶을 필요는 없다.

### STR-03 — prepared lifetime and bounded roots (prepared-publication owner; ENG-10/13)

Owner paths: `facade/src/publish_pipeline/prepared_publication.rs`,
`shared/measure.rs` 계약 설명, charge rustdoc 및 cross-store axis test.
STR-01 뒤에 같은 service를 수정한다.

- persistent identity lease를 D2의 Metadata bucket에 놓고 active lease는 Payload로 유지한다.
  replay/abort/GC/close에서 각 lease의 bytes·kind·owner·존재 여부를 함께 검증한다.
- 독립 formula-v1 oracle: identity `[b'i']`, 빈 `Vec<u8>`의 경우 active=17,
  persistent=129. prepare는 `(payload, metadata)=(17,129)`, abort 후 `(0,129)`,
  close 후 `(0,0)`이어야 한다. 기대값 계산에 production cost helper를 호출하지 않는다.
- admission denial, replay conflict, abort/GC 환불 실패는 모든 axis와 payload postimage를
  보존한다. 총량과 기존 cap/실패 원자성은 그대로 유지한다.
- GC root normalization은 읽기 mutex와 mutation permit 획득 전 수행한다.
  normalized roots를 현재 state에 대조하고 **전체 참조 검증 후** 한 번의 GC를 commit한다.
  normalize 후 state가 바뀌면 당시 state에서 typed rejection 또는 유효한 결과가 나와야 한다.
- root raw-input count 기본 상한은 16,384로 제안한다. 현재 broker live-ticket 상한에
  맞춘 보수적 management bound이며 p99/RSS 보장이 아니다. duplicate도 입력 count에
  포함하고 초과는 `Storage(MemoryError::Capacity(_))`로 변이 전에 거절한다.
  상한은 prepared owner의 단일 상수 `MAX_REFERENCED_ROOT_TOKENS`로 관리하며 unbudgeted fixture도
  우회하지 않는다. 기존 무제한 slice 입력의 계약 변경을 migration에 명시한다.
- empty/max/max+1, duplicate-heavy, unknown/aborted roots가 valid root 앞뒤에 섞인 경우,
  denial 후 payload/ledger 동일성을 검증한다. bounded lock-time 주장은 별도 측정이 필요하다.

### STR-04 — broker input and reclaim contracts (broker owner; ENG-02/07/08)

Owner paths: `core/src/api/control.rs`, `ports/reclaim.rs`, telemetry contract;
필요한 facade 통합 fixture만 추가한다.

- batch 길이의 불가능 조건과 모든 lease의 broker affinity/active 상태를 먼저 검사한다.
  유효한 batch에만 ticket vector를 만든다. 전체 postimage 검증·원자적 release를 유지한다.
- `MAX_LIVE_TICKETS_PER_BROKER`보다 큰 active-only 유효 batch는 존재할 수 없다.
  초과/외부 broker/이미 해제된 lease/byte overflow/rebuild 거절 모두 lease authority를 보존한다.
- source 순서와 owner-local 실패 테스트로 **거절 전 O(batch length) ticket vector를 만들지
  않음**을 확인한다. 실제 allocator 호출 수를 주장할 때는 별도 benchmark 계측을 사용한다.
  typed error 문자열·event sink까지 포함한 절대 0 allocation을 요구하지 않는다.
- same-store callback의 Busy를 typed 결과로 처리하는 명시적 테스트를 둔다. hard denial 후
  기존 resident 값과 bytes가 그대로 남아야 한다. 별도 store victim reclaim은 실제 usage 감소를
  확인한 후 재입장하고 성공해야 한다. callback의 claimed bytes만 신뢰해서는 안 된다.
- deterministic barrier로 callback이 멈춘 동안 다른 broker operation이 완료되는지 검사한다.
  이는 broker mutex를 안 잡는다는 증명이며 callback deadline 보장은 아니다. 자동 spin retry 금지.

### STR-05 — proof, documentation, existing CI (proof owner; ENG-09)

- README의 example execution을 기존 CircleCI main extraction job에 실제 연결한다.
  extracted directory에서 두 public example을 `--locked`로 실행한다.
- GitHub dispatch-only/자동 main-only를 정확히 적고, `ports/`는 pure trait·DTO·pure
  helper를 허용한다고 authority source를 수정한 뒤 `pm sync`한다.
- multi-thread workload 설명을 실제 benchmark matrix와 일치시킨다.
- 기존 script contract test 위치에서 branch/job/example reachability를 검사한다.
  local 설정 검증과 실제 hosted job success는 각각 기록한다. 외부 branch protection은
  조회 없이 enabled로 적지 않는다.
- 신규 owner regression의 feature·selector·실행 수를 정식 rail에 연결한다.
  진단 프로그램, README, 테스트 함수 이름의 존재 자체를 qualified evidence로 취급하지 않는다.

### STR-06 — targeted performance and bounds (measurement owner; ENG-05/06/07)

성능 결함은 아직 실측되지 않았다. 별도 측정 패키지이며 STR-01–05의 결함 수정을
막지 않는다. 성능 개선/동시성 SLO를 주장할 때는 이 패키지를 완료해야 한다.

- 실제 경로를 추가한다: group 0/100/1,000에서 retained-map overwrite/recharge,
  reclaim-plan·observe, 증가하는 session/row/evidence에서 transcript stats와 concurrent append,
  retained-map 1/2/8/32-thread hit/miss 및 read/write mix, root 상한에서 GC.
- end-to-end latency와 성공/denial/Busy 수를 분리한다. lock wait/callback 시간을
  진단할 때 계측 자체의 비용도 기록한다. admission만 측정해 recharge 성능을 대신하지 않는다.
- allocation 모드와 tracking 없는 timing 모드를 benchmark 전용 설정으로 분리한다.
  mode·workload·compiler/features·machine·source·harness digest를 receipt에 묶는다.
- harness 변경을 먼저 고정한다. baseline과 candidate는 **같은 harness/mode**를 실행해야 한다.
  새 harness 결과를 구형 matrix의 receipt와 비교하지 않는다. product 개선이 없으면 baseline
  수집으로만 기록하고 문서 commit 차이를 최적화 성과로 삼지 않는다.
- 실제 consumer 크기/동시성/poll 주기와 p99·successful throughput·Busy 한계를 사전에
  명시한다. 보편적 ns 목표를 임의로 만들지 않는다. 기존 verifier가 요구하는 모든 release
  threshold를 지정하고, quiet host에서 독립 baseline 3회 + candidate 3회로 비교한다.
  noise 초과는 INCONCLUSIVE이며 임계값을 뒤에서 완화하지 않는다.
- policy map이 trusted small config이면 그 상한/운영 전제를 명시한다. dynamic 입력을
  허용할 때는 owner/group/binding count를 validated management config에서 제한한다.
  live-ticket 상한으로 policy-map 상한까지 증명했다고 적지 않는다.
- 실측 병목이 확인되면 scheduler 전체 clone을 transactional delta/preflight로 교체하거나
  hot stats와 full invariant 검사를 분리한다. 전자는 rollback/동일 snapshot,
  후자는 독립 full recomputation oracle을 유지해야 한다. ledger sharding·lock-free rewrite는
  현재 승인할 근거가 없다.

### STR-07 — serial integration and qualification (integrator)

- feature-isolated owner tests와 최소 public integration을 먼저 통과시킨다.
  cross-store axis/lifecycle, same-store denial/other-store reclaim 두 경계만 integration으로
  올리고 scalar/constructor/panic 원인은 소유 모듈 테스트에서 닫는다.
- 의미 있는 code/config 변경의 최종 rail은 `python3 tools/pm/pm.py sync`와
  `scripts/check-workspace.sh`다. 개발 중 `--in-memory`는 유용하지만 full rail과 동치가 아니다.
  full rail의 optional-backing 회귀검증을 했다고 Redb product 지원을 선언하지 않는다.
- Cargo/cross-crate use path가 달라지면 committed clean candidate에서 extraction dry-run.
  실제 standalone 지원/CI 성공을 보고할 때는 실제 extraction의 rail/examples receipt가 필요하다.
- exact HEAD/tree, dirty diff digest, selector와 실행 수, exit status, feature/toolchain,
  raw evidence를 기록한다. focused/외부 진단 프로그램은 정식 rail을 대체하지 않는다.
- host API/metric 계약 변경은 Semantica에 전달할 migration 목록을 만든다. QMP 로컬 GREEN과
  해당 host revision의 빌드/E2E는 별개이며 여기서 외부 repo를 수정하지 않는다.
- 문서·구현·owner proof·integration proof·host activation 상태를 각각 기록한다.
  미구현 0.3/측정 패키지를 수치 점수나 체크 표시로 완료 처리하지 않는다.

## 3. Conditional 0.3 authority work

ENG-03/04는 현재 host의 활성 ledger 결함으로 재분류할 근거가 없다. 기존 RFC owner에게
다음 요구를 연결하고, 별도 필요성 없이 현 correctness 보수의 선행조건으로 만들지 않는다.

- **Rebuild:** multi-coordinator를 지원할 때 broker instance + epoch에 결합된 non-Clone
  permit으로 종료 권한을 한 곳에 둔다. 두 번째 begin, foreign/stale finish는 거절한다.
  token을 추가하면서 무토큰 `finish_rebuild()`를 남기면 authority gap은 닫히지 않는다.
- `Drop`이 미완료 rebuild를 성공으로 열어 주면 안 된다. abandoned fence는 관측 가능하게
  fail-closed로 남기고 coordinator의 명시적 recovery 또는 platform 재생성으로 처리한다.
  현재 in-memory scope에서 데이터 손실 허용은 무제한 stale token 허용의 근거가 아니다.
- fence 종료와 deferred cleanup 결과를 구분한다. 현재 `finish_rebuild`는 먼저 fence를
  열고 cleanup에서 error를 반환할 수 있다. 새 API는 fence 상태와 남은 authority를
  receipt로 구분하고 retry cleanup은 bounded batch로 제공한다. 0-byte ticket도 포함한다.
- **Taxonomy/headroom:** host-defined class/context, memory 외 execution policy 분리,
  non-forgeable headroom role은 RFC-0001 migration에서 같이 수행한다. 동일한 headroom/floor
  수학을 보존하고 host broker 접근을 대체할 API가 생기기 전에 기존 접근을 제거하지 않는다.
- **Out-of-lock client code:** STR-01의 helper가 이 요구를 충족한다고 선언하지 않는다.
  stable lookup identity, read/charge lifetime, reentrant/slow/panic 호출과 close 의미를
  함께 이행하는 별도 migration acceptance가 필요하다.

## 4. Why owner-local tests missed these cases

| Cause | Existing evidence | Required correction |
| --- | --- | --- |
| API별 fault inventory 누락 | transcript panic test는 retain/truncate/versioned retain을 순회하지만 `session_bytes`는 제외 | public operation별 caller-trait 경계 표와 read-path 반례; 보호 경계를 공통 소유 코드로 이동 |
| total-only oracle | prepared tombstone test는 total bytes와 close drain만 확인 | owner/class/context의 각 kind 값과 중간 lifetime 상태를 독립 계산 |
| 순서 공간 축소 | scratch test는 400→1000만 관측 | 감소/동일/초과/해제 후의 전이 시퀀스 |
| constructor 검증과 API 불변식 혼동 | thresholds 생성자는 거절하지만 public fields로 우회 | non-forgeable validated value + external compile-fail |
| 유효 소입력 편향 | GC는 valid/unknown reference, batch는 semantic rejection 위주 | max/max+1·대량 중복·invalid 위치·사전 할당/잠금 boundary |
| 지원 계약 부재 | self-reclaim과 rebuild coordinator 요구가 명시되지 않음 | 지원/거절 결과를 먼저 고정하고 실제 연결을 담은 최소 통합 테스트 |
| scope보다 넓은 해석 | conformance sum/GREEN, benchmark schema 성공, main CI를 일반 보장처럼 해석 가능 | 각 receipt에 검증 predicate와 source/feature/host/trigger 범위 명시 |

테스트 runner는 존재하는 assertion을 실행했다. 테스트 개수를 더 늘리는 것만으로 위 빈틈을
막을 수 없다. 각 owner regression에 하나의 대표 mutant/반례를 붙이고 그 실패 이유가 원하는
predicate인지 확인한다. 모든 패키지마다 광범위 mutation sweep를 반복할 필요는 없다.

## 5. Execution order and disposition

```text
STR-00 contract/oracle inventory
  |
  +-- A: STR-01 facade reads -> STR-03 prepared lifetime/roots --+
  +-- B: STR-02 core scalar/constructor -----------------------+
  +-- C: STR-04 broker batch/reclaim --------------------------+
  +-- integrator: STR-05 proof/docs/CI ------------------------+
                                                              |
                                   all required owners done --+
                                                              v
                            STR-07 serial integration, frozen candidate

STR-06 measurement: quiet host, after writers stop; separate performance decision
0.3 authority/taxonomy/callback migration: existing RFC track, explicit activation trigger
```

위 작업 lane은 파일 소유권이 겹치지 않을 때만 병렬 실행한다. `api/control.rs`,
`shared/builder.rs`, `domain/ledger.rs`와 generated guidance는 integrator가 순차 반영한다.
benchmark는 Cargo-heavy 작업과 병행하지 않는다.

| Finding | Disposition / completion owner |
| --- | --- |
| ENG-01 | STR-01 + STR-07, required availability fix |
| ENG-02 | STR-04, documented bounded denial plus cross-store positive; self-reclaim redesign only for a named requirement |
| ENG-03 | conditional 0.3 authority migration; current single-coordinator restriction remains explicit |
| ENG-04 | existing RFC-0001; not duplicated as a new immediate fix |
| ENG-05 | STR-06; family-specific bounds/SLO, not a measured performance failure |
| ENG-06 | STR-06; remains unqualified until compatible controlled receipts and explicit limits exist |
| ENG-07 | STR-04 operating contract + STR-06 if callbacks are activated |
| ENG-08 | STR-04, prevalidation before proportional scratch allocation |
| ENG-09 | STR-05; existing main CI truth and docs |
| ENG-10 | STR-03; bounded normalization outside the critical section |
| ENG-11 | STR-02, required public observation fix |
| ENG-12 | STR-02, next versioned API change; not blocked on all of RFC-0001 |
| ENG-13 | STR-00/03 + STR-07, bucket contract and lifetime oracle |

Completion for the immediate package means STR-00–05/07 and any explicitly deferred
API item are reported individually. It does not mean 100/100, full 0.3 completion,
host activation, or a SOTA performance result. No implementation was performed
while writing this plan.
