# Test-Optimization Wave — Integrator Receipts

Status: `HISTORICAL INTEGRATOR OBSERVATION at 41faf05; later owner proofs and v5 qualification closed that source; post-v5 qualification separate`.
Sections 1–10 record the incomplete `41faf05` observation. Sections 11–15
record subsequent owner proof and review corrections. The annotated
`qmp-final-qualified-sep23-followup-v5` tag later qualified a clean source
containing the TO-01..09 work. Resolve its target through
[FINAL-CLOSEOUT.md](FINAL-CLOSEOUT.md); it does not qualify later HEADs.

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

## 8. Owner-verified addendum (post-integration, same host)

Verified by the wave owner directly against final HEAD `41faf05`:

- TO-01 after-median (warm, same host/profile, 3 runs,
  `concurrent_accepts_lose_no_handle_and_assign_distinct_keys`,
  `--features spill-redb`): test-time 1.19s / 0.65s / 0.76s →
  median **0.76s** vs before-median 6.53s = **~88% reduction**
  (ticket target ≥80% met; not hardcoded as an assertion).
- TO-05 after-median (same conditions,
  `single_release_preflight_is_bounded_independent_of_owner_cardinality`):
  0.02s / 0.00s / 0.00s → median **0.00s** vs before-median 0.09s.
- TO-09 negative proof (temporary swap mutant in `append`: `evidence_bytes`
  ↔ `key_bytes` cost sources; total preserved at 235): focused test RED at
  `service.rs:1510` with `left: 41, right: 148` on the exact
  `evidence_bytes` component assertion — validation passed, exact oracle
  fired. Mutant reverted; `transcript_store` suite green (47 passed).
  During the proof the owner briefly introduced, then fully removed, a
  duplicated `session.key_bytes` accumulation in `checked_component_totals`;
  production regions verified byte-identical to HEAD afterwards.
- Extraction rail review: the outer `cargo metadata |
  check-extracted-metadata.py` call is intentionally absent because the
  extracted destination's canonical `scripts/check-workspace.sh` runs the
  same locked resolved-graph check fail-closed before any later rail stage.
  Keeping both checks would be duplicate validation of the same destination
  and source graph. Dry-run still refuses on a dirty tree by design (manifest
  freeze), so gate function needs a clean-tree run to confirm.
- Remaining gaps (still owed by ticket owners, not inferred here):
  per-ticket mutant-RED excerpts for TO-01..TO-08. Ticket status is
  `IMPLEMENTED; final exact-source receipt pending`, not `DONE`.

## 9. Exact-source repository/extraction candidate (not host qualification)

The repository and real standalone extraction were rerun from a clean detached
worktree at implementation candidate `71f06e49319b7466da7683c36992f3b64c7d2173`
(tree `52a35b4167e44cad73fe0b83dbba045aec3c340a`). This is a candidate
observation, not the final annotated receipt and not host activation evidence.

- Command: `scripts/extract-to-standalone-repo.sh /tmp/qmp-audit-extract-71f06e4`
- Canonical source digest:
  `5535aaa9f222a089255136e5d882b6abb50f60e71cdab6097c1f51a7037fbdbb`
- Extracted bootstrap commit/tree:
  `b7a22dbb2b61309b375737cfbb3a9f8f4147a566` /
  `327d7ac20100fceb9c8664baa82f5b9326b500d8`
- Extracted `scripts/check-workspace.sh`: exit `0`; 34 script contract tests,
  13 prompt-manager tests, 642 workspace/all-feature Rust tests, 10 non-zero
  feature capability selectors, 166 documented selectors, Semgrep, ShellCheck,
  PM lint, feature matrix, and `cargo deny` all passed.
- The earlier Semgrep startup blocker in this file is historical: current
  exact-source execution scanned all 66 Rust targets with all six rules and
  returned zero production findings.
- At that snapshot, the remaining work was the per-ticket mutation excerpts,
  an immutable receipt bound after the last tracked ticket change, host-pair
  qualification, deployment, and activation. This historical list is not a
  current completion status; resolve the newest annotated receipt instead.

## 10. TO-02 scaling smoke artifact

The clean source `00ab2a68ee47bf23ce746ba6ff242365b07175bf`
(tree `7fa06d3da1d4d28c3c8f9152c8a95ffe8082f715`) ran
`scripts/run-scaling-bench.sh --smoke` with exit `0`.

- Validator: `verify-scaling-results: OK (33 samples)`.
- All samples ended with semantic and allocator live bytes at zero, allocator
  allocated bytes equal to deallocated bytes, and no allocator underflow.
- Every scheduler-admission repetition reported allocation, deallocation,
  live, and high-water bytes all exactly zero during the scoped admission
  epoch.

This closes the missing TO-02 smoke artifact. It does not close the remaining
temporary-mutant or host-pair requirements, and it is not a deployment receipt.

## 11. Owner-executed mutant proofs (this session, same host)

Each proof: temporary mutant applied → focused test RED (excerpt below) →
mutant reverted → `git diff HEAD -- <file>` empty → focused test green.
No mutant was committed; all production regions verified byte-identical
after revert. Tickets remain `IMPLEMENTED`, not `DONE` (exact-source
annotated receipt still owed per §9).

- TO-04 (`faults/scenarios.rs`, success arm → `Contained`):
  `spill_fault_classification_reports_escaped_for_a_lying_controller`
  FAILED at `scenarios.rs:263`; drift/unreadable poles still passed
  (separate arms, correct). Post-revert: 3 passed.
- TO-06 (`ledger.rs` `validate_reserve_postimage`, pool allowance +1):
  `zero_shared_pool_cannot_extend_borrowing_class_hard_cap` FAILED at
  `policy_contracts.rs:310` (+1-byte `Capacity` denial missing).
  Post-revert: 1 passed.
- TO-07 (`builder.rs` `build`, reclaimer → `None`):
  `builder_reclaimer_reaches_build_live_broker` FAILED at `builder.rs:297`
  (request-count assertion). Post-revert: 1 passed.
- TO-05 (`ledger.rs` `preflight_release_batch`, full `owner_usage` scan
  added to probe counter): cardinality test FAILED at `ledger.rs:1518`
  with `left: 35, right: 3` (3 + 32 owners) on the exact-`3` assertion.
  Post-revert: 1 passed.
- TO-01 (`redb.rs` `reserve_record_id`, constant duplicate ID 0):
  `concurrent_accepts_lose_no_handle_and_assign_distinct_keys` FAILED —
  worker threads panicked at `redb.rs:1243` (duplicate-key commit
  rejected; receipts never reached the sequence oracle). Post-revert:
  1 passed (0.42s).
- TO-02a (`payload_store/service.rs`, temporary `pub fn new`):
  doctest `SimplePayloadStore (line 81) - compile fail` FAILED (snippet
  compiled once the ctor was public). Post-revert: doctests green.
- TO-02b (`facade/Cargo.toml`, `unbudgeted-stores` added to `full`):
  `scripts/verify-feature-matrix.sh` exit `1` with
  `feature-matrix: facade feature 'full' must not enable unbudgeted-stores`
  plus offending tree. Post-revert: `feature-matrix: OK`.
- TO-03: `cargo bench -p quanta-memory-platform-facade --features
  payload_store,revocable_store --bench scaling --no-run` builds
  (`scaling-25a2413b61efab4d`). Epoch revert-mutant RED still owed
  (requires a bench execution, not just compile).
- TO-08 keeper-implication diff proof (analysis, ticket requires no
  mutant): (a) deleted telemetry case's exact row `(2_000, 1_000, 1_000,
  true)` present in keeper table (`telemetry.rs:267`) with
  semantic-authority assertion; (b) deleted
  `assert_ne!(after.total_owner_bytes(), 600_000)` implied by keepers
  `after == before` (`control.rs:2083`) + `after == 400_000` (`:2088`);
  (c) eight→nine comment matches nine-element exhaustive array
  (`public_api_surface.rs:174-179`, no wildcard arm). `git diff` shows no
  production hunk for all three files.
- Still owed: TO-02c (linear-scan probe-bound RED), TO-02d
  (quadratic-drain normalized-growth RED on 1k/10k fixtures), TO-03
  epoch revert-mutant RED.

## 12. Extraction-gate reconciliation

The wave owner's earlier objection to the `check-extracted-metadata.py`
dest-pipe removal is withdrawn. Verified live: the check now runs in
extraction pre-flight (step 0b, `extract-to-standalone-repo.sh:80`
against `repo_root`) and `scripts/check-workspace.sh:26` runs the locked
resolved-graph check inside the destination rail — the removed call was
genuinely duplicate validation. No edit war; §8 as rewritten stands.

## 14. Final audit round (this session)

- TO-07 reroute mutant executed (closes the §11 waiver):
  `build_live` rerouted past reclaimer wiring → request-count RED
  (`builder.rs:299`, `left: 0, right: 1`); post-revert green.
  TO-07 test body re-checked line-by-line against the ticket
  (`class_budget(0, 50, 100)` = reserved 0 / soft 50 / hard 100 /
  borrow false; full chain; trigger/class/owner/bytes/max_attempts;
  attempts 1; total 60; drop → 0; test-local fake).
- Full suite re-run at `1634c3f`: 12/12 binaries green, 642 passed,
  0 failed. `cargo fmt --check` exit 0, `pm lint OK (3 targets)`.
- `scripts/check-workspace.sh` single-command run: passes fmt,
  shellcheck, resolved graph, workspace policy, Python contracts,
  and 4 extraction-contract checks, then halts at the consolidated
  semgrep fixture scan — same environmental `ca-certs: empty trust
  anchors` binary crash (exit 2), unchanged since the integrator
  window. Post-guard stages verified individually green instead
  (feature-matrix OK, 642 tests, doctests, bench smoke artifact §10).
- Conclusion: all nine tickets are implementation- and
  proof-complete. Only the annotated exact-source qualification
  receipt (tag authority) remains, which this session does not issue.

## 13. Owner-executed mutant proofs, round 2 (closes §11 arrears)

Same protocol as §11 (mutant → RED → revert → `git diff` empty →
green). This closes TO-02c, TO-02d, and TO-03. All nine tickets now
have owner-verified negative proofs; none is `DONE` (annotated
exact-source receipt still owed).

- TO-02c (`retained_map/service.rs` `get_checked`, hash lookup →
  linear scan over `entries.iter()`, identical results):
  `first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1`
  FAILED at `service.rs:1398` (`lookup hashes the exact key once`,
  `left: 0, right: 1`). Post-revert: 1 passed (0.02s).
- TO-03 (`benches/scaling.rs`, `Vec::with_capacity(entries)` moved
  back inside the epoch): bench `--smoke` binary exit `101`,
  panic at `scaling.rs:571` (`allocation epoch must start with zero
  tracked allocations`, `left: 8000, right: 0`). Note: the
  `run-scaling-bench.sh` wrapper refuses dirty trees (exit 2), so the
  binary was invoked directly with identical args; the oracle (not the
  wrapper gate) produced the RED. Post-revert: mutant-free tree.
- TO-02d (`revocable_store/service.rs` `reclaim_one`, redundant O(N)
  `charged_bytes` rescan + `black_box` per reclaim; totals and order
  unchanged): full `--sizes=1000,10000` artifact (170 samples) →
  verifier exit `1` with `revocable_fill_and_drain growth: per-entry
  cost grew 6.96x from 1000 to 10000 entries (median 889.8ns/entry ->
  6195.2ns/entry, allowed 4.0x)`. Post-revert clean rerun: `OK (170
  samples)`, exit `0`.
- TO-02d process note (no-false-positive evidence): two earlier
  candidate mutants stayed GREEN for principled reasons —
  `VecDeque::remove(0)` is O(1) on a ring buffer (ratio 1.12x), and
  `make_contiguous()` is a no-op on a never-wrapped deque (ratio
  1.03x). The rail correctly passed both and fired only on the
  genuinely quadratic rescan.

## 15. Review correction after `943cf0a` (candidate evidence, not a tag)

The review of `main@943cf0aaff3177831c3fb75ffa10ceb39e3eab99` found a
late-attach false negative: adding a public `charged_against(self) -> Self`
to retained map left its `compile_fail` doctest GREEN because the example used
`store.charged_against` as a field. The other store examples used the same
pattern; payload's actual call depended on a specific method signature.

- All five late-attach snippets now resolve the associated method item by
  concrete type. Clean `cargo test -p quanta-memory-platform-facade --doc
  --all-features` passed 23/23 before mutation.
- Retained-map public-method mutant: doctest exit 101, exactly its late-attach
  snippet failed with `Test compiled successfully, but it's marked
  compile_fail` (22 passed, 1 failed).
- The same public-method mutant applied to payload, revocable, transcript,
  and prepared publication: doctest exit 101; all four corresponding snippets
  failed by compiling (19 passed, 4 failed). Mutants were removed; no public
  method remains.
- The unclassified `e535f54` production `ticket_states: HashMap` change is
  reverted to `BTreeMap`, as the TO-05 writer lease specified. The 32/128
  owner bounded-probe keeper is unchanged.
- TO-02's matrix wording now admits the already implemented deduplication of
  identical resolved feature sets. The three escape-hatch absence profiles,
  per-crate bare checks, and workspace all-features union remain executable.

At the reviewed `943cf0a` source, independent mutant checks also reproduced
TO-05 (`35` vs `3`), TO-09 (`41` vs `148`), and TO-02d (normalized growth
`17.71x` vs `4.0x`). Each returned to focused GREEN, and each owner-file diff
returned to its pre-mutation state. `scripts/check-workspace.sh` exited 0 on
that pre-correction source, including Semgrep 6 rules / 66 targets / 0 findings.
The correction still requires a committed source identity, full rail, and a
new annotated receipt; earlier tags do not cover it.

## 16. Current-worktree audit after `0284489` (2026-09-23, non-qualifying)

Audit base: `028448946caa95676df034481a830c1d3e3b1f33` plus an uncommitted
SEP-23/FUP overlay. The SHA-256 over the sorted `path + content digest` list
for every file below `crates/` (including untracked source/test files) is
`70d0b8572313f5015105fde9c61407484d9882b48c91553cd017783c2f4f93fb`.
This digest identifies this audit input but is not a Git source identity.

The audit found and repaired six executable gaps:

- TO-04's file-local command omitted the `faults` feature and selected zero
  tests. The ticket now uses the locked `faults::scenarios` selector; it
  selected and passed all seven scenarios.
- Production-only builds warned about a test-only scheduler accessor and a
  redb-only restore-claim constructor. Their compile gates now match their
  consumers; the `spill`-only slice no longer carries unused constructors.
- The SEP-23 overlay contained four redundant ownership clones plus a
  redundant `map_err` closure and a non-minimal boolean. The code now moves
  final-use values and expresses the direct predicates; no warning suppression
  was added.
- The overlay was not rustfmt-clean. `cargo fmt --all` normalized the touched
  Rust files, and the canonical format check now passes.
- The new SIGKILL rail used a 300ms sleep to hope a non-empty prefix had
  committed. Its child now commits record zero before the `READY` handshake,
  so the parent can kill immediately with no timing oracle.
- A focused `--test spill_redb_kill_reopen` run could execute an existing stale
  example binary. The integration test now performs one locked/offline
  freshness build per test process before spawning the helper. Before this
  repair, the sleep-free focused rail ran the stale READY-before-commit helper
  and failed `the stream committed before the kill` (two passed, one failed);
  after the freshness repair, all three process cases passed.

Current-worktree evidence after those repairs:

- `scripts/check-workspace.sh`: exit `0`. This includes warning-denied
  per-feature clippy, workspace/all-feature tests, 10 non-zero advertised
  feature capability selectors, 166 documented selectors, Semgrep (six rules,
  zero findings), ShellCheck, extraction contract tests, PM lint, and
  `cargo deny`.
- `cargo test --locked --workspace --all-features`: exit `0`; core `222`,
  facade `278`, testkit `82`, all integration suites (including the three real
  SIGKILL/reopen cases), and doctests passed.
- All three untracked Rust files (`domain/admission.rs` plus the crash helper
  and integration rail) were also passed explicitly to the same Semgrep
  config: six rules, three targets, zero findings.
- `cargo bench --locked -p quanta-memory-platform-facade --all-features
  --bench scaling --no-run`: exit `0`.
- Both public examples (`hello_world`, `effective_budget_report`) ran with exit
  `0` against the real facade API.

The SIGKILL rail proves three explicit boundaries: committed rows plus a
persisted restore claim, a forced kill while the child continuously issues
reserve/commit operations, and a reserved-but-never-committed hole. The exact
scheduler point inside an individual redb transaction is not observed. Kills at
the ACK, source-release, restore-delete, and finalize boundaries, plus power
loss, remain unverified and must not be inferred from these three cases.

Qualification remains open by construction. The exact command
`scripts/extract-to-standalone-repo.sh --dry-run <empty-dir>` exits `1` with
`tracked extraction input differs from frozen HEAD`, because the audited
source is still dirty. Do not mark this overlay or TO-01..TO-09 `DONE`, and do
not mint a replacement annotated receipt, until the intended code is committed,
the source digest is re-frozen, the standalone extraction succeeds, and the
required host-pair/tag gates are rerun on that exact commit.

## Current status after the historical observation

The preceding `0284489` paragraph was correct for its dirty candidate.
Subsequently, the TO-01..09 owner proofs, late-attach compiler correction,
and BTreeMap restoration were committed, and the annotated
`qmp-final-qualified-sep23-followup-v5` tag qualified source `f637d42`.
Therefore §§1–10 `UNRESOLVED` rows and §16's `DONE` prohibition must not be
read as the current ticket queue. The clean `db0b03c` audit base is newer
than the tag and includes a benchmark verifier change. Later committed base
`65f57d5` adds docs and comments but no executable behavior change. This
reconciliation commit defines the receipt rule: current-source repository
qualification requires an annotated `qmp-local-qualified-*` tag targeting the
exact source after canonical and fresh full benchmark runs. Historical owner
proofs and the v5 tag remain evidence for their named source only.
