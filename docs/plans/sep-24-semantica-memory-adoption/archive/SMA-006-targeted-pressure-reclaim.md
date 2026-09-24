# SMA-006 — 필요한 owner에 한정한 공통 pressure/reclaim

ARCHIVED DESIGN — 실행 티켓 아님. 아래 요구는 향후 해당 범위를 채택할 때만 참고한다.

Priority: CONDITIONAL. Status: CONDITIONAL.
Owner: Semantica host reclaimer + QMP admission API.
Admission gate: 003/005/007에서 실제 회수 가능한 owner와 제품 pressure 수요가 확인됨.
필요가 없으면 reason/artifact를 남겨 DEFERRED. Blocks: 채택 시 009.
Required contract: [C의 lock/destruction 경계](../INTERFACE-SSOT.md).

## 변경 후보

- Semantica `C/quanta-memory-host/src/runtime/`와 해당 canonical memo/IDE owner
- QMP `crates/quanta-memory-platform-core/src/ports/reclaim.rs`
- QMP `crates/quanta-memory-platform-core/src/api/control.rs`
- QMP `crates/quanta-memory-platform-core/src/domain/admission.rs`
- facade public exports/examples, host 구현 caller 및 관련 conformance.

## 작업

1. owner가 실제로 해제할 수 있는 data를 선택한다. source authority, live pin,
   prepared publication, 결과를 쓰는 caller의 allocation은 임의 eviction하지 않는다.
2. cap axis를 해결할 수 있는 victim만 대상으로 bounded work/item/byte/time 정책을 둔다.
   owner/group/class/global breach의 eligible set이 다를 수 있다.
3. 내부 LimitBreach의 axis/limit/postimage/deficit을 typed request로 전달할 필요가 확인되면
   QMP public port를 확장한다. string error parsing이나 host 독자 admission 원장을 만들지 않는다.
   soft threshold와 hard deficit은 구분하며 live policy 변경 후 broker의 재측정이 최종 판정이다.
4. canonical owner의 API를 통해 index와 값·charge를 함께 제거한다.
   callback은 broker 및 cache lock 밖에서 호출한다. broker의 existing single-flight와
   at-most-one attempt 의미를 보존한다.
5. callback이 보고한 reclaimed_bytes를 원장 사실로 믿지 않는다.
   bytes가 실제 반환되지 않은 pinned/borrowed eviction은 admission 성공 근거가 아니다.
6. 0.3 전체 API 전환과 별도 작업을 섞지 않는다. 현재 ManagedCharge로 불가능한
   measured commit/resize가 003에서 증명됐을 때만 좁은 primitive를 별도로 설계한다.
7. platform→callback→cache→charge→platform의 강한 Arc cycle을 피한다.
   필요한 cache callback은 Weak로 연결하고 registry에서 후보를 snapshot한 뒤 lock 밖 호출한다.
   제거된 값의 Drop/refund도 lock 밖이다. 별도 reclaimer usage 원장/회수 queue를 만들지 않는다.

## 테스트/검증

QMP owner-local: global/class/owner/group 각각의 breach target/deficit, 불가능한 요청의
no-reclaim, 거짓 reclaim result, zero progress, partial progress+error, concurrent
reconfigure, bounded reentry/panic/slow callback. 기존 tests 중 해당되는 것을 재사용한다.
Host owner-local: pinned victim skip, eligible peer eviction, canonical index consistency,
계속 사용 중인 Arc의 charge 보존. 제품 integration은 서로 다른 owner 두 개만 묶어
release→peer admission을 검증하고 전체 matrix를 중복하지 않는다.
외부 handle 종료 후 weak cache/platform이 사라지는지 검사한다. 의도된 deferred authority는
drain 전후를 분리하며 이벤트 sink 재진입/Drop도 bounded하게 종료해야 한다.

QMP focused control test 후 canonical rail, Semantica host/owner QBC, public example compile.
완료: 실제 named consumer와 양쪽 source에 묶인 receipt. API만 추가하면 완료가 아니다.
