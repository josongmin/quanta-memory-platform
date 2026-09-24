# Memory/state substrate review checklist

검토 기준일: 2026-09-23

대상: QMP와 같은 process-local memory/state substrate

성격: 13개 영역, 100개 계약의 **리뷰 도구**. 점수표나 현재 구현의 합격 판정이 아니다.

가장 최근 항목별 점수·적용 범위·증거 상한은 [2026-09-24 `db0b03c` 재채점](memory-state-substrate-score-reaudit-2026-09-24-db0b03c.md)이며, `db0b03c` 소스 snapshot에 한정된다. 최신 committed base는 `65f57d5`; 이후 Cargo/Rust 변경은 주석이고 실행 동작은 바뀌지 않았지만, 점수 재산정이나 current-HEAD qualification은 하지 않았다. 현재 shared worktree에는 별도의 미커밋 RFC 편집이 있다. 이 체크리스트의 미체크 박스는 점수나 구현 상태를 뜻하지 않는다.

## 사용 원칙

- 기본 항목은 해당 substrate가 표방하는 계약에 적용한다. `조건:` 항목은 명시한 capability를 제공할 때만 적용한다. 미지원 기능은 이유와 노출되는 오류를 기록한 뒤 N/A로 둘 수 있다.
- `[ ]`는 미검증을 뜻한다. 문서 명시, 구현 확인, 테스트 통과, 실제 host 검증을 서로 다른 상태로 기록한다. 테스트 파일의 존재만으로 테스트 통과를 표시하지 않는다.
- 각 판정에 `항목 ID | 적용 여부 | 상태 | source SHA/경로 | test/receipt | 담당 경계 | 잔여 위험`을 남긴다. 현재 소스와 다른 SHA의 receipt는 현재 판정의 증거가 아니다.
- semantic charge는 등록된 비용 모델을 집행한다. allocator allocation, RSS, 임시 복사, 디스크 사용량의 상한을 자동으로 뜻하지 않는다. QMP의 범위는 [설계 가이드](../../README-detailed.md)와 [최종 closeout 기록](../../tickets/FINAL-CLOSEOUT.md)을 각각 확인한다.
- QMP가 소유한 객체/guard의 수명과 caller에게 넘긴 clone/버퍼의 수명을 구분한다. 공유 backing이나 내부 가변성으로 물리 payload가 남는 경우에는 누가 과금하거나 제한하는지 계약에 적는다.
- 특정 알고리즘(Hamilton, dynamic borrowing, S3-FIFO, W-TinyLFU, DRF, lock-free 등)은 필수로 두지 않는다. 현재 workload와 실패 모델에서 계약이 성립하는지를 본다.

권장 상태값: `미검증 / 문서 명시 / 구현 확인 / 테스트 통과 / host 검증 / N/A(이유)`.

## A. 범위·보장·책임 경계 (6)

- [ ] **A01 계량 대상** — semantic bytes, allocator allocation, RSS, disk bytes의 단위와 집행 대상을 분리한다.
- [ ] **A02 process-local 범위** — 등록된 상태만 통제한다고 명시하고 외부 할당·GPU·다른 프로세스에 대한 보장을 주장하지 않는다.
- [ ] **A03 단일 budget 권한** — 같은 domain의 store와 runtime scratch가 같은 broker에 묶이며, 다른 broker로의 이동은 거절하거나 명시적 이전 절차를 따른다.
- [ ] **A04 capability 선언** — eviction, spill, 재시작 복구, async waiting, live reconfiguration의 지원·미지원과 오류를 밝힌다.
- [ ] **A05 신뢰 경계** — `ByteSized`, tenant/context 매핑, 외부 버퍼 등록, 과금 후 내부 가변성의 책임을 host와 나눈다. in-process API를 보안 샌드박스로 취급하지 않는다.
- [ ] **A06 도메인 의미 분리** — 결과 유효성, 재계산 가능성, 작업 취소 가능성은 host가 제공하며 substrate가 추정하지 않는다.

## B. Accounting·reservation 정합성 (10)

- [ ] **B01 계정 분류** — pending reservation, retained payload/metadata, live scratch의 포함 관계와 합계를 정의한다.
- [ ] **B02 다축 합계** — class/owner/context 집계가 한 원장과 일치한다. 축별 합계를 다시 합산해 총량으로 표시하지 않는다.
- [ ] **B03 사전 admission** — substrate가 새로 보유할 상태는 승인 전에 공개하지 않는다. caller가 미리 만든 값과 substrate 내부 생성물의 책임을 구분한다.
- [ ] **B04 실패 rollback** — 예약 뒤 fallible allocation, serialization, index insertion이 실패해도 예약과 store 상태가 계약대로 복구된다. 프로세스 abort까지 rollback 대상으로 주장하지 않는다.
- [ ] **B05 checked arithmetic** — 덧셈·곱셈·차감·단위 변환의 overflow/underflow를 검사하고 semantic/physical 단위를 섞지 않는다.
- [ ] **B06 비용 공식 버전** — `ByteSized`와 고정 overhead의 범위를 공개하고 기존 charge는 원래 금액으로 반환한다. 과금 후 공유 payload의 크기 변경 허용 여부를 명시한다.
- [ ] **B07 관리 메타데이터** — ticket/owner/pin/pending/진단 자료의 증가를 charge 또는 독립된 개수·byte 한도로 제한한다. 특히 0-byte 권한을 대량 보유해도 메타데이터가 무제한 늘지 않는지 확인한다.
- [ ] **B08 공유 payload 귀속 — 조건: 공유·중복 제거** — global charge와 owner attribution을 구분하고 참조 변경으로 무료 메모리나 이중 환급이 생기지 않는다.
- [ ] **B09 교체 중 겹침** — QMP가 계속 소유하는 old/new/guard/staging의 동시 수명을 계량하거나 제한한다. caller 소유 clone과 임시 복사는 비용 모델 밖이라면 그 경계·peak 위험을 공개한다.
- [ ] **B10 권한 분할·이관 — 조건: 해당 API** — resize/split/merge/transfer/batch release의 실패·취소·재시도 후 총량과 단일 release 권한이 보존된다.

## C. Admission·quota·경합 정책 (8)

- [ ] **C01 즉시 거절** — 단일 요청이 허용량을 넘으면 무한 reclaim 없이 제한 scope를 포함한 오류를 반환한다.
- [ ] **C02 모든 강제 한도** — global/class/owner/group의 postimage를 한 grant에 대해 검증하고 경쟁 요청 후에도 재검증한다.
- [ ] **C03 quota 용어** — guarantee, protection, entitlement, burst, hard cap의 의미를 구분한다. borrow 가능한 값을 절대 hard cap처럼 설명하지 않는다.
- [ ] **C04 보장량 실현성 — 조건: 보장·계층** — 하위 보호량과 상위 용량을 대조하고 회수 불가능한 차입 상태의 실패 동작을 정한다.
- [ ] **C05 scope 수명** — 알 수 없는 group의 처리, ID 재사용, scope 삭제, owner 이동 중 기존 charge를 정의한다.
- [ ] **C06 대기 — 조건: waiting** — 대기 개수·bytes·deadline·취소 정리·순서·head-of-line blocking을 제한하거나 설명한다.
- [ ] **C07 대여·반환 — 조건: borrowing** — 차입 상한, 반환 유발 조건, 회수 가능성, 회수 실패를 계약화한다.
- [ ] **C08 fairness 주장** — memory share와 throughput/latency fairness를 구분하고 starvation 가능성을 공개한다.

## D. Handle·pin·객체 수명 (8)

- [ ] **D01 수명과 charge** — substrate가 보유하거나 guard로 수명을 연장하는 payload는 마지막 관리 참조까지 charge가 남는다. caller에게 반환한 clone은 별도 소유 경계로 정의한다.
- [ ] **D02 read/pin guard — 조건: 참조 반환** — guard 동안 backing이 해제·재사용되지 않는다. 복사 반환이면 복사 비용·수명은 누구 책임인지 명시한다.
- [ ] **D03 삭제와 회수** — lookup 제거, 마지막 관리 참조 해제, semantic quota 환급, allocator 반환을 구별한다.
- [ ] **D04 권한 복제 금지** — 사용자 handle 공유는 가능해도 release authority는 이중 소비할 수 없다.
- [ ] **D05 stale handle — 조건: ID/slot 재사용** — generation/epoch 또는 동등한 규칙으로 ABA 접근을 막는다.
- [ ] **D06 owner 종료** — 살아 있는 관리 참조와 child scope의 charge를 잃지 않도록 defer/transfer/reject를 정한다.
- [ ] **D07 release 실패** — rebuild/poison 중 explicit release와 최종 Drop이 실패해도 정리 권한을 보존하고 추적한다.
- [ ] **D08 close 의미** — close의 idempotency, 잔여 pin·pending 처리, 후속 접근, 물리 메모리와의 관계를 정의한다.

## E. 동시성·재진입·취소·panic (8)

- [ ] **E01 선형화 지점** — reserve/replace/release/pin/close의 효력 발생 지점을 정하고 반쪽 상태 관측을 막는다.
- [ ] **E02 잠금·외부 코드** — lock 순서를 정하고 재진입·blocking 가능한 callback과 I/O를 권한 lock 아래에서 실행하지 않는다. 불가피한 예외는 계약으로 제한한다.
- [ ] **E03 mutation 복구** — operation lease와 preimage/commit 규칙으로 callback 실패 뒤에도 ticketless entry·잠금 누수가 없다.
- [ ] **E04 오래된 plan** — lock 밖에서 만든 계획은 적용 전 generation, policy version, pin 상태를 재검증한다.
- [ ] **E05 panic 경계 — 조건: unwind 복구** — unwind, abort, 이중 panic, 강제 종료를 나누고 외부 callback의 side effect까지 rollback한다고 주장하지 않는다.
- [ ] **E06 async 취소 — 조건: async 작업** — caller future가 사라진 뒤에도 실제 I/O와 pending 권한을 살아 있는 coordinator가 관리한다.
- [ ] **E07 재시도 — 조건: retryable mutation** — 재실행, 동일 결과 반환, identity conflict를 구분하며 사용자 callback의 exactly-once를 추정하지 않는다.
- [ ] **E08 busy와 outcome unknown** — 경합, capacity denial, closed, 결과 불확실성을 caller가 구조화된 값이나 조회 API로 구분해 올바른 다음 행동을 선택한다.

## F. Reclaim·진행성·고갈 대응 (8)

- [ ] **F01 회수 가능성** — discardable/recomputable/spill-required/pinned/unique state를 구분하고 유일 상태를 캐시처럼 폐기하지 않는다.
- [ ] **F02 보고량 재측정** — callback 추정치만으로 grant하지 않고 최종 원장을 다시 검사한다.
- [ ] **F03 선정과 권한 변경** — policy가 후보를 제안하며 authoritative 경로가 generation/pin/charge를 검증한다.
- [ ] **F04 unused capacity — 조건: capacity 별도 관리** — 미사용 권한 반환과 실제 payload eviction을 구분한다.
- [ ] **F05 회수 경로의 생존** — 제공하는 reclaim/spill 경로에 필요한 scratch·metadata의 한도 또는 사전 할당을 정한다. 단순 release가 새 일반 admission에 의존하지 않는다.
- [ ] **F06 재귀·storm** — reclaimer가 같은 broker에 재진입할 때의 결과와 중복 pressure 요청의 상한을 정한다.
- [ ] **F07 제한된 작업** — scan/batch/I/O/retry를 제한하고 회수 불가 시 오류로 끝난다. 비협조적 callback의 강제 중단은 약속하지 않는다.
- [ ] **F08 최소 작업집합** — scratch도 모자라면 host가 batch 축소·대기·거절을 선택할 수 있다.

## G. Physical pressure·제어 안정성 (6)

- [ ] **G01 신호와 원장 분리** — RSS/cgroup/allocator/PSI 값을 semantic counter에 더하거나 그 값으로 보정하지 않는다.
- [ ] **G02 sample 품질 — 조건: physical adapter** — 시각·단위·scope·stale/unavailable을 제공하고 누락을 0으로 바꾸지 않는다.
- [ ] **G03 제어 진동 — 조건: 자동 pressure 제어** — 진입·해제 조건, hysteresis/cooldown, 목표량을 정하고 진동을 시험한다.
- [ ] **G04 emergency** — 새 증가를 막을 때도 release/cleanup은 살아 있으며 process kill은 host 정책으로 남긴다.
- [ ] **G05 OOM 주장 범위** — 관리 밖 할당, allocator slack, page cache, transient copy 때문에 semantic cap이 RSS hard bound가 아님을 명시한다.
- [ ] **G06 재현 가능한 입력 — 조건: adaptive policy** — clock/sample을 주입하고 같은 입력에서 결정을 재현한다. 숨겨진 blocking sampling을 hot path에 넣지 않는다.

## H. Cache·retention·엔진 의미 (7)

- [ ] **H01 cache와 유일 상태** — miss 시 재계산 가능한 값과 폐기하면 유실되는 값을 다른 eviction 계약으로 취급한다.
- [ ] **H02 cache identity — 조건: 엔진 결과 캐시** — host가 revision/branch/config/model/schema 등 유효성 identity를 제공한다.
- [ ] **H03 무효화와 수명 — 조건: invalidation/TTL** — 새 lookup 차단과 기존 관리 참조의 안전한 수명을 분리한다.
- [ ] **H04 byte 목표 — 조건: eviction** — entry 수가 아닌 회수 byte 목표와 pin 상태를 확인한다.
- [ ] **H05 policy metadata — 조건: 학습형 정책** — sketch/ghost/event 자료의 공간과 작업량을 제한한다.
- [ ] **H06 동시 miss — 조건: single-flight** — owner/waiter/취소/실패 전파와 in-flight registry 상한을 정의한다. 중복 계산을 허용하면 그 비용을 host와 나눈다.
- [ ] **H07 replay horizon — 조건: replay/publication** — payload 제거와 dedup 증거 삭제를 구분하고 conflict·horizon 종료·saturation을 정한다.

## I. Spill·restore·재시작 복구 (10)

아래 각 항목의 조건을 별도로 판정한다. 임시 spill 제공만으로 durable recovery 전체를 필수로 만들지 않는다.

- [ ] **I01 보장 수준 — 조건: spill** — 임시 외부화와 재시작 복구를 구분하고 restore 후 재차 crash한 경우를 정의한다.
- [ ] **I02 transfer identity — 조건: retryable transfer** — 논리 transfer ID와 attempt ID를 구별하고 committed/not-committed/unknown을 표현한다.
- [ ] **I03 commit 의미 — 조건: durable backing** — write accepted, transaction commit, durable sync와 지원 failure model을 구분한다.
- [ ] **I04 권한과 사본 — 조건: spill** — 논리 결정 권한이 하나여도 RAM/disk/staging 사본은 공존할 수 있다. 각 자원 점유와 이중 공개·환급을 검증한다.
- [ ] **I05 외부화 예산 — 조건: spill** — disk/record/in-flight I/O/codec scratch 한도와 disk full·부분 write 실패 동작을 정한다.
- [ ] **I06 restore 선승인 — 조건: restore** — resident charge와 필요한 scratch를 먼저 확보하고 실패 때 유일한 유효 사본을 보존한다.
- [ ] **I07 reconciliation — 조건: unknown outcome** — record ID·generation·내용을 확인하고 backing의 일관성 계약 없이 not-found를 미커밋 증거로 쓰지 않는다.
- [ ] **I08 재시작 발견 — 조건: 재시작 복구** — 메모리 pending 목록 외의 발견 수단과 오래된 완료를 막는 fencing을 둔다.
- [ ] **I09 형식·무결성 — 조건: durable bytes** — schema/cost-model version, checksum, 길이·decode 상한을 검증하고 보안 책임을 threat model에 맞춰 나눈다.
- [ ] **I10 orphan GC — 조건: GC/publication** — host root snapshot의 완전성과 동시 activation에 대한 barrier를 요구한다.

## J. Live reconfiguration·rebuild (6)

- [ ] **J01 policy version — 조건: live config** — decision/receipt/snapshot이 적용된 정책을 식별하고 version 재사용을 방지한다.
- [ ] **J02 limit 축소 — 조건: live config** — usage 아래로 낮출 때 reject/drain/reclaim 중 정책과 반환 상태를 정의한다.
- [ ] **J03 기존 권한 정리** — 낮아진 cap이나 rebuild가 기존 charge의 최종 반환을 영구히 막지 않는다.
- [ ] **J04 원자적 교체 — 조건: live config** — override/group/allocation table의 교체 단위와 오래된 계획 재검증을 정한다.
- [ ] **J05 scope 이동 — 조건: scope 변경** — usage가 남은 owner/group 삭제를 reject/drain/migrate 중 하나로 처리하고 default 이동으로 quota를 우회하지 않는다.
- [ ] **J06 rebuild — 조건: rebuild** — 차단 operation, 관측 상태, 작업 예산, 실패 후 권한과 cleanup backlog를 정의한다.

## K. SDK·확장 경계 (7)

- [ ] **K01 생성 시 결합** — builder/typestate 등으로 store를 budget domain에 묶고 raw ticket 위조·late accounting attachment를 차단한다.
- [ ] **K02 사용 가능한 API** — reserve/guard 또는 명시적 copy/replace/remove/close/observe를 안전하게 조합한다. host가 batch 처리를 요구하면 전체 materialization 전에 chunk별 admission이 가능한지 평가한다.
- [ ] **K03 확장 surface — 조건: custom store/backing** — policy/codec/reclaim SPI와 accounting 권한을 갖는 backing SPI의 신뢰 수준을 나누고 conformance를 요구한다.
- [ ] **K04 runtime 독립성** — core의 executor·hidden thread·global singleton 의존성과 blocking/shutdown 책임을 명시한다.
- [ ] **K05 feature 계약** — compile/runtime capability가 일치하고 feature 통합으로 unbudgeted 경로가 production에 노출되지 않는다.
- [ ] **K06 typed errors** — denial scope, bytes, 정책 버전, retryability, unknown outcome을 가능한 범위에서 구조화한다. explain 조회는 admission/reclaim을 실행하지 않는다.
- [ ] **K07 외부 버퍼 — 조건: FFI/foreign buffers** — buffer 수명, charge, deallocator, thread/runtime 종료 책임을 함께 정의한다.

## L. 관측·운영·진단 (6)

- [ ] **L01 일관된 snapshot** — semantic snapshot의 원자성·버전을 명시하고 별도 시점 metrics를 admission 판단에 사용하지 않는다.
- [ ] **L02 사용량 관계** — reserved/retained/scratch/pinned/pending/spilled의 포함 관계를 보여주고 겹치는 수치를 다시 더하지 않는다.
- [ ] **L03 실효 성능** — 성공 grant, busy retry, denial, reclaim, spill의 처리량과 tail latency를 분리한다.
- [ ] **L04 관측 비용** — metric cardinality, event queue, tracing, top-owner 조회의 상한을 둔다.
- [ ] **L05 정체 진단 — 조건: pending 작업** — pending age, 실패한 reclaim, 긴 pin, cleanup backlog와 안전한 다음 행동을 노출한다.
- [ ] **L06 잔여 권한** — quiescent 시점의 outstanding charge/owner/handle을 추적하고 ledger 누수와 allocator retained memory를 구분한다.

## M. 검증·성능·host 채택 (10)

- [ ] **M01 독립 oracle** — 단순 reference model 및 긴 operation sequence를 비교하고 broken fake를 거부한다.
- [ ] **M02 컴파일·타입 계약** — ticket 복제, 잘못된 builder, foreign authority, 금지된 attachment를 compile-fail 또는 typed reject로 검사한다.
- [ ] **M03 동시성 검증 범위** — 실제 production 코드, 축약 모델, 계측 primitive의 범위를 구분한다.
- [ ] **M04 실패 주입** — Hash/Eq/Clone/Drop/ByteSized panic, poison, fallible adapter의 실패를 주입한다. 일반 allocator OOM/abort에서 unwind 복구를 약속하지 않는다.
- [ ] **M05 crash/reopen — 조건: 재시작 복구** — write/commit/ACK/source release/restore/delete 경계에서 실제 종료·재시작하고 failure model을 기록한다.
- [ ] **M06 경합·카디널리티** — 공유/독립 store, thread 수, owner/context 수, entry 크기를 바꾸며 성공 처리량과 busy 비율을 함께 본다.
- [ ] **M07 장시간·단계 변화** — pin 축적, policy 변경, reclaim storm 뒤 semantic charge를 확인한다. RSS=0을 종료 조건으로 삼지 않는다.
- [ ] **M08 실제 trace — 조건: cache 정책 선택** — 같은 예산에서 byte miss, 재계산 비용, CPU, tail latency, metadata를 비교한다.
- [ ] **M09 host E2E — 조건: host 채택 판정** — 실제 엔진의 결과 정합성, 취소, 메모리 peak, 성능을 측정한다. 라이브러리 검증과 별도 gate다.
- [ ] **M10 재현 가능한 receipt** — source SHA, toolchain, features, allocator, machine, seed, faults, 반복, raw 결과를 남기고 구현·검증·배포·활성화 상태를 분리한다.

## 우선 검증 시나리오

아래는 결함 판정이 아니라 실패 모델을 드러내는 실행 후보이다. 적용 capability와 공격자가 조절할 수 있는 입력을 먼저 적는다.

| 시나리오 | 기대 결과 | 관련 항목 |
| --- | --- | --- |
| 0-byte managed charge를 대량 보유 | byte cap과 별개로 ticket metadata의 상한 또는 명시적 host 제한을 확인 | B07, L04 |
| `get_checked`로 clone을 받은 뒤 remove/replace | QMP 소유 charge와 caller 소유 clone의 경계가 선언한 계약과 일치 | A05, B09, D01–D03 |
| rebuild fence 중 최종 `ManagedCharge` Drop | 권한이 보존되고 fence 종료 후 정확히 한 번 반환 | D07, J03, J06 |
| 전부 pinned이거나 reclaimer가 같은 broker에 reserve | 강제 해제·재귀 폭주·무한 retry 없이 정해진 거절 또는 bounded 대기 | F01, F06–F08 |
| spill용 scratch도 부족 | 제공하는 실패 모델 안에서 source와 권한을 보존하거나 제한된 여유로 완료 | F05, I04–I06 |
| caller 취소 직후 I/O 완료 | coordinator가 pending과 완료 결과를 판정하고 이중 공개를 막음 | E06–E08, I02 |
| durable commit 직후 ACK 유실·재시작 | backing 증거와 안정적인 identity로 reconciliation | I02, I03, I07, I08, M05 |
| restore가 durable 사본을 지운 직후 재차 crash | 약속한 temporary/durable 보장에 맞는 재계산 또는 복구 | I01, I06, M05 |
| GC root 수집 중 host activation | 안정된 root/barrier 없이는 삭제하지 않음 | I10, M09 |
| 공유 payload의 과금 후 내부 크기 변경 | 금지·재과금·host 책임 중 명시한 정책대로 처리 | A05, B06, B08 |

## 판정 gate

1. **Core:** 적용되는 권한·accounting·관리 객체 수명 계약에 재현 가능한 P0 실패가 있으면 미통과. 안전한 거절은 정상 결과다.
2. **Feature:** spill, durable recovery, waiting, FFI 등 선택 capability는 해당 failure model을 입증하거나 지원 범위를 축소한다. 전체 I 영역을 무조건 core P0로 취급하지 않는다.
3. **Host:** 실제 host의 결과 정합성·취소·메모리·성능과 정확한 QMP/host 소스 쌍을 별도로 검증한다. 라이브러리 테스트는 host activation을 증명하지 않는다.

판정 기록 예시:

```text
ID: B07
적용: 예 / QMP live broker
상태: 미검증
source: <QMP SHA>:<path:line>
test/receipt: <selector or receipt / 실행하지 않았으면 없음>
담당 경계: core / host
잔여 위험: 0-byte live ticket 수에 대한 독립 한도 확인 필요
```
