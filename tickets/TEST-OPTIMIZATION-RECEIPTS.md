# Test-Optimization Wave — Integrator Receipts

Status: `INTEGRATOR-COLLATED, NOT QUALIFIED` — no ticket is marked `DONE` by this file.

## 1. Integrator observation window

- Session HEAD walk (shared workspace, concurrent committers active):
  `415e659` → `3713dcc` → `41faf05b71af004835ade39690e07ce9ebddd6db` (final).
- Final clean-tree state: `git status --short` empty, `git diff --name-only` empty.
- Prior workflow child results (1–8) carried no inspected evidence
  (summaries only), so every per-ticket field below that needs ticket-author
  evidence is recorded as `UNRESOLVED`, not inferred.

## 2. Step 1 — diff write-set check

- Final `git diff --name-only`: empty (clean tree) → trivially satisfied,
  no violation pending.
- During the session two dirty states were observed and then absorbed by
  concurrent commits outside integrator control:
  (a) 36-file set including `.cursorrules`, `AGENTS.md`, `CHANGELOG.md`,
  `CLAUDE.md`, `tools/pm/**`, `scripts/tests/test_workspace_policy.py`,
  `scripts/extract-to-standalone-repo.sh` — most outside the
  `TEST-OPTIMIZATION-EXECUTION.md` writer leases (lease covers only the
  per-ticket owner files; `test_workspace_policy.py` and `tools/pm/**`
  and doc roots are not leased);
  (b) 4-file set (`CHANGELOG.md`, `docs/verification-scenarios.md`,
  `scripts/tests/test_workspace_policy.py`, `scripts/verify-feature-matrix.sh`).
- VIOLATION REPORT: at final state there is no pending violation (clean).
  Historical note: set (a) contained out-of-lease paths; they are now
  committed history (HEAD moved twice by other agents), not integrator
  changes. Re-verifying those committed contents against leases is left
  to ticket owners / audit.

## 3. Step 2 — `cargo fmt --all -- --check` (check only)

- Exit `0` on all three runs (two dirty states + final clean `41faf05`).

## 4. Step 3 — `scripts/check-workspace.sh` full rail

Rail run twice (dirty states at `415e659`-era tree and `3713dcc` tree);
final-tree post-guard stages re-run individually at `41faf05`:

| Stage | Result |
|---|---|
| fmt --check | pass |
| shellcheck | pass |
| resolved workspace graph (`cargo metadata --locked`) | pass |
| workspace policy | `OK` |
| Python contract tests | `OK` (34–35 tests) + `tools/pm` 13 tests `OK` |
| extraction-contracts.sh | all `PASS` |
| regression-guards (semgrep) | **BLOCKED, environmental** |
| feature-matrix | `OK (all feature combinations compiled warning-free)` |
| `cargo test --locked --workspace --all-features` | **12/12 suites ok, 642 passed, 0 failed** |
| feature-capabilities | `OK (10 advertised feature paths executed non-zero tests)` |
| doc-test-selectors | `OK (166 selectors resolve)` |

- Blocker detail: `semgrep` binary crashes at startup before reading any
  rule or source file:
  `Failure("Failed to create system store X509 authenticator: ca-certs:
  empty trust anchors...")` (OCaml `opentelemetry_client_cohttp_eio`,
  exit 2). Reproduced with `OTEL_SDK_DISABLED=true`; same crash.
  Unrelated to ticket changes — no ticket edits semgrep config, rule
  inputs, or the guard script. Rail `set -euo pipefail` stops there, so
  all later stages were executed individually (results above).
- Full rail exit code as a single command: `2` (solely the semgrep guard).

## 5. Step 4 — extraction dry-run

- SKIPPED with reason: `git diff --name-only -- Cargo.toml Cargo.lock
  'crates/*/Cargo.toml'` is empty at final state; no manifest, dependency,
  or dependency-edge change. The only `use`-line additions observed were
  function-local test imports (`MemoryError` in testkit test fns) inside
  the pre-existing testkit→core edge — no new cross-crate edge.
- Note (risk, not integrator action): an earlier dirty state contained a
  weakening of `scripts/extract-to-standalone-repo.sh` (removal of the
  `cargo metadata --locked | check-extracted-metadata.py` verification in
  the non-dry-run path). Now committed history. Owners should confirm
  whether that was intended; the resolved-graph check still runs in
  `check-workspace.sh`.

## 6. Per-ticket closeout collation (TO-01 … TO-09)

All nine ticket files remain `Status: OPEN` with specification content
only — no filled preimage/postimage HEAD, no digests, no exact
commands+exit codes, no mutant-RED excerpts, no after-medians recorded
in-repo. Prior child results supplied no inspected evidence for any of
these. Therefore per the execution SSOT none can be marked `DONE`.

| Ticket | Owner file(s) | Receipt state |
|---|---|---|
| TO-01 redb concurrency | `facade/src/spill/backings/redb.rs` | `UNRESOLVED`: no postimage HEAD/digest, no focused-test command log, no temporary-mutant RED excerpt, no after-median (before cited 6.53s; after unrecorded) |
| TO-02 contract authority | `scaling.rs` (after TO-03), payload/retained/revocable services, `facade/src/lib.rs`, facade `public_api_surface.rs`, `verify-feature-matrix.sh`, `verify-scaling-results.py`, `test_scaling_results.py` | `UNRESOLVED`: no compile-fail keeper logs, no mutant REDs (raw-ctor / feature-activation / linear-scan / quadratic-drain), no `rg` zero-grep proof |
| TO-03 scaling epoch | `facade/benches/scaling.rs` | `UNRESOLVED`: no bench `--no-run` log, no `Vec::with_capacity`-revert mutant RED |
| TO-04 fault driver negative pole | `testkit/src/faults/scenarios.rs` | `UNRESOLVED`: no `spill_failure` focused log, no success-arm→`Contained` mutant RED on the public driver |
| TO-05 ledger cardinality | `core/src/domain/ledger.rs` | `UNRESOLVED`: no bounded-probe test log, no full-scan mutant RED at exact-`3` assertion, no after-median (before cited 0.09s; after unrecorded) |
| TO-06 zero-pool admission | `core/src/domain/policy.rs`, `core/tests/policy_contracts.rs` | `UNRESOLVED`: no public-contract RED vs 1-byte-admission mutant |
| TO-07 builder reclaimer | `core/src/api/builder.rs` | `UNRESOLVED`: no `builder_reclaimer` behavior log, no `None`-reclaimer mutant RED |
| TO-08 core test cleanup | `core/src/ports/telemetry.rs`, `core/src/api/control.rs`, `core/tests/public_api_surface.rs` | `UNRESOLVED`: no keeper-implication diff proof for the removed duplicate/dead assertions |
| TO-09 transcript partitions | `facade/src/transcript_store/service.rs` | `UNRESOLVED`: no independent-partition oracle log, no mutant/keeper evidence |

- Integrator-observed substitute evidence (not a per-ticket closeout):
  final-tree `cargo test --locked --workspace --all-features` 642/642 green
  (counts: 264+199+78+23+20+18+15+13+7+4+1+0+0 across 12 suites),
  doctest compile-fail keepers passing (23 facade doctests including the new
  `AccountingSink`/`SharedAccountingSink` negatives), feature-matrix clean.
- TO-01/TO-05 medians: before = 6.53s / 0.09s (per wave brief); after
  medians `UNRECORDED` — same-host/same-profile/warm-cache 3-run medians
  still owed by ticket owners. Timing goals must not be hardcoded as test
  assertions.

## 7. Known risks / rollback / reopen conditions

1. Semgrep guard environmentally unrunnable here → regression-guard
   coverage is unverified in this session, not waived. Re-run rail where
   `semgrep` can start (CA trust anchors present) before any qualification
   claim.
2. Concurrent-commit absorption: out-of-lease paths observed dirty are now
   committed without integrator review (including the extraction-script
   verification removal). Audit HEAD `41faf05` against writer leases.
3. Per-ticket mutant-RED and median evidence is entirely outstanding;
   any `DONE` flip without it violates the execution SSOT §Required
   closeout receipt.
4. Rollback: each ticket reverts to its owner file(s) only; shared
   `scaling.rs` must unwind TO-02 before TO-03 (writer order TO-03→TO-02).
