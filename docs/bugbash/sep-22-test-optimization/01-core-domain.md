# Track 1 — Verified core-domain work

Snapshot: `main@101781c2ea6122602b4acd9aa69ebfc13fda958f`.

## TO-05 — Reduce cardinality setup without weakening the bounded-preflight proof

- Evidence: `crates/quanta-memory-platform-core/src/domain/ledger.rs:1498-1527`.
- Measured warm exact-test time: **0.82 s**. The complete core library binary
  took **2.37 s**, so this single test is material.
- The semantic oracle is the instrumented affected-slot count (`3`), not 10,000
  populated owners. A full scan fails that assertion at much lower cardinality.
- Change: use a smaller but non-trivial owner population, retain the exact slot
  bound, and put large-cardinality throughput in a benchmark only if capacity
  evidence is still required.
- Acceptance: the existing result and affected-slot assertions are unchanged;
  warm exact-test time is recorded before/after.

## TO-06 — Make the zero-shared-pool claim behavioral

- Evidence: `crates/quanta-memory-platform-core/src/domain/policy.rs:662-689`.
- The test is named `borrow_flag_with_zero_shared_pool_is_valid_but_grants_no_burst`,
  but its final oracle only reads the configured zero back from
  `shared_burst_pool_bytes`. It never attempts an over-hard-cap reserve.
- Change: keep the policy-validation assertions and add a ledger reserve at the
  hard cap followed by a one-byte denial, with an unchanged snapshot assertion.
  Alternatively rename the test to claim only configuration preservation, but
  the behavioral denial is the stronger contract.
- Acceptance: a deliberately broken ledger that admits one borrowed byte with a
  zero pool must fail this test.
