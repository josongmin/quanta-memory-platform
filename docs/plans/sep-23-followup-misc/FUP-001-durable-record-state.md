# FUP-001 — Durable record의 정확한 terminal state

Status: `HISTORICAL VERIFIED — excluded from the current Redb-free product plan; owner-local GREEN + temporary-mutant RED, not an active FUP-006 gate`
Priority: `historical P1; no current product priority`
Owner: `Lane A`
Depends on: 실행 인덱스의 source freeze
Blocks: historically `FUP-002` and `FUP-006`'s durable-enabled candidate; not the current in-memory gate
Checklist: `I07`의 정확한 record disposition, `I02`의 identity 일부. `I03/I05/I09`의 전체 계약을 이 티켓만으로 닫지 않는다.

## 문제와 근거

`spill/backings/redb.rs::reserve_record_id`는 사용되지 않는 sequence hole을 허용한다.
그런데 `resolve_record`는 queue/recovery에 없고 ID가 `FINALIZED_THROUGH` 이하면
실제 저장 여부와 무관하게 `Finalized`를 반환한다.

재현: ID 0 예약만 수행 → ID 1 commit → ID 1 restore/finalize → ID 0 조회.
현재 결과는 `Finalized`, 올바른 결과는 `Absent`다. `spill/mod.rs`는 두 결과를
"실제 존재 후 소비됨"과 "예약됐지만 durable해진 적 없음"으로 구분한다.
`revocable_store/service.rs::reconcile_durable_spill`은 `Finalized`를 source retire의
근거로 사용한다. **오판은 실행 재현됐고, 실제 I/O fault부터 source-loss까지는 미재현**이다.

현 HEAD `028448946caa95676df034481a830c1d3e3b1f33`의 격리 clone에서도
`reaudit_reserved_hole_is_not_finalized_by_later_record` 1개가 선택되어 exit 101:
`left: Ok(Finalized)`, `right: Ok(Absent)`였다. 이는 올바른 계약을 assert한 RED이지
현재 제품 테스트의 PASS가 아니다. 실행 명령과 임시 test 경로는
`docs/misc/memory-state-substrate-gap-reaudit-2026-09-23.md`에 기록돼 있다.
기본 `full`에는 Redb가 없고 `spill-redb`/`full-backings`에서 opt-in이다. 따라서
이 티켓은 해당 durable capability의 출하 차단이며, 미사용 host의 기본 core가
곧바로 동일한 결함에 노출된다고 주장하지 않는다.

`RedbSpillBacking::create`도 queue/next sequence에서 finalized watermark를 추정한다.
후속 finalize가 없는 단순 reserve→close→reopen도 별도 regression 대상에 포함한다.

## Write set

- `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs`
- `crates/quanta-memory-platform-facade/src/spill/{mod,service}.rs`
- `crates/quanta-memory-platform-facade/src/revocable_store/service.rs`
- 필요 시 facade `tests/public_api_surface.rs`, `src/lib.rs`의 직접 영향 범위.
- 저장 포맷/운영 문서 변경안은 I에 전달. 다른 store와 core ledger는 변경하지 않는다.

## 구현 단계

1. 위 hole 재현을 실제 Redb owner 테스트로 옮긴다. 기대값 `Absent`로 현재 구현 RED를
   확보한다. queue/recovery payload 수와 실제 bytes도 확인한다.
2. sequence allocator와 **정확한 record disposition**을 분리한다. 기본안은
   schema-versioned record metadata에 `Reserved / Queued / Recovery / Finalized`를
   저장하고 payload·phase 변경과 같은 Redb transaction에서 갱신하는 것이다.
   동등한 exact tombstone 표현은 가능하지만 `max(sequence)` 추정은 금지한다.
3. `commit_spill`, `commit_restore`, `finalize_restore`, `resolve_record`,
   `reconcile_restore` 모두 같은 persisted state authority를 사용하게 한다.
   write resolution의 `Present`와 restore publication permission을 혼동하지 않는다.
   recovery payload가 남아 있는 상태는 consume 완료로 뭉개지 말고 phase를 보존한다.
4. 동일 ID/동일 bytes retry는 중복 insert 없이 허용하고, 다른 bytes는 거절한다.
   실제 finalized ID는 다시 생성할 수 없어야 한다. allocator ID는 재사용하지 않는다.
5. **기존 DB 호환성:** watermark-only 파일에서 absent ID가 hole인지 과거 소비인지
   복원할 수 없다. queue/recovery의 명시적 증거만 변환하고, 불명확한 과거 ID의
   조회/재사용은 typed unresolved error로 fail-closed 처리한다. `Finalized`나 `Absent`를
   추측하지 않는다. 새 ID 공간과 schema version을 같은 transaction에서 기록한다.
   데이터가 있는 파일을 자동 초기화하거나 삭제하는 migration은 금지한다.
6. 기존 `MemoryResult`의 명시적 오류로 불명확성을 표현할 수 있으면 enum 변경을
   최소화한다. 새 상태가 필요하면 모든 match/consumer와 public API 영향까지 갱신한다.
7. metadata 행 증가량과 디스크 비용을 기록한다. 안전한 replay horizon 없이 tombstone을
   GC하지 않는다. compaction 최적화는 정확성을 약화하지 않는 별도 변경으로 남길 수 있다.

## 검증 계약

- [x] 낮은 ID hole + 높은 ID finalize 후 hole은 `Absent`; reopen 후에도 동일.
- [x] 예약만 하고 reopen; queue empty를 전체 consumed로 오판하지 않음.
- [x] ID 예약/commit 순서가 달라도 정확한 상태; 실제 finalized ID 재삽입 거절.
- [x] 같은 ID/같은 bytes retry는 inventory 불변; 다른 bytes는 기존 payload 보존.
- [x] commit ACK와 probe가 모두 불확실한 테스트에서 source는 fenced·charged 상태 유지.
- [x] 위 pending source와 다른 store의 후속 finalize를 결합해 잘못된 source retire를 막음.
- [x] legacy DB fixture의 queued/recovery bytes 보존; 불명확한 history는 명시적 오류.
- [x] watermark 추정으로 되돌린 mutant를 hole regression이 검출.
- [x] owner-local RED→GREEN에서 위 **1 selected** 반례가 GREEN이 되고, 같은
  final source에서 reopen/legacy/ACK-loss 음성 검증이 유지된다. selector 0건은 무효.

Fault injection은 test-only Redb 경계에 둔다. 실제 disk 장애 재현과 deterministic
injected fault 검증을 receipt에서 구분한다. production public API 성공 경로도 유지한다.

```bash
cargo test --locked -p quanta-memory-platform-facade --all-features sep23_fup001 -- --nocapture
cargo test --locked -p quanta-memory-platform-facade --all-features durable_ -- --nocapture
```

위 `sep23_fup001`은 신규 테스트의 예정 prefix다. 실행 전에 `--list`로 존재/선택
건수를 확인한다. 임시 clone의 테스트를 공유 checkout에 복사했다는 사실만으로
고정 source의 GREEN이 되는 것은 아니다.

## Handoff / stop 조건

- 002에 schema, exact state 전이표, legacy 처리, regression 결과를 전달한다.
- 기존 DB 상태를 안전하게 분류할 방법이 없으면 보수적 오류를 유지한다. 이를 가용성
  이유로 성공 처리하려면 별도 migration 계약 결정이 필요하다.
- scope 밖 DB 삭제, opaque byte 재해석, commit 결과 추정, fail-open fallback은 금지한다.
- 모든 구현·검증 항목은 현재 미수행. 공통 receipt를 작성한 뒤 상태를 변경한다.

## Handoff 실제 기록 (Lane A, 2026-09-23)

Changed: `spill/backings/redb.rs` per-record disposition
(`SPILL_RECORD_STATE`: Reserved/Queued/Recovery/Finalized), single-owner
proof (`SPILL_FINALIZED_BY`), same-transaction migration to schema v2,
`revocable_store/service.rs` uncertain-commit fencing.

Record-state transition table (to FUP-002, authoritative):

| Persisted evidence | `resolve_record` | Note |
| --- | --- | --- |
| no state row, id < next-seq, no payload | `Absent` | reserved-never-committed hole; never guessed |
| `Queued` + queue payload | `Queued` | restorable |
| `Recovery` + recovery payload | `Recovery` | committed, needs recovery path |
| `Finalized` + proof row | `Finalized` | terminal; proof names the generation |
| `Finalized` without proof / payload without state | explicit `InvalidState` | contradiction; migration fills legacy rows |

Verified: `sep23_fup001` 6/6 GREEN (facade all-features, final
source); mutant Reserved→Finalized RED 3/6; legacy watermark migration
preserves bytes and fails closed on ambiguous history.

Disk cost (measured, macOS/arm64, dev profile, redb): empty schema DB and
1,000 committed 32-byte records + 1 prepared claim are both exactly
1,589,248 bytes — initial file allocation dominates; marginal per-record
cost is below segment granularity at this scale. File growth is
segment-stepped, not per-record linear. Do not extrapolate linearly.

Not run: physical power loss; disk-fault injection beyond in-process
crash points (see FUP-006 residual boundaries).

Known risk: none open in this ticket's scope. Ambiguous legacy rows stay
explicit errors by design (availability trade-off documented above).
