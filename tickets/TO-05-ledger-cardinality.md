# TO-05 — Prove Bounded Release Preflight without 10,000 Owners

Status: `IMPLEMENTED; final exact-source receipt pending`
Priority: `P1`
Depends on: none

## Problem and keeper invariant

`single_release_preflight_is_bounded_by_affected_slots_at_10k_owners`는 10,000개 owner와
ticket을 구성하고 정리하는 데 약 0.82초를 쓴다. 실제 regression detector는 cardinality
자체가 아니라 single release가 정확히 다음 3개 slot만 preflight하는지다.

- class/kind
- owner/kind
- context/kind

단순히 cardinality를 작은 상수 하나로 바꾸면 “입력 크기와 무관”이라는 제목보다 약한
증명이 된다. 두 개의 서로 다른 non-trivial cardinality에서 probe 수가 동일함을 비교한다.

## Write set

- `crates/quanta-memory-platform-core/src/domain/ledger.rs` test module only
- production ledger/preflight code는 실제 결함을 재현하지 않는 한 변경하지 않는다.

## File-by-file implementation plan

### `crates/quanta-memory-platform-core/src/domain/ledger.rs`

- Edit only the `#[cfg(test)]` module around
  `single_release_preflight_is_bounded_by_affected_slots_at_10k_owners`.
- Extract test-local helper `assert_single_release_probe_bound(owner_count: usize)`.
- Helper builds a fresh ledger with global capacity at least `owner_count + 1`, reserves one byte
  per unique owner in `ArtifactCache`, and records every ticket.
- Release the final ticket, then assert exact probe count `3`, exact remaining owner cardinality,
  semantic/non-headroom totals, and `assert_axes_balanced`.
- Batch-release remaining tickets and assert zero totals plus balanced axes.
- Rename the test to `single_release_preflight_is_bounded_independent_of_owner_cardinality` and
  invoke helper for `32` and `128` owners.
- Delete the 10,000-owner fixture and old name. Do not change
  `preflight_release_batch`, `last_release_preflight_axis_probes`, map types, reservation/release
  behavior, or other ledger tests.
- File-local verification:
  `cargo test -p quanta-memory-platform-core single_release_preflight_is_bounded_independent_of_owner_cardinality -- --nocapture`.

## Implementation

- test helper `assert_single_release_probe_bound(owner_count)`를 만든다.
- 예: 32와 128 owners를 각각 구성한다. 최소값은 affected axes보다 충분히 큰 16 이상이다.
- 각 run에서 마지막 ticket 하나를 release하고 다음을 확인한다.
  - `last_release_preflight_axis_probes == 3`
  - owner map length가 정확히 `owner_count - 1`
  - semantic/non-headroom total이 정확히 `owner_count - 1`
  - `assert_axes_balanced(snapshot)`
  - 나머지 batch release 후 모든 total이 0이고 axes가 balanced
- test 이름에서 `10k`를 제거하고 cardinality independence를 명시한다.
- owner 수를 1로 낮추거나 cleanup assertions를 삭제하지 않는다.

## Negative proof

temporary mutant로 preflight에 전체 `owner_usage` scan을 추가하고 probe counter도 방문
slot마다 증가시킨다. 큰 cardinality run이 exact `3` assertion에서 RED여야 한다. 단순
wall-time 증가만으로는 negative proof로 인정하지 않는다.

## Verification

```bash
cargo test -p quanta-memory-platform-core single_release_preflight_is_bounded -- --nocapture
cargo test -p quanta-memory-platform-core --lib
```

기존 10k test와 새 test를 동일 warm binary에서 각각 5회 측정하고 median을 기록한다.
목표는 75% 이상 감소이며 코드에 시간 assertion을 넣지 않는다.

## Acceptance / reopen

- 서로 다른 두 cardinality 모두 exact 3-slot bound와 final drain을 검증한다.
- full-scan mutant가 GREEN이거나 owner cardinality dependence를 assertion에서 제거하면
  reopen한다.
