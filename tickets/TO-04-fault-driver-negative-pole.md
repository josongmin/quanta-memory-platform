# TO-04 — Route the Broken Spill Fake through the Real Fault Classifier

Status: `IMPLEMENTED; final exact-source receipt pending`
Priority: `P0`
Depends on: none

## Problem

`drive_spill_failure_after_reservation()`에는 before/result/after를 함께 판정하는 실제
classifier가 있다. 그러나 `spill_fault_classification_reports_escaped_for_a_lying_controller`
는 테스트 안에 축약 classifier를 다시 작성한다. production driver가 항상 `Contained`를
반환하도록 망가져도 negative pole이 통과할 수 있다.

## Write set

- `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs`
- public no-argument driver API는 유지한다.

## File-by-file implementation plan

### `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs`

- Extract immediately above `drive_spill_failure_after_reservation`:
  `fn classify_spill_failure<C: SpillController<String>>(controller: &C) -> FaultOutcome`.
- Move the existing before/result/after match into this helper without shortening branches or
  evidence strings. Inventory read error와 drifted count는 계속 `Escaped`다.
- Reduce public driver body to unavailable `InMemorySpillTarget` construction plus one helper call.
- Delete the test-local `classify` nested function entirely.
- Keep `LyingSpillController` test-only. Its `spill` returns `Ok(InProcess { inventory_after: 0 })`
  and the test passes it to the extracted production helper.
- Honest positive pole also calls the same helper; public driver test remains as end-to-end coverage.
- Do not export the helper, change `FaultOutcome`, or modify facade `SpillController` APIs.
- File-local verification:
  `cargo test -p quanta-memory-platform-testkit spill_failure -- --nocapture`.

## Target seam

- private generic helper를 둔다:
  `classify_spill_failure<C: SpillController<String>>(controller: &C) -> FaultOutcome`.
- helper 하나가 `spilled_count before → spill → spilled_count after`를 읽고 모든
  `Contained`/`Escaped` 분기를 소유한다.
- public `drive_spill_failure_after_reservation()`는 unavailable honest controller를 만들고
  그 helper를 호출만 한다.
- lying controller test도 정확히 같은 helper를 호출한다. 테스트 로컬 분류 함수는 삭제한다.
- honest pole은 fail-closed error와 unchanged count를, lying pole은 success를
  `Escaped`로 증명한다.

helper를 public API로 만들거나 facade의 production trait를 testkit에 맞게 바꾸지 않는다.

## Required tests

- honest unavailable target: `Contained`, count `0 → 0`, error evidence 포함
- lying controller: `Escaped`, success-on-unavailable evidence 포함
- count read가 실패하거나 error 뒤 count가 drift하면 `Escaped`
- 기존 public driver test가 helper를 통한 동일 evidence를 보존

## Negative proof

temporary mutant로 success arm을 `Contained`로 바꾼다. lying-controller test가 RED여야
한다. 별도 local classifier가 RED인 것은 인정하지 않는다.

```bash
cargo test -p quanta-memory-platform-testkit --features faults spill_failure -- --nocapture
cargo test -p quanta-memory-platform-testkit --features faults faults::scenarios
```

## Acceptance / reopen

- honest/lying component가 같은 classifier code path를 공유한다.
- public driver의 branch를 바꾼 mutant를 negative pole이 잡는다.
- test module에 duplicated `match controller.spill` classifier가 다시 생기면 reopen한다.

## Closeout receipt (owner-verified, IMPLEMENTED — annotated receipt owed)

- Mutant: success arm (`(_, Ok(_), _)`) → `Contained` in production
  `classify_spill_failure`.
- RED: `spill_fault_classification_reports_escaped_for_a_lying_controller`
  FAILED at `scenarios.rs:263`; drift/unreadable poles passed (separate
  arms). Proves both poles share the production code path.
- Post-revert: 3 passed; file byte-identical to HEAD.
