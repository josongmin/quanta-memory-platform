# FUP-007 — 0-byte ticket의 group 수명과 최종 반환

Status: `VERIFIED — committed/pending/Drop/rebuild matrix GREEN + temporary-mutant RED`
Priority: `P1 — core authority/lifecycle correctness`
Owner: `Lane B0` (`api/control.rs` 단독 writer)
Depends on: source freeze
Blocks: `FUP-006`의 모든 core-enabled 후보; `FUP-004`/`FUP-005`의 겹치는 파일 lease
Checklist: `C05`, `J03`, `J05`, `B07`의 **release 수명 부분**. B07의 metadata 개수 상한은 이 티켓 밖이다.

## 문제, 현재 증거, 실패 경계

`ledger.rs:678-702`는 0-byte reserve에도 live ticket을 만들지만 owner/class/context의
양수 usage entry는 만들지 않는다. `control.rs:1341-1355`의 live reconfigure는
양수 `snapshot.owner_usage`만 검사해 기존 owner→group mapping과 group의 제거를 허용한다.
이후 원래 발급 group을 기억하는 ticket은 새 scheduler의 unknown-group 검사에서
`release_on` 또는 `rollback_pending_on`이 실패한다(`control.rs:955-992`,
`application/scheduler.rs:81-90`).

현 HEAD `028448946caa95676df034481a830c1d3e3b1f33`의 격리 checkout에서
group `g`의 0-byte `ManagedCharge` → `g`와 owner mapping 제거 재설정 성공 →
`release()`가 `InvalidState("cannot record release for unknown resource group g")` →
Drop 뒤 `live_ticket_count == 1` → `finish_rebuild()`도 같은 오류인 경로가
**1개 선택된 실패 계약 테스트로 재현**됐다. 명령·임시 테스트 경로는
`docs/misc/memory-state-substrate-parallel-audit-2026-09-23.md`의 focused execution에 있다.

추가 소스 경로: `reserve_owned(..., 0)`의 `PendingReservation` Drop은
`rollback_pending_ticket_on_drop`을 호출하지만 현재 `rollback_pending_on` 오류를
버린다(`control.rs:123-127,975-1012`). group 제거 뒤에는 pending ticket의
마지막 권한이 deferred queue에도 들어가지 않고 사라질 수 있다. **이 pending-Drop
경로는 아직 별도 실행 재현이 없다.** 0 semantic bytes라도 ticket authority 유실은
ledger invariant/cleanup 위반이다. 비제로 byte leak이나 외부 입력 도달은 주장하지 않는다.

## 구현 소유권과 선택지

- Primary owner: `crates/quanta-memory-platform-core/src/api/control.rs`의 재설정 검사,
  ticket→issuing-group authority, committed/pending 최종 정리.
- 필요한 경우 `src/domain/ledger.rs`의 ticket inventory/query와
  `src/application/scheduler.rs`의 순수 검증, core `tests/{policy_contracts,public_api_surface}.rs`
  및 owner-local `#[cfg(test)]`. B/C의 `control.rs`·`ledger.rs` 쓰기는 이 티켓 종료까지 금지한다.
- **기본 결정:** live ticket이 하나라도 묶인 owner mapping/group을 제거하거나 바꾸는
  재설정을 원자적으로 거절한다. 사용량 0은 권한 부재가 아니다. 최종 ticket 종료 뒤에는
  같은 재설정이 성공해야 한다. 동등한 retiring-group 보존 설계도 가능하지만 발급 group의
  release/rollback 권한, 새 grant 차단, final cleanup을 모두 증명해야 한다.
- `release_on`의 unknown-group 오류를 무조건 무시하거나 ticket을 조용히 삭제하는
  방안은 금지한다. scheduler와 ledger가 서로 다른 상태로 성공을 보고하지 않아야 한다.
- pending Drop의 오류 무시는 마지막 권한 유실 위험이다. primary reconfigure guard를
  우회하는 internal invariant break에도 권한을 잃지 않는지 점검하고, 필요하면 exact
  pending ticket을 추적 가능한 deferred authority로 보존한다. deferred queue 추가만으로
  잘못된 재설정을 허용하는 해결책은 인정하지 않는다.
- 재설정은 느린 제어 경로이므로 live ticket 검사 비용을 기록한다. reserve/release hot
  path에 매번 owner 전체 scan이나 무제한 metadata 복제를 추가하지 않는다.

## RED → GREEN 검증 계약

1. 같은 공개 builder/manifest에서 group `g`와 owner mapping을 만든다. 0-byte
   committed charge를 유지한 채 둘을 제거하는 **유효한 새 정책**을 적용한다. 기대
   계약을 먼저 고정한다: 재설정이 원자적으로 거절되거나 retiring-group 설계라면
   accepted 이후에도 release가 성공한다. 두 경우 모두 ticket이 stranded 되면 RED.
2. committed explicit `release`, 최종 Drop, rebuild fence 중 Drop→`finish_rebuild`를
   각각 별도 테스트한다. 모든 성공 경로에서 `live_ticket_count == 0`, 축별 semantic
   counters 원상 복귀, group usage 원상 복귀, deferred backlog 0을 확인한다.
3. pending `reserve_owned(..., 0)`의 explicit rollback과 Drop을 각각 검증한다.
   재설정 실패 시 원본 policy/revision/scheduler/ledger가 불변이고 pending 종료 뒤
   ticket count 0이어야 한다. Drop 오류를 삼키며 ticket만 남기는 mutant가 RED여야 한다.
4. 여러 ticket 중 일부만 0-byte, 서로 다른 owner/group, owner remap만 변경,
   group cap만 변경, 이미 종료한 ticket, 0-byte ungrouped ticket을 포함한다.
   살아 있지 않은 과거 ID 때문에 재설정을 영구 거절하면 안 된다.
5. 기존 양수 charge에 대한 group reconfigure 거절, limit-lowering, batch release,
   exact deferred release, ledger fail-closed 테스트를 유지한다. forged/foreign ticket은
   release authority가 될 수 없다.

예정 selector는 구현자가 신규 테스트에 붙인 뒤 `--list`/selected count로 실재를
확인한다. 아래 명령은 **현재 아직 PASS 증거가 아니다**.

```bash
cargo test --locked -p quanta-memory-platform-core --all-features sep23_fup007 -- --nocapture
cargo test --locked -p quanta-memory-platform-core --all-features --test policy_contracts
cargo test --locked -p quanta-memory-platform-core --all-features --test public_api_surface
```

## 완료·handoff 경계

- Owner-local committed/pending RED→GREEN의 exact command, exit, selected count,
  source-before/after hash, original reproduction과 새 negative oracle를 기록한다.
- 변경한 public policy/reconfigure 오류 의미와 migration 영향을 I에 전달한다.
- `control.rs` 및 필요한 `ledger.rs`/scheduler lease를 반환한 뒤에만 004/005/003의
  겹치는 writer를 입장시킨다.
- B07의 무제한 0-byte ticket **개수/메타데이터 상한**, J06의 일반 deferred backlog
  상한은 이 티켓의 완료로 GREEN이 되지 않는다. host 노출과 비용을 별도 P2 범위로 판정한다.
- 이 티켓만 VERIFIED여도 repository/extraction/host qualification, 배포·활성화는
  별개이며 FUP-006의 선택한 scope receipt가 필요하다.

## Handoff 실제 기록 (Lane B0, 2026-09-23)

Changed: `ticket_groups`/`ticket_owners` authority indexes record EVERY
live ticket including 0-byte ones; group/owner remap and removal consult
the indexes (not positive usage); pending-Drop and rebuild-Drop failures
defer authority instead of swallowing; `control.rs`/`ledger.rs` leases
returned to lanes C/B/B in order.

Public error-meaning changes: removing a group or remapping an owner
with ANY live ticket (even 0-byte) is now denied with `InvalidState`
where a 0-byte-only mapping was previously removable. No signature
changed. Host impact: none (`quanta-memory-host` never reconfigures).

Live-ticket check cost (recorded): reserve/release/recharge do O(log n)
index insert/remove per ticket; each live reconfigure scans the live
indexes once (O(live tickets)) to build the pinned-owner set. No
per-reserve full scan. B07/J06 cardinality caps stay OPEN by design.

Verified: `sep23_fup007` 8/8 GREEN (committed/pending/Drop/rebuild
matrix + scope matrix); mutant unrecorded-zero-byte RED 7/8 with the
documented survivor; deferred-drain preservation mutants D1/D2/D3 RED
1/1 each.

Not run: B07/J06 exhaustion (out of scope, stays OPEN).

Known risk: none open in this ticket's scope.
