# FUP-002 — Backend 범위의 single-owner recovery

Status: `VERIFIED — owner-local GREEN + temporary-mutant RED; FUP-006 exact-source gate 별도`
Priority: `P1`
Owner: `Lane A`
Depends on: `FUP-001`의 persisted state와 migration 계약
Blocks: `FUP-006`의 durable-enabled 후보
Checklist: `I04`의 이중 공개/환급, `I08`의 restart claimant/fencing, `I02`의 attempt authority. Disk/staging byte budget과 실제 강제 종료 복구까지 이 티켓만으로 닫지 않는다.

## 문제와 근거

`redb.rs::recover_active_restore`는 persisted candidate를 읽어 복사할 뿐 새 recovery
주체를 지정하지 않는다. 두 store의 lifecycle permit은 서로 독립적이다.
`restore_durable_candidate`는 Committed phase에서 commit 검증을 건너뛰고 publish하며,
`finalize_restore`의 `(None, false) => Ok(())`는 두 번째 주체도 성공시킨다.

실제 Redb에서 이전 controller를 drop/reopen한 뒤 두 새 store가 barrier로 동시
recovery하면 둘 다 `Restored(0)`, durable inventory 0, resident payload 2개가 된다.
"이전 volatile owner가 사라져야 한다"는 문서 전제를 충족한 재현이다.
ledger가 두 copy를 각각 charge하더라도 durable record의 단일 소유권은 위반된다.

현 HEAD `028448946caa95676df034481a830c1d3e3b1f33`의 격리 clone에서
`reaudit_only_one_store_recovers_committed_record` 1개가 선택되어 exit 101:
`restored=2 first_bytes=38 second_bytes=39 ledger_bytes=77 durable_records=0`이었다.
올바른 expectation은 정확히 한 `Restored`이며, 두 copy의 ledger 과금 합계가
맞는다는 사실은 custody 위반을 해소하지 않는다. 실제 프로세스 kill이 아닌
이전 controller drop/reopen + 두 새 store barrier 동시 실행이다. 명령과 임시
test 경로는 `docs/misc/memory-state-substrate-gap-reaudit-2026-09-23.md`에 있다.

## Write set

- FUP-001과 같은 facade spill/revocable/public API 파일. A가 연속 소유한다.
- capability regression을 위한 facade doctest/public API test.
- core accounting, unrelated stores, 다른 lane의 testkit oracle는 변경하지 않는다.

## 구현 단계

1. 실제 DB reopen + 두 store + thread barrier 테스트를 추가한다. sleep 기반 경합이나
   fake-only 재현에 의존하지 않는다. 정확히 한 복구 주체만 성공해야 한다.
2. **record identity와 recovery attempt authority를 분리한다.** backend-scoped record
   identity는 재시작/재시도 동안 안정적으로 유지하고, 복구 주체마다 새 generation/claim을
   부여한다. 현재 bare `u64`를 backend가 다른 token에도 그대로 인정하지 않는다.
3. 기본안은 비복제·비위조 lease와 backend 전체의 active claimant 관리다.
   lease는 acquire→decode→charge→publish→finalize 또는 fence까지 살아 있어야 한다.
   persisted phase/generation과 volatile owner가 일치하는지 각 권한 변경에서 검증한다.
   read 함수 한 번에만 mutex를 추가하거나 store별 lock을 늘리는 것으로 종료하지 않는다.
4. claim 획득/전이는 짧은 backend transaction으로 처리한다. 사용자 decode/callback
   동안 broker/state mutex나 Redb write transaction을 잡지 않는다. 다른 호출은
   기다리며 deadlock하지 않고 `OperationInProgress` 등 명시적 결과를 받는다.
5. `prepare/commit/finalize/abort/reconcile`를 같은 lease 계약으로 연결한다.
   같은 claimant의 ACK-loss retry만 idempotent 성공으로 인정한다. 외부에 공개된
   raw sequence 기반 API가 새 검증을 우회하면 제거/제한하고 caller migration을 적는다.
6. ambiguous outcome에서는 provisional charge/payload와 lease를 pending state가 소유한다.
   panic/drop/close/retry 각각에서 누가 lease와 durable recovery를 보존하는지 표로 명시한다.
   이전 프로세스 종료 후 reopen은 새 claim generation으로 복구할 수 있어야 한다.
7. backend identity는 rename/reopen에 안정적이어야 한다. pathname/메모리 주소만을
   persisted identity로 쓰지 않는다. 필요한 entropy/I/O는 backing adapter에서만 처리한다.
   새 의존성이나 공개 API 변경은 006의 extraction/API gate에 전달한다.

## 검증 계약

- [x] 동시 새 recovery 두 개 중 성공은 정확히 하나; 다른 store에는 읽을 수 있는 copy 없음.
- [x] 패자와 재시도 경로의 charge leak 없음; 최종 close 후 정확한 ledger 0.
- [x] 같은 claimant의 commit/finalize ACK-loss retry는 성공, 다른 claimant는 거절.
- [x] Prepared/Committed 각각의 reopen recovery 성공; stale generation은 거절.
- [x] decode 실패·panic, admission 거절, finalize 불확실성에서 payload 또는 recovery copy 보존.
- [x] 서로 다른 DB가 같은 numeric sequence를 발행해도 foreign claim/ID는 권한 변경 불가.
- [x] 현재 ordinary restore의 단일 active claim, FIFO, pending close 거절 계약 유지.
- [x] claim 검증 제거 mutant에서 중복 recovery regression RED.
- [x] 외부 crate가 lease를 임의 생성/복제하거나 raw sequence로 우회할 수 없는 compiler 계약.
- [x] 위 현재-head **1 selected** 반례가 같은 최종 소스에서 GREEN. 패자는
  `Restored`를 반환하지 않고 읽을 수 있는 resident copy/charge도 남기지 않는다.
- [x] `finalize_restore`의 idempotent `Ok`는 동일 claimant의 재시도에만 허용한다.
  이미 완료한 다른 claimant는 bare sequence만으로 성공할 수 없다.

```bash
cargo test --locked -p quanta-memory-platform-facade --all-features sep23_fup002 -- --nocapture
cargo test --locked -p quanta-memory-platform-facade --all-features durable_ -- --nocapture
cargo test --locked -p quanta-memory-platform-facade --all-features --doc
```

`sep23_fup002`는 예정 selector다. 구현 후 `--list`와 selected count를 확인한다.
기존 단일 store drop/reopen GREEN은 동시 recovery의 음성 oracle를 대신하지 못한다.

## Handoff / stop 조건

001과 함께 compatibility/migration/API 변경표를 I에 전달한다. public API 이름은
구현 전 계약 검토에서 고정하고 type 이름보다 위 소유권 전이와 음성 검증을 우선한다.
동일 claimant를 판별할 수 없는 상태를 단순 `Ok`로 처리하거나 문서에만
"동시에 호출하지 말 것"을 추가하는 수정은 완료로 인정하지 않는다.
실제 exactly-once 사용자 callback 실행이나 process crash 후 일반 resident data의
영속성까지 보장 범위를 확대하지 않는다.

## Handoff 실제 기록 (Lane A, 2026-09-23)

Changed: `DurableRestoreClaim` (backend, generation, sequence; no public
constructor and no `Clone`/`Copy` — only backends mint; candidate/recovery
envelopes are non-`Clone`), per-record generation checks on
commit/finalize/abort/reconcile, `revocable_store` lease fencing
(`pending_restores`) with catch-unwind decode/finalize paths, sealed
`DurableSpillTarget`/`DurableRestoreTarget` traits.

API change table (to FUP-006; BREAKING for external implementers):

| Before | After | Caller migration |
| --- | --- | --- |
| `commit/restore/finalize/abort/reconcile_restore(seq: u64)` | `...(claim: &DurableRestoreClaim)` | use `candidate.claim` from `prepare`/`recover` |
| `DurableRestoreCandidate { sequence, bytes }` | adds `claim: DurableRestoreClaim` | read `claim.backend/generation/sequence` for identity |
| unconditional second-claimant `Ok` | per-record proof (`SPILL_FINALIZED_BY`) | same-claimant retry still idempotent `Ok` |

Host impact: none — `quanta-memory-host` enables facade `full`
(no `spill-redb`) and imports no durable API. See `MIGRATION.md`.

Lease ownership table (panic/drop/close/retry × phase):

| Event | Prepared phase | Committed phase |
| --- | --- | --- |
| decode `Err` | abort claim, `DurableCommitFailure` preserved source | keep durable recovery, `OperationInProgress`, retry via recover |
| decode panic | abort claim, unwind resumes, retry is a plain prepare | durable recovery untouched, unwind resumes, retry recovers |
| finalize panic | n/a (nothing published yet) | fence published entry, unwind resumes, reconcile publishes |
| store close with pending | close rejected (`pending close` contract) | close rejected; durable recovery survives for another store |
| same-claimant retry | prepare again (idempotent) | commit/finalize idempotent `Ok` |
| other-claimant retry | `OperationInProgress` / stale rejection | `Superseded` or stale rejection; loser retires provisional |

Verified: `sep23_fup002` 8/8 GREEN; stale-generation mutant RED 1/8;
barrier race exactly-one-owner with loser charge 0 and final ledger 0;
compiler forge/reject doctests GREEN (`--doc` 24/24 incl. compile-fail).

Not run: multiprocess kill of a MID-finalize transaction at the exact
instruction point (SIGKILL rail covers commit/stream/hole kill points);
power loss.

Known risk: none open. Claim generation is a persisted `u64` counter;
exhaustion returns a typed error (no wraparound) and is unreachable in
practice.
