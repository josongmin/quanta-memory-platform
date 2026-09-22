# QMP local-main integration — 2026-09-22

This is an integration record, not a replacement for per-ticket qualification
receipts or the status inventory in `tickets/README.md`.

## Source and ancestry

- Local `main` product/lint HEAD: `747f48a086ec3c5f19105ecbe5158d561b7bbf27`;
  tree: `ef887edfdbfff45b3e54d2127be61278011132a3`.
- `codex/qmp-remaining-hardening@4f321f7` entered local `main` through
  `ff766be`. The independent `codex/qmp130-upstream-hardening@a3e3228`
  entered ancestry through `b1f65aa`. That merge mistakenly committed conflict
  markers; `464fcb2` removed its entire tree delta, preserving the newer
  QMP-remaining contracts and tests. The final product tree equals `ff766be`.
- QMP-020 `4533146` and QMP-120 `359776b` (including their predecessors) were
  patch-equivalent to commits already on the integrated branch. `86d35ef` and
  `137e7f9` recorded their ancestry without restoring superseded content,
  including the deliberately removed GitHub workflow. Their local branch refs
  were then deleted. No QMP local branch remains unmerged into `main`.
- `747f48a` resolves the merged scaling benchmark's `unsafe` conflict with
  the workspace lint policy: production library roots retain `forbid`, the
  workspace default is `deny`, and the benchmark is the sole checked exception.

## Exact-source verification at `747f48a`

- `scripts/memory-platform-doctor`: exit 0 (fmt, clippy `-D warnings`,
  regression rules, feature matrix, advertised feature runtime checks,
  workspace test, doc selectors, examples, cargo deny).
- `scripts/run-scaling-bench.sh --smoke`: exit 0; 33 samples accepted.
- `scripts/extract-to-standalone-repo.sh --dry-run` against an empty temporary
  destination: exit 0, frozen source tree above. This is a pre-flight, not an
  actual extracted bootstrap.
- `python3 tools/pm/pm.py lint` and workspace-lint unit tests: exit 0.
- Worktree was clean after the source commit and before this record.

## Proof and publication boundary

- `scripts/verify-tickets.sh`: exit 1. All 28 validator unit tests pass, but
  terminal receipts are stale after integration-source changes. The first
  rejection is QMP-020 (`core/src/lib.rs`); the affected latest receipts are
  QMP-020, QMP-030, QMP-060, QMP-080, QMP-090, QMP-100, QMP-110, and QMP-120.
  The doctor and smoke results above do not silently requalify these tickets;
  each requires its own current-source acceptance proof/receipt or an explicit
  status transition. QMP-100 full benchmark baseline and QMP-120 actual
  extraction were not rerun for this integration.
- QMP-130 remains qualified only for its previously recorded exact host/QMP
  pair. The newer Semantica host `8e174d9` is local and unqualified; QMP-140
  host-main merge and activation remain blocked. This local QMP merge does not
  change that external host state.
- At this check, live `origin/main` was `b1f65aa`, whose facade `Cargo.toml`
  contains conflict markers. Local repairs were intentionally **not pushed**:
  the user requested local-main integration. Remote main therefore remains
  broken until a separately authorized push or another writer's repair.
