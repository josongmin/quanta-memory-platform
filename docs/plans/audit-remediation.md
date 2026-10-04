# QMP 구조적 수정 계획

계획 재감사일: 2026-10-04. 상태: oct-4 전용 worktree에서 구현 진행; 체크포인트마다 통합 source를 고정하고 일괄 검증.

**확정 순서:** 재현된 accounting/identity/evidence/guard 수리 → local source qualification →
선택한 큰 인터페이스 확장과 API 축소의 별도 qualification → 조건부 소비자/운영 후속.
새 IR 세대, token epoch/TTL, RSS/SLO 약속과 전면 파일 재편은 필수 범위에 넣지 않는다.

## 과거 감사 기준과 증거 범위

아래 digest와 관측은 과거 고정본의 기록이다. 현재 candidate의 구현/검증 상태는
다음 절에서 따로 관리한다. 과거 문장의 "대기"나 pass를 현재 source 상태로 읽지 않는다.

- QMP base: `88093064ccaacaba3f00eb6e22e9d0a33d68ac22`, branch `codex/qmp-audit-remediation`.
- 원 감사 digest: `4d697c991061cd7b99380b7b65ac4f730d884326a5a56cef6f11837a04a3526a`.
- 이번 재감사는 당시 dirty overlay를 포함한 별도 고정본에 수행했다.
  Digest: `93fbddc9a49b9485374bbab0e26c847b31ed49013804ebd778e9629a03b5fc98`.
  이후 공유 checkout 변경은 이 결과의 검증 범위 밖이다.
- Core/public API, facade 수명, guard/통합 경계의 세 영역을 병렬 재감사했다.
  Python 계약 26건, 실제 Semgrep 계약 2건, CircleCI config validation은 통과했다.
  그 상태에서도 CI 추가 step/checkout/pin 변형과 Python source 위장/중복 실행이 통과했다.
  전체 Rust gate, standalone extraction, host compile과 새 hosted 실행은 하지 않았다.
- 이후 변경된 Python runner/policy만 새 고정본으로 확인했다.
  Digest: `056e20c42c3e1809fa52cc903d2c8c133f3dfde8fd90fb19b082e10e3620bbf4`.
  두 owning module의 19건과 policy verifier는 통과했고, 별도 imported-class 위장과
  duplicate-case 재현도 거부됐다. 이 결과를 앞 고정본의 pass와 합쳐 전체 green으로 간주하지 않는다.
- 마지막 보완 고정본 digest: `02059822d2558506bc53082ec9e25432840a6cf83ab71879ac16cc5037340e0b`.
  A/B와 F/G에는 새 누락이 없었다. D의 고갈·race·retired replay oracle을 보완했다.
  Python에서 같은 파일의 `load_tests`가 실패 case를 선택에서 빼는 추가 우회를 재현했다.
  정상 수집은 2건 실행 후 실패, 변형은 정상 1건만 실행하고 성공했다. 이 코드 수리는 아직 대기다.
- Semantica 재확인 HEAD: `bfd47b89001f54b0053a32326dd1653de7202455`, dirty 30경로.
  두 manifest는 QMP `14cf0345004d0703c7d32da34055af5367a9212d`를 참조한다.
  이 소비자 조사는 소스 계약 확인이며 QMP/host 조합의 실행 증거가 아니다.

## 구현 실행 경계

- 구현 branch: `codex/oct-4-qmp-contracts`; worktree: `oct-4-qmp-contracts`.
- Base `88093064`의 dirty overlay 50경로를 고정 복사했다. Manifest SHA-256:
  `f47c59eed8dc811981f7b257a83181997dba0edb65e731c1959f151cd00c9a4a`. 원 checkout은 보존한다.
- 사용자가 전체 구현과 큰 체크포인트별 통합·일괄 검증을 요청했다. 필수 수리 candidate를 먼저
  고정하고, 후속 권한/API 변경과 QMP Transcript read/retirement 계약을 별도 통합한다.
- QMP public examples/tests를 실제 caller로 이행한다. Semantica 제품 activation은 선택하지
  않았으므로 host 종료 hook/producer 이행과 QBC qualification은 소비자 adoption 경계로 남는다.
- 최종 canonical/extraction rail은 모든 구현을 통합한 clean candidate에 묶는다.
  중간 checkpoint receipt는 그 commit만 증명하며 이후 변경을 포함하지 않는다.
- 필수 수리 checkpoint `0b2fb29`에서 full canonical `scripts/check-workspace.sh`가 통과했다.
  실행 root는 별도 detached checkout이며 extraction와 hosted receipt는 아직 없다.
  이후 D/API/feature/example 변경은 이 pass에 포함되지 않는다.

## 최종 계획 감사에서 보완한 사항

- **기능 port 누락:** bounded read와 version 조건부 retain을 concrete store에만 두면
  `SessionTranscriptStore`로 사용하는 caller가 계약을 이용할 수 없다. 두 메서드를 기존 trait에
  직접 추가한다. Full snapshot은 convenience이며 bounded 계약의 증거로 쓰지 않는다.
- **부분 page와 전체 retention:** page는 session의 일부다. `retain_sequences`와 CAS는
  session 전체의 exact selector다. Page의 행만 그대로 넘기면 다른 page의 행이 퇴거된다.
  Version 비교는 concurrent mutation만 검출하며 selector 완전성을 보장하지 않는다.
- **수명 conformance의 독립 음성:** append/replay/cost가 정상이고 retire만 성공으로 삼키는
  fake를 suite가 거부해야 한다. 기존 replay-broken fake의 실패만으로 retirement를 증명하지 않는다.
- **삭제 후 통합 경로:** legacy API 제거 시 public exports뿐 아니라 feature rail의 실제 selector,
  source-owner map, guidance와 examples도 이행한다. Zero-test 성공으로 capability를 판정하지 않는다.
- **검증 경계:** local canonical + clean extraction/examples는 QMP source qualification이다.
  Hosted job/binding은 CI 운영 증거로 별도 추적한다. Hosted 미실행을 local 실패나 성공으로 바꾸지 않는다.
- **결정적 token:** prepared token은 같은 identity/key에서 store나 restart 경계를 넘어 같은 값이다.
  임의의 foreign-store 거부나 nonce/epoch를 붙이면 기존 replay 계약을 깨뜨린다.

현재 진행 중인 D 및 API 축소는 사용자가 선택한 별도 확장 candidate다. 아래 우선순위는
원 결함 수리의 필수 조건과 확장 계약을 구분하며, 이미 있는 구현을 다시 만드는 작업 목록이 아니다.

## 원칙과 우선순위

1. 기존 IR·identity·request·policy·stats·snapshot을 직접 수정한다.
   같은 의미의 V2/V3, 구·신 IR 병행, compatibility alias와 변환 shim을 추가하지 않는다.
2. Breaking change 허용은 불필요한 API 교체의 근거가 아니다.
   A의 재현된 accounting 수리, B/C/E와 F1을 먼저 닫고 G로 그 source를 검증한다.
   Scheduler 공개 제거와 D/F2는 별도 candidate와 receipt로 검증한다.
3. Ledger가 accounting의 단일 권위다. Handle/read pin은 별도 ledger를 만들지 않는다.
   Fallible preflight 후 authority를 분리하고 실패 시 content/identity/charge를 보존한다.
4. `core <- facade <- testkit`, builder-first, pure domain/ports를 유지한다. 제품 crate를 추가하지 않는다.
5. API 변경의 owning tests, exports, examples와 migration은 같은 단위로 이행한다.
   소비자 계약 호환성과 제품 adoption/실행 qualification은 구분한다.
6. [RFC 0001](../rfcs/0001-sdk-interface-contract.md)의 전체 taxonomy/config DSL과
   Redb production qualification은 이번 결함 수정의 전제조건이 아니다.

## 실행 순서 — 먼저 닫을 범위

| 단계 | 확정 범위 | 단계 종료 조건 |
| --- | --- | --- |
| 1. 필수 수리 | A accounting·축 검증, B String identity·삭제 순서, C 전체 identity evidence 예약·공개 의미, E 검증 우회, F1 기존 restore 의미 | Owning positive/negative oracle, public API 계약과 source 통합 |
| 1의 검증 | G의 canonical gate·clean standalone extraction/public examples | 같은 clean SHA의 local receipt; hosted는 별도 CI 운영 항목 |
| 2. 선택한 인터페이스 확장 | 기존 Transcript port의 handle·page·CAS·retire, scheduler 내부화, 사용처 확인한 legacy/feature 축소 | 기존 IR 직접 수정, exports·실제 caller/example·독립 음성 oracle, 별도 통합 source 검증 |
| 3. 조건부 후속 | 실제 host adoption, SLO/장기 workload, 추가 책임별 파일 이동, durable 제품화 | 명명된 소비자·측정 또는 회귀 근거와 별도 failure/운영 계약 |

1단계 source를 고정한 뒤 별도 작업 source에서 2단계를 진행할 수 있다. 2단계 dirty 변경을
1단계 receipt에 합치지 않는다. Hosted 실행이 외부 사유로 불가능하면 CI 운영 미완료를 남기되
local source qualification과 분리한다. Hosted 완료나 전체 green을 주장하지 않는다.
이미 공유 checkout에 있는 변경은
소유자를 확인해 통합 source에 포함하거나 owner와 분리하며, 과거 pass로 면제하지 않는다.

## 큰 인터페이스·맥락·IR의 확정 계약

| 경계 / 기존 표현 | 확정할 의미 | 이번 변경 |
| --- | --- | --- |
| Facade/platform builders → core broker | 한 platform의 budgeted store는 같은 ledger authority를 사용. Host 구현과 I/O는 QMP 밖 | 기존 builder-first 경계 유지 |
| `AdmissionRequest` / `PreflightInputs` / `AdmissionVerdict` | Locked ledger·policy와 class/owner/context/kind/bytes로 판정. Attempt와 같은 순수 검사·같은 첫 오류. Preflight는 예약이나 미래 성공 보장이 아님 | 기존 private IR 수정; 공개 universal IR 추가 없음 |
| `MemoryContextHandle` / `MemoryContextKind` | Runtime/owner/context는 accounting attribution. Context는 합계 bucket·overflow guard이며 독립 hard cap이나 session incarnation이 아님 | 의미 유지; context에 retirement/authorization 책임을 붙이지 않음 |
| `PendingReservation` / `ManagedCharge` | Broker에 묶인 non-Clone authority. 실제 ticket의 attribution으로 commit/release/recharge. 실패 시 이전 charge와 상태 보존 | A 축 검증 수리; scheduler는 파생 cache |
| Payload `String` reference | Store-issued process-local lookup identity. Live canonical replay는 동일 reference, 삭제 후 새 게시와 다른 store는 새 identity | B seal·checked nonce/sequence; 영속 ID·권한 토큰으로 사용 금지 |
| `PreparedPublicationIdentityV1` / `RequestV1` / `TokenV1` | Caller가 logical attempt identity와 안정된 canonical key를 제공. 기존 domain·길이 encoding·32-byte deterministic token 유지 | C에서 기존 타입 직접 수정; token 재발급/epoch 없음 |
| Prepared limits / errors / stats | 한도는 live+aborted identity evidence 전체. Fresh prepare만 admission; abort/GC는 이미 예약된 evidence로 전환 | C의 명명·관측값을 같은 의미로 맞춤 |
| Canonical root / GC | Host가 visibility와 완전하고 안정된 보호 token 집합을 소유. QMP는 제공된 집합에 없는 resident entry만 수거 | 기존 host fence 보존; QMP에 root registry/commit state 추가 없음 |
| `RetainedCostV1` / `ByteSized` / stats | Admission 때 checked semantic cost를 charge와 함께 고정. Release는 재측정하지 않음. Lease별 payload/metadata bucket이며 RSS·field별 실측이 아님 | 비용 상수 유지; 변경 필요 시 별도 정책 결정·receipt 재측정 |
| Spill `restore` / feature | Retaining 구현의 `Some`은 복구, `None`은 정상 empty, read 불가 상태는 오류. Discard/no-op의 기본 `None`은 유효. `spill`/`full`은 in-process, `spill-durable`은 opt-in protocol | F1과 선택한 feature 축소를 별도 검증 |
| Transcript `SessionHandle` / append identity | Caller SessionId는 유료 metadata. `open_session`만 생성하며 동명 open도 서로 다른 incarnation. 모든 작업은 발급 handle 사용 | 기존 request의 session 축을 handle로 수정; 문자열→최신 handle 라우팅 없음 |
| Transcript policy / stats / retirement | Store 전체의 evidence count/bytes 및 open session count/key bytes 한도. 0행도 key charge. Retain은 evidence 보존, retire/close는 해당 horizon 종료 | 기존 policy/stats/error 확장; 새 IR/ledger/TTL 없음 |
| Transcript snapshot / cursor / CAS | 기존 snapshot에 handle/version/next cursor. Page row·semantic-byte 한도, cursor는 동일 handle/version. CAS는 전체 session exact selector | 기존 trait에 page/CAS 추가; full snapshot과 caller-owned Clone은 별도 의미 |

Admission 거부는 `ReclaimCandidate`일 때만 hard reclaim 후 한 번 재측정한다.
`TerminalDenial`/`Blocked`에는 reclaim을 호출하지 않는다. Payload pin/unpin/delete의 missing/foreign,
prepared unknown/aborted/conflict, retention saturation, storage closed/busy와 accounting failure를
성공·empty로 변환하지 않는다. 기존 `pin_count(missing) = 0` 조회 의미는 유지한다.
각 거부의 charge/content/identity preimage 보존을 owning test로 확인한다.

### Prepared root 경계에서 빠지면 안 되는 조건

- Prepare는 payload residency와 replay evidence를 만든다. Canonical root 공개/commit은 host가 수행한다.
- GC의 보호 집합에는 공개 root뿐 아니라 아직 terminal ACK되지 않은 outstanding prepare도 포함된다.
  Root 공개·퇴거·outstanding marker 변경과 GC mark snapshot의 순서를 host가 직렬화한다.
- 조사한 Semantica writer에는 `stage_commit_serial_v1`, 공개 token + durable outstanding token의
  합집합, registry 미확정 시 GC 보류가 이미 있다. Composer는 다른 process의 unknown token을
  resident 집합에서 제외하고 known aborted token은 오류로 처리한다. 새 fence 결함으로 단정하지 않는다.
- 이 소스 계약은 host 실행 증거가 아니다. Host adoption 시 해당 순서·restart 재구성을 QBC로 검증한다.

## 현재 구현과 원 감사 항목 대조

이는 움직이는 candidate의 소스 상태다. "구현"은 검증 통과를 뜻하지 않는다.
최종 canonical/extraction receipt는 별도의 clean commit에 고정한다.

| 원 항목 | 현재 고정본 | 남은 작업 |
| --- | --- | --- |
| 0바이트 class 제거 / zero-cap borrowing victim / Headroom wrong release | 패치·owning tests 있음; scheduler 내부화 별도 candidate에 있음 | A 필수 통합 검증 + 내부화의 public/benchmark 검증 |
| Payload reference 재사용 | String seal + checked store nonce/sequence 패치·회귀 있음 | B 공개 거부/수명 계약 검증 |
| Prepared retirement 포화 | Fresh prepare가 전체 evidence 공간 예약 | C 불변식·retirement·공개 evidence 의미·독립 root-limit oracle |
| Raw C / benchmark unsafe / adapter 타입·값·별칭 | 패치·actual-engine fixtures 있음 | E canonical 검증 |
| CI 조기 종료·pin | 동적 추가 step·checkout·pin 우회 재현 | E 허용 실행 경로·exact pin 보강 |
| Python 발견·실행 누락 | Recursive/source-origin/duplicate 검사와 `load_tests` hook 거부 구현 | Inherited contract 유지, actual mutation + canonical 검증 |
| Transcript 수명·snapshot 비용 | 별도 candidate에 handle/page/CAS/retire 구현 및 owning caller 이행 중 | Public port·고갈/race/rollback/독립 retirement oracle 통합 검증 |
| Durable maintenance / legacy API / 책임별 모듈 | Feature 경계와 legacy 제거 별도 candidate에 있음 | 실제 public/feature 경로 검증; 전면 파일 재편은 후속 |
| Spill default restore | Target offline 거부 패치 있음 | F1 retaining read 불가와 정상 empty 구분; no-op 기본값 허용 |
| 정책 산술 중복 | Reserve/preflight는 이미 공통 판정 | 동작 불일치 증거 없음; A dead branch 정리는 후속 |
| CircleCI 실행 완료 | 이전 runner 시작 전 실패 기록 | G exact SHA hosted 증거 별도 확보 |

Metrics recorder·allocation probe 등 함께 존재하는 overlay도 최종 통합 source의 owning rail로
확인한다. 이 계획이 그 변경을 새 결함으로 추가하거나 이미 검증됐다고 주장하지 않는다.

## A. Core accounting 수리, 이후 공개 권한 축소

소유: core `api/control`, `domain/admission.rs`, `domain/ledger/operations.rs`,
`application/{mod.rs,scheduler.rs}`, `lib.rs`, public-surface tests.

- Live-class 제거 거부는 pending/committed/0바이트 ticket 모두에 적용한다.
  클래스가 남는 cap 변경과 마지막 authority 반환 후 제거는 허용한다.
- 양수 borrowing usage가 있는 hard-cap-zero 클래스를 reclaim 후보에 포함한다.
  Callback이 실제 charge를 반환하고 requester가 재측정 후 성공하는 oracle을 유지한다.
- Headroom/non-Headroom release는 해당 축의 실제 usage를 넘으면 변경 없이 거부한다.
  Broker는 ticket의 실제 class/owner/bytes로 usage를 갱신한다.
- Reserve/preflight의 기존 `check_ledger_postimage`를 유지한다.
  Hot-path reserve/release에 snapshot 복제나 ticket 전수 검사를 추가하지 않는다.

완료 oracle: 거부 전후 policy revision/report/ledger/usage/floor/admission 동일,
정상 release·cap 변경·reclaim 성공, 기존 preflight/attempt 일치와 keyed-release/cache 회귀 통과.

후속 API 축소: scheduler root re-export와 `application::scheduler` 모듈 경로를 모두 내부화한다.
현재 public scheduler는 독립 객체이며 외부에서 live broker scheduler/ledger를 수정할 수 있다는
증거는 없다. 내부화는 파생 cache를 별도 public authority로 노출하지 않으려는 구조 정리다.
Facade scaling workload, `verify-scaling-results.py`, public-surface tests와 migration을 함께 이행하고,
외부 compiler fixture가 root/module 양쪽 접근을 거부해야 한다. Broker workload와 이전
standalone scheduler 수치를 같은 성능 계약으로 비교하지 않는다. Reconfigure validator의 미사용
`Some(delta)` 분기 제거도 후속으로 두며, 새 추상 계층은 만들지 않는다.

## B. Payload identity 수리와 fail-closed 순서

소유: facade `payload_store`, owning tests와 실제 public API fixture.

- Production reference를 String으로 seal하고 store nonce/단조 증가 sequence를 검사하는 패치를 유지한다.
  사용자 non-injective `From<String>`는 compiler가 거부한다.
- Same canonical live payload는 같은 reference와 한 charge를 유지한다.
  삭제 후 재게시 reference는 새 값이며 stale/foreign pin/delete가 새 entry를 건드리지 않는다.
  Nonce/sequence 고갈은 charge 이전 typed error이며 기존 pin-count 조회 의미도 유지한다.
- Reference는 process-local lookup identity이며 authorization token이나 영속 ID가 아니다.
  소비자가 이전 `payload-N`을 파싱하거나 restart 경계에서 재사용하지 않는지 확인한다.
- Delete의 byte-counter 감소를 `entry.charge.take()`보다 먼저 계산한다.
  현재 순서는 내부 counter 불일치 시 Err와 함께 entry charge를 분리한다.
  정상 공개 API로 그 불일치에 도달한다는 증거는 없어 방어적 invariant repair로 분류한다.
- Corruption fixture에서 거부 후 entry/reference/content/pin/charge/ledger가 동일한지 검증한다.

Typed `PayloadId`는 원 P2 수리의 필수 조건에서 제외한다. 별도 API 단순화가 필요하면 기존
generic을 직접 제거하고 builder/testkit/benchmark/소비자와 `semantic_cost_v1` 공개 비용 계약을
함께 판단한다. 비용 산식을 조용히 바꾸지 않는다. String 소비자에 불필요한 타입 이행을 요구하지 않는다.

## C. Prepare evidence 예약을 단일 admission 계약으로 마무리

소유: facade `publish_pipeline/prepared_publication`과 builder/tests.

- Count는 `identity_index.len()`, bytes는 모든 `IdentityEvidence.retained_bytes` 합이다.
  QMP에 별도 committed state는 없다. Host root에 게시한 token도 `Prepared`로 한도를 점유한다.
- Fresh prepare가 persistent lease 확보 전에 전체 evidence 한도를 검증하는 패치를 유지한다.
  Abort/GC의 오래된 aborted-only capacity 검사를 제거하거나 예약 불변식 검사로 명확히 바꾼다.
  Retirement에서 새 공간을 admission하지 않는다. 기존 검사만으로 현재 abort 실패를 단정하지 않는다.
- 기존 builder의 `with_tombstone_limits`를 `with_identity_evidence_limits`로 직접 바꾸고,
  기존 error enum의 `TombstoneCapacity`를 `IdentityEvidenceCapacity`로 바꾼다.
  기본 count/bytes 값은 유지한다. Stats에는 `identity_evidence_count`/`identity_evidence_bytes`를
  추가하고 기존 `prepared_count`/`tombstone_count`/`tombstone_bytes`는 각 live/aborted 관측값으로 유지한다.
  새 V2 stats, compatibility alias와 중복 capacity authority를 만들지 않는다.
  Builder 호출·error match·stats struct literal·exports·examples·migration을 같은 단위로 수정한다.
  새 이름의 public caller를 실행하고, 제거한 builder method와 error variant는 각각 독립된
  external compile-fail fixture로 확인한다. 하나의 선행 compiler error로 다른 제거를 가리지 않는다.
- `persistent_bytes = Σ evidence.retained_bytes`,
  `charged_bytes = persistent_bytes + Σ active_bytes`를 각 전환에서 보존한다.
  `identity_evidence_count = prepared_count + tombstone_count`를 확인한다.
  단일 abort/batch GC의 broker release 실패는 정확한 preimage를 복원한다.
- Deterministic 32바이트 token과 derive 입력/encoding을 유지한다.
  Semantica restart 후 동일 manifest로 같은 token을 복원하는 계약을 바꾸지 않는다.
- Root-limit 테스트는 evidence 한도가 먼저 막지 않도록 별도 높은 evidence 설정을 쓴다.
  Live entry 한도 oracle은 unbudgeted fixture로 broker의 live-ticket/byte 한도를 분리한다.
  Fresh identity 한도, live root 수 한도, raw root slice 한도를 독립적으로 증명한다.

완료 oracle: 마지막 슬롯까지 준비한 항목 모두 abort/GC 가능, 다음 fresh identity charge 전 거부,
포화 중 replay/conflict 정상, close에서 active/persistent lease 반환. 장기 회전은 old root/replay
수명이 종료됐다는 host 계약이 필요하다. 자동 TTL·epoch·token salt와 무제한 이력은 추가하지 않는다.

| 전환 | Active payload lease | Persistent identity lease | 결과 |
| --- | --- | --- | --- |
| Fresh prepare | 신규 charge | 신규 charge·retirement 공간 예약 | `Applied` |
| Live exact replay / conflict | 변경 없음 | 변경 없음 | `Replayed` / `IdentityConflict` |
| Abort / orphan GC | 반환 | 보존, aborted로 전환 | 새 capacity admission 없음 |
| Aborted identity의 exact prepare retry | 변경 없음 | 보존 | `AbortedToken`; 다른 canonical key는 `IdentityConflict` |
| Aborted token의 abort 재호출 | 변경 없음 | 보존 | `Replayed` |
| Close | 전부 반환 | 전부 반환 | Terminal store; 새 작업 거부 |

## D. 선택한 Transcript 인터페이스 확장 — 별도 candidate

기존 close-only evidence horizon과 포화 시 fresh append 거부는 명시된 fail-closed 계약이다.
Page/retire를 원 데이터 손실 결함으로 분류하지 않는다. 사용자가 전체 구현을 선택했으므로
QMP public example/test caller를 handle 기반으로 이행한다. 실제 host activation은 후속이다.

### D1. Bounded read / contention 개선

실행 범위: QMP port와 public caller의 bounded-result 계약. D2와 같이 적용하는 이번 candidate는
처음부터 handle을 사용한다. 별도 성능 SLA나 host full-Vec load의 개선을 주장하지 않는다.

- 기존 `TranscriptSnapshotV1`과 read 계약에 version/last sequence cursor, row/semantic-byte
  limits를 직접 반영한다. Session 축은 같은 store-issued `SessionHandle`을 사용한다.
  Limit 0, 한 행의 byte limit 초과, mutation 후 stale cursor 결과를 명시한다.
  Version increment 고갈은 원상보존하며 거부하고, `u64::MAX` sequence 뒤 cursor는 wrap 없이 종료한다.
- `read_page`와 `retain_sequences_if_version`을 기존 `SessionTranscriptStore` trait에 직접 둔다.
  Page는 부분집합이고 retention selector는 전체 session 대상이다. Cursor의 version은 concurrent
  mutation guard이며 selector의 완전성을 증명하지 않는다. Page를 전부 수집하려면 같은 version의
  cursor를 끝까지 따라야 하고, 중간 mutation은 재시작해야 한다.
- Read는 lock 안에서 내부 immutable Arc만 잡고 mutex와 global mutation permit 없이 Clone한다.
  Arc는 public 결과에 노출하지 않고 메서드 종료 전에 모두 반환한다.
  Returned Clone은 caller-owned 결과이며 그 비용을 store retained charge에 포함했다고 주장하지 않는다.
- 읽는 대상 row의 eviction과 전체 close는 release 전 typed busy로 거부한다.
  ManagedCharge는 store entry가 계속 소유한다. Deferred-release queue는 만들지 않는다.
  Close가 CLOSING을 먼저 획득하면 새 read는 busy/closed로 거부되고 내부 Arc도 획득하지 않는다.
- Arc의 custom Drop 또는 필드별 retirement로 기존 generic 필드별 panic containment를 유지한다.
  Partial Clone panic, 복수 panicking Drop, 더 강한 `Sync` auto-trait 요구 여부를 public fixture로 확인한다.
- Raw retention selector 길이는 proportional set 할당 전에 제한한다. 중복도 raw length에 센다.
- QMP page 한도와 full-Vec host load를 구분한다. 제품 전체 bounded read를 요구할 때만
  SDK port/caller도 같은 변경에서 이행한다. Pinned system prompt와 CAS 상한을 유지한다.
- Append callback을 lock 밖으로 옮기는 것은 D1에 자동 포함하지 않는다. 별도로 시작하면
  injective identity, Hash/Eq 또는 검증된 canonical bytes, collision equality를 먼저 확정한다.
  Generic CanonicalKey digest만으로 identity equality를 대체하지 않는다.

완료 oracle: busy/accounting failure 원상보존, read 종료 후 eviction/close, read/close 선점 양방향
barrier, partial Clone/Drop panic containment, read 종료 후 close 재시도의 ledger zero.
Slow Clone 중 정상 String 기반 다른 session append가 성공하는 좁은 contention oracle을 쓴다.
임의 callback latency/RSS SLA를 주장하지 않는다.

### D2. Session evidence 수명 / retirement

실행 범위: QMP public example/test에서 handle을 전파하고 open→append/page→retire→reopen을
실행한다. 현재 Semantica 제품 port에는 end가 없으므로 그 producer/end 이행은 후속 adoption이다.

- `open_session`은 store-issued nonreused `SessionHandle`을 반환한다. 기존 append identity의
  session 축을 직접 바꾸고 Caller SessionId는 metadata로만 보관한다.
  Store instance/session sequence는 checked nonwrapping 발급이며 고갈 시 charge 전에 typed error로 거부한다.
  Handle은 발급 process/store 수명에 한정하고 영속 ID나 authorization token으로 취급하지 않는다.
- 동명 Caller SessionId의 동시 open을 허용하며 별개의 row/evidence/version을 가진다.
  하나를 retire해도 다른 동명 handle은 그대로다. Retire 재호출은 `InvalidState`이며 store close만
  terminal idempotent다. Retired handle의 exact retry도 성공으로 변환하지 않는다.
- 0행 session도 key/handle/index를 점유한다. 기존 policy/stats/error를 직접 확장해
  open session count와 key bytes 한도를 budgeted/unbudgeted 양쪽에 적용한다.
- Producer request까지 handle 또는 opaque wrapper를 전파한다.
  같은 문자열을 최신 handle로 자동 라우팅하지 않는다.
- Host mapping을 운용하는 경우 failed retire는 mapping을 유지하고 successful retire 후 제거한다.
  재open 후 지연된 옛 append는 새 incarnation으로 들어갈 수 없어야 한다.
  Handle 전파 대신 drain/reopen fence를 쓰려면 실제 producer 종료 증거가 필요하다.
- Append/load/retain/page/retire는 같은 handle을 받는다. Foreign/retired handle은 session을 만들지 않는다.
  Retire는 key/row/evidence charge를 원자적으로 반환한다.
- 기존 snapshot/cursor의 session 축도 같은 handle로 변경한다. D1이 적용됐다면
  해당 session의 active read가 끝날 때까지 retire는 release 전에 busy로 거부한다.
- Version은 fresh append와 실제 active row eviction에서만 checked 증가한다. Exact replay,
  no-op retain, reconfigure 및 거부에는 불변이다. Stale CAS는 `StaleSnapshot` disposition,
  stale/foreign page cursor와 retired/foreign handle은 `InvalidState`, pin 충돌은
  `OperationInProgress`다. 반환된 Clone 비용은 caller 책임이며 RSS 상한을 의미하지 않는다.

완료 oracle: 0행 admission 한도, retire 후 component 반환, stale/foreign/late handle 거부,
accounting 실패의 원상보존, exact replay/conflict 유지.
Replay/conflict는 열린 handle에서만 유지하고 retired handle의 exact replay도 거부한다.
0행 open/retire, failed retire, full close에서 session count/key bytes/total retained stats와 ledger를 대조한다.
Conformance는 retire만 삼키는 otherwise-honest fake를 독립적으로 거부한다.

## E. 남은 guard 우회 폐쇄

소유: `scripts/lib`, owning Python verifiers/tests, Semgrep rules/fixtures, CI config/binding checker.

- CI 정상/heavy job의 허용 step 순서와 run entry point를 정확히 대조한다.
  추가 run step, 조건부 경로, 동적 early halt, checkout provider/ref override를 거부한다.
  승인된 shell 자체는 owning test로 검증하며 일반 shell 의미 증명을 약속하지 않는다.
  Checkout step의 구조는 CI 계약 검사가, 등록 provider/repo/ref binding은 live checker가 소유한다.
- Rust pin은 `rust-toolchain.toml`, executor/tool/cache pin은 현재 owning config의 승인 값과
  exact assertion으로 대조한다. 날짜 형식만 확인하지 않는다.
  Project/repo/config binding도 실제 등록값과 대조한다. 새 pin/binding manifest는 필수 조건에서 뺀다.
- Python recursive inventory와 module `__file__`/실제 test method source path를 대조한다.
  `__module__` 위장, imported TestCase, 동일 basename, 비패키지 폴더, 빈 모듈,
  import 실패, duplicate test ID/execution, skip/expected failure를 거부한다.
  실제 class 선언 origin을 확인하는 후속 패치와 정상 local subclass의 inherited contract는 유지한다.
  같은 파일의 선언된 계약 case가 선택에서 누락되는 것도 거부한다. 부분 선택 `load_tests` fixture에서
  실패 case가 실제 실행되거나 수집 단계에서 거부돼야 한다. 선택 수와 실행 수가 같다는 것만으로 충분하지 않다.
  현재 계약 module에 custom `load_tests`는 없으므로 runner가 수집을 소유하고 module hook을 거부하는
  최소 수리가 가능하다. Hook을 허용한다면 선언/허용 case 집합과 selected/executed 집합을 대조해야 한다.
- 이미 추가된 plain C/raw C/byte/raw string, literal spoof와 executable source-load fixtures를 유지한다.
- Unsafe exception은 delegated GlobalAlloc impl에만 둔다. Pure-layer adapter import/call/type/value/alias를
  기존 token/Semgrep 엔진에서 검사한다. Strings/comments와 정상 ports 경로는 허용한다.

완료 oracle: 실제 compiler와 owning engine의 bad/allowed fixtures가 의도한 결과를 낸다.
새 AST crate·일반 Rust 이름 해석기·shell interpreter를 추가하지 않는다.
Config validation과 fixture pass는 hosted 실제 실행 증거와 분리한다.

## F. 기존 restore 계약 수리, 이후 필요한 API 정리

### F1. 1단계 기존 계약 검증

- Target offline 거부 패치를 통합한다. Retaining Target/Controller는 restore를 구현하고
  read 불가와 정상 empty를 구분해야 한다. `is_available`는 write admission이므로
  write-full/readable 상태를 offline으로 오인하지 않는 fixture도 둔다.
- Discard/write-only/no-op controller의 default `Ok(None)`은 문서상 유효하다.
  이 기본값 전체를 강제 오류로 바꾸지 않는다. Retaining fake가 offline을 `None`으로 숨기면
  conformance가 거부해야 한다. 내장 backing data-loss 결함으로 확대 해석하지 않는다.
- Read/write capability의 새 분리는 F1의 선행조건이 아니다.

### F2. 선택한 API 축소와 조건부 후속 정리

이번 candidate는 이미 사용처를 확인한 legacy 제거와 in-process/durable feature 분리를 포함한다.
이는 1단계 결함 수리의 선행조건이 아니다. 추가 module 이동은 행동 owner를 개선할 때만 수행한다.

- Maintenance 경계 분리 시 `spill`은 in-process, opt-in `spill-durable`은 durable protocol/restore,
  `spill-redb`는 후자를 요구한다. 새 storage crate 없이 기존 타입의 module/cfg를 변경한다.
- Read/write capability 분리가 필요하면 기존 계약을 직접 수정하고 필수 restore 구현을 요구한다.
  Noop/discard를 복구 가능한 구현으로 취급하지 않는다.
- Feature 분리 작업의 oracle은 `full` durable API 비노출/Redb 부재, in-process transfer,
  fake durable, Redb와 full-backings/all-features maintenance rail로 나눈다.
  기존 `full`의 durable protocol 노출을 줄이는 API 정리이며 Redb 의존 결함과 구분한다.
- Legacy PublishPipeline/CommitTransition/SimplePublishPipeline와 builder/export는 실제 사용 재확인 후
  제거한다. Legacy-only StagedHandle/PublishError도 확인한다.
  필요한 non-retaining transition을 retaining service로 기계적으로 대체하지 않는다.
- Prepared identity/admission/retirement/GC 이동은 C 이후 같은 owner가 수행한다.
  수정하는 행동의 테스트만 owner로 옮긴다. 번호 기반 파일 전체 재편은 필수 gate가 아니다.
  Selector/source-owner map/examples를 함께 갱신한다.

## G. QMP 완료와 별도 소비자 adoption

1. 안정된 구현에서 focused owner/public API와 actual-engine bad/allowed fixtures를 실행한다.
   발견·선택·실행 count를 확인하며 zero-test success를 증거로 쓰지 않는다.
2. `python3 tools/pm/pm.py sync` 후 canonical `scripts/check-workspace.sh`를 실행한다.
3. Clean committed candidate에서 extraction dry-run 후 `scripts/verify-extracted-examples.sh`로
   실제 extraction/full gate/공개 examples를 검증한다. 새 Transcript example도 명시된 feature로
   실행한다. Extractor가 실행한 rail을 중복 실행하지 않는다.
4. Hosted binding과 job success를 따로 확인한다. 같은 최종 SHA의 extraction/example 실행을 확인한다.
   이전 pipeline #58/job #85는 credits 부족으로 runner 시작 전에 실패했다.
   이 기록을 현재 상태로 간주하지 않고 실제 실행 가능 여부부터 다시 조회한다.
   Hosted 운영 증거는 local source qualification과 분리한다. 차단 시 원인·SHA·NOT_RUN을 남긴다.
5. Consumer adoption은 별도 작업이다. 채택하는 Semantica 두 manifest/lockfile/metadata를
   한 QMP revision으로 맞추고 QBC로 실제 경로를 검증한다.
   API 계약 이행과 host activation을 QMP library green에 섞지 않는다.

## 병렬 실행과 충돌 경계

| Stage | Lane 1 | Lane 2 | Lane 3 | Integrator |
| --- | --- | --- | --- | --- |
| 1a 필수 수리 | A core accounting | B payload; 이후 C prepared evidence | E CI/Python 잔여 우회와 기존 fixtures | F1, source/dirty 소유권, builder/exports/migration 통합 |
| 1b qualification | 각 owner의 구체 실패 수정 | Public API / negative oracle | Extraction/CI 실패 진단 | G exact-source canonical/extraction/hosted |
| 2 후속 축소 | Scheduler 내부화·benchmark 이행 | 필요한 F2 module/legacy 정리 | 필요한 feature 경계 정리 | 공통 surface 통합·새 source 검증 |
| 3 선택한 계약 확장 | D1/D2 owning source·tests | QMP conformance·broken fakes | Feature/public examples | 공통 surface 통합·별도 candidate qualification |
| 4 조건부 후속 | 관측된 성능 병목 | 실제 producer/end 이행 | 선택한 consumer adoption | 해당 consumer/운영 계약과 별도 증거 |

- Shared builder/lib/core exports, 공통 conformance dispatch, selector/feature scripts와 migration/RFC는
  integrator만 쓴다. Lane은 필요한 hunk/signature를 전달한다.
- Scaling `mod.rs`는 scheduler 후속 workload와 E unsafe guard가 공유하므로 integrator가 수정한다.
  C와 F2가 prepared 파일을 동시에 이동하지 않는다. Module 정리는 C 검증 뒤 같은 owner 작업이다.
- QMP caller 이행과 host producer/end 이행을 구분한다. 후자를 local QMP 완료의 전제조건으로
  만들지 않는다. API 변경 시 실제 consumer 이행 범위를 먼저 고정한다.
- Canonical rail은 통합 source가 안정된 뒤 실행한다. 실패나 후속 변경이 있을 때만 affected rail을 재실행한다.
  사전 extraction/CI 실패 진단은 병렬로 진행할 수 있지만 최종 extraction/hosted 검증은 모든 owner 변경을 고정한 뒤 실행한다.
- 결과는 commit/digest, feature/config, execution root, command와 실행 count에 묶는다.
  다른 lane/SHA/dirty source의 pass를 합쳐 final green으로 보고하지 않는다.

## 제외한 과범위 설계

- Typed PayloadId를 원 stale-reference 결함의 필수 수리로 요구하는 것.
- 새 universal IR, prepared epoch/token salt, tombstone TTL과 무제한 이력.
- Scheduler 공개 제거·dead branch 정리·durable feature 분리를 재현된 accounting 수리의 선행조건으로 두는 것.
- No-op/write-only restore의 정상 `None`을 일괄 오류로 바꾸는 것.
- Host string→최신 handle 라우팅으로 late-operation ABA가 해결됐다고 주장하는 것.
- Charge 반환 후 내부 Arc를 읽게 하는 snapshot, deferred authority queue.
- 새 CI manifest/AST crate, line-count-only crate split과 테스트 전체 재편을 필수 gate로 만드는 것.
- 전체 SDK taxonomy, host adoption과 Redb production qualification을 QMP 완료 조건에 포함하는 것.

관련 권위: [현재 SDK 범위](../../README.md), [source-bound qualification](../adr/0003-source-bound-qualification.md),
[host adoption 경계](../adr/0004-host-adoption-boundary.md), [regression oracle](../adr/0005-regression-oracle-acceptance.md),
[migration](../../MIGRATION.md).
