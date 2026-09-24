# SMA-008 — Named durable reuse의 도입 결정

ARCHIVED DESIGN — 실행 티켓 아님. 아래 요구는 향후 해당 범위를 채택할 때만 참고한다.

Priority: DEFERRED. Status: DEFERRED.
Owner: Semantica artifact/cache identity + persistence.
현재 실행 대상 아님. QMP Redb는 current product unsupported 상태를 유지한다.
Admission gate: named consumer, measured restart recompute cost, 독립 authoritative source.

## 결정에 필요한 파일/증거

- `C/quanta-runtime/src/adapters/artifact_registry/`: 기존 FS CAS·manifest·GC 재사용 가능성
- `C/quanta-runtime-context-shared/src/artifact_registry_v1.rs`: 실제 저장하는 artifact 종류
- `C/quanta-runtime-cache-identity-wire/src/`: stable key·dependency identity contract
- QMP `README.md`와 0.3 RFC의 제품 경계; Redb backing은 후보 비교 대상으로만 읽는다.

## 작업

1. 저장할 결과 하나, 실제 producer/consumer, cold computation 비용, serialization/restore
   비용, 예상 reuse, disk footprint를 측정한다. 측정 전에 backend를 고르지 않는다.
2. key에는 결과에 영향을 주는 content/dependency/config/engine/schema/codec/profile을
   포함한다. process-local ID, stale snapshot, 다른 workspace/tenant 권한을 재사용하지 않는다.
   경로 변경/relocation의 cache hit 허용 여부도 명시한다.
   기존 cache-identity-wire/CAS가 identity SSOT다. 누락 필드가 증명되면 그 owner를
   확장하고 backing별 독립 key builder/버전 체계를 추가하지 않는다.
3. 기존 CAS+metadata, SQLite, Redb를 같은 corpus·durability·sync·concurrency 조건에서 비교한다.
   duplicate storage, DB/page cache, in-flight buffers, codec/decode scratch, GC/repair 시간을 측정한다.
4. regenerable cache loss와 authoritative state loss의 계약을 분리한다.
   전원 손실 보장 필요 여부, write success의 durability 의미, process/file locking을 명시한다.
5. disk quota/entry quota, corrupt/truncated payload, digest/version mismatch, decompression/
   decode expansion cap, ENOSPC/EIO, readonly/permission, concurrent writer/reader,
   interrupted GC와 restart를 설계한다.
6. cache read 실패는 안전한 recompute 또는 typed failure로 귀결된다.
   검증되지 않은 bytes를 정답으로 사용하거나 authoritative corruption을 cache miss로 숨기지 않는다.

## 완료 판정

`ADOPT`: named workload의 정량 이득, 선택 backend 이유, byte/key/lifecycle contract,
fault matrix, 구현 파일과 owner가 모두 있는 별도 구현 티켓을 발행.
`DEFER`: 이득/요구 부족의 자료와 재개 조건을 기록.
이 결정 티켓만으로 disk code나 Redb 지원 상태를 변경하지 않는다.

실제 채택 후 owner-local codec/transaction tests→실제 subprocess restart→
Linux EIO/ENOSPC 순서로 검증한다. SIGKILL 통과를 power-loss qualification으로 표시하지 않는다.
SMA-009의 in-memory 출하를 이 보류 결정으로 막지 않는다.
