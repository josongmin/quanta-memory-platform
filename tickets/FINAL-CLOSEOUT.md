# Final Closeout SSOT

Status: active. This document is the sole closeout record for the current
hardening set; it is not a replacement for a source-bound qualification receipt.

## Scope and non-goals

The repository provides a portable, host-independent memory budgeting and
accounting library. Its dependency direction is `core <- facade <- testkit`.
Semantica host adoption, deployment, and activation are separate authorities.

## Completion gates

| Gate | Owner | Required evidence | Status |
| --- | --- | --- | --- |
| FINAL-001 source freeze | repository | Every diff path classified; generated files owned by `tools/pm/sources`; no unexplained deletion or upward dependency | committed set passed; untracked copied inputs pending |
| FINAL-002 repository qualification | repository | `pm sync`, `pm lint`, `check-workspace`, locked clippy/test/deny on one source tree | passed locally; source not yet clean |
| FINAL-003 exact-HEAD freeze | repository | One reviewed commit, clean worktree, HEAD/source digest recorded after verification | blocked by active untracked inputs |
| FINAL-004 standalone extraction | repository | Actual extraction plus extracted-repo locked qualification and matching manifest digest | blocked on FINAL-003 |
| FINAL-005 host adoption | Semantica | Both workspace roots resolve the pinned QMP source; adapter suites use it | not started; outside this repository |
| FINAL-006 closeout | repository | This document updated with exact SHA, command receipts, residual risks, and cleanup decision | active |

## Source-freeze findings

- Core authority is builder-first: raw `AccountingLedger`, tickets, and live
  broker constructors are crate-private. Reserve/release/reconfigure paths
  preflight before mutation and are covered by invariant tests.
- Facade and testkit preserve `facade -> core` and `testkit -> facade, core`.
  Feature gates and public examples use the builder/preset surface.
- The extraction manifest now rejects tracked **and untracked** files under any
  copied root. A source checkout with untracked copied inputs is not eligible
  for a clean exact-HEAD extraction claim.
- `docs/bugbash/sep-22-test-optimization/` is retained as a bounded audit
  record, not ticket-campaign tooling. Its index links the five scoped reports.
- `FINAL-001-file-inventory.md` classifies every path in the reviewed committed
  hardening set; later untracked inputs remain explicitly outside that set.

## Local qualification receipt (not an exact-HEAD receipt)

The following passed after the extraction guard and public-surface pin changes:

```text
python3 tools/pm/pm.py sync
python3 tools/pm/pm.py lint
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo deny --locked --all-features check
scripts/check-workspace.sh
```

`check-workspace` additionally completed its feature matrix, 10 non-zero
advertised-feature runtime paths, 166 documented test selectors, prompt-manager
drift check, Semgrep regression guards, and dependency policy checks. This is
local qualification only: it cannot become an exact-HEAD receipt until the
copied roots are clean and the resulting commit is re-qualified.

## Untracked copied-root admission

The current copied `docs/` root contains audit inputs that must be resolved
before FINAL-003:

| Path | Classification | Required decision |
| --- | --- | --- |
| `docs/bugbash/sep-22-test-optimization/04-facade-pipeline-spill-shared.md` | completed v1 track report | review, then include with its v1 index or remove deliberately |
| `docs/bugbash/sep-22-test-optimization/99-index.md` | completed v1 synthesis | review, then include with its v1 reports or remove deliberately |
| `docs/bugbash/sep-22-test-optimization-deep/` | incomplete v2 scaffold and timing evidence | finish as a bounded report set, or move it outside copied roots; do not silently exclude it |

The extractor rejects all three while they remain untracked. This is expected:
`git archive HEAD` cannot preserve untracked files, so allowing them would make
the frozen-source claim false.

## Current checkpoint

- A committed hardening checkpoint exists locally; populate the exact SHA/tree
  only after the copied roots are clean and FINAL-003 can issue its receipt.
- `scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-final-extraction`
  correctly rejects the remaining untracked copied-root inputs.
- Removed clean, merged detached worktrees: `ag5-closure` and
  `ief220-scan-qbc-v6`.
- Preserved dirty snapshots: the cache checkout and `ag9-live-sibling-snapshot`.
  `ag5-final-integration` reappeared after cleanup and is retained until its
  current owner is known.

## Required receipt fields before closure

Record all values only after FINAL-003 and FINAL-004 succeed:

```text
source HEAD:
source tree:
source manifest digest:
extracted HEAD:
extracted manifest digest:
repository qualification commands and exit status:
host QMP SHA / Semantica SHA pair:
```

## Residual operational risk

- A local rail does not prove Semantica resolution, host adapter behavior,
  deployment, or activation.
- Full extraction is intentionally fail-closed until copied roots have no
  staged, unstaged, mode-only, or untracked drift.
- Do not delete detached worktrees unless each is clean and its HEAD is an
  ancestor of `main`; preserve non-ancestor or dirty snapshots.
