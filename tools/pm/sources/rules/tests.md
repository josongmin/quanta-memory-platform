## Test Topology

Tests must **catch real regressions** in budget accounting, policy resolution,
pressure/scheduling, and the portable stores — not merely import modules.

Placement:

| If it proves… | Put it in… |
| --- | --- |
| Ledger reserve/commit/release + its declared invariants | `crates/quanta-memory-platform-core/src/domain/ledger.rs` `#[cfg(test)]` |
| Policy / manifest / preset / residency validation | `crates/quanta-memory-platform-core/src/domain/{policy,config}.rs` `#[cfg(test)]` |
| Pressure controller / scheduler behavior | `crates/quanta-memory-platform-core/src/application/{controller,scheduler}.rs` `#[cfg(test)]` |
| Public broker surface end-to-end (reserve→commit→snapshot→release→rebuild) | `crates/quanta-memory-platform-core/src/api/control.rs` `#[cfg(test)]` |
| Cross-cutting policy contracts | `crates/quanta-memory-platform-core/tests/policy_contracts.rs` |
| A facade store / pipeline behavior | that feature's `service.rs` `#[cfg(test)]` |
| A storage backing (redb) round-trip | the backing module under its feature |
| "the conformance suite actually bites" | `testkit/src/conformance/{mod,suites}.rs` tests and `testkit/tests/facade_conformance_bite.rs` |
| Deterministic fault/chaos scenarios | `crates/quanta-memory-platform-testkit/src/faults/` |

Conventions:

- **Feature-gate tests** with the feature they exercise; `--all-features` must be
  green and each feature must build alone.
- Observe the claimed authority: exact identity/content/accounting postimages,
  independent expected values and actual measured work for complexity claims.
  Follow `docs/adr/0005-regression-oracle-acceptance.md` for regression oracles.
- A test name, coverage or `Ok` alone does not prove the claimed behavior.
  Mutation evidence requires a contract-breaking, buildable change and the
  intended test's baseline / mutant / restored outcomes with executed counts.
  Compiler errors and unrelated assertion failures are not behavioral proof.
- Subprocess selectors must execute exactly the intended child test; a
  successful zero-test exit is invalid evidence. Verify its terminal result.
- When adding/changing a static regression guard, exercise the real engine with
  repository-shaped owner paths. Cover the original bad form, relevant bypasses
  (renamed bindings, compound operations or broad module exceptions), and
  permitted neighboring forms. Config parsing or one representative match is
  insufficient. Keep these contracts in `scripts/tests/test_regression_rules.py`
  and `scripts/tests/fixtures/semgrep/`; compiler rejection and behavioral
  rejection are separate proofs. Do not add redundant tests when an existing
  contract already distinguishes the changed behavior.

The cheatsheet owns assertion/ignore conventions. The determinism section owns
the conformance negative-pole requirement and invariant semantics.
