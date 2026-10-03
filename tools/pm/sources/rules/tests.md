## Test Topology

Tests must **catch real regressions** in budget accounting, policy resolution,
pressure/scheduling, and the portable stores — not merely import modules.

Placement:

| If it proves… | Put it in… |
| --- | --- |
| Ledger reserve/commit/release + its declared invariants | The `domain::ledger` test module, inline or in adjacent `ledger/tests/` files |
| Policy / manifest / preset / residency validation | The owning `domain::{policy,config}` test module, inline or adjacent |
| Pressure controller / scheduler behavior | The owning `application::{controller,scheduler}` test module, inline or adjacent |
| Public broker surface end-to-end (reserve→commit→snapshot→release→rebuild) | The `api::control` test module, inline or in adjacent `control/tests/` files |
| Cross-cutting policy contracts | `crates/quanta-memory-platform-core/tests/policy_contracts.rs` |
| A facade store / pipeline behavior | That feature's `service` test module, inline or in adjacent `service/tests/` files |
| A storage backing (redb) round-trip | The backing test module under its feature, inline or adjacent |
| "the conformance suite actually bites" | `testkit/src/conformance/{mod,suites}.rs` tests and `testkit/tests/facade_conformance_bite.rs` |
| Deterministic fault/chaos scenarios | `crates/quanta-memory-platform-testkit/src/faults/` |

Conventions:

- **Feature-gate tests** with the feature they exercise; `--all-features` must be
  green and each feature must build alone.
- Keep private-state tests in their owning Rust module. An adjacent test file or
  subtree may use ordinary `mod tests;`; do not move a large inline test block
  into one equally large file. Update documented selectors, their source-owner
  map, and capability selectors whenever a nested module changes test names.
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
