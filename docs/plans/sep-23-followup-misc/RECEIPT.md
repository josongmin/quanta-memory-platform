# Sep-23 follow-up qualification receipt

Status: `PARTIALLY QUALIFIED — local repository, extraction, benchmark GREEN; exact-source host-pair OPEN; no v5 tag`
Scope: `full-followup` (`FUP-001` through `FUP-007`)
Date: `2026-09-23 Asia/Seoul`

This file is part of the candidate tree, so it cannot contain its own final
commit ID. Resolve the annotated tag and verify that it targets the current
candidate:

```bash
git rev-parse qmp-final-qualified-sep23-followup-v5^{}
git show --no-patch --format=fuller qmp-final-qualified-sep23-followup-v5
```

The prospective tag annotation is the immutable source/result authority.
Until the exact-source host-pair passes and that tag exists, this receipt is
partial evidence only, not a release qualification.

## Source lineage

- Audit plan base: `028448946caa95676df034481a830c1d3e3b1f33`.
- First clean implementation candidate:
  `beba43760d76a60fbce7ccbf8ed97d3985aeaaf3`, tree
  `6c88d590f4b67c232098599ca99c1570c4d0a0e5`.
- Second clean candidate (peer review + self-audit oracles + ticket
  state + this receipt):
  `c7706d00b8011826533ddff55719773723535a21`, tree
  `59bc232527cf93fac0dbf95b2a9d76867575e2fc`.
- The final candidate adds the lazy-breach delta on top:
  `crates/quanta-memory-platform-core/src/{domain/admission.rs,application/scheduler.rs}`
  only (zero-alloc `admits` restoration, no manifest/use-path change).
  The final tag must target the commit that contains it and this receipt.
- The final public-authority delta removes `Clone`/`Copy` from
  `DurableRestoreClaim` and `Clone` from the claim-bearing candidate/recovery
  envelopes. Private `Arc` custody only pins the same claim across a lock-free
  controller call under the store mutation permit; it is not exposed as a
  second caller-owned restore authority.
- No Cargo manifest (after `c7706d0`), dependency edge, public builder
  surface, or storage schema was changed by the final self-audit delta.

## Owner-local positive proof

At clean candidate `beba437`:

| Ticket | Command selector | Selected / result |
| --- | --- | --- |
| FUP-001 | facade all-features `sep23_fup001` | 6 / GREEN |
| FUP-002 | facade all-features `sep23_fup002` | 8 / GREEN |
| FUP-003 | core all-features `sep23_fup003` | 7 / GREEN |
| FUP-004 | core all-features `sep23_fup004` | 4 / GREEN |
| FUP-004 | testkit all-features `sep23_fup004` | 4 / GREEN |
| FUP-005 | core all-features `sep23_fup005` | 2 / GREEN |
| FUP-006 | core all-features `sep23_fup006` | 2 / GREEN |
| FUP-007 | core all-features `sep23_fup007` | 8 / GREEN |

The final self-audit delta added and selected:

- core `sep23_audit_`: 8/8 GREEN — max-scale overflow, failing deferred
  drains, group hard-plan invariance, wrong-victim release, forged reclaim
  count, callback panic containment, and zero-byte no-callback behavior.
- facade `sep23_audit_`: 4/4 GREEN — finalize retry while another claim is
  live, proof/payload inconsistency rejection, and prepared/committed
  decode-panic recovery.
- `sep23_fup003_preflight_attempt_agreement_randomized` compares the pure
  preflight with the actual attempt path over deterministic policies, classes,
  owners, kinds, edge byte counts, and evolving live state.
- Its first draft retained the broker mutex guard through the rebuild branch
  and deadlocked on reentrant `reserve`. The guard now has an explicit
  `drop(core)` before the attempt; the selector completes 1/1 in 0.01s.

The annotated tag must record the final-candidate rerun counts; the table above
is not a substitute for that final run.

## Negative proof

Temporary mutants ran in an isolated detached worktree and were removed before
publication:

| Ticket | Mutant | Expected RED |
| --- | --- | --- |
| FUP-001 | classify `Reserved` as `Finalized` | 3 of 6 selected tests failed |
| FUP-002 | remove backend claim identity check | foreign-backend permission test failed |
| FUP-003 | classify zero-state breach as reclaimable | terminal/no-callback tests failed |
| FUP-004 | remove cross-axis kind comparison | 3 of 4 core tests failed |
| FUP-005 | append active provenance on every reconfigure | 10k bounded test failed with 10,000 entries |
| FUP-007 | ignore zero-byte live-ticket owners during reconfigure | 7 of 8 tests failed |

Each mutant command returned exit `101`; the isolated worktree was restored
clean and removed. The main checkout never published a mutant.

## Repository and standalone proof

The first clean implementation candidate `beba437` completed a real
standalone extraction to
`/private/tmp/qmp-beba437-extract.WRT5OU`:

- frozen tree: `6c88d590f4b67c232098599ca99c1570c4d0a0e5`
- canonical source SHA-256:
  `ffc75918fa8e29a0bbe0e9adbf0f2987ac0505d8450b60913f04dc13fd7b77fa`
- extracted bootstrap commit:
  `15b4c3b3b941a266e6575de14e5abc69e6b27192`
- extracted tree:
  `a309844669412955ac3c70ad7435325bf245f7df`
- extracted `scripts/check-workspace.sh`: exit `0`

That rail covered formatting, ShellCheck, resolved dependency graph, workspace
policy, Python/PM/extraction contracts, six Semgrep rules, feature matrix,
workspace/all-feature Rust tests, 10 non-zero advertised capability selectors,
166 documented selectors, PM lint, and `cargo deny`.

Because the final self-audit delta changes Rust and documentation after
`beba437`, the final tag annotation must contain a second clean
`scripts/check-workspace.sh` result and a second real standalone extraction
bound to the tag target. The historical extraction above is lineage evidence,
not final-source substitution.

## Historical host-pair result (v3/v4 source only)

Read-only Cargo metadata from both Semantica workspace roots resolved the same
QMP `0.2.0` manifests under
`/Users/songmin/Documents/code-new/quanta-memory-platform`.

- The Semantica host-owned default-provenance defect was repaired in commit
  `4767bcfec2b9e2e9269f423f2b29a3216166f4bd` and qualified in merge
  `86e90abba4cb70d9bd7e7a4872a1ea5abce0a665`.
- QBC `quanta-runtime-memory-compose` lib/all-features: `45 / 45` GREEN.
- QBC `quanta-memory-host` all-tests/all-features: `114 + 5 + 7` GREEN.
- The paired default, explicit-overlay, and explicit-inherit provenance
  oracles are all members of the compose target. The receipts above are the
  historical v3/v4 source digest; the exact-source v5 host rerun is a separate
  required gate after the two-test delta.
- Exact owner patch, prior RED, receipts, and reopen conditions are in
  `tickets/HOST-PAIR-RUNTIME-DEFAULT-PROVENANCE.md`.

Therefore:

| Boundary | Verdict |
| --- | --- |
| QMP owner-local behavior | GREEN on current candidate; see final v5 tag counts |
| QMP canonical repository rail | final v5 tag annotation authority |
| QMP standalone extraction | final v5 tag annotation authority |
| Semantica dependency identity | final v5 source digest in tag annotation |
| `quanta-memory-host` QBC owner rail | final v5 QBC receipts in tag annotation |
| Semantica compose/QBC host-pair | final v5 exact-source tuple in tag annotation |
| Hosted CI | NOT RUN |
| Deploy / activation | NOT RUN |

## Explicit residual boundaries

- The SIGKILL rail proves: committed rows plus persisted restore claim; kill
  while reserve/commit repeats; reserved-but-never-committed hole.
- It does not observe the exact instruction point inside a Redb transaction.
- ACK, source-release, restore-delete, finalize kill points and physical power
  loss are unverified. Deterministic ACK-loss injection is not power-loss
  evidence.
- B07 zero-byte ticket metadata-count admission and J06 general deferred
  backlog admission are not part of these correctness fixes. No current host
  reachability or measured exhaustion evidence promoted them to blockers.
- No repository qualification result is deployment or activation evidence.

## Lane I completion record (final workdir source)

### Corrected and completed counts

- Core `sep23_audit_`: **10/10** GREEN (the 8 listed above plus
  admitted-recharge and batch-release kind-vector exactness).
- Facade `sep23_audit_`: 4/4 GREEN (unchanged).
- Full per-ticket rerun on the final source: 001:6, 002:8, 003:7,
  004:4+4, 005:2, 006:2+3, 007:8 — all GREEN, all non-zero selected.
- `durable_` 10/10, `policy_contracts` 15/15, `public_api_surface` 7/7,
  facade `--doc` 24/24 (incl. compile-fail), WrongChargeKindAxis 1/1,
  property trace 1/1 GREEN.

### Completed negative proof (15 mutants, final source)

Beyond the six above (reproduced on the final source with the same
RED splits):

| Mutant | Target | RED |
| --- | --- | --- |
| freeze `policy_revision` on accepted reconfigure (M6) | coupling receipt | 1/2 |
| swallow Drop-seed / drain re-push / committed restore (D1/D2/D3) | deferred authority | 1/1 each |
| finalize proof-match after active-state (P2-1 old order) | idempotent retry | 1/2 (order-independent peer test passes) |
| re-mint backend identity on every open (KILL2) | SIGKILL rail + reopen identity | 1/1 both |
| double-callback hard plan (SF) | single-attempt contract | 1/1 both oracles |
| suite kind verdict always-true (M4T) | fake rejection | 2/4 documented split |
| terminal-arm removal / trigger downgrade (003A/003B) | preflight plans | 3/7, 2/7 |
| deny-vanish `into_breach` (G1) | lazy breach | group oracle RED |

Correction: the first kill-rail mutant (constant `+1` identity shift)
was INVALID — both processes shifted together, and its one RED came
from a stale child binary, not the mutant. It is superseded by KILL2
(re-mint on open), which breaks the cross-process agreement the rail
exists to prove. The rail's example-freshness build now prevents
stale-binary artifacts.

Each mutant was applied to the workdir, run, and reverted with a
SHA-256 equality check; the tree is clean of mutants at handoff except
the intended lazy-breach delta files.

### Self-audit P1: zero-alloc `admits` regression (found by bench, fixed)

The full scaling bench failed on the `c7706d0` source:
`scheduler admission must not allocate after policy construction`
(136,000 bytes tracked). Root cause: the FUP-003 delegation made the
boolean `admits` probe format the denial message on every deny and
discard it. Fix: `check_group_admission` now returns a lazy
`GroupAdmission` decision; `into_breach` is the single message
construction site, called only when a breach must be reported. Message
text is byte-identical to the attempt path (verified: the attempt path
reports the denial text for unrepresentable postimages too, because
`admits` returns false before any overflow guard runs). The bench was
re-run on the fixed source: exit 0, verifier OK (170 samples),
source-bound to `c7706d0` + dirty `2cb59323`.

Two more self-audit closings: the global aggregate is read with
`checked_total` (defense-in-depth; live totals are representable by
construction, pinned by a direct pure-function test plus a max-scale
`u64::MAX` broker test), and `finalize_restore` checks the terminal
proof before the active-claim state (same-claimant retry succeeds even
while an unrelated claim is live).

### Final-source gates (Lane I)

- `scripts/check-workspace.sh`: exit 0 on the final workdir (and exit 0
  on clean `c7706d0` before it).
- Public examples `hello_world`, `effective_budget_report`: run GREEN.
- Feature checks: `--no-default-features`, `spill`-only, `full` GREEN;
  `redb` absent from the `full` graph (host-dependency proof).
- Real standalone extraction of `c7706d0` + in-extract rail: exit 0
  (bootstrap `276893f`, extracted tree `8e10fc78…`). The final tag annotation
  is the authority for the repeated extraction that includes the committed
  lazy-breach delta.
- Host-pair is CLOSED after the host-owned provenance repair. The final tag
  annotation binds the final QMP commit to the repeated Semantica QBC tuple.
- Absolute latency assertions: none added anywhere, per contract.

## Independent adversarial review supplement — v5 candidate

Review base: `3038dae7d472ad37085ed309851b40b3e064e5fb` / tree
`a495da407331529ba39d4a4a260878c924bd1496`. The only Rust changes in this
supplement are two positive boundary oracles; production behavior is unchanged.
They add two core tests (235 -> 237); facade and testkit counts remain 282 and
82. `docs/verification-scenarios.md` lists the new policy-boundary test.

### Required independent mutants

Each target test was first run GREEN with `--locked --all-features`. The
mutant was applied only in a detached temporary worktree, and each expected
RED was a behavioral assertion failure (Cargo exit 101), not a compile error.
The mutated file was restored and its SHA-256 matched the worktree's HEAD blob:

| Mutant | Exact target | RED evidence | Restored SHA-256 |
| --- | --- | --- | --- |
| M4 equivalent: skip cross-axis kind-vector comparison | `ledger.rs`; `sep23_fup004_class_kind_swap_preserving_totals_is_rejected` | `validate_exact()` returned `Ok(())` for the corrupted class split | `e8fb281e8a1aa0efb029fbb91e1b2be2e257adbdcc4340da69bdfd5453cbfd7c` |
| M2 equivalent: accept a foreign backend claim | `redb.rs`; `sep23_fup002_foreign_backend_claim_rejected_on_every_permission_change` | foreign `commit_restore` was accepted | `68b7f5b19d621acf38588e3dbf369e5b3785b5b53c0eabad82e408cc394a4e56` |
| M7 equivalent: omit zero-byte live owner index | `control.rs`; `sep23_fup007_zero_byte_committed_blocks_group_removal_until_explicit_release` | group-removal reconfigure returned `Ok(())` while the ticket was live | `8595281a2cabfc771f9cbe11d099dc5d1b17e1db17e5ad35da2cae34848daf0d` |
| M6 equivalent: stop incrementing policy revision | `control.rs`; `sep23_fup005_ten_thousand_reconfigures_keep_provenance_bounded` | revision remained 1 instead of 10001 | same `control.rs` SHA above |
| G1 equivalent: omit lazy denial breach | `admission.rs`; `unknown_group_denies_terminally_with_legacy_message` | required typed breach was absent | `6371e59ae8dc1f38f37bef65dd6ba16e64e4ff2be29a44426ea028750317ca74` |

An initial attempted M7 mutation omitted `ticket_groups`; its test remained
GREEN because zero-byte scope authority is intentionally held in
`ticket_owners`, not the positive-byte scheduler-usage map. It was not counted
as a killed mutant; the corrected `ticket_owners` mutation above was RED.

### Two new boundary mutants

| New mutant | Existing suite before supplement | Added oracle and result |
| --- | --- | --- |
| Scratch-cap predicate `live_after > max` -> `>= max` | Existing 80k + 40k test stayed GREEN; it did not test exact-cap admission | `execution_scratch_budget_accepts_exact_cap_and_denies_one_more` now fails under the mutant at `reserve to 100000`; unmutated test is GREEN and checks no ledger drift after +1 |
| Group validation `reserved > soft` -> `>=` | Existing coherent-policy test stayed GREEN; it did not exercise equality | `validate_accepts_resource_group_reserved_equal_to_soft_cap` now fails under the mutant; unmutated test is GREEN and pins inclusive `reserved <= soft` |

Both reverted mutant worktree files matched the updated review source byte for
byte. These two additions close observed test-strength gaps without changing
production semantics.

### Exact candidate local qualification

- QMP commit/tree: `466f08648af05b7b509fb74dbfdce3c6da1c213d` /
  `113868d0a7d309596b82994ef56e5667675c8278`.
- Canonical `scripts/check-workspace.sh`: exit 0 (`check-workspace: clean`) on
  a dedicated clean target. All-feature unit/integration/doc suites passed:
  core 237, facade 282, testkit 82; Hellgate E2E 18, public API 20, SIGKILL
  reopen 3, facade doctests 28. The feature matrix compiled warning-free,
  10 advertised feature paths each executed a non-empty capability test,
  217 documentation selectors resolved, generated guidance lint and cargo-deny
  passed. Both `hello_world` and `effective_budget_report` examples ran.
- Real extraction destination:
  `/private/tmp/qmp-v5-extraction.KwcKya`; canonical source/extract digest
  `aa425a8591975b5236b8c72fb4d8549cf70a89d113c5efd816a2dd13490dc915`;
  extracted bootstrap commit/tree `27d0570641989a2bb80b8eccadf2da25f56726c4` /
  `2db4b0f9e473f0d6e6aceb06bee39ff20b9ce3e6`. In-extract
  `scripts/check-workspace.sh` exited 0 and preserved the canonical digest.
- Full `scripts/run-scaling-bench.sh`: verifier `OK (170 samples)` on that
  commit/tree using rustc/cargo 1.92.0, Darwin 24.6.0 arm64. This is verifier
  qualification, not a latency or cross-machine performance claim.
- Exact-source host-pair is **OPEN**. Read-only metadata from both Semantica
  workspace roots resolves the same QMP core/facade manifests at the candidate
  path. However Semantica is at dirty `a226ea29a0dc64823fbfd2f5f3d1f02664a66319`
  (16 commits ahead of origin with many modified files); QIndex is at dirty
  `384cecb5825d8ebf38ff6c07bc6bf64ffc43fbe0` with broad modified sources; a
  workspace bench build was active during review. Earlier QBC source-closure capture also
  drifted while QIndex changed. No new QBC receipt was accepted or synthesized.
  Re-run compose/host QBC after the sibling repositories can be frozen, binding
  the exact resulting QMP source digest. Historic v3/v4 receipts do not close
  this gate.

### Cross-dimension review

- The randomized preflight/attempt test is deterministic (fixed xorshift seed),
  uses four policy shapes and 500 transitions per shape, and compares exact
  `MemoryError` values. Independent semantic rules are pinned by the terminal,
  hard-remeasure, borrowing-class, protected-floor, and execution-scratch tests;
  FUP-004 has independent class/owner/context kind-swap probes plus fake-rejection
  conformance tests. The random test is not the only oracle.
- The SIGKILL rail uses a child `READY` protocol and explicit `Child::kill`, not
  sleep ordering. The 30-second deadline only bounds a hung child; a fresh
  locked build of the helper is required before execution. The rail proves its
  documented interruption points, not power loss or an instruction point
  inside a redb transaction.
- Restore callbacks are outside the store-state mutex and serialized by the
  lifecycle mutation permit. Decode panic on a Prepared claim aborts and then
  resumes unwind; a Committed claim remains recoverable. Finalize error/panic
  fences the published charged copy and claim for reconciliation. Poisoned
  locks return errors; the remaining `expect` calls are after checks protected
  by the mutation permit, not on caller-controlled data.
- FUP-001 through FUP-007 focused selectors all passed on this source. The
  exact-source `scripts/check-workspace.sh` passed on a clean, dedicated
  `CARGO_TARGET_DIR` (`/Volumes/Extreme SSD 1/build-cache/qmp-v5-canonical-target`);
  it covered workspace/all-features tests, doc tests, feature matrix/runtime
  capability checks, 217 resolved selectors, generated-doc lint, and cargo-deny.
  Both public examples also ran successfully. An earlier attempt using a target
  shared with the detached mutation checkout is discarded: it reused a mutant
  binary and failed one boundary test; a separate clean target rebuilt and
  passed the full canonical rail. Extraction, full scaling benchmark, and exact-
  source host-pair remain required before any v5 qualification tag; prior v3/v4
  receipts are not substituted for those gates.
- This supplement does not claim 100/100 across the separate 100-item product
  checklist. Host activation, hosted CI, physical power loss, finalize-point
  kill, physical memory caps, and other explicitly conditional capabilities
  remain separate or unverified boundaries.
