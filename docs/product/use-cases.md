# QMP 유즈케이스와 책임 경계

감사일: 2026-10-06. 소스 기준: `63b31fabae400fdd188547ed9b818b1a38b12269`.
대상: 현재 `0.2.0` portable Rust library, in-memory accounting·reclaim.

이 문서는 **누가, 어떤 상황에서, 무엇을 위해 QMP를 사용하는가**의 단일 진입점이다.
아래 사용자는 소비자 역할의 예시이며 해당 제품에 이미 배포됐다는 뜻이 아니다.
동작의 권위는 현재 소스와 [ADR](../adr/README.md), 테스트 목록은
[verification scenarios](../verification-scenarios.md), API 이행은
[MIGRATION](../../MIGRATION.md)에서 확인한다. 이 문서는 새 기능·IR·release receipt를 만들지 않는다.

## 1. 라이브러리가 해결하는 문제

여러 runtime·owner·store가 같은 budget domain에서 데이터를 보유할 때,
**보유를 허용할지 결정하고, 한 ledger에 귀속시키고, 소유 수명이 끝나거나 이전될 때
정확한 charge authority를 반환·이전한다.**

예를 들어 query worker의 임시 작업 공간, artifact payload, memo entry, session transcript가
하나의 budget을 공유할 수 있다. QMP는 이 데이터의 의미·유효성·작업 실행을 결정하지 않는다.
호스트가 제공한 policy, identity, semantic cost와 수명 전환을 검증하고 account한다.

```mermaid
flowchart LR
    H["Host: policy / identity / lifecycle"] --> B["MemoryPlatformBuilder"]
    B --> P["LiveMemoryPlatform"]
    P --> S["Budgeted facade stores"]
    P --> R["Broker-bound custom charges"]
    P --> V["RuntimeMemoryView / scratch"]
    S --> L["One broker / semantic ledger"]
    R --> L
    V --> L
```

### 현재 범위와 상태

| 구분 | 상태 |
| --- | --- |
| 아래 UC01–UC10 | 현재 0.2 구현·공개 계약에 근거한 사용 흐름 |
| `MemoryConfig → MemoryScope → MemoryConsumer` DSL, host-neutral taxonomy | [RFC 0001](../rfcs/0001-sdk-interface-contract.md)의 미구현 0.3 설계 |
| `spill-durable`, `spill-redb`, codec·disk recovery | Opt-in maintenance. 현재 생산용 제품 지원·durability 보장 없음 |
| Host adoption·배포·SLO·hosted CI | 소비자와 운영 환경의 별도 실행 증거 필요 |

현재 API는 facade builder-first이며 dependency 방향은 `core <- facade <- testkit`이다.
`full`은 in-memory 기능을 묶으며 durable API·Redb를 포함하지 않는다.
`unbudgeted-stores`는 명시적으로 account하지 않는 별도 fixture/use 타입이다.

## 2. 유즈케이스 선택

| ID | 사용자 / 발생 상황 | 얻는 결과 | 현재 진입점 |
| --- | --- | --- | --- |
| UC01 | 여러 worker·store가 같은 예산을 사용 | 한 domain의 admission·attribution, 구성만 미리 확인 가능 | `MemoryPlatformBuilder`, `LiveMemoryPlatform` |
| UC02 | Host 자체 index·buffer·memo 구조를 보유 | QMP store 밖의 데이터에도 같은 ledger와 move-only charge 적용 | `reserve_owned`, `PendingReservation`, `ManagedCharge` |
| UC03 | Query·분석 작업이 임시 공간을 사용 | 실행 scratch의 예약·관측·반환 | `attach_runtime`, `RuntimeMemoryView::scratch` |
| UC04 | 같은 artifact payload를 반복 등록하거나 pin | Canonical dedup, 수명에 묶인 reference·charge | `payload_store_builder`, `PayloadStore` |
| UC05 | 키로 조회하는 retained metadata/value를 보유·교체 | Atomic insert/recharge·checked read·정확한 sweep 환급 | `retained_map_store_builder`, `RetainedMapStore` |
| UC06 | 오래된 임시 항목을 버려 capacity를 되찾음 | FIFO reclaim·ledger 재측정 후 제한된 admission 재시도 | `revocable_store_builder`, `MemoryReclaimer` |
| UC07 | 재시도되는 session append와 일부 transcript 조회 | Issued handle, replay evidence, bounded page, 명시적 수명 종료 | `transcript_store_builder`, `SessionTranscriptStore` |
| UC08 | Payload를 먼저 준비하고 host root에서 공개 | Deterministic token, prepare/abort/resolve/GC의 charge custody | `prepared_publication_builder`, `PreparedPublicationService` |
| UC09 | 같은 process 안에서 보유 항목을 tier로 이동 | 원 charge를 유지한 transfer·FIFO restore | `spill_one_into`, `SpillController`, `accept_restored` |
| UC10 | 운영 중 사용량 확인·policy 교체·rebuild | Lock-consistent observation·live-state validation·fence 소유권 | `observe`, `try_live_reconfigure`, `begin_rebuild` |

## 3. 모든 유즈케이스의 공통 계약

- **한 권위:** 같은 domain의 store·scratch·custom charge는 같은 broker를 사용한다.
  별도로 build한 platform 두 개는 각각의 ledger이며 합산 process cap을 만들지 않는다.
- **귀속:** class·owner·context·kind·group projection은 같은 ticket을 설명한다.
  Context는 attribution bucket과 overflow guard이며 독립 hard cap·권한·session incarnation이 아니다.
  현재 `MemoryClass`의 host-origin 이름은 ledger key이며 host 알고리즘 구현이 아니다.
- **비용:** `ByteSized`와 `semantic_cost_v1`의 retained cost는 선언된 semantic lifetime 비용이다.
  Store는 admission 때 비용을 고정하고 그 charge를 반환한다. Mutable 값의 비용 변경은
  적절한 replace/recharge와 결합해야 한다. Payload/metadata 통계는 lease별 bucket이다.
- **거부:** accounting mutation은 전체 postimage를 적용하거나 기존 상태를 유지한다.
  Capacity·closed·busy·invalid identity·accounting 오류를 성공·empty·새 session으로 변환하지 않는다.
  Host callback 내부의 임의 부작용까지 rollback한다는 보장은 없다.
- **수명:** lease·rollback receipt·read pin이 실제 보유 기간을 따라야 한다.
  반환된 caller-owned clone, transient canonicalization, allocator overhead와 RSS는
  store retained accounting으로 자동 측정되지 않는다.
- **정리:** explicit `close`/`release`의 오류를 처리한다. 정상 Drop 경로의 반환과 rebuild 중
  보수적 deferred cleanup은 소유권 계약의 일부다. `mem::forget`, process exit/abort,
  `panic=abort`에서 Drop 실행을 보장하지 않는다.
- **입력 한도:** 0-byte ticket도 authority를 점유한다. Ticket count·identifier 길이,
  transcript open-key/evidence, prepared evidence/root input 등 management 한도는
  byte cap과 독립이다. 이 한도가 RSS 상한이나 caller callback latency를 보장하지 않는다.

## 4. 유즈케이스별 흐름과 실패 계약

### UC01. 여러 구성 요소의 예산을 한 domain에 묶기

- **사용자/트리거:** 서비스 host가 worker, payload, metadata, transcript의 예산을 구성한다.
- **흐름:** manifest/preset/override를 builder에 넣고 `build()`에서 검증·구축한다. 구성 검토만
  필요하면 `build_report_only()`에서 검증·report를 얻는다. Runtime view와 budgeted store builder는
  같은 platform에서 얻는다.
- **결과:** global·class·owner·group·scratch·protected floor 등 해당 admission 축을 함께 검사한다.
  Report-only 결과에는 runtime·accounting authority가 없다.
- **실패/host 의무:** 잘못된 policy를 build 전에 거부한다. Host가 capacity 입력·owner 구분·
  독립 domain 구성을 결정한다. 같은 예산을 공유하려는 구성 요소에 별도 platform을 만들지 않는다.
- **근거:** [facade builder](../../crates/quanta-memory-platform-facade/src/shared/builder.rs),
  [공개 builder/accounting tests](../../crates/quanta-memory-platform-facade/tests/public_api_surface.rs),
  [report example](../../crates/quanta-memory-platform-facade/examples/effective_budget_report.rs).

### UC02. Host 자체 데이터 구조의 보유 비용 account하기

- **사용자/트리거:** index segment·memo row·buffer 같은 host-owned 구조가 QMP 내장 store 밖에서 자란다.
- **흐름:** platform의 live broker에서 `reserve_owned(...)`로 pending authority를 얻고,
  실제 보유를 게시할 때 `commit()`하여 `ManagedCharge`를 함께 소유한다.
  이미 확정한 비용은 `reserve_managed_charge(...)` 경로를 사용할 수 있다.
  크기·귀속 교체는 `recharge`, 수명 종료는 `release`/Drop과 결합한다.
- **Resize 경계:** committed charge의 절대 크기를 원자적으로 교체하며 새 ticket ID를 발급한다.
  감소는 차액을 반환하지만 pending/scratch authority의 resize나 기존 charge의 부분 분할은 아니다.
  `recharge`는 새 reserve의 soft reclaim·hard reclaim/retry 경로를 호출하지 않는다.
- **결과:** custom component와 내장 store가 같은 ledger cap을 경쟁한다. Charge는 non-Clone이며
  발급 broker에 묶인다. 내부 preflight의 판정은 예약 authority가 아니며 미래 성공 보장이 아니다.
  별도 public preflight API를 제공하지 않는다.
- **현재 taxonomy:** 기존 `MemoryClass`에 class를 선택하고 owner/context를 귀속시킨다.
  임의 class 등록 API나 host-neutral `MemoryConsumer` registry는 현재 지원하지 않는다.
- **실패/host 의무:** pending 포기는 rollback한다. 정상 recharge denial은 기존 charge를 유지하며
  release 실패도 retry authority를 보존한다. Host는 실제 데이터와 charge의 게시/해제 순서,
  비용 측정·취소·rollback custody를 소유한다. Payload가 남아 있을 때 capacity를 먼저 반환하지 않는다.
  0.3의 `reserve_up_to`/`commit_measured`/`MemoryConsumer`는 현재 API가 아니다.
- **근거:** [owned authority](../../crates/quanta-memory-platform-core/src/api/control.rs),
  [authority operations](../../crates/quanta-memory-platform-core/src/api/control/authority.rs),
  [broker tests](../../crates/quanta-memory-platform-core/src/api/control/tests/cases_01.rs).

### UC03. 실행 임시 공간을 예약하고 반환하기

- **사용자/트리거:** query·static analysis·batch 작업이 scoped temporary workspace를 필요로 한다.
- **흐름:** `attach_runtime`으로 view를 얻고 owner/context를 정한다. `ScratchAllocator`가
  `ScratchRequest`에 대한 `ScratchReservation`을 발급한다. 관측 peak를 기록하고 scope 종료 시 반환한다.
- **결과:** live scratch는 해당 broker의 semantic charge다. `grow_to_observed`는 예약 범위 안의
  peak 기록이며 추가 capacity 확보가 아니다. 작은 후속 관측이 peak를 낮추지 않는다.
- **실패/host 의무:** 예약 초과 관측, 반환 후 관측·double release를 거부한다.
  Host가 실제 scratch allocation과 예약의 수명을 맞춘다. CPU·wall time·query count·graph edge
  limit은 host가 제어한다. QMP가 allocation 자체를 수행하거나 추적하지 않는다.
- **근거:** [runtime port](../../crates/quanta-memory-platform-core/src/api/broker.rs),
  [scratch contract/tests](../../crates/quanta-memory-platform-core/src/ports/scratch.rs),
  [live scratch adapter](../../crates/quanta-memory-platform-core/src/adapters/scratch.rs).

### UC04. 중복 payload 등록과 pin 수명 관리

- **사용자/트리거:** artifact producer가 같은 canonical payload를 반복 등록하고 참조 중 삭제를 막는다.
- **흐름:** `put_canonical`로 store-issued `String` reference를 받는다.
  Concrete store의 pin/unpin으로 개별 삭제를 막고 `delete_if_unpinned`로 삭제한다.
  `close`는 store 전체를 종료한다.
- **결과:** 같은 live canonical payload는 같은 reference와 단일 charge를 유지한다.
  삭제 뒤 재등록·다른 store의 reference는 구분되며 stale reference가 새 entry를 수정할 수 없다.
- **실패/host 의무:** pinned delete, foreign/stale mutation, identity 발급 고갈을 거부한다.
  Reference는 process/store-local lookup identity이며 권한 토큰·durable ID가 아니다.
  Host는 reference를 파싱하거나 restart 경계에서 재사용하지 않는다.
  **Pin은 개별 delete만 막는다. `close`는 pin count와 관계없이 모든 entry를 해제한다.**
  Host가 사용자의 수명과 store 종료를 조정해야 하며 pin을 store-close 방지 lease로 취급하지 않는다.
  **이 store에는 reference로 payload를 읽는 get/resolve API가 없다.** 조회가 필요한 데이터 흐름은
  별도로 선택한다; UC08의 prepared resolve와 혼동하지 않는다.
- **근거:** [payload port](../../crates/quanta-memory-platform-facade/src/payload_store/mod.rs),
  [pin/reference/charge tests](../../crates/quanta-memory-platform-facade/src/payload_store/service/tests/cases_01.rs),
  [public identity tests](../../crates/quanta-memory-platform-facade/tests/public_api_surface.rs).

### UC05. 키로 조회하는 retained entry를 원자적으로 게시·교체하기

- **사용자/트리거:** host가 memo metadata·retained result·key/value 상태를 공유하고 중복 producer가 경쟁한다.
- **흐름:** bound retained-map builder로 store를 만든다. Checked insert-if-absent로 creator를 결정하고,
  checked read로 값을 확인한다. Replacement는 recharge와 결합하고 remove/sweep/close가 비용을 반환한다.
- **결과:** key·index·value의 semantic cost를 account한다. Atomic insert-if-absent는
  read-then-insert race로 기존 entry를 덮어쓰지 않는다. Enumeration은 key의 deterministic order를 제공한다.
- **실패/host 의무:** checked API에서 absence와 storage/accounting 오류를 구분한다.
  Replacement denial은 기존 entry·charge를 유지한다. Key의 Eq/Hash 법칙과 실제 값 유효성,
  dependency invalidation·TTL·query recomputation은 host가 소유한다. Cache coherence engine으로 사용하지 않는다.
- **근거:** [retained-map port](../../crates/quanta-memory-platform-facade/src/retained_map/mod.rs),
  [owning tests](../../crates/quanta-memory-platform-facade/src/retained_map/service/tests/mod.rs),
  [concurrency/public caller](../../crates/quanta-memory-platform-facade/tests/public_api_surface.rs).

### UC06. 버려도 되는 항목을 reclaim하고 admission 재시도하기

- **사용자/트리거:** reusable temporary data를 보유한 worker가 pressure 또는 capacity denial을 만난다.
- **흐름:** FIFO revocable store에 insert하고 builder의 `with_reclaimer`로 host callback을 설치한다.
  Callback이 `reclaim_one`/`reclaim_until`로 버릴 항목을 제거한다. 공개 `reserve_owned`/
  `reserve_managed_charge` 또는 새 reservation을 발급하는 store insert admission에서 broker가
  내부적으로 재측정·재시도한다. 기존 charge를 교체하는 `recharge`에는 이 경로가 적용되지 않는다.
  Revocable store를 만들기만 하면 callback에 자동 등록되는 것은 아니다.
- **결과:** release가 실제로 반환한 ledger capacity만 다시 사용할 수 있다.
  Callback의 reported reclaimed bytes는 advisory다. Reclaimer는 broker lock 밖에서 호출한다.
- **실패/host 의무:** terminal denial·blocked 요청에는 reclaim을 호출하지 않는다.
  Hard admission은 reclaim 뒤 최대 한 번 재시도하며 실패하면 오류를 반환한다.
  Revocable store의 `reclaim_one`/`reclaim_until` 자체가 실패하면 해당 operation의 entry/FIFO/charge를
  보존한다. Host callback이 다른 store에서 이미 수행한 부작용까지 broker가 rollback하지 않는다.
  Host가 버려도 되는 victim과 대체 작업을 결정한다.
  Same-store callback은 busy를 받을 수 있다. FIFO store에는 key lookup API가 없다.
  Reclaim rung은 host hint이며 자동 tier ladder·async wait·최소 회수량·throughput fairness를 보장하지 않는다.
- **근거:** [revocable port](../../crates/quanta-memory-platform-facade/src/revocable_store/mod.rs),
  [reclaim tests](../../crates/quanta-memory-platform-facade/src/revocable_store/service/tests/mod.rs),
  [broker admission](../../crates/quanta-memory-platform-core/src/api/control/admission.rs).

### UC07. Session 재시도·일부 읽기·명시적 retirement

- **사용자/트리거:** conversation/log producer가 append를 재시도하고 긴 session을 나눠 읽으며 종료한다.
- **흐름:** `open_session(caller_id)`의 opaque `SessionHandle`을 producer까지 전달한다.
  Handle+sequence+message identity로 append한다. `read_page` cursor로 같은 version을 읽고,
  필요하면 whole-session selector로 `retain_sequences_if_version`을 수행한다.
  Session 종료는 `retire_session`; store 전체 종료는 close다.
- **결과:** 동명 caller ID로 열린 handle들도 독립 incarnation이다. 열린 handle의 exact replay는
  새 row/charge를 만들지 않는다. Eviction 후에도 identity evidence를 유지한다.
  Retire는 key·active rows·evidence를 함께 반환하고 늦은 옛 handle을 거부한다.
- **실패/host 의무:** 0행 session도 count/key bytes를 점유한다. Row retention과 evidence/open 한도는
  독립이다. Page 한도는 row 수와 active semantic cost이며 caller clone의 RSS/latency 상한이 아니다.
  Zero limit·첫 row가 byte limit보다 큰 요청은 Capacity, foreign/stale cursor는 InvalidState,
  stale CAS는 `StaleSnapshot`이다. 내부 Arc를 잡고 caller Clone이 진행 중인 row의 eviction,
  해당 session의 retire·전체 close는 charge 반환 전에 busy로 거부한다.
  Pin은 in-flight read 종료까지이며 반환된 owned snapshot/page는 charge pin을 보유하지 않는다.
  **Page는 일부 행이고 retain selector는 전체 session의 exact 집합이다.** 한 page의 sequence만
  넘기면 다른 page가 퇴거된다. Cursor version/CAS는 selector 완전성을 보증하지 않는다.
  Host는 session end hook·handle mapping·재시도 horizon을 결정한다. Retire 실패 시 mapping을 유지한다.
  Retired exact retry도 거부하며 문자열을 최신 handle로 자동 라우팅하지 않는다.
- **근거:** [public trait/IR](../../crates/quanta-memory-platform-facade/src/transcript_store/mod.rs),
  [lifecycle/page tests](../../crates/quanta-memory-platform-facade/src/transcript_store/service/tests/lifecycle_paging.rs),
  [retirement negative conformance](../../crates/quanta-memory-platform-testkit/src/conformance/suites/tests/cases_01.rs),
  [runnable public example](../../crates/quanta-memory-platform-facade/examples/transcript_session.rs).

### UC08. Payload 준비와 host root 공개를 분리하기

- **사용자/트리거:** artifact publisher가 재시도 가능한 준비를 수행한 뒤 별도 host table에서 공개한다.
- **흐름:** 안정된 logical identity·canonical key로 `prepare`하고 receipt의 token을 얻는다.
  Host가 자신의 canonical root에 token을 게시한다. `resolve`는 resident prepared payload를 조회한다.
  Abort 또는 host-provided root snapshot에 대한 orphan GC는 active payload를 반환한다.
- **결과:** 같은 identity/key의 32-byte token은 deterministic이다. Fresh prepare에서
  eventual retirement를 포함한 전체 identity evidence 공간을 예약한다. Abort/GC는 새 capacity
  admission 없이 evidence를 aborted 상태로 보존하고 active charge만 반환한다. Evidence는 close까지 남는다.
- **실패/host 의무:** identity conflict, aborted/unknown token, evidence 포화와 과대 root input을 거부한다.
  **QMP에 commit API·host root registry는 없다.** Host는 공개 root와 아직 terminal 처리하지 않은
  outstanding prepare를 포함한 완전하고 안정된 보호 집합을 GC에 제공하고, 그 snapshot과 root 게시·
  제거·outstanding 전환을 직렬화한다. 보호 집합에서 빠진 live token은 GC 대상이다.
  Deterministic token 재계산은 process restart 후 resident payload/evidence 복구 보장이 아니다.
  TTL·epoch·token salt로 evidence를 자동 만료시키지 않는다.
- **근거:** [prepared lifecycle](../../crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs),
  [upfront evidence admission](../../crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication/prepare.rs),
  [owning tests](../../crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication/tests/mod.rs),
  [host custody boundary](../adr/0004-host-adoption-boundary.md).

### UC09. In-process tier 사이에서 charge custody 이전하기

- **사용자/트리거:** revocable store의 항목을 같은 process의 retaining tier에 보관했다가 되돌린다.
- **흐름:** `spill_one_into`가 entry와 charge를 소유한 handle을 검증된 in-process controller로 이전한다.
  `restore()`가 oldest handle을 반환하면 `accept_restored(handle)`로 source에 재수용한다.
- **결과:** Source에서 사라져도 target이 payload를 보유하면 같은 charge가 살아 있다.
  Source close가 target의 live charge를 환급하지 않는다. In-process spill은 메모리를 해제하는
  disk offload가 아니므로 이 transfer만으로 전체 ledger headroom이 증가하지 않는다.
- **실패/host 의무:** `spill_one_into`의 destination 실패는 controller가 반환한 원 handle·charge를
  source FIFO에 복원하고 host에는 오류를 반환한다. `accept_restored` 실패는 caller에게 charged
  handle을 `SpillFailure`로 돌려준다. Restore는 정상 empty일 때 `None`, retaining tier의 read 불가일 때
  오류다. Foreign broker/store로의 adoption과 closed-source 수용은 거부한다.
  Host는 수용 실패로 돌아온 handle의 후속 custody를 처리한다.
  Noop/discard의 기본 `None`은 복구 가능한 retaining 성공의 증거가 아니다.
- **근거:** [spill contracts](../../crates/quanta-memory-platform-facade/src/spill/mod.rs),
  [in-process transfer/restore tests](../../crates/quanta-memory-platform-facade/src/revocable_store/service/spill.rs),
  [offline/empty tests](../../crates/quanta-memory-platform-facade/src/spill/service.rs).

### UC10. 운영 관측·live policy 변경·rebuild fencing

- **사용자/트리거:** operator가 capacity를 변경하거나 host가 rebuild 동안 mutation을 제한한다.
- **흐름:** `observe()`로 한 broker의 lock-consistent 정책/사용량을 얻는다.
  `try_live_reconfigure`는 현재 live attribution을 새 policy로 검증한 뒤 적용한다.
  Rebuild는 move-only fence를 발급하고 소유자가 finish/receipt를 처리한다.
- **결과:** 새 설정이 미래 reserve뿐 아니라 기존 ticket도 만족해야 한다.
  0-byte pending/committed ticket도 class authority다. Rebuild 중 committed Drop은
  정확한 authority를 보존하며 보수적으로 charged 상태로 남을 수 있다.
- **실패/host 의무:** invalid live reconfigure·competing/잘못된 fence·observation 오류를 거부한다.
  `finish` 없이 Fence를 Drop하면 abandoned 상태와 mutation 거부를 남긴다. Host는
  `rebuild_fence_status`를 확인하고 `begin_rebuild`로 재인수해 finish한다.
  Finish receipt의 fence-open 결과와 deferred-drain 결과는 별도로 처리한다.
  Drain 오류는 fence를 다시 닫지 않는다. Deferred committed release의 재시도는 다음에 수락된
  fence finish로 수행하며, live reconfigure의 cleanup 재시도는 pending rollback만 처리한다.
  Poison/corruption을 zero snapshot으로 바꾸지 않는다. Host는 통계 수집·경보·fence 완료를 소유하고
  independently sampled report를 atomic observation으로 표시하지 않는다.
  Metrics/allocator probes는 advisory이며 ledger admission·semantic pressure를 덮어쓰지 않는다.
- **근거:** [observation](../../crates/quanta-memory-platform-core/src/api/control/observation.rs),
  [fence/lease types](../../crates/quanta-memory-platform-core/src/api/control.rs),
  [live class authority regression](../../crates/quanta-memory-platform-core/src/api/control/tests/live_class_authority.rs),
  [ledger/policy contracts](../adr/0001-portable-accounting-authority.md).

## 5. 소비자가 확인할 acceptance

위 유즈케이스를 제품에 채택할 때 다음을 실제 consumer 경로에서 관측한다.
Owning unit test 이름이나 library compile 성공으로 제품 실행을 대체하지 않는다.

| 경계 | 필요한 관측 |
| --- | --- |
| Domain 연결 | 사용 store·scratch·custom component가 Cargo-resolved 같은 QMP revision과 같은 broker domain 사용 |
| Admission/오류 | 실제 caller에서 typed denial, 거부 후 content/identity/charge 보존, 환급 후 readmission |
| 수명/취소 | Producer end·cancel·replace·rollback·close에서 실제 data와 charge custody가 일치 |
| 동시성 | 실제 handle·read/CAS·root/GC fence를 사용하는 caller; caller callback과 host lock 순서 확인 |
| 관측 | Ledger projection과 component 수명이 일치; semantic charge·allocator/RSS를 분리해 보고 |
| 비용/성능 | 실제 retained cost와 workload를 확인. SLO는 사전 목표치·호환 baseline·반복 실행으로 별도 검증 |

현재의 [검증 카탈로그](../verification-scenarios.md)는 positive/negative owning tests와
broken fake rejection을 연결한다. Source qualification은
[ADR 0003](../adr/0003-source-bound-qualification.md), host adoption은
[ADR 0004](../adr/0004-host-adoption-boundary.md)의 경계를 따른다.
이 문서 작성은 Rust gate·host 실행·CI 성공·성능 측정을 새로 수행했다는 뜻이 아니다.

## 6. QMP에 넣지 않는 책임과 후속 시작 조건

| 요청 | 책임 / 시작 조건 |
| --- | --- |
| Physical allocation·RSS/cgroup·mmap·GPU 메모리의 hard cap | OS/allocator/host. Semantic accounting으로 해당 한도를 주장하지 않음 |
| Query 실행·결과 유효성·cache dependency·tenant authorization | Host. QMP identity/class/context는 권한 또는 알고리즘이 아님 |
| Worker/process/여러 platform의 합산 예산 | Host의 domain composition. Process-local broker가 distributed quota를 제공하지 않음 |
| Session TTL·prepared 자동 history pruning | Host가 retry/visibility horizon을 확정해야 함. 무제한 이력과 자동 만료를 기본 요구로 추가하지 않음 |
| Host-neutral SDK taxonomy·새 public DSL | 실제 소비자 이행이 선택되면 기존 IR/API를 직접 수정. [미구현 RFC](../rfcs/0001-sdk-interface-contract.md)를 현재 유즈케이스와 혼동하지 않음 |
| Disk spill·restart recovery·power-loss 보장 | Named durable consumer·failure model·evidence owner가 생길 때만 별도 범위. [ADR 0002](../adr/0002-store-custody-and-durable-spill.md) |
| 성능 SLO·추가 module 분할·대규모 검증 | 측정 목표·책임 경계의 결함·유효한 회귀가 확인될 때 시작. Generic checklist를 필수 backlog로 바꾸지 않음 |

## 7. 공식 레퍼런스와 QMP에 적용한 판단

확인일: 2026-10-06. 아래는 계약 비교이며 QMP가 해당 라이브러리의 구현·성능·기능을
계승한다는 주장이 아니다. 외부 reference가 local source/ADR의 권위를 대체하지 않는다.

| 공식 자료 | 확인한 패턴 | QMP 적용과 차이 |
| --- | --- | --- |
| [Apache DataFusion MemoryPool](https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/trait.MemoryPool.html) | 보유 데이터의 consumer가 공유 pool에 capacity를 요청하고, 거부 시 회수/오류를 처리. Reservation은 RAII 수명이며 `try_grow` 실패가 예약을 증가시키지 않음 | UC01/UC02/UC06의 shared admission·custody 근거. QMP는 semantic 다축 ledger이고 SQL operator/spill orchestration이 없음. DataFusion의 infallible `grow`를 QMP의 일반 admission 허용으로 옮기지 않음 |
| [Apache Arrow C++ Memory Management](https://arrow.apache.org/docs/cpp/memory.html) | Buffer backing을 실제 MemoryPool로 allocate하며 작은 객체·temporary workspace 등은 다른 allocator를 사용할 수 있음 | Accounting 대상의 경계를 명시할 근거. QMP는 선언된 semantic 비용을 account하며 Arrow-style buffer allocator·물리 allocation 추적을 제공하지 않음 |
| [Rust Reference — Destructors](https://doc.rust-lang.org/reference/destructors.html) | Scope/field의 Drop 순서와 destructor가 실행되지 않는 forget·process termination 경계를 정의 | UC02/UC03의 lease 수명과 공통 cleanup 조건. QMP는 tested unwind/deferred authority를 추가하지만 abort·누락된 Drop에서도 실행되는 cleanup을 보장하지 않음 |

### 7.1 기능 커버리지 재감사

2026-10-06, QMP `b4e37960127b715d02baf2b912fd7aa31e95cc51`의 공개 API·실제 구현·owning tests와
공식 레퍼런스를 세 영역에서 병렬로 대조했다. 아래 테스트 근거는 소스를 읽은 결과이며
이 재감사에서 Rust·Cargo·CI·consumer 실행은 **NOT_RUN**이다. 새 회계 결함은 확인하지 못했다.
기능 공백의 존재와 해당 기능을 지금 추가해야 하는지는 별개로 판정한다.

| 비교 기능 | 현재 QMP | 판정 / 소비자 시작 조건 |
| --- | --- | --- |
| 공유 budget·다축 귀속·RAII | 같은 broker의 reserve/commit/release와 class·owner·context·group projection. Pending 포기와 committed Drop의 authority를 보존 | 현재 커버. 새 consumer마다 별도 pool을 만드는 방식으로 합산 cap을 주장하지 않음 |
| 성장·축소·전체 custody 이전 | `ManagedCharge::recharge`는 committed charge의 net postimage를 원자적으로 교체하며 denial 때 old charge 보존. 전체 이전은 Rust move로 가능 | 기본 resize·전체 handoff는 커버. 새 ticket을 발급하는 swap이므로 in-place incremental reservation과 완전히 같지는 않음. Ticket ID 고갈 때 shrink도 거부 가능하며 explicit release는 별도 경로 |
| 성장 중 pressure 처리 | 신규 reserve는 soft callback·hard callback 한 번·재측정. `recharge`는 scheduler/ledger의 atomic replacement를 직접 호출 | 조건부 기능 공백. Mutable component/retained-map의 성장 caller가 다른 store를 회수해 계속 진행해야 할 때 평가 |
| 기존 reservation의 부분 handoff | 같은 owner에 처음부터 여러 charge를 발급할 수 있으나 기존 하나를 추가 byte admission 없이 원자적으로 나누는 API는 없음 | 조건부 기능 공백. 독립 수명의 subobject를 넘기는 caller 필요. Refund-first는 live data를 undercharge하고 reserve-first는 transient double count로 cap에서 거부될 수 있음 |
| 소비자별 공정 배분 | Policy-static group burst share와 live group usage, owner/class/global cap, peer reserved floor를 검사 | 정책 차이. DataFusion의 등록 수 기반 fair fraction과 동등하지 않음. 실제 operator 수 변화·예산 독점 사례 없이 dynamic registry를 추가하지 않음 |
| 현재 상위 owner·denial 진단 | Public `BrokerObservation`의 owner usage에서 top-K를 off-lock 계산 가능. 내부 `LimitBreach`는 구조화됐지만 public `Capacity(String)`에서 해당 필드가 손실 | Top formatter는 편의 기능. 실패 후 별도 `observe()`는 같은 실패 상태를 보장하지 않으므로 machine-readable denial이 필요한 caller에는 final verdict에 묶인 detail이 개선 후보 |
| 전체 peak·측정 구간 | Scratch scope의 observed peak는 있음. Broker 전체·owner별 high-water mark와 구간 reset은 없음 | 측정 기능 공백. Accepted semantic peak 목표가 있으면 최소 scalar부터 평가. 누락 가능한 advisory event나 polling으로 exact peak를 주장하지 않음 |
| Buffer allocation·반환 데이터 수명 | `ByteSized`는 admission-time semantic 비용. Vec/String 기본값은 length이며 allocator capacity가 아님. Map/prepared/transcript의 caller-owned 반환 clone은 store retained charge 밖 | 명시된 범위 차이. 긴 수명 결과를 account하려면 host가 별도 charge와 실제 backing 수명을 결합. 현재 generic read lease/allocator/zero-copy buffer API는 없음 |

근거: [공개 charge authority](../../crates/quanta-memory-platform-core/src/api/control.rs),
[reserve/recharge 경로](../../crates/quanta-memory-platform-core/src/api/control/admission.rs),
[atomic replacement와 floor tests](../../crates/quanta-memory-platform-core/src/api/control/tests/cases_04.rs),
[group scheduler](../../crates/quanta-memory-platform-core/src/application/scheduler.rs),
[observation IR](../../crates/quanta-memory-platform-core/src/domain/stats.rs),
[내부 admission detail](../../crates/quanta-memory-platform-core/src/domain/admission.rs),
[semantic measurement 계약](../../crates/quanta-memory-platform-facade/src/shared/measure.rs),
[read pin/clone tests](../../crates/quanta-memory-platform-facade/src/transcript_store/service/tests/lifecycle_paging.rs).

#### 레퍼런스에서 가져올 때 수정해야 할 가정

- DataFusion docs.rs `latest`는 확인 시 **55.1.0**을 표시했다.
  [MemoryReservation](https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/struct.MemoryReservation.html)의
  `split`/`new_empty`는 같은 consumer registration을 공유한다. QMP는 `OwnerKey`로 여러 charge를
  이미 묶을 수 있으므로 consumer 등록 API의 부재를 accounting 불가로 판정하지 않는다.
- DataFusion FairSpillPool의 분모는 **살아 있는 등록된 spillable consumer 수**이며, 등록 후
  0-byte 상태도 포함한다. `split`/`new_empty`로 분모가 증가하지 않는다. Growth check는 해당
  reservation의 size에 적용되므로 strict consumer-total equal-share라고 표현하지 않는다.
  Pool은 직접 eviction·disk spill·async wait를 수행하지 않는다.
  [공식 55.1.0 구현](https://docs.rs/datafusion-execution/55.1.0/src/datafusion_execution/memory_pool/pool.rs.html).
- [TrackConsumersPool](https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/struct.TrackConsumersPool.html)은
  등록 consumer별 current/peak와 상위 소비자 오류 메시지를 제공한다. Inner pool과 tracker의 업데이트는
  분리되므로 pool 전체·policy와 같은 순간의 snapshot을 보장하는 비교 근거로 쓰지 않는다.
- [PeakRecordingPool](https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/struct.PeakRecordingPool.html)의
  reset은 현재 reserved bytes를 baseline으로 포함한다. 겹치는 query의 수명을 자동 분리한
  per-query peak가 아니다. QMP의 공유 broker에 global reset을 넣기 전에 측정 구간의 owner를 정해야 한다.
- Arrow 공식 문서는 확인 시 **25.0.1**이다. 실제 backing allocator와 shared Buffer slice 수명은
  QMP의 semantic store accounting과 다른 계층이다. 반환 Arc의 backing 전체를 마지막 참조까지
  account하려면 backing에 charge를 묶고 store의 key/index 비용과 중복 계산을 피하는 host 설계가
  필요하다. 이는 가능한 적용 방식이며 현재 내장 read-lease API가 있다는 뜻이 아니다.

### 7.2 후속 후보와 최소 계약

아래는 현재 release gate가 아니라 소비자 요구가 확인될 때의 우선순위다.

1. **구조화 denial:** 실제 caller가 문자열 파싱 없이 제한 축·limit·postimage·policy revision을
   처리해야 하면 기존 `MemoryError`/admission detail을 확장한다. Final locked verdict에서 값을
   확보하고 generic store capacity 오류에 가짜 axis를 만들지 않는다. Top-K 전체 snapshot·정렬을
   reserve hot path에 추가하지 않으며 owner ID를 metric label로 사용하지 않는다.
2. **전체 semantic peak:** Budget tuning/benchmark가 정확한 high-water mark를 요구하면 lifetime
   broker scalar부터 추가한다. 성공한 accounting operation의 최종 postimage에서 기록하고,
   거부된 요청 자체의 미게시 postimage는 기록하지 않는다. 승인되어 ledger에 게시된 pending
   reserve는 이후 rollback되더라도 도달한 lifetime peak에 포함한다. Callback·동시 연산의
   승인된 변동은 별도로 반영하며 deferred charge도 실제 release 전까지 포함한다.
   Owner별 history·global reset·동시 query별 구간은 별도 요구가 없으면 추가하지 않는다.
3. **성장 reclaim:** Pressure에서 진행해야 하는 실제 mutable caller가 있으면 기존 `recharge`에
   net replacement preflight·lock 밖 callback·재검증·최대 한 번 retry를 결합한다.
   Callback 뒤 broker 잠금을 재획득해 old ticket의 유효성, 현재 policy와 group/scratch/headroom을
   포함한 net replacement postimage를 다시 검사하고 같은 잠금 아래 적용한다.
   Denial 때 old charge를 보존한다.
   Retained-map의 store mutation lane은 callback 중에도 잡혀 있으므로 same-store busy·reentry·
   fence/reconfigure 경합과 다른-store victim 성공을 owning negative/positive tests로 고정해야 한다.
4. **부분 split / 동적 policy / 반환 lease:** Subobject handoff·operator 등록 수 변화·반환 backing
   수명이라는 구체적인 caller가 확인될 때만 해당 기존 authority/관측/port를 확장한다.
   부분 split은 단일 broker 잠금 아래 원 charge와 새 charge의 총 byte·class/owner/context/kind/group
   귀속을 보존한다. 추가 ticket 수·ID 고갈·fence 거부에서는 원 charge authority를 그대로 유지한다.
   Public API/examples/conformance를 함께 바꾸며 별도 V2 IR·호환 shim으로 의미를 병행하지 않는다.

추가 운영 관측 후보는 `BrokerObservation`의 pending/committed ticket 분리, deferred cleanup depth,
fence 상태다. 현재 ticket count는 합계이고 fence status/finish cleanup receipt는 별도 API다.
이 차이도 실제 운영 질의가 확인되어야 기존 observation에 추가하며 새 correctness 결함으로 취급하지 않는다.

### 7.3 추가 유즈케이스 감사와 도입 조건

2026-10-06, QMP `79541311434cce918cdfd5546191670769fb0958`에서 consumer authority,
store lifecycle, policy/운영 경계를 세 영역으로 병렬 감사했다. API·구현·owning test 소스를
읽었으며 Rust·Cargo·CI·consumer 실행은 **NOT_RUN**이다. 아래 8개는 앞 절의 후보 외에
검토할 기능이다. 이번 감사에서 도입을 요구하는 실제 caller나 부하 결과는 확인하지 않았다.
기존 API의 의미가 잘못됐다는 판정이나 새 release gate가 아니다.

#### 먼저 소비자 적용에서 확인할 후보

| 후보 | 현재 대안 / 추가할 조건 | 기존 계약에 넣을 최소 변경 |
| --- | --- | --- |
| 상한 예약 후 실측 commit | `reserve_owned(max)` → `commit()` → `recharge(actual)`로 보수적으로 account할 수 있음. 후속 recharge의 fence·ticket ID 거부 때문에 상한 charge가 계속 남는 것이 실제 문제일 때 | 기존 `PendingReservation`에 `actual <= reserved`인 단일 shrink-and-commit 전이. 별도 `MemoryConsumer` 도입 없이 구현 가능. RFC 0001에 이미 제안됐으며 현재 미구현 |
| Retained-map 조건부 교체·삭제 | Immutable entry는 insert-if-absent, mutable entry는 모든 writer가 공유하는 host mutex로 처리 가능. 독립 writer의 read→unconditional insert/remove가 lost update를 만들고 host 직렬화가 불가능할 때 | 기존 retained-map port에 expected value/version 기반 조건부 mutation. `insert_if_absent`는 생성 경쟁만 해결하며 기존 값의 조건부 갱신은 제공하지 않음 |
| Policy revision CAS | 단일 policy owner 또는 모든 writer를 직렬화한 host read/update로 처리 가능. 복수 관리자가 같은 revision을 읽고 전체 policy를 교체해 먼저 적용된 변경을 덮어쓸 때 | 기존 `try_live_reconfigure`에 expected policy revision 검사와 typed conflict. 기존 observation의 revision을 재사용하며 별도 policy IR를 만들지 않음 |

각 후보의 실패 계약:

1. **실측 commit:** 차액 반환과 pending→committed 전이를 같은 broker 잠금에서 적용한다.
   모든 byte/귀속 축·group floor·scratch/headroom·ticket-count 불변식을 유지한다.
   상한 초과·fence·commit 실패 시 value는 게시하지 않는다. RFC의 consuming API를 적용하면
   pending을 rollback하고 cleanup 실패 시 정확한 authority를 deferred retry에 남긴다.
   0-byte commit도 한 ticket을 유지한다. 향후 broker lifetime peak를 도입한다면 상한 pending
   예약의 실제 도달치를 실측 commit의 차액 반환으로 지우지 않는다.
2. **Map 조건부 mutation:** 같은 store mutation lane에서 기대값 확인·net recharge·게시를
   결합한다. Mismatch와 capacity 거부는 old value·charge·순서를 보존한다. 기대값 비교의
   Eq/Hash/Clone panic·reentry를 기존 hostile-trait 경계로 처리한다. ABA를 구분해야 하면
   단순 value equality 대신 host value의 version 계약이 필요하다. 반환 clone의 비용은 별도다.
3. **Policy CAS:** expected revision 비교를 validation/install과 같은 broker 잠금에 둔다.
   호출 전에 별도 `observe()`로 비교하는 것만으로는 race가 사라지지 않는다. Stale conflict는
   policy·revision·scheduler·ledger·epoch를 유지한다. Caller는 최신 policy를 다시 읽고
   자신의 변경을 재합성하며, 이전 전체 policy를 그대로 retry하지 않는다.

근거: [pending/managed authority](../../crates/quanta-memory-platform-core/src/api/control.rs),
[commit/replacement ledger](../../crates/quanta-memory-platform-core/src/domain/ledger/operations.rs),
[기존 실측 commit 설계](../rfcs/0001-sdk-interface-contract.md),
[map port](../../crates/quanta-memory-platform-facade/src/retained_map/mod.rs),
[map 구현](../../crates/quanta-memory-platform-facade/src/retained_map/service.rs),
[live reconfigure](../../crates/quanta-memory-platform-core/src/api/control/observation.rs).
Version으로 stale update를 거부하는 비교 패턴은
[Kubernetes 공식 API 계약](https://kubernetes.io/docs/reference/using-api/api-concepts/#updates-to-existing-resources)에도
있다. 이는 QMP가 Kubernetes의 분산 consistency나 API 기능을 제공한다는 뜻이 아니다.

#### Store 사용 방식이 확인된 뒤 검토할 후보

| 후보 | 현재 대안 / 추가할 조건 | 필요한 경계 |
| --- | --- | --- |
| Payload reference 조회 | 현재 payload store에는 get/resolve가 없음. 처음부터 retained-map 또는 host-owned payload authority를 선택할 수 있음. Deduplicated canonical payload 자체가 읽기 원본이어야 할 때 | 기존 payload port에 issued-reference checked read. Foreign/stale/absence와 read failure를 구분하고 clone panic·close 경합을 처리. 반환 clone은 자동 charge하지 않으며 현재 pin은 close를 막지 않는다는 경계를 명시 |
| 제한된 map 열거 | 현재 keys/entries checked API는 전체 clone 후 잠금 밖에서 정렬. 알려진 key의 point read나 host-owned index로 대체 가능. 큰 map의 전체 복사가 실제 비용일 때 | 기존 port에 row/semantic-byte bound와 mutation-version cursor. 출력 수만 제한한 HashMap scan은 scan/latency 상한이 아님. Ordered index가 필요하면 그 비용과 변경도 같은 fail-closed postimage에 포함 |
| Transcript 일부 sequence 삭제 | 현재 `retain_sequences*`는 session 전체의 keep set. 전체 selector를 만들고 CAS할 수 있고, 앞쪽 N개 보존/뒤쪽 삭제에는 `truncate_to`를 쓸 수 있음. 일부 page만 읽고 그중 선택 항목을 지워야 하며 전체 selector 구축 비용이 클 때 | 기존 port에 expected version을 받는 선택 삭제. 다른 page의 row는 유지. Duplicate/unknown sequence의 의미, stale/foreign 거부, read pin, release-batch 복구, replay evidence 보존, 실제 변경 시 version 증가를 함께 정의 |

Payload pin은 reference metadata이므로 stronger close-blocking lease와 같지 않다.
단순 RAII unpin helper도 closed/poisoned 상태에서 fallible unpin의 성공을 보장하지 못한다.
새 읽기 API를 설계할 때 pin 이름만으로 수명 보장을 추정하지 않는다.

근거: [payload port](../../crates/quanta-memory-platform-facade/src/payload_store/mod.rs),
[payload pin/close 구현](../../crates/quanta-memory-platform-facade/src/payload_store/service.rs),
[map 전체 열거 구현](../../crates/quanta-memory-platform-facade/src/retained_map/service.rs),
[transcript selector port](../../crates/quanta-memory-platform-facade/src/transcript_store/mod.rs),
[paging/read-pin tests](../../crates/quanta-memory-platform-facade/src/transcript_store/service/tests/lifecycle_paging.rs).
[Rust BTreeMap range](https://doc.rust-lang.org/std/collections/struct.BTreeMap.html#method.range)는
정렬된 key 범위를 순회하는 자료구조 예시다. 현재 QMP map은 HashMap이며 자료구조 교체만으로
snapshot paging·accounting·latency 계약이 완성되지는 않는다.

#### 비용이 큰 authority 확장은 후순위

- **동일 backing의 scratch→retained 전이:** 현재는 retained를 먼저 reserve하고 scratch를
  반환할 수 있다. 실제로 같은 allocation을 인계할 때만 이중 charge 없이 단일 잠금에서
  pending scratch→committed retained 전이를 검토한다. 임시 buffer와 새 retained buffer가
  동시에 존재하면 둘 다 charge해야 한다. Denial은 scratch authority를 보존하고 retained를
  게시하지 않는다. 기존 [scratch port](../../crates/quanta-memory-platform-core/src/ports/scratch.rs)와
  authority를 확장하며 allocator 추정이나 refund-first 우회를 넣지 않는다.
- **여러 component의 원자적 admission:** 현재 host-owned custom charge는 개별 pending으로
  stage하고 모두 commit된 뒤 host value를 게시할 수 있다. 서로 다른 facade store mutation까지
  이 방식으로 원자화할 수는 없다. Staging의 임시 점유가 실제 contention 문제가 될 때만
  기존 broker의 batch reserve/commit을 검토한다.
  Staging은 atomic batch가 아니며 중간 commit 실패 시 이미 committed된 lease를 host가
  회수하거나 cleanup이 가능할 때까지 보존해야 한다. Rebuild 중 Drop은 deferred cleanup에 남을 수 있다.
  Mixed-axis postimage·ticket 수·ID·floor를 같은 잠금에서 검사하고 일부 성공을 노출하지 않는다.
  Ticket의 일괄 성공은 host publication까지 자동 transaction으로 만들지 않는다.
  Batch release는 이미 [공개 `release_managed_many`](../../crates/quanta-memory-platform-core/src/api/control/authority.rs)로
  지원하므로 신규 기능으로 다시 넣지 않는다.

#### 기능을 늘리기 전에 가능한 보완

- 공개 custom charge 사용 예제와 `Weak`로 다른 store를 참조하는 reclaimer 예제를 먼저
  보강할 수 있다. Strong capture는 broker→callback→store→broker cycle을 만들 수 있다.
  Safe late wiring은 이미 [cross-store test](../../crates/quanta-memory-platform-facade/tests/cross_store_hellgate.rs)에
  있으므로 새 reclaim registry를 만들 필요는 없다.
- Shared backing과 charge를 같은 host-owned Arc 안에 묶는 방식은 기존 API로 가능하다.
  Private fields에서 value를 charge보다 먼저 두고 charge의 독립 조기 release를 막는다.
  Mutable backing의 비용 변화는 상한 또는 recharge/replace로 account하며 store의 key/index
  비용과 backing charge를 중복 계산하지 않도록 host가 설계한다.
  Generic charged-value helper 부재를 새 core 기능 공백으로 단정하지 않는다.
- `remove`의 old value 반환, batch release, 0-byte FIFO entry의 metadata 비용은 이미 커버한다.
  Session/root registry·TTL/LRU·async wait·분산 예산·durable recovery는 현재 caller 근거 없이
  이 감사에서 신규 필수 작업으로 추가하지 않는다.

도입 시 owning tests와 public examples/conformance를 함께 변경한다. 이 절의 우선순위는
실제 caller가 기존 대안으로 해결되지 않는 경우에만 적용하며 §7.2와 함께 자동 일괄 구현하지 않는다.

## 8. 문서 소유권

- **이 문서:** 소비자 목표·사용 흐름·QMP/host 책임·현재 지원 범위의 단일 진입점.
- **[README](../../README.md):** 빠른 시작·feature 선택·문서 navigation.
- **[ADR](../adr/README.md):** 권위·accounting·custody·qualification 결정.
- **[Verification scenarios](../verification-scenarios.md):** 세부 테스트 selector와 실행 경로.
- **[RFC 0001](../rfcs/0001-sdk-interface-contract.md):** 미구현 차기 SDK 설계.
- **[Work scope](../plans/README.md):** 후속 시작 조건. 기능 표만 보고 새 필수 작업을 만들지 않는다.

기능/계약을 바꾸면 해당 유즈케이스, owning tests/public examples, ADR 또는 migration을 함께
갱신한다. 새 번호의 IR·중복 use-case 문서·호환 shim을 만들어 의미를 병행하지 않는다.
