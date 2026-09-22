# TO-08 — Remove Exact Duplicate/Dead Assertions Only

Status: `OPEN`
Priority: `P2`
Depends on: none

## Scope

이 티켓은 동작 범위를 줄이는 최적화가 아니라 세 개의 확정된 noise만 제거한다.

1. `core/src/ports/telemetry.rs`: 같은 table/enum coverage에서 이미 포함된 exact duplicate
   case 한 개 제거. unique label/non-empty/outcome coverage는 유지.
2. `core/src/api/control.rs`: owner hard-cap denial 뒤 `total == 400_000`을 이미 assert한 다음
   `total != 600_000`을 재검사하는 dead `assert_ne!` 제거.
3. `core/tests/public_api_surface.rs`: 실제 9개 variant를 “eight”라고 부르는 comment를
   “nine”으로 수정. exhaustive variant array와 test 이름은 유지.

## Write set

- `crates/quanta-memory-platform-core/src/ports/telemetry.rs`
- `crates/quanta-memory-platform-core/src/api/control.rs`
- `crates/quanta-memory-platform-core/tests/public_api_surface.rs`

## File-by-file implementation plan

### `crates/quanta-memory-platform-core/src/ports/telemetry.rs`

- Delete the complete single-case test
  `advisory_flags_mismatch_and_reports_semantic_as_authority`.
- Keep `compare_reports_signed_drift_and_mismatch_boundary`; its `(2_000, 1_000, 1_000, true)`
  row and semantic-authority assertion are the exact keeper.
- Keep equality, missing-data, overflow, and observation-only tests untouched.

### `crates/quanta-memory-platform-core/src/api/control.rs`

- In `reserve_denied_owner_hard_cap_leaves_ledger_untouched`, delete only the comment
  `negative mutation...` and `assert_ne!(after.total_owner_bytes(), 600_000)`.
- Keep typed Capacity assertion, before/after equality, and exact `400_000` postimage.
- Do not rename the test or edit reserve/commit production code.

### `crates/quanta-memory-platform-core/tests/public_api_surface.rs`

- Change only `All eight MemoryError variants` to `All nine MemoryError variants`.
- Keep test name, nine-element array, exhaustive match, and documented variant list unchanged.

File-local verification commands are the three focused selectors already listed below. `git diff`
must show no production hunk and no other test deletion.

production logic, enum variants, error strings, public exports는 변경하지 않는다. 비슷해 보이는
다른 assertion을 이 티켓에 추가로 묶지 않는다.

## Verification

```bash
cargo test -p quanta-memory-platform-core ports::telemetry
cargo test -p quanta-memory-platform-core reserve_denied_owner_hard_cap_leaves_ledger_untouched
cargo test -p quanta-memory-platform-core --test public_api_surface memory_error_has_exactly_the_nine_documented_variants
cargo test -p quanta-memory-platform-core --all-features
```

diff review에서 각 삭제가 keeper assertion으로 완전히 함의되는지 한 줄씩 기록한다.
성능 개선이나 coverage 증가를 주장하지 않는다.

Negative-pole/mutant는 `N/A`다. 이 티켓의 증명은 삭제된 assertion 각각에 대해 더 강한
keeper assertion과 동일 실행 경로를 지목하는 diff proof다.

## Acceptance / reopen

- 정확히 duplicate/dead assertion과 stale comment만 바뀐다.
- unique outcome/variant/capacity invariant가 같이 삭제되거나 production diff가 생기면
  reopen한다.
