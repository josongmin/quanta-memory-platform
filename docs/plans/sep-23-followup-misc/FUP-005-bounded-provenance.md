# FUP-005 — Reconfigure provenance의 bounded representation

Status: `VERIFIED — 10k bounded oracle + temporary append-mutant RED`
Priority: `P2 — 운영 비용, 장애 임계치 미측정`
Owner: `Lane B`
Depends on: source freeze와 `FUP-007`의 `control.rs` lease 반환
Blocks: `FUP-003`, `FUP-006`의 full-followup 후보. P1-only 후보에서는 OPEN으로 남을 수 있다.
Checklist: `L04`의 reconfigure provenance 보관/clone 비용 부분. Event-sink 재진입 및 일반 telemetry queue/time bound는 별도 계약이다.

## 문제와 근거

`api/control.rs::BrokerCore::install_policy`는 accepted reconfigure마다
`override_provenance.push(OverrideRecord::new(source, "residency.active"))`를 수행한다.
`OverrideRecord`에는 `String`이 있고 `effective_report`/`observe`는 broker lock 아래에서
report를 clone한다. 따라서 동일 정책 재설정도 횟수에 따라 보관량·복사 비용을 늘린다.
실제 장애 또는 p99 임계치는 측정하지 않았으므로 availability P1로 분류하지 않는다.

현 HEAD에서도 accepted reconfigure마다 `OverrideRecord { String, ... }`가
append되고 `effective_report`/`observe`가 broker lock 아래 report를 clone한다
(`control.rs:1405-1444,1504-1510`). 이는 source-confirmed O(N) 보관/관측
비용이며 10,000회/latency 실행 결과는 아직 없다. `L04`의 다른 누락인
사용자 event sink의 `observe()` 재귀는 격리 실행됐지만 **이 티켓의 변경 범위가
아니다**. 같은 `control.rs`를 쓸 수 있으므로 별도 writer와 lease를 직렬화한다.

## Write set

- `crates/quanta-memory-platform-core/src/api/control.rs`
- `crates/quanta-memory-platform-core/src/domain/{stats,config}.rs`
- `crates/quanta-memory-platform-core/tests/policy_contracts.rs`
- 필요 시 같은 공개 report를 사용하는 core 예제/API 테스트의 직접 영향.
- `ledger.rs`, scheduler, facade, testkit은 편집하지 않는다.

## 구현 단계

1. 동일/교차 source로 많은 accepted reconfigure를 수행하는 테스트에서
   `override_provenance.len()` 증가를 확인한다. timing threshold가 아니라 보관량을 oracle로 쓴다.
2. 기본안은 **초기 resolver provenance + 현재 active residency 1개**다.
   초기 필드별 provenance는 보존하고 `residency.active` 항목은 append하지 않고 upsert한다.
   활성 정책 보고서와 전체 변경 이력을 같은 Vec으로 취급하지 않는다.
3. `active_policy_source`, 기존 source-label summary의 의미와 순서를 유지한다.
   source enum이 유한한 현재 계약을 활용하고, accepted reconfigure마다 revision은
   정책 값이 같아도 정확히 1 증가한다. rejected reconfigure는 report 전체를 보존한다.
4. runtime history가 필요하면 기존 외부 telemetry/audit 경계의 별도 후속 요구사항으로
   남긴다. 이 티켓에서 새 파일 I/O/clock/영구 로그 subsystem을 만들지 않는다.
5. report rustdoc을 실제 bounded 의미로 수정하고 `from_manifest`/`from_resolved` 및
   live reconfigure가 같은 contract를 유지하도록 한다. 임의로 초기 provenance를 지우거나
   매번 전체 policy history를 다른 필드로 옮겨 누적하지 않는다.

## 검증 계약

- [x] 10,000회 accepted reconfigure 후 entry 수가 초기 수 + active 1개 이하.
- [x] 초기 resolved field provenance 보존, 현재 active source 정확, 중복 active 항목 없음.
- [x] accepted N회 후 revision은 initial + N; rejected operation은 revision/report 불변.
- [x] `observe`와 `effective_report`가 동일 active 정책과 provenance 의미를 제공.
- [x] 다시 append하도록 한 mutant에서 bounded-size regression RED.
- [x] absolute latency assertion 없음. 추가 측정 시 source/machine/profile과 표본을 기록.

```bash
cargo test --locked -p quanta-memory-platform-core --all-features sep23_fup005 -- --nocapture
cargo test --locked -p quanta-memory-platform-core --all-features --test policy_contracts
```

`sep23_fup005`는 신규 예정 selector다. 선택 건수와 entry-count oracle의
RED→GREEN 없이는 계획만으로 완료 처리하지 않는다.

## Handoff / stop 조건

기존 caller가 runtime chronological history를 계약으로 요구하는지 grep/consumer 확인을
먼저 수행한다. 요구가 확인되면 조용히 truncate하지 말고 bounded history의 용량/누락
표시 또는 별도 audit export 계약을 결정한 뒤 진행한다.
005 완료 후 control/report hash를 고정하여 003이 그 postimage에서 시작하게 한다.
policy revision 삭제·중복 정책 update 무시·clock 기반 epoch 도입은 이 티켓의 해결책이 아니다.

## Handoff 실제 기록 (Lane B, 2026-09-23)

Changed: accepted live reconfigures upsert the single `residency.active`
provenance record instead of appending; initial resolver records untouched;
`override_provenance` rustdoc now states the bounded (non-chronological)
contract. Control/report hash fixed for FUP-003's postimage start.

Consumer check (required before truncate): `override_provenance` has NO
readers outside core — facade, testkit, and `quanta-memory-host` sources
contain zero references. No caller requires chronological history, so no
bounded-history capacity marker or audit-export contract was needed.

Verified: `sep23_fup005` 2/2 GREEN (10,000 accepted reconfigures keep
`initial + 1`, revision `initial + 10,000`, active source exact, no
duplicate active, `observe().report == effective_report()`); denied
reconfigure preserves revision/report/provenance byte-for-byte; mutant
always-append RED 1/2 with the correct split.

Not run: absolute latency assertions (none added, per contract).

Known risk: none open. Revision still counts every accepted reconfigure;
only the history representation is bounded.
