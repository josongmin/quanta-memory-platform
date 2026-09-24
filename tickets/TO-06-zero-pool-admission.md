# TO-06 — Make Zero Shared-Pool Borrowing an Executable Public Contract

Status: `IMPLEMENTED; owner proof recorded; included in historical qmp-final-qualified-sep23-followup-v5 source; current HEAD qualification separate`
Priority: `P1`
Depends on: none

## Problem

`borrow_flag_with_zero_shared_pool_is_valid_but_grants_no_burst`는 policy 값만 읽고
`burstable = shared_burst_pool_bytes`라는 tautology를 검사한다. reserve를 실행하지 않아
ledger가 class hard cap을 넘어 1 byte를 허용해도 통과한다.

## Ownership

- Validation owner: `crates/quanta-memory-platform-core/src/domain/policy.rs`
- Cross-cutting behavior owner:
  `crates/quanta-memory-platform-core/tests/policy_contracts.rs`
- production policy/ledger는 failing behavioral test가 결함을 입증할 때만 변경한다.

## File-by-file implementation plan

### `crates/quanta-memory-platform-core/src/domain/policy.rs`

- Rename `borrow_flag_with_zero_shared_pool_is_valid_but_grants_no_burst` to
  `borrow_flag_with_zero_shared_pool_is_valid_and_preserved`.
- Keep construction of a class with `borrow_from_shared_pool=true`, hard cap `800`, pool `0`.
- Keep only three validation facts: `validate_residency` succeeds, pool remains `0`, borrow flag
  remains true.
- Delete `let burstable = ...`, its equality assertion, and comments claiming reserve denial.
- Do not change `validate_residency`, `MemoryClassBudget`, default classes, or policy constructors.

### `crates/quanta-memory-platform-core/tests/policy_contracts.rs`

- Add `zero_shared_pool_cannot_extend_borrowing_class_hard_cap` beside existing
  `shared_burst_pool_bounds_borrowing_through_public_broker`.
- Reuse existing `broker_from_residency` and `class_budget` helpers; do not add a parallel broker
  fixture.
- Build global `10_000`, pool `0`, ArtifactCache `soft=600`, `hard=800`, borrow true.
- Reserve one 800-byte `ManagedCharge` through `reserve_managed_charge`; capture full snapshot.
- Attempt one additional byte and require `MemoryError::Capacity`.
- Assert full snapshot equality and total owner bytes `800`; then explicitly release/drop live
  charge and assert total `0`.
- Do not add the behavior test to `policy.rs` or call internal ledger methods directly.
- File-local verification:
  `cargo test -p quanta-memory-platform-core --test policy_contracts zero_shared_pool_cannot_extend_borrowing_class_hard_cap -- --nocapture`.

## Implementation

1. policy unit test는 `borrow_flag_with_zero_shared_pool_is_valid_and_preserved`로 바꾼다.
   validation 성공, pool `0`, borrow flag 보존만 검사한다. admission을 주장하는 주석과
   `burstable == 0` tautology는 삭제한다.
2. `policy_contracts.rs`에 public broker contract를 추가한다.
   - global limit은 충분히 크게 둔다.
   - 대상 class: `soft=600`, `hard=800`, `borrow=true`, shared pool `0`.
   - public `reserve_managed_charge`로 정확히 800 bytes를 성공시킨다.
   - ledger snapshot을 저장한다.
   - 추가 1 byte reserve가 `MemoryError::Capacity`인지 확인한다.
   - denial 뒤 snapshot이 byte-for-byte 동일하고 total이 800인지 확인한다.
   - live charge drop/release 뒤 total이 0인지 확인한다.

내부 `Ledger::reserve`만 검사하거나 config field를 재계산해 예상값으로 쓰지 않는다.
builder/public broker → ledger admission → fail-closed snapshot 전체 경로가 authority다.

## Negative proof

temporary mutant로 zero pool에서 borrow class에 1 byte를 허용하도록 admission predicate를
바꾼다. public contract가 RED여야 한다. validation-only test 실패는 대체 증거가 아니다.

```bash
cargo test -p quanta-memory-platform-core --test policy_contracts zero_shared_pool -- --nocapture
cargo test -p quanta-memory-platform-core borrow_flag_with_zero_shared_pool
cargo test -p quanta-memory-platform-core --all-features
```

## Acceptance / reopen

- hard cap 성공, hard cap + 1 typed denial, unchanged snapshot, final zero가 모두 존재한다.
- public broker를 우회하거나 denial 뒤 state equality를 없애면 reopen한다.

## Historical owner proof (subsequently covered by the v5 source receipt)

- Mutant: `validate_reserve_postimage` pool allowance +1 (admits the
  over-cap byte).
- RED: `zero_shared_pool_cannot_extend_borrowing_class_hard_cap` FAILED
  at `policy_contracts.rs:310` (+1-byte `Capacity` denial missing).
  Failure is through the public broker, not the validation-only test.
- Post-revert: 1 passed; production byte-identical to HEAD.
