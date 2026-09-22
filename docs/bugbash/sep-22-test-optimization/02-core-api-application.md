# Track 2 — Verified core API/application work

Snapshot: `main@101781c2ea6122602b4acd9aa69ebfc13fda958f`.

## TO-07 — Close the builder-first wiring gap

- Evidence: `crates/quanta-memory-platform-core/src/api/builder.rs:74-113`.
- `with_event_sink` is exercised through `ProcessMemoryBrokerBuilder`, but no
  test calls `with_reclaimer`, and no test calls the public compatibility alias
  `build_live`.
- This matters because the repository contract requires consumers to use the
  builder surface; tests that call `LiveProcessMemoryBroker` internals do not
  prove builder wiring.
- Change: one builder test should install a recording reclaimer, call
  `build_live`, cross a soft threshold, and assert the callback and accounting
  postimage. Do not add separate compile-only tests for each method.

## TO-08 — Remove confirmed dead/duplicate assertions and stale contract text

- `crates/quanta-memory-platform-core/src/ports/telemetry.rs:235-243` is exactly
  the `(2000, 1000, 1000, true)` row already covered by the table at
  `:272-286`. Delete the single-case test; retain the table.
- `crates/quanta-memory-platform-core/src/api/control.rs:2088-2090` asserts
  `== 400_000` and then `!= 600_000`. Delete the second assertion.
- `crates/quanta-memory-platform-core/tests/public_api_surface.rs:174-179` says
  “eight” while the exhaustive public contract and test name contain nine
  variants. Change the comment to “nine”.

No concurrency-count reduction remains in this track. Warm measurements were
0.06 s for the 16×100 reserve test, 0.01 s for reserve/reconfigure, 0.09 s for
the 10k-group scheduler test, and below timer resolution for the reference
model. Those are not current optimization problems.
