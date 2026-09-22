## Test Topology

Tests must **catch real regressions** in budget accounting, policy resolution,
pressure/scheduling, and the portable stores — not merely import modules.

Placement:

| If it proves… | Put it in… |
| --- | --- |
| Ledger reserve/commit/release + the 4 invariants | `core/src/domain/ledger.rs` `#[cfg(test)]` |
| Policy / manifest / preset / residency validation | `core/src/domain/{policy,config}.rs` `#[cfg(test)]` |
| Pressure controller / scheduler behavior | `core/src/application/{controller,scheduler}.rs` `#[cfg(test)]` |
| Public broker surface end-to-end (reserve→commit→snapshot→release→rebuild) | `core/src/api/control.rs` `#[cfg(test)]` |
| Cross-cutting policy contracts | `core/tests/wave1_policy_contracts.rs` |
| A facade store / pipeline behavior | that feature's `service.rs` `#[cfg(test)]` |
| A storage backing (redb) round-trip | the backing module under its feature |
| "the conformance suite actually bites" | `testkit` conformance + `self_tests` |
| Deterministic fault/chaos scenarios | `testkit/src/faults/` |

Conventions:

- **Conformance must bite.** The testkit's meta-requirement: feed a deliberately
  broken/lying fake into the public suite and assert it is REJECTED, alongside the
  honest-fake and real-broker positive cases. A suite that passes everything is
  worthless — preserve this when extending.
- **Assert invariants, not vibes.** After an op, assert the ledger invariants
  (`total_owner_bytes`, class/owner balance) hold — not just that a call returned
  `Ok`. No `assert!(true)` / `assert_eq!(x, x)` placeholders.
- **Assert on the `MemoryError` variant**, not on stringly-typed message
  substrings where avoidable.
- **Feature-gate tests** with the feature they exercise; `--all-features` must be
  green and each feature must build alone.
- `#[ignore]` needs a reason: `#[ignore = "<why>"]`.

Run before closeout:

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
scripts/regression-guards.sh          # semgrep regression guards (in check-workspace + doctor too)
scripts/check-workspace.sh
python3 -m pytest tools/pm/tests -q   # if you touched tools/pm
```

Regression guards (`scripts/regression-guards.sh`, rules in
`tools/semgrep/qmp-regressions.yml`) pin bug classes this repo already hit —
magnitude-to-signed casts, saturate-before-divide ratios, and underflowing
`fetch_sub`. A flagged-but-safe use needs a trailing
`// nosemgrep: <rule-id> -- reason: <…>`.
