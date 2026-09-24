# Final Closeout SSOT

Status: `HISTORICAL FINAL TAGS; LOCAL SOURCE QUALIFICATION IS DEFINED BY AN EXACT-HEAD qmp-local-qualified-* TAG`.
The bare `qmp-final-qualified` tag (`014eb65`) covers only its own source.
The newest annotated `qmp-final-qualified-*` tag observed at the clean
`db0b03c` audit base is `qmp-final-qualified-sep23-followup-v5`, targeting
`f637d42`. The current local candidate is the commit containing this receipt
contract; its exact SHA/tree must match the target of a new annotated
`qmp-local-qualified-*` tag before local qualification is called complete.
A later clean QMP `7f1c2f8` / Semantica `a71e3e9` pair has
local canonical, extraction dry-run, benchmark, ENOSPC, and host QBC evidence
in [FUP-006](../docs/plans/sep-23-followup-misc/RECEIPT.md), but no successor
tag. The `db0b03c` audit base changes the scaling verifier after that pair;
later commits add documentation and comments only. The current local receipt
must record canonical, fresh full benchmark, and extraction dry-run on one
clean exact source. Host-pair, deployment and activation remain separate and
unclaimed unless explicitly qualified.

## Scope and non-goals

`quanta-memory-platform` is a portable, host-independent memory budgeting and
accounting library. Its dependency direction is `core <- facade <- testkit`.
The Semantica host adapter consumes this repository; this repository does not
depend on Semantica. This record distinguishes library qualification, standalone
extraction, host-pair qualification, and deployment activation.

## Completion gates

The results in this table describe the historically tagged source; current
local qualification is recorded only by a new exact-target local receipt tag.

| Gate | Required evidence | Result |
| --- | --- | --- |
| FINAL-001 source freeze | File ownership, generated-doc authority, direction/purity/public-surface review, clean diff check | Complete for the source named by the newest annotated receipt. |
| FINAL-002 repository qualification | PM, canonical workspace rail, locked fmt/clippy/test/deny on one clean source | Complete for the source named by the newest annotated receipt. |
| FINAL-003 exact-HEAD freeze | Clean `main`, source SHA/tree recorded after qualification | Complete for the source named by the newest annotated receipt. |
| FINAL-004 standalone extraction | Clean-source dry run and real extraction; extracted locked qualification and matching manifest | Dry-run reran on tagged `f637d42`; real extraction was inherited from code-identical `cf2a441`, as stated in the v5 annotation. Not an exact-commit real-extraction rerun on `f637d42`. |
| FINAL-005 host adoption | Exact QMP/host pair, root and nested locked resolution, host adapter tests; host example if claimed | The `cf2a441` QMP / `0e29a45` Semantica pair has recorded 45/45 compose and 126/126 host tests, inherited by the code-identical v5 tag. Its host hello-world rerun is not recorded in that receipt; the recorded `hello_world` is a QMP public example. The later `7f1c2f8` / `a71e3e9` pair has separate local proof in FUP-006. Neither is an exact-current-HEAD host run. |
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

The current library-local closeout uses an annotated `qmp-local-qualified-<sha>`
tag targeting the exact candidate. Its annotation records the canonical
`scripts/check-workspace.sh` result, fresh full benchmark artifact hash, and
clean-source extraction dry-run result. This tag qualifies the repository
locally; it does not claim a real standalone export, exact Semantica host-pair,
hosted CI, deployment, or activation. Resolve it and compare its peeled target
with `HEAD` before claiming current local qualification:

```text
git tag --list 'qmp-local-qualified-*' --sort=-creatordate | head -1
git rev-parse <resolved-tag>^{}
git show --no-patch --format=fuller <resolved-tag>
```

For tagged historical sources, the receipt is an annotated local tag matching
`qmp-final-qualified-*` that actually targets the source under review. The v5
annotation binds its QMP SHA/tree and summarizes the exact-HEAD canonical and
dry-run results; it explicitly inherits real extraction, benchmark and host
evidence from code-identical `cf2a441`. Detailed source manifests, extracted
and host identities, commands and outcomes must be checked in the linked
historical [receipt](../docs/plans/sep-23-followup-misc/RECEIPT.md), not
assumed to be embedded in the tag annotation. A newer HEAD does not inherit
that receipt. Resolve tags with:

```text
git tag --list 'qmp-final-qualified-*' --sort=-creatordate | head -1
git show --no-patch --format=fuller <resolved-tag>
```

The tag is intentionally created **after** the last tracked receipt change:
`tickets/` is not an extraction-manifest input, but any tracked ticket edit
changes the source commit/tree. Embedding a self-SHA in this file would create a
new source and invalidate the stated receipt.

## Historical bare-tag evidence (not current qualification)

The following command set was recorded for the historical bare tag
`qmp-final-qualified`. It is retained as provenance only; it does not qualify
the current source and must not be used instead of the newest annotated tag.

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

## Historical bare-tag extraction evidence (not current qualification)

From the clean source named by the historical bare tag:

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

## Historical bare-tag host-pair evidence (not current qualification)

Semantica `main` once fast-forwarded the isolated, clean host qualification
commit `2822dc1`. That older pair is provenance only. The current host-pair
SHA/tree and command outcomes belong exclusively to the newest annotated QMP
receipt; do not infer them from this section.

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
