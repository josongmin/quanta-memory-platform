# TO-03 — Make the Scaling Allocation Epoch Measure Admission Only

Status: `IMPLEMENTED; final exact-source receipt pending`
Priority: `P0`
Depends on: none

## Problem

`scheduler_admission_group_cardinality`는 allocation epoch를 연 뒤 `BTreeMap`, group
strings, scheduler, target, latency buffer를 만든다. 결과의 allocation sample은 admission
loop가 아니라 fixture construction을 포함한다. 단순히 `begin_allocation_epoch`만 아래로
옮기고 scheduler를 epoch 안에서 drop하면, 추적 전에 allocate한 객체를 추적 중
deallocate하여 counter underflow/오염이 생긴다.

## Write set

- `crates/quanta-memory-platform-facade/benches/scaling.rs`
- emitted JSON schema와 benchmark name은 변경하지 않는다.

## File-by-file implementation plan

### `crates/quanta-memory-platform-facade/benches/scaling.rs`

- Edit owner: `scheduler_admission_group_cardinality`와, panic-safe disable이 필요할 때만
  test/bench-local allocation epoch helper.
- Setup phase로 이동할 statements: `BTreeMap` 생성/채움, target `String`, scheduler 생성,
  `record_usage`, `Vec::with_capacity(entries)`.
- `begin_allocation_epoch`는 setup 직후, `Instant::now()`와 admission loop 직전에 호출한다.
- epoch 시작 직후 allocated/deallocated/live가 0인 clean precondition을 assert한다.
- loop 안에는 `scheduler.admits`, latency timestamp, preallocated vec push만 남긴다.
- loop 종료 직후 counters unchanged를 확인하고 `end_allocation_epoch`를 호출한다.
- `summarize_latencies`, `drop(scheduler)`, `drop(target)`은 epoch가 꺼진 뒤 실행한다.
  추적 전 allocate한 값을 추적 중 drop하지 않는다.
- `emit`의 operation/mode/entries/threads/schema field는 byte-for-byte 유지한다.
- scoped guard를 도입하면 `finish()`가 sample을 한 번만 반환하고 `Drop`은 tracking flag만
  fail-safe로 내리게 한다. 전역 counter를 reset하거나 다른 scenario allocation을 삼키지
  않는다.
- TO-02가 같은 파일의 `revocable_fill_and_drain`을 후속 수정하므로 이 파일을 먼저
  넘기고 postimage digest를 기록한다.
- File-local verification: `cargo bench -p quanta-memory-platform-facade --features payload_store,revocable_store --bench scaling --no-run`.

## Phase ownership

1. Tracking disabled: groups, target, scheduler, `Vec::with_capacity(entries)` 구성 및 seed usage.
2. `begin_allocation_epoch`; counters가 0인지 확인.
3. Admission loop만 실행. loop 전/후 allocated/deallocated가 동일해야 한다.
4. `end_allocation_epoch`로 tracking을 먼저 끄고 sample을 확정한다.
5. Tracking disabled 상태에서 latency sort/summarize와 scheduler/target/vector drop.
6. 기존 schema로 emit한다.

panic 시 tracking flag가 다음 scenario로 새지 않는지 확인한다. 현재 begin/end API가 이를
보장하지 않으면 test/bench-only scoped epoch guard를 추가해 `Drop`에서도 tracking을
비활성화하되, sample을 읽는 정상 경로는 명시적 `finish()`로 유지한다.

## Required assertions

- admission loop의 `allocated == 0`, `deallocated == 0`
- measurement 종료 뒤 tracker inactive
- cardinality별 admit(1)/deny(2) semantics 불변
- JSON 필드명, units, scenario label 불변

production scheduler를 fake하거나 allocator assertions를 삭제하지 않는다.

## Verification

```bash
cargo bench -p quanta-memory-platform-facade --features payload_store,revocable_store --bench scaling --no-run
```

clean/frozen commit에서:

```bash
scripts/run-scaling-bench.sh --smoke
```

출력 artifact에서 `scheduler_admission_group_cardinality`의 allocation/deallocation이 0이고
모든 configured cardinality row가 존재하는지 확인한다.

## Negative proof / reopen

temporary mutant로 `Vec::with_capacity` 또는 scheduler construction을 epoch 내부로 되돌렸을
때 non-zero allocation assertion이 RED여야 한다. 추적 전 allocate/추적 중 drop 패턴이
남거나 JSON 소비자 schema가 바뀌면 reopen한다.

## Closeout receipt (owner-verified, IMPLEMENTED — annotated receipt owed)

- `cargo bench -p quanta-memory-platform-facade --features
  payload_store,revocable_store --bench scaling --no-run` builds
  (`scaling-25a2413b61efab4d`).
- Mutant: `Vec::with_capacity(entries)` moved back inside the epoch →
  bench `--smoke` binary exit 101, panic at `scaling.rs:571`
  (`allocation epoch must start with zero tracked allocations`,
  `left: 8000, right: 0`). Binary invoked directly (wrapper refuses
  dirty trees); oracle produced the RED. Post-revert: clean tree.
