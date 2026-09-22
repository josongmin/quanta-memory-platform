# HOST-PAIR-001 — Semantica default budget provenance projection

Status: `CLOSED — Semantica main implementation + source-bound QBC GREEN`
Priority: `P1 for FUP-006 host-pair closure; not a QMP repository defect`
Owner repository: `/Users/songmin/Documents/code-new/semantica-codegraph-v2`
Resolved by: Semantica commit `4767bcfec2b9e2e9269f423f2b29a3216166f4bd`
Qualification merge: `86e90abba4cb70d9bd7e7a4872a1ea5abce0a665`
Unblocks: QMP `FUP-006` host-pair boundary only; later unqualified QMP worktree deltas remain separate

## Closure receipt

The host-owned projection now records `runtime_defaults` if and only if the
caller supplies no selection. It still validates every explicit selection
through `ValidatedMemoryBudgetSelectionV2`, and it does not prepend the default
source to explicit overlay provenance. A new behavioral negative oracle covers
an explicit inherit selection so `None` and `Some(inherit)` cannot collapse
back into the same provenance classification.

Qualified on Semantica source `86e90abba4cb70d9bd7e7a4872a1ea5abce0a665`
with QBC source digest
`16504f75f8b28c86187b9b6b2e217378c6a462dbfecace91d840b3281e329c5f`:

- `quanta-runtime-memory-compose`, lib, all features: `45 passed / 0 failed`.
- `quanta-memory-host`, all tests, all features: `114 + 5 + 7` passed,
  `0 failed`.
- Both Semantica workspace roots resolved QMP core/facade `0.2.0` to
  `/Users/songmin/Documents/code-new/quanta-memory-platform/crates/...`.
- The exact default-provenance, explicit-overlay, and explicit-inherit
  behavioral tests all passed before the final whole-target rerun; all three
  are members of the final 45-test receipt.

QBC receipts:

- compose:
  `/Volumes/Extreme SSD 1/build-cache/semantica-codegraph-v2-target/quanta-build-cli-qmp-host-provenance/_state/execution-roots/semantica-codegraph-v2-qmp-host-provenance-cf8208a1f5d3/lanes/qmp-host-default-provenance/verification-results/20260922T232149.918358Z-637bd6b385f2/receipt.json`
- host:
  `/Volumes/Extreme SSD 1/build-cache/semantica-codegraph-v2-target/quanta-build-cli-qmp-host-provenance/_state/execution-roots/semantica-codegraph-v2-qmp-host-provenance-cf8208a1f5d3/lanes/qmp-host-default-provenance/verification-results/20260922T232244.387614Z-ccdc3e2fbe13/receipt.json`

The fix and qualification merge were pushed to Semantica `main`. The next
remote main commit `0d595a2a576ae05f75c476e0bc9035a396cbf0aa` changed only
`tools/quanta-build-cli/test_owner_execution_resolver_v1.py`; the two owner
files below were unchanged. Hosted CI, deploy, and activation remain distinct
and were not run.

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
- Reopen the host-pair boundary if any condition above occurs. QMP
  repository/extraction qualification and later dirty worktree deltas remain
  independently source-bound; this closure does not qualify those deltas.
