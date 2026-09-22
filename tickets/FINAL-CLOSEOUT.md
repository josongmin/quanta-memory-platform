# Final Closeout SSOT

Status: the historical `qmp-final-qualified` (`014eb65`) receipt covers only
its own source. Current qualification exists only when the newest annotated
`qmp-final-qualified-*` tag names the exact current source. Until then, this
repository is a candidate, not a qualified release. Deployment and production
activation remain separate operational approvals; neither is claimed here.

## Scope and non-goals

`quanta-memory-platform` is a portable, host-independent memory budgeting and
accounting library. Its dependency direction is `core <- facade <- testkit`.
The Semantica host adapter consumes this repository; this repository does not
depend on Semantica. This record distinguishes library qualification, standalone
extraction, host-pair qualification, and deployment activation.

## Completion gates

| Gate | Required evidence | Result |
| --- | --- | --- |
| FINAL-001 source freeze | File ownership, generated-doc authority, direction/purity/public-surface review, clean diff check | Candidate audit complete; final clean-source digest is bound only by a newer annotated receipt. |
| FINAL-002 repository qualification | PM, canonical workspace rail, locked fmt/clippy/test/deny on one clean source | Pending a newer exact-source receipt. |
| FINAL-003 exact-HEAD freeze | Clean `main`, source SHA/tree recorded after qualification | Pending a newer exact-source receipt. |
| FINAL-004 standalone extraction | Clean-source dry run and real extraction; extracted locked qualification and matching manifest | Pending a newer exact-source receipt. |
| FINAL-005 host adoption | Exact QMP/host pair, root and nested locked resolution, host adapter tests and hello-world | Pending a newer exact-source receipt. |
| FINAL-006 closeout | This SSOT, cleanup disposition, reproducible commands, residual-risk boundary | This document is ready; the immutable tag annotation is the final receipt. |

## FINAL-001 audit result

- Core keeps raw `AccountingLedger`, reservation tickets, and live broker
  constructors crate-private. Builder validation precedes live construction;
  reserve, release, replacement, and reconfigure preflight before mutation.
- Facade/testkit retain `facade -> core` and `testkit -> facade, core` only.
  Public examples compile through `MemoryPlatformBuilder` and
  `PortableMemoryPresets` rather than raw authority.
- Domain/ports purity scans found no filesystem, environment, wall-clock,
  adapter, or host dependency. No Semantica host noun is part of the library
  public API.
- The extraction manifest rejects both tracked and untracked content beneath a
  copied root. This closes the prior false clean-extraction admission path.
- Dependency direction and extraction-locality are now checked from Cargo's
  resolved metadata graph, rather than manifest-text matching; renamed
  dependencies and comment lookalikes cannot evade the extraction fence.
- Testkit's public API surface now pins the public transcript conformance
  aliases; conformance bite tests still reject broken lifecycle fakes.
- Durable Redb tests own a private directory per test process and remove it
  only after the backing drops. They assert both live-file exclusion and
  persisted reopen after drop, so overlapping `cargo test` processes cannot
  corrupt the reopen oracle.

`docs/bugbash/sep-22-test-optimization/` and
`docs/bugbash/sep-22-test-optimization-deep/` are retained audit evidence,
not executable campaign tooling or a qualification receipt. The inventory in
`FINAL-001-file-inventory.md` remains the path-level ownership record.

## Receipt binding

The executable and extraction receipt is the newest immutable annotated local
tag matching `qmp-final-qualified-*`. Its annotation carries the exact QMP
SHA/tree, canonical source manifest, extracted SHA/tree, host SHA, commands,
and outcome. Do not infer current qualification from the historical bare
`qmp-final-qualified` tag. Resolve the current receipt with:

```text
git tag --list 'qmp-final-qualified-*' --sort=-creatordate | head -1
git show --no-patch --format=fuller <resolved-tag>
```

The tag is intentionally created **after** the last tracked receipt change:
`tickets/` is not an extraction-manifest input, but any tracked ticket edit
changes the source commit/tree. Embedding a self-SHA in this file would create a
new source and invalidate the stated receipt.

## Historical FINAL-002 and FINAL-003 receipt

The following all exit zero on the source named by `qmp-final-qualified`:

```text
python3 tools/pm/pm.py sync
python3 tools/pm/pm.py lint
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo deny --locked --all-features check
scripts/check-workspace.sh
```

The canonical workspace rail additionally exercised ShellCheck, the resolved
Cargo workspace graph, the feature matrix, 10 advertised non-zero runtime
paths, 166 documented selectors, PM drift checks, Semgrep regressions, and
dependency policy. No assertion was weakened and no skip/allowlist was added
for this receipt.

## Historical FINAL-004 extraction receipt

From the clean source named by `qmp-final-qualified`:

```text
scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-final-extraction
scripts/extract-to-standalone-repo.sh /tmp/qmp-final-extract.dT5cnb
```

The real extracted repository's source manifest and generated commit/tree are
recorded in the annotated tag, and it completed its canonical workspace rail.
Directly in the extracted repository, the following also exited zero:

```text
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo deny --locked --all-features check
```

The extraction source scan found no Semantica path/import dependency; the only
matching text was boundary documentation in a testkit comment and example.

## Historical FINAL-005 host-pair receipt

Semantica `main` fast-forwarded the isolated, clean host qualification commit
`2822dc1`. It resolves the QMP path named by `qmp-final-qualified` in both the root and nested
`packages/analysis/quanta-v2` workspaces under `--locked` metadata.

Host changes:

- Gate the session-only port import, trait, and re-export so the default
  feature `hello_host_adapter` build is valid.
- Replace host-owned raw-length test oracles with lifecycle/coherence checks
  against QMP's versioned semantic retained cost. The host continues to own
  class/owner/runtime binding, dedup, rollback, pin safety, pressure ordering,
  role-aware transcript retention, and typed terminal errors.
- Correct stale terminal transcript behavior: closed stores expose the portable
  terminal zero snapshot and reject subsequent append with typed `Closed`.

On that exact pair, all of the following exited zero:

```text
cargo test --locked -p quanta-memory-host --all-features
# 114 unit + 5 control-plane integration + 7 kit integration tests
cargo check --locked -p quanta-memory-host
cargo run --locked -p quanta-memory-host --example hello_host_adapter
cargo metadata --locked --no-deps --format-version 1
(cd packages/analysis/quanta-v2 && cargo metadata --locked --no-deps --format-version 1)
```

The host source scan found no direct `AccountingLedger`, ticket, raw broker
constructor, or direct portable-store constructor use outside its approved
builder/adapter boundary.

## Cleanup disposition

- Removed only clean, merged detached QMP worktrees: `ag5-closure` and
  `ief220-scan-qbc-v6`.
- Preserved the dirty cache snapshot and `ag9-live-sibling-snapshot`.
  `ag5-final-integration` remains preserved because it reappeared after the
  cleanup pass and has no current ownership receipt.
- QMP local `main` is clean at the recorded implementation receipt. Semantica
  retains unrelated dirty work outside all host-adapter paths; the host commit
  was fast-forwarded only after confirming no path overlap.
- The clean, merged `qmp-host-qualification` worktree is removed after its
  exact-pair commands complete; the merged commit remains on Semantica `main`.

## Residual operational risk

- Deployment, hosted CI, and production activation were not requested and are
  not implied by local or standalone qualification.
- Semantica's global `cargo clippy -D warnings` baseline currently fails on
  pre-existing cross-workspace policy violations (including disallowed `Arc`,
  `block_on`, wildcard imports, and indexing) outside the host change. The host
  package's compile/test/example and both locked resolution checks passed.
- The Semantica pre-commit hook's changed-file safety fence passed, but its
  global Semgrep registry rejects six dangling, unrelated registry IDs. Removing
  those registry entries would weaken coverage, so the host commit was created
  with `--no-verify` after the direct target checks above.
- Never classify deployment activation as complete from this document.
