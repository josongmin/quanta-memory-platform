# SMA-003 — Canonical memo 행의 설치 broker 과금

Priority: P1 product correctness. Status: MAIN_INTEGRATED_OWNER_PROOF_PENDING.
Owner: memo adapter + host-retention owner; ports/receipt, Cargo.lock, composer,
SDK ingress, runtime error 경계는 serial integrator가 최종 반영한다.
Depends on: SMA-001의 platform-domain 계약. Blocks: SMA-009 product gate.
Historical design: [archive](archive/SMA-003-canonical-memo-accounting.md).

## 현재 소스와 계약

- Main 통합: Semantica `31a07f43e1f09ab795d7e8832eb82273e334cd98`.
  원본 `80822f963760cfd33628c1085cc0ec2ceea2411f` + dirty diff는 보존했다.
  QMP `a1179a10c5569619210cb1c03ceeae7430109c25` 자체 코드는 변경하지 않았다.
- `quanta-contract-ports/src/memo_retention.rs`: host-neutral checked semantic cost와
  move-only lease. `MemoEntry`의 artifact payload는 artifact store가 별도 계상한다.
- `quanta-adapters-in-memory-shared/src/in_memory/core/memo_table.rs` 및 `gc.rs`:
  admission을 memo lock 밖에서 수행하고 row/receipt/prepared authority가 lease를
  보존한다. 제거된 row의 최종 lease는 잠금 해제 후 반환한다.
- `quanta-memory-host/src/runtime/memo_retention.rs`: 설치된 QMP broker의
  `MemoResidency` / `RetainedMetadata` charge에 lease를 결합한다.
- `quanta-runtime-memory-compose/src/lib.rs`와
  `quanta-sdk-runtime-composition-owner/src/bootstrap_inputs_v1/runtime_executor_ingress_packet.rs`:
  한 platform broker를 artifact/evidence/memo에 공유하고, memo account가 없으면
  SDK ingress bootstrap을 실패시킨다. Runtime executor는 capacity를 typed
  admission failure로 보존한다.

이는 *canonical table 소유 행과 index의 의미론적 보유 비용* 계약이다. allocator
capacity, peak RSS, caller가 `lookup()`으로 받은 deep clone 전체의 물리 메모리
상한으로 표시하지 않는다. 동일 key 교체 중 old/new 및 receipt가 살아 있으면
동시에 charge한다. Row 비용은 최초 admission에서 확정하고 lease 수명만큼 유지한다.

후보 수정: canonical row와 receipt/prepared token의 `MemoEntry`를 불변 `Arc`로
공유하고, prepared query/row 배열도 clone 시 공유하도록 전환했다. 변경 후 raw
Cargo 비교 테스트는 ports clone 1/1, adapter retention 5/5, adapter 기본-feature
lib 75/75 통과했다. 단, QBC/all-features, SDK 실제 query 경로 및 최종 통합
소스는 아직 검증하지 않았다. 공개 lookup/snapshot의 caller-owned deep clone은
별도 수명이며 이 비용 계약에 포함하지 않는다.

## 필수 독립 oracle

1. Baseline row의 선언 비용 1,120 units를 production sizing helper를 호출하지
   않는 테스트 상수로 검증한다. variable `snapshot_inputs`, digest/index,
   revision/URI 크기는 독립 숫자 oracle을 둔다. artifact payload bytes는 제외한다.
2. Exact cap / cap+1에서 insert·replace·reserved insert의 visible row, 모든
   locator, publication generation, installed ledger가 불변인지 비교한다.
3. Receipt clone/drop/retract, successor, foreign/stale receipt, prepared
   commit/discard, invalidate, GC, cancelled reservation에서 ticket 중복·조기
   반환·누락이 없어야 한다. full cap에서 retract 복원은 새 admission을 요구하지 않는다.
   특히 가변-size entry를 포함한 receipt/prepared 반복 clone이 metadata를
   deep-copy하지 않는지 포인터 identity 또는 allocation oracle로 검증한다.
4. Admission/release callback이 memo table로 재진입해도 잠금 아래 호출되지
   않아야 한다. 순서를 barrier로 고정한 publish↔invalidate/GC 경합을 실행한다.
5. 실제 composer에서 만든 memo account와 artifact/evidence store가 동일
   `LiveProcessMemoryBroker` snapshot을 바꾸는지 검증한다. 현재 교차-owner 테스트는
   후보에 추가됐지만 외장 빌드 볼륨 해제 이후 **미실행**이다.
6. SDK 제품 경로에서 `query → memo miss → publication → hit → invalidate/GC`
   및 `capacity denial → typed failure → 정상 요청`을 검증한다. owner-local
   `InMemoryMemoTable` 테스트나 source census만으로 이 gate를 통과시키지 않는다.

## 실행과 닫기

- Semantica QBC owner lib/all-features의 이전 스냅샷 결과는 adapter 122/122,
  memory-host 119/119, composer 51/51. composer 결과 이후 테스트/manifest/lock이
  바뀌어 현재 최종 source receipt는 아니다. QBC가 허용하는 artifact volume을
  다시 연결한 뒤 현재 소스 digest에서 rerun한다.
- Ports → adapter → host → composer → SDK composition/public smoke → runtime
  query product E2E 순서로 증가시킨다. 테스트 개수 0인 filter는 GREEN이 아니다.
- `git diff --check`, 최소/production feature compile, 변경 owner QBC, 제품 E2E,
  양 repo exact dependency source/dirty fingerprint를 묶어 SMA-009에 넘긴다.
- Main 이식은 완료됐다. 기존 manifest GC 수정과 회귀 테스트는 보존하고
  메모리 관련 hunk만 합쳤다. `0d04d66`의 이전 부분 후보 GREEN을
  최종-main proof로 합산하지 않는다.
- 제품용 무과금 `InMemoryMemoTable::new()` 생성자가 남아 있는 모든 경로를
  reachability로 분류한다. 제품 경로면 설치 account를 주입하거나 해당 경로의
  budget claim을 명시적으로 제외한다. 테스트 전용 생성자는 바꾸지 않는다.

완료 조건: 실제 SDK query에서 canonical memo 행의 설치 broker charge와
정확한 최종 release가 확인되고, typed denial이 빈 성공/cache miss로 바뀌지
않으며, 최종 통합 소스의 owner+product receipt가 모두 GREEN이다.
