# SMA-003 — Canonical memo owner에 charge 수명 결합

ARCHIVED DESIGN — 실행 티켓 아님. 아래 요구는 향후 해당 범위를 채택할 때만 참고한다.

Priority: CONDITIONAL product adoption. Status: CONDITIONAL — 활성 실행 목록 제외.
Admission gate: canonical memo byte accounting을 제품 범위로 명시적으로 채택한 경우만 시작.
Owner: Semantica in-memory memo + quanta-memory-host.
Depends on: 001 domain 계약 (owner test 설계는 병렬 가능). Blocks: 006, 009.
Evidence: [E05/E08](../AUDIT.md).
Required contract: [C/D 전체](../INTERFACE-SSOT.md). 추가 근거: [F04/F05/F06](../FINAL-AUDIT.md).

## 변경 파일

- `C/quanta-adapters-in-memory-shared/src/in_memory/core/memo_table.rs`
- `C/quanta-adapters-in-memory-shared/src/in_memory/core/memo_table/gc.rs` 및 기존 owner tests
- `C/quanta-contract-types-core/src/values/memo_entry.rs` (필요한 size/access 계약만)
- `C/quanta-contract-ports/src/memo_table.rs` 및 최소 retention-admission port (신규 경로는 owner가 확정)
- `MemoInsertReceiptV1` producer/decorator/mock/consumer: receipt custody 변경 시 같은 cutover에 포함
- `C/quanta-memory-host/src/runtime/{memo_residency.rs,ports_impl.rs}`, `src/ports.rs`
- composition wiring은 A lane 이후 integrator가 `quanta-runtime-memory-compose/src/lib.rs`에 적용.

## 구현 순서

1. entry, key clones, shard/global locators, snapshot index, publication bookkeeping의
   **계상 단위와 수명** 표를 만든다. shared Arc는 allocation owner에서 한 번만 계상한다.
   의미상 size와 allocator bucket/RSS 추정은 다른 지표로 둔다.
   MemoEntry는 CAS reference/metadata다. CAS output payload는 artifact owner가 계상하므로
   다시 더하지 않는다. checked formula version/cost를 admission 때 저장해 같은 양을 반환한다.
2. 반환된 값/Arc/clone이 memo에서 제거된 뒤에도 살아 있는지 확인한다.
   그렇게 살아 있다면 cache-residency cap과 total retained cap을 구분하고, total retained
   claim에는 마지막 실제 owner까지 charge를 보존한다. 단순 eviction=메모리 해제 가정 금지.
3. contract-ports의 최소 object-safe retention seam 뒤에 기존 QMP managed charge를 둔다.
   composition에서 domain/owner/class/kind를 바인딩하고 호출은 checked bytes만 받는다.
   반환은 opaque non-Clone lease/typed failure다. 실행 materialization port는 lifetime이 달라
   대체재가 아니다. canonical adapter에 QMP import, host→adapter 역방향 edge를 추가하지 않는다.
   serializable MemoEntry DTO에 lease를 넣거나 generic Retained<T> 프레임워크를 만들지 않는다.
4. reserve와 잠재적 reclaimer callback은 memo lock 밖에서 실행한다.
   lock을 얻은 뒤 generation/fence를 재확인하고 race면 예약을 반환하거나 재시도한다.
5. prepare candidate→reserve/commit→row+index publish의 실패 postimage를 정의한다.
   구성 중 메모리의 upper bound를 보장할 수 없다면 그 값은 retained-only 계약임을 밝히고
   scratch claim을 만들지 않는다. 정당한 measured-commit primitive 필요가 나오면
   006에 별도 API dependency로 기록하고 독자적인 우회 원장을 만들지 않는다.
6. 교체는 old/new가 겹쳐 살아 있는 동안 두 비용을 정직하게 유지한다.
   cap 초과 시 기존 entry를 보존한다. 제거/GC 후 실제 payload를 놓은 다음 refund한다.
   공유 locator node의 마지막 참조 제거와 실제 byte 차이를 반영한다.
   기존 cloneable receipt가 previous row를 보관한다. receipt clone은 immutable rollback
   custody/charge를 공유하도록 바꾸고 필요한 rollback capacity를 publish 전에 확보한다.
   retract가 exhausted budget에서도 기존 custody를 복원할 수 있어야 한다.
   publication reservation의 table/generation/cancellation 권한은 memory reservation과 분리한다.
   release/Drop도 외부 진입이다: lock 아래에서 garbage를 분리한 뒤 guard 밖에서 파괴/환불한다.
7. 안 쓰는 FIFO byte mirror adapter는 caller census 후 제거/대체한다.
   독립 serialized copy를 QMP에 넣어서 canonical memo가 budgeted라고 주장하지 않는다.
8. cache retention denial이 query 전체 실패인지 non-retained recompute인지 기존 Memo port
   계약에 맞춰 결정한다. mandatory evidence/publication 실패는 cache miss로 변환하지 않는다.

## Owner-local 테스트

- insert, same-key replace, 동일 digest의 다른 snapshot/query, index cleanup의 exact cost.
- reserve admission failure: old row·모든 locator·원장 불변.
- late generation drift: old row·모든 locator 불변. pending abort는 rollback;
  commit 후 abort가 rebuild fence와 경합하면 charge authority가 bounded deferred queue에
  남을 수 있다. 해당 보수적 overcharge를 snapshot에 드러내고 fence 종료/drain 후 baseline을
  검증한다. 실패 반환 즉시 항상 total=baseline이라고 잘못 기대하지 않는다.
- prepared invalidation 중 publish/GC 거절, commit/abort 후 올바른 visible generation.
- 동시 publish와 invalidate/GC를 barrier로 정렬; ticket duplication/loss 없음.
- borrowed/shared value가 남은 eviction, 최종 owner Drop, 0-byte authority, deferred refund.
- receipt clone/drop/retract, old→new→successor, foreign/stale/repeated receipt,
  receipt 보유 중 GC, cancelled publication; 이전 행 조기 환불/후속 행 삭제 없음.
- full-cap 상태 rollback에 새 admission이 필요하지 않음; receipt/data 최종 해제 시 exact refund.
- release event sink가 memo로 재진입하는 결정적 테스트: 성공/error/unwind 모두 lock 밖 해제.
- 반환 deep DTO clone이 total-retained 보장 밖이면 명시하고, 그 보장을 주장할 때는 실제
  shared charged custody/별도 caller authority를 증명한다. 무계상 복제로 우회하지 않는다.
- break sizing/index/charge 연결을 각각 했을 때 해당 테스트가 실패하는 독립 oracle.
  테스트 기대 비용에 production size helper를 호출하지 않는다.

## 검증/완료

QBC package: `quanta-adapters-in-memory-shared`, `quanta-memory-host`;
composer wiring은 `quanta-runtime-memory-compose`.
기존 feature/Cargo topology를 유지하고 필요한 새 edge는 architecture 검토 후 선언한다.
`canonical-cbor` 등 기존 memo test gate를 켜고 nonzero 실행 수를 기록한다.
contract/receipt 변경은 integrator가 한 번에 반영하며 별도 v2 publication path를 병존시키지 않는다.

완료: 제품 memo publish가 실제 broker usage를 변경하고 full lifecycle에서 기대값으로
돌아오는 owner receipt; final product query proof는 009.
Reopen: 실제 data lifetime과 charge lifetime 불일치, lock 아래 외부 callback 필요,
admission 실패가 기존 query 결과 의미를 바꾸는 경우.
