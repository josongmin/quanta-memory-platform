# QMP-000 — Campaign admission, source freeze, and receipt authority

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: none  
Blocks: every implementation ticket

## Objective

모든 remediation 작업을 exact source, exclusive write ownership, reproducible RED/GREEN receipt에 묶는다. 이 ticket은 product behavior를 고치지 않는다. 잘못된 source에서 실행된 구현이나 stale GREEN이 campaign closure로 승격되는 것을 막는다.

## Confirmed risk

- 감사 base는 `e1037716...`이지만 구현 시점의 HEAD는 이동할 수 있다.
- 루트 감사 문서는 untracked auditor-owned artifact다.
- external Semantica host는 dirty shared main이었다.
- focused/current-source/static/extraction/host proof가 서로 다른데 기존 문서는 이를 혼합해 “no known limitation”을 주장한다.

## Write set

- `tickets/**`
- 필요 시 ticket 검증 전용 `scripts/verify-tickets.sh` 또는 `tools/tickets/**`

금지:

- `crates/**`
- product `Cargo.toml`
- generated agent docs

## Required implementation

1. `tickets/README.md` inventory의 ID, 경로, dependency가 실제 파일과 일치하는 validator를 추가한다.
2. 다음을 검사한다.
   - 모든 inventory ticket에 `TICKET.md` 존재
   - unknown/duplicate ticket ID 없음
   - dependency cycle 없음
   - finding crosswalk의 P1/P2가 owning ticket 없이 남지 않음
   - receipt가 명시한 SHA와 현재 실행 SHA 구분
3. 구현 worker용 claim record 형식을 정의한다.
   - ticket ID
   - claim owner
   - base SHA
   - exact write paths
   - foreign dirty overlap
   - start timestamp
4. receipt template을 validator가 확인 가능한 최소 field로 고정한다.
5. source가 실행 중 바뀌면 terminal receipt를 거부한다.

## Tests

추가할 validator tests:

- `ticket_inventory_rejects_missing_ticket_file`
- `ticket_inventory_rejects_duplicate_id`
- `ticket_dependencies_are_acyclic`
- `every_confirmed_finding_has_an_owner`
- `receipt_rejects_source_sha_drift`
- `claim_rejects_overlapping_foreign_dirty_path`
- `implemented_without_required_receipt_is_not_terminal`

## Acceptance

- [x] ticket tree를 한 명령으로 검증할 수 있다.
- [x] fixture에서 missing file, cycle, uncovered finding, stale receipt가 모두 RED다.
- [x] 실제 tree는 GREEN이다.
- [x] validator는 product code를 수정하지 않는다.
- [x] `README` 상태 변경과 receipt 생성 절차가 결정적이다.

## Verification

```bash
git rev-parse HEAD
git status --short
scripts/verify-tickets.sh
python3 tools/pm/pm.py lint
```

## Non-goals

- product defect 수정
- CI provider 선택
- Semantica host qualification

## Closeout evidence

`STATIC`, validator negative fixtures, current tree GREEN receipt가 필요하다. 문서가 존재하는 것만으로 QMP-000을 닫지 않는다.

## Receipt

- Validator implementation commits: `fd4fdf08c362131193856898a4d7aeb539f59d53`, `dc2e7d0373911ec4fa943a5c0129ebf680ca0817`
- Receipt: [`receipts/2026-09-20T183609Z-dc2e7d0.md`](receipts/2026-09-20T183609Z-dc2e7d0.md)
