# TO-09 — Assert Exact Independent Transcript Cost Partitions

Status: `IMPLEMENTED; owner proof recorded; included in historical qmp-final-qualified-sep23-followup-v5 source; current HEAD qualification separate`
The problem and edit plan below are a pre-implementation snapshot; the owner proof and v5 receipt govern the historical result.
Priority: `P1`
Depends on: none

## Problem

`retention_stats_separate_active_evidence_and_session_costs`는 세 partition이 `> 0`이고 합이
total이라는 것만 확인한다. active/evidence/session counter가 서로 바뀌어도 total이 같으면
통과할 수 있다. accounting test는 총량뿐 아니라 각 소유 partition의 exact 의미를
독립적으로 계산해야 한다.

## Write set

- `crates/quanta-memory-platform-facade/src/transcript_store/service.rs` test module only
- production `active_row_cost`, `identity_evidence_cost`, `session_key_cost`는 failing test가
  결함을 입증할 때만 변경한다.

## File-by-file implementation plan

### `crates/quanta-memory-platform-facade/src/transcript_store/service.rs`

- Add test-only named constants next to existing independent `cost` helper:
  `ORDERED_INDEX_BYTES_ORACLE=24`, `DIGEST_BYTES_ORACLE=32`,
  `ENTRY_STATE_BYTES_ORACLE=16`.
- Add `expected_partition_costs(session_id, message_id, body) -> (u64, u64, u64, u64)` using
  only string lengths and those test constants. Do not call production cost helpers/constants.
- Leave existing `cost(session_id, rows)` unchanged because other tests consume it; avoid a broad
  oracle refactor in this ticket.
- In `retention_stats_separate_active_evidence_and_session_costs`, replace the three `> 0`
  assertions with exact active/evidence/session/total values from the new helper.
- Also assert the literal fixture result `(46, 148, 41, 235)` so an accidental change to the
  test-side formula is visible in review.
- After `truncate_to`, assert row/active `0`, evidence `1/148`, session `1/41`, total `189`, and
  retain the component-sum invariant.
- Do not modify append/truncate implementation, retention structs, production counters, or public
  stats types.
- File-local verification:
  `cargo test -p quanta-memory-platform-facade --all-features retention_stats_separate_active_evidence_and_session_costs -- --nocapture`.

## Independent oracle

fixture `session="s"`, `message_id="m0"`, `body="body"`에 대해 production helper/constant를
호출하지 않고 test-side named constants로 계산한다.

```text
ordered-index = 24
digest = 32
entry-state = 16

active  = body(4) + message-id(2) + 24 + 16 = 46
evidence = 2*message-id(2) + 2*24 + 2*32 + 2*16 = 148
session = session-id(1) + 24 + 16 = 41
total = 46 + 148 + 41 = 235
```

test-only helper는 `(active, evidence, session, total)`을 반환해 component name을 보존한다.
production `semantic_cost_v1` constants나 cost functions를 import하지 않는다.

## Required transitions

- append 후 row/evidence/session counts가 각각 1.
- bytes가 정확히 active `46`, evidence `148`, session `41`, total `235`.
- truncate 후 active count/bytes `0`, evidence `1/148`, session `1/41`, total `189`.
- `total == active + evidence + session`도 보조 invariant로 유지한다.

## Negative proof

temporary mutant로 publication 시 evidence/session counter를 서로 바꾸거나 한 partition에
다른 partition 비용을 더한다. total을 보존해도 exact component assertion이 RED여야 한다.

```bash
cargo test -p quanta-memory-platform-facade --all-features retention_stats_separate_active_evidence_and_session_costs -- --nocapture
cargo test -p quanta-memory-platform-facade --all-features transcript_store::service
```

## Acceptance / reopen

- expected partition이 production helper에서 파생되지 않는다.
- append와 truncate postimage 모두 exact하다.
- `> 0`/sum-only assertion으로 되돌리거나 partition swap mutant가 GREEN이면 reopen한다.

## Historical owner proof (subsequently covered by the v5 source receipt)

- Mutant: `append` swaps `evidence_bytes` ↔ `key_bytes` cost sources;
  total preserved at 235.
- RED: `retention_stats_separate_active_evidence_and_session_costs`
  FAILED at `service.rs:1510` on the exact `evidence_bytes` component
  assertion (`left: 41, right: 148`); counter validation passed, oracle
  fired. Oracle never calls production cost helpers.
- Post-revert: `transcript_store` suite 47 passed; production regions
  byte-identical to HEAD.
