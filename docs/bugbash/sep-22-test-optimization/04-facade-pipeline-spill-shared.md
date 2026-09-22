# Track 4 — Verified pipeline/spill/shared work

Snapshot: `main@101781c2ea6122602b4acd9aa69ebfc13fda958f`.

## TO-01 — Fix the 20-second redb concurrency hotspot

- Evidence: `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:1213-1251`.
- Nightly full-suite timing recorded **10.946 s**; an isolated warm rerun under
  current host load recorded **20.10 s**. The full facade library binary in the
  latter run took **49.34 s**.
- The test performs 8×25 durable writes and then 200 durable restore/finalize
  cycles. The contract is concurrent record-id allocation, no lost write, unique
  keys, exact multiset recovery, and final zero inventory; it does not require
  200 fsync-backed records.
- Change: add a start barrier so writers demonstrably overlap, then use a small
  count (for example 4 writers × 2 records). Preserve exact multiset and final
  inventory assertions.
- Acceptance: the same-host warm exact test is at least 80% faster and still
  fails a backing that allocates record IDs outside the serialized transaction.

## TO-02B — Replace public-contract string parsing

- Evidence: `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:84-133`.
- `accounting_mode_negative_contracts_remain_in_the_public_source` greps source
  for absent methods/re-exports even though the store types already have
  `compile_fail` examples.
- `full_and_default_features_do_not_enable_unbudgeted_stores` parses TOML and a
  wrapped README sentence with `contains`/`split_once`. The authoritative rail
  already uses `cargo metadata`/`cargo tree` in
  `scripts/verify-feature-matrix.sh`.
- Change: retain compile-fail API tests and move all aggregate-feature assertions
  to the metadata-backed script. Remove README formatting from executable tests.

## TO-03 — Move setup outside the benchmark allocation epoch

- Evidence: `crates/quanta-memory-platform-facade/benches/scaling.rs:511-546`.
- `scheduler_admission_group_cardinality` calls `begin_allocation_epoch` before
  building the formatted group map and scheduler. The later no-allocation check
  snapshots counters only after setup, so the epoch's reported allocation totals
  include setup even though admission itself is the measured operation.
- Change: build groups/scheduler/target first, begin the epoch immediately before
  the admission loop, and keep the before/after no-allocation assertion around
  that loop.

## TO-10 — Make redb temp databases process-safe and RAII-cleaned

- Evidence: `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:886-950`
  and repeated cleanup in `:959-1280`.
- Paths use a process-local sequence without a PID/random component; concurrent
  test processes can choose the same path. Manual pre/post `remove_file` leaks
  files on assertion panic.
- Change: use a test-only RAII temp-db owner with a process-unique path and `Drop`
  cleanup. Keep reopen tests able to close and reopen the same owned path.
