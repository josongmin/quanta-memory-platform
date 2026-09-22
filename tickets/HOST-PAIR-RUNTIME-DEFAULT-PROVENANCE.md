# HOST-PAIR-001 — Semantica default budget provenance projection

Status: `OPEN — external host repository`
Priority: `P1 for FUP-006 host-pair closure; not a QMP repository defect`
Owner repository: `/Users/songmin/Documents/code-new/semantica-codegraph-v2`
Blocked by: Semantica shared-main writer/integrator ownership
Blocks: QMP `FUP-006` host-pair GREEN only

## Current-source evidence

QMP source `beba43760d76a60fbce7ccbf8ed97d3985aeaaf3` resolved from both
Semantica workspace roots to the same path crates:

- `/Users/songmin/Documents/code-new/quanta-memory-platform/crates/quanta-memory-platform-core/Cargo.toml`
- `/Users/songmin/Documents/code-new/quanta-memory-platform/crates/quanta-memory-platform-facade/Cargo.toml`

`quanta-memory-host --all-features --tests --locked` passed all owner tests:
`114 + 5 + 7`, zero failures. The downstream composition comparison selected
44 tests and returned `43 passed / 1 failed`:

```text
tests::runtime_memory_bootstrap_core_owns_production_budget_and_live_dependencies
left:  []
right: ["runtime_defaults"]
```

The mismatch is produced inside the Semantica projection path, not by the QMP
broker. `RuntimeCutoverMemoryDeps::effective_memory_budget_report_v1` creates
an inherit overlay and returns
`ValidatedMemoryBudgetSelectionV2::effective_report()`; that report contains
only explicit overlay sources. With `selection == None`, no source is added.
The adjacent validation path already constructs the canonical default report
with `["runtime_defaults"]`, so the two host paths disagree.

The Semantica checkout was already dirty in unrelated files during this audit.
Do not fold this one-file repair into an unowned `git add -A` publication.

## Exact owner files

| File | Required change |
| --- | --- |
| `packages/analysis/quanta-v2/crates/quanta-runtime-memory-compose/src/lib.rs` | In `RuntimeCutoverMemoryDeps::effective_memory_budget_report_v1`, preserve `runtime_defaults` only for the no-selection/default projection. Keep explicit overlay reports limited to the actual `overlay.*` sources. |
| `packages/analysis/quanta-v2/crates/quanta-runtime-memory-compose/src/lib_tests.rs` | Keep the existing default-provenance assertion and the existing exact overlay-source assertion as paired positive/negative oracles. Add no string-grep test. |

## Implementation contract

1. Capture `selection.is_none()` before consuming the selection.
2. Build and validate the selected report through
   `ValidatedMemoryBudgetSelectionV2` exactly as today; do not bypass overlay
   admission or construct a second budget calculator.
3. If and only if this is the default/no-selection path, set
   `override_sources` to exactly `["runtime_defaults"]`.
4. Do not prepend `runtime_defaults` to an explicit overlay report. The
   existing overlay test must remain exactly:
   `overlay.class.session_transcript`, `overlay.global_limit_mb`,
   `overlay.scratch_mb`.
5. Do not change QMP's `MemoryBudgetEffectiveReport`, preset resolver, or
   provenance semantics to compensate for this host projection bug.

## Proof rails

Run from the Semantica repository with a dedicated QBC lane:

```bash
scripts/quanta-build-cli owner run \
  --lane qmp-host-default-provenance \
  --package quanta-runtime-memory-compose \
  --execution-kind test \
  --target-kind lib \
  --selector-mode exact \
  --selector tests::runtime_memory_bootstrap_core_owns_production_budget_and_live_dependencies \
  --compile-policy package-all-features

scripts/quanta-build-cli owner run \
  --lane qmp-host-default-provenance \
  --package quanta-runtime-memory-compose \
  --execution-kind test \
  --target-kind lib \
  --selector-mode exact \
  --selector tests::runtime_cutover_memory_budget_report_projects_runtime_defaults_and_code_overlay \
  --compile-policy package-all-features

scripts/quanta-build-cli owner run \
  --lane qmp-host-default-provenance \
  --package quanta-runtime-memory-compose \
  --execution-kind test \
  --target-kind lib \
  --selector-mode whole-target \
  --compile-policy package-all-features
```

Admission requires non-zero selection for both exact tests and all 44 library
tests GREEN on the same Semantica source digest. Re-run both workspace-root
`cargo metadata --locked` source checks in the final host receipt. Raw Cargo
results above are diagnostic evidence, not Semantica QBC qualification.

## Stop/reopen conditions

- Reopen if an explicit overlay starts reporting `runtime_defaults`.
- Reopen if either workspace root resolves a different QMP manifest path.
- Stop and hand back to the Semantica integrator if the owner file becomes
  dirty from another writer or QBC reports a source-lock/digest mismatch.
- QMP repository/extraction qualification may proceed independently, but
  `FUP-006 host-pair GREEN` must remain open until this ticket is closed.
