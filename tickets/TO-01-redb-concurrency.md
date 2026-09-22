# TO-01 — Keep the redb Race Oracle, Remove the 200-Transaction Hotspot

Status: `IMPLEMENTED; final exact-source receipt pending`
Priority: `P0`
Depends on: none; use `TestDb` landed in `014eb65`

## Problem and invariant

`concurrent_accepts_lose_no_handle_and_assign_distinct_keys`는 8 threads × 25 writes와
200 restores를 수행해 단독 10.946–20.10초를 소비한다. 중요한 의미는 transaction
개수가 아니라 concurrent writer가 겹쳤을 때 다음 불변식이 유지되는지다.

- record ID가 write transaction의 serialized authority에서 발급된다.
- 모든 성공 write의 sequence가 유일하고 연속적이다.
- accepted payload가 유실/중복 없이 정확히 한 번 restore된다.
- drain 후 durable inventory가 0이다.

## Write set

- `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs`
- 현재 `TestDb` RAII fixture를 사용한다. production transaction code는 동작 결함이 발견되지
  않는 한 바꾸지 않는다.

## File-by-file implementation plan

### `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs`

- Edit owner: test module의
  `concurrent_accepts_lose_no_handle_and_assign_distinct_keys`만 수정한다.
- Reuse unchanged: `TestDb`, `test_db`, `spill_backing`, `restore_backing`.
- Replace `threads=8`, `per_thread=25` with named constants `WRITERS=4`,
  `WRITES_PER_WRITER=2`; expected total은 두 상수의 checked multiplication에서 만든다.
- `Arc<Barrier>::new(WRITERS + 1)`를 만들고 각 worker가 barrier에서 대기한 다음 main이
  마지막 participant로 release한다.
- worker return type을 `Vec<(Vec<u8>, SpillReceipt)>`로 고정한다. thread 내부에서 receipt를
  버리지 않는다.
- join 단계에서 panic을 먼저 전파한 뒤 모든 payload/receipt를 한 collection으로 합친다.
- `SpillReceipt::Durable { sequence, .. }`만 허용하고 sequence set을 `0..expected`와 exact
  비교한다. 다른 receipt variant는 즉시 panic한다.
- expected payload multiset과 restore된 payload multiset을 exact 비교한다. count-only
  assertion은 보조로만 둔다.
- `retained_count == expected`와 final `0`을 유지한다.
- 같은 파일의 crash/reopen/exhausted-keyspace tests, `TestDb` lifecycle, production
  `reserve_record_id`/`commit_spill`은 수정하지 않는다.
- File-local verification:
  `cargo test -p quanta-memory-platform-facade --features spill-redb concurrent_accepts_lose_no_handle_and_assign_distinct_keys -- --nocapture`.

## Implementation

- sample을 4 writers × 2 records로 축소한다. 최소 2 writers/2 writes보다 낮추지 않는다.
- `Barrier(writers + 1)`로 모든 writer를 준비시킨 뒤 main thread가 동시에 시작시킨다.
  thread spawn 자체를 concurrency 증명으로 취급하지 않는다.
- 각 thread는 `(payload, SpillReceipt)`를 반환한다. receipt를 버리지 않는다.
- join 후 sequence set이 정확히 `0..N`이고 receipt sequence가 모두 unique인지 확인한다.
- expected payload와 drained payload를 `BTreeSet`/multiset으로 비교한다. 개수만 비교하지
  않는다.
- `retained_count == N`과 drain 후 `retained_count == 0`을 모두 유지한다.
- sleep, retry, relaxed final assertion, in-memory substitute를 넣지 않는다.

## Negative proof

temporary mutant 하나로 record ID 계산을 serialized transaction 밖으로 옮기거나 동일
key를 반환하게 만든다. focused test가 duplicate/loss/sequence assertion에서 RED가 되는
로그를 남긴 뒤 mutant를 제거한다. compile failure는 negative proof가 아니다.

## Verification

```bash
cargo test -p quanta-memory-platform-facade --features spill-redb concurrent_accepts_lose_no_handle_and_assign_distinct_keys -- --nocapture
cargo test -p quanta-memory-platform-facade --all-features spill::backings::redb
```

동일 warm binary에서 before/after를 각각 3회 측정해 median을 기록한다. 목표는 기존
median 대비 80% 이상 감소다. 임계값은 코드 assertion이 아니라 closeout receipt다.

## Acceptance / reopen

- Barrier가 없거나, receipt sequence를 검증하지 않거나, exact payload round-trip/zero
  inventory 중 하나라도 빠지면 미완료다.
- runtime만 줄고 temporary mutant가 GREEN이면 오라클이 약해진 것이므로 reopen한다.

## Closeout receipt (owner-verified, IMPLEMENTED — annotated receipt owed)

- Mutant: `reserve_record_id` returns constant duplicate ID 0.
- RED: `concurrent_accepts_lose_no_handle_and_assign_distinct_keys` FAILED,
  worker threads panicked at `redb.rs:1243` (duplicate-key commit rejected).
- Post-revert: 1 passed (0.42s); production byte-identical to HEAD.
- Medians (same host/profile, warm, 3 runs): before 6.53s → after 0.76s
  (~88% reduction; target ≥80%). No time assertion in code.
