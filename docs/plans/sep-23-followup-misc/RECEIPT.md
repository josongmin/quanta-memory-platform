# Sep-23 follow-up qualification receipt

Status: `Historical v5 receipt and QMP 7f1c2f8 / Semantica a71e3e9 local pair below. Focused verifier evidence is bound to db0b03c. Current local closeout is authoritative only when an annotated qmp-local-qualified-* tag targets exact HEAD and records canonical, fresh full benchmark, and extraction dry-run results. Host-pair, hosted CI, deploy, activation and physical power loss are separate.`
Scope: historical `full-followup` (`FUP-001` through `FUP-007`); 2026-09-24 addendum qualifies the in-memory consumer pair, with optional Redb evidence recorded separately
Date: `2026-09-24 Asia/Seoul` (historical entries retain their original dates)

The historical v5 receipt was written inside its candidate tree and could not
contain its own final commit ID. Resolve its annotated tag to verify the
historical source it targets:

```bash
git rev-parse qmp-final-qualified-sep23-followup-v5^{}
git show --no-patch --format=fuller qmp-final-qualified-sep23-followup-v5
```

The v5 annotated tag exists and covers only its named Sep-23 source. The
2026-09-24 pair below has no successor tag. Neither qualifies the current
`db0b03c` audit base, which changed the scaling verifier after the pair run.
Hosted CI, deployment, and activation are not claimed here.

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

- Rust/test candidate commit/tree: `466f08648af05b7b509fb74dbfdce3c6da1c213d` /
  `113868d0a7d309596b82994ef56e5667675c8278`.
- Receipt-only follow-up commit `ca9ac79d5e6b81638329fd6e57ca91b688fc5249`
  changed only these qualification docs; its tree is
  `e63ed76287292e60e770e02519d52a1d9e60776a`. Canonical
  `scripts/check-workspace.sh` was rerun on both source commits and exited 0
  (`check-workspace: clean`) on a dedicated clean target. All-feature
  unit/integration/doc suites passed:
  core 237, facade 282, testkit 82; Hellgate E2E 18, public API 20, SIGKILL
  reopen 3, facade doctests 28. The feature matrix compiled warning-free,
  10 advertised feature paths each executed a non-empty capability test,
  217 documentation selectors resolved, generated guidance lint and cargo-deny
  passed. Both `hello_world` and `effective_budget_report` examples ran.
- Real extraction destination for the Rust/test candidate above:
  `/private/tmp/qmp-v5-extraction.KwcKya`; canonical source/extract digest
  `aa425a8591975b5236b8c72fb4d8549cf70a89d113c5efd816a2dd13490dc915`;
  extracted bootstrap commit/tree `27d0570641989a2bb80b8eccadf2da25f56726c4` /
  `2db4b0f9e473f0d6e6aceb06bee39ff20b9ce3e6`. In-extract
  `scripts/check-workspace.sh` exited 0 and preserved the canonical digest.
- Full `scripts/run-scaling-bench.sh`: verifier `OK (170 samples)` on that
  code/test commit/tree using rustc/cargo 1.92.0, Darwin 24.6.0 arm64. This is verifier
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

## Final exact-source candidate gates (2026-09-23)

This section supersedes earlier statements in this file that the v5 candidate
host-pair, extraction, or benchmark was still pending. Candidate QMP commit
`cf2a4412616f2e246ecd75bdb32ce05c55304a89` (tree
`facb59d5c93f11cf6748d940a635af7138b02339`) was clean and exact-source for
these runs. This receipt is now being updated by a documentation-only commit;
the final candidate source digest and applicable gates must be rerun on that
post-documentation commit before promotion.

### Host-pair tuple and receipts

All five sibling worktrees were clean and detached in
`/private/tmp/qmp-v5-host-pair-bundle-20260923`:

| Repository | Commit |
| --- | --- |
| `quanta-memory-platform` | `cf2a4412616f2e246ecd75bdb32ce05c55304a89` |
| `semantica-codegraph-v2` | `0e29a45f9db1a3739b853d0bad75e33eb61fe43f` |
| `quanta-index` | `f478f69e5e0006afb7ea36360be9b4dd7558d252` |
| `quanta-gqlang-v2` | `daa1dfde8afb605abe42128cf5974ac6373f9fea` |
| `quanta-taskmesh` | `1d2bb2fedcbe45ed5809d5f87e4f66560f5c7d9b` |

Both Semantica Cargo metadata roots resolved QMP core/facade manifests from
this bundle. Frozen cross-repository source snapshot digest:
`4592db63e3f884d84edc1291c6d2774736bd910c678788278523602f60704a31`.
Each command pinned
`QUANTA_BUILD_EXPECTED_SOURCE_SNAPSHOT_DIGEST_V1` to that value; accepted
receipts report matching before/after source digest.

| QBC target | Run ID | Result | Raw receipt |
| --- | --- | --- | --- |
| `quanta-runtime-memory-compose` exact `runtime_memory_bootstrap_core_owns_production_budget_and_live_dependencies` | `20260923T044012.161834Z-11aa0636f876` | 1/1 GREEN | `/Users/songmin/Library/Caches/semantica-codegraph-v2-target/quanta-build-cli/_state/execution-roots/semantica-codegraph-v2-15454730f53e/lanes/qmp-v5-hostpair-bundle/verification-results/20260923T044012.161834Z-11aa0636f876/receipt.json` |
| `quanta-runtime-memory-compose` exact `runtime_cutover_memory_budget_report_projects_runtime_defaults_and_code_overlay` | `20260923T044441.494126Z-184e82873ca8` | 1/1 GREEN | `/Users/songmin/Library/Caches/semantica-codegraph-v2-target/quanta-build-cli/_state/execution-roots/semantica-codegraph-v2-15454730f53e/lanes/qmp-v5-hostpair-bundle/verification-results/20260923T044441.494126Z-184e82873ca8/receipt.json` |
| `quanta-runtime-memory-compose` whole target | `20260923T045049.285259Z-e1f78a901121` | 45/45 GREEN | `/Users/songmin/Library/Caches/semantica-codegraph-v2-target/quanta-build-cli/_state/execution-roots/semantica-codegraph-v2-15454730f53e/lanes/qmp-v5-hostpair-bundle/verification-results/20260923T045049.285259Z-e1f78a901121/receipt.json` |
| `quanta-memory-host --tests --all-features` | `20260923T045146.387677Z-d5a936d86399` | 126/126 GREEN | `/Users/songmin/Library/Caches/semantica-codegraph-v2-target/quanta-build-cli/_state/execution-roots/semantica-codegraph-v2-15454730f53e/lanes/qmp-v5-hostpair-bundle/verification-results/20260923T045146.387677Z-d5a936d86399/receipt.json` |

The two exact compose oracles prove runtime default ownership and explicit
overlay projection on this tuple. Host all-tests proves the selected host
suite, not deployment or activation.

### Local repository, extraction, and scaling evidence on `cf2a441`

- `scripts/check-workspace.sh`: exit 0, final output `check-workspace: clean`,
  using `/Volumes/Extreme SSD 1/build-cache/qmp-v5-canonical-target`. Selected
  counts: core 237, facade 282, testkit 82; Hellgate 18, public API 20,
  SIGKILL 3, doctests core 13/facade 28; 217 documentation selectors, feature
  matrix/runtime capabilities, and cargo-deny all passed.
- Both public examples (`hello_world`, `effective_budget_report`) ran
  successfully.
- Real extraction: `/private/tmp/qmp-v5-final-extraction.lgm2kR`; canonical
  source/extract digest
  `e4915ff2ac5c6d48dafe65ed3cecd5fcc1c72936f37d3cbd02d2810a9b8bc4ec`;
  in-extract `scripts/check-workspace.sh` exit 0.
- `scripts/run-scaling-bench.sh`: `verify-scaling-results: OK (170 samples)`
  on Darwin 24.6 arm64 with rustc/cargo 1.92.0. This validates the recorded
  sample set and harness, not a latency or cross-machine performance claim.

### Final disposition and residual scope

FUP-001 through FUP-007 and the `full-followup` candidate gates are locally
qualified on the pinned tuple above. The separate 100-item product checklist
is not 100/100: B07 metadata-count admission and J06 general backlog bounds
remain OPEN; power-loss durability, mid-finalize instruction-point kill,
hosted CI, deploy, activation, and physical memory-cap qualification were not
run. Earlier v3/v4 immutable tags remain unchanged. A v5 annotated tag/push
was not created because repository policy makes tag/release publication a
separate approval boundary.

Addendum (2026-09-23, post-v5): the mid-finalize instruction-point kill
residual above is superseded by the DURABILITY-001 L1 rail (2/2 GREEN,
4/4 negative mutants RED with byte-identical reverts, 20-iteration stress
GREEN, canonical rail exit 0 with 224 selectors). The v5-candidate
statement is preserved as history. Power loss stays UNVERIFIED (L3);
frozen-candidate Slice D receipt is tracked in `tickets/DURABILITY-001-*`.

Management-bound clarification (2026-09-23, `edfef66`): the candidate adds
independent live-ticket, identifier-byte, and deferred-drain batch bounds. This
closes those concrete zero-byte/deferred authority cases only. The broader B07
policy/metadata cardinality and J06 general backlog rows remain OPEN where no
family-specific cap or replay-horizon contract exists. In particular, durable
record-state/finalize-proof retention is an explicit safety-over-space policy;
it is not a correctness failure, and it must not be garbage-collected without a
persisted replay horizon and typed stale-token semantics.

Addendum (2026-09-23, DURABILITY-001 L2 syscall subrail): `pwrite64` and
`fdatasync` EIO were injected by strace at the first matching syscall after
the staged finalize marker. Both cold-reopen oracles converged (2/2); this
does not qualify a block device, filesystem crash-recovery path, or power loss.
Exact dirty source digest, raw artifact hashes, and residual scope are in
`tickets/DURABILITY-001-power-loss-finalize-kill.md`.

### Current callback/read-panic hardening pass (2026-09-23)

Candidate identity: HEAD `edfef66c3d9dabbe4d01a98754a44d32ef9c940e`, tree
`4f1a7299af049e7fa1c0eeefc8e683f01239d462`; L2 executable-input digest
`185dd3329b00d685b866c992c03e50b1bcaa1f8a1097b0310275abb05d4dd2fa`.
The checkout remains dirty/shared; this is an integrated-worktree result, not a
tagged or release-qualified source tuple.

- Hardened same-broker reclaimer single-flight and typed synchronous reentry;
  event sinks are single-flight across threads and drop overlapping advisory
  events rather than queueing/replaying. Read-side caller `Clone`/`Ord`/identity
  panics still propagate, but only after the guard is normally unlocked. Slow
  callback bounds and caller interior side effects remain open boundaries.
- `scripts/check-workspace.sh`: exit 0. Core 245, facade 282, testkit 82,
  facade E2E 18, policy contracts 15, facade public surface 20, core public
  surface 7, finalize-hook kill 2, SIGKILL/reopen 3, facade conformance 4,
  testkit public surface 1, and 10 non-zero feature-capability paths passed.
  Doc selector validation: 228/228; feature matrix, PM lint, cargo-deny passed.
- Both public examples ran successfully. `cargo fmt --all -- --check` and
  `git diff --check` passed.
- Five re-applied ticket mutants RED: Reserved misclassified Finalized (FUP-001);
  foreign backend claim accepted (FUP-002); class-axis kind swap accepted
  (FUP-004); 10,000 provenance records appended (FUP-005); and live zero-byte
  ticket owner ignored during group removal (FUP-007).
- Two new boundary mutants RED: omitting guard release before panic propagation
  caused the panic-read regression to observe `Poisoned`; omitting the
  `payload_store` feature from the helper gate caused the payload-only build to
  fail. Every production file was restored byte-identically to its pre-mutation
  SHA-256: Redb `d7d44c1d71fa9ba788dae963ab6347d530f817386f331894434d5c6dfdf8e554`,
  ledger `4750d6a428a5fe8974a706226b611cf983e5f40302056de94ca4a2b8e5a2ce29`,
  broker `4fde67275bcc1f31a0e9c35748fa1e09df159d2f2f15515248c42c36e82c5b24`,
  and lifecycle `c69bbaec096a1db1e870458e0ecf8acbb7fa89967d4affd9f0533e61d855b0ee`.
- The Linux L2 receipt at executable digest `add1f907…` predates this pass and
  is not current-source evidence. Clean extraction, scaling verifier, exact
  host/QBC pair, current-source Linux L2, hosted CI, deploy, and activation are
  not established by this entry. Device-mapper L2 and physical L3 remain open.

### Final source-capture check (2026-09-23)

The shared worktree intentionally deletes tracked `.cursorrules` while moving
guidance to `.cursor/rules`. `scripts/durability/run-emulated-linux.sh` used
`git ls-files -co`, which includes deleted tracked paths; hashing them made the
current-source receipt fail before test execution. The digest filter now binds
only existing, non-excluded candidate inputs. Its current digest is
`51433fd5d80f61348a288bf17d2947b4a3cd8ca0278c757de9c0279215d8b1a6`.
ShellCheck, `bash -n`, `git diff --check`, and `cargo fmt --all -- --check`
pass; executing the L2 runner on Darwin exits 77 with its documented Linux-only
unavailable result. No current-source L2 injection is claimed. A clean isolated
validation snapshot at `9b14a332c07afd9002d6c1ec06b6cf7899a2a8a3` (tree
`ea575ce01f43e511442e972f85fa0edd2f2e7276`) passed standalone extraction and
the full extracted `scripts/check-workspace.sh`; canonical extraction digest
was `45a22536e4bb5422fbf3ba341b5303c0389f76523a89af1e9cd8406c6f56c819`.
`scripts/run-scaling-bench.sh --smoke` verified 33 samples. The snapshot is a
temporary validation commit, not a commit to the shared branch. Physical L3,
block-device L2, host/QBC, CI, deploy, and activation remain outside this local
proof.

### Session-hardening follow-up (2026-09-23)

Review found that the Linux L2 example copied trace/database artifacts into the
reported artifact directory but left the duplicate work directory under the
system temp root. Added explicit cleanup after artifact copy plus a best-effort
RAII fallback on early return. The updated executable-input digest is
`ef0484f9776a4e51da9ba33dacda9a3dbd5a954485ed0adc08c77c34d0813ad7`.
Validation snapshot `e4b97393f0d984dd6f9fce6afcaf21b5da38207b` (tree
`c13632baa95cef4d0c24725bb23c3fba2ca2060e`) passed standalone extraction and
the full extracted workspace rail; canonical extraction digest was
`07d17bee1f15caea44761e3644ed6bd3a364429c50aecb008cc0fde04ce0f6fd`.
Scaling smoke verified 33 samples. Linux-only L2 example behavior remains
unexecuted on this Darwin host; cleanup has static/format proof only. The
snapshot is temporary validation state, not a shared-branch commit.

### Independent session-hardening audit rerun (2026-09-24)

The checkout remains dirty/shared: `HEAD` is
`edfef66c3d9dabbe4d01a98754a44d32ef9c940e`, tree
`4f1a7299af049e7fa1c0eeefc8e683f01239d462`, with 44 paths in
`git status --short`. The executable-input digest used by the L2 runner's
filter (docs/tickets excluded) changed during the canonical run from
`72d056152f12cc2433b04904c7ea3a4f9857c8c02788f9fcc042652d6401328f` to
`6b258febdb6e8503ab87575506b0c8a2e8967eb35d8658ba2f8170c76984a7d3`.
The canonical rail exited 0, but its mixed pre/post-change result is not
source-frozen qualification. The later owner-selector, panic-path, and example
runs below were each guarded by matching before/after executable-input digests
at `6b258f…`.

- Canonical `scripts/check-workspace.sh`: exit 0, `check-workspace: clean`;
  observed 251 core, 292 facade, 82 testkit unit tests; policy contracts 15,
  Hellgate 18, public API 20, finalize-kill 3, SIGKILL/reopen 4, facade
  conformance 4, testkit public surface 1, core/facade doctests 13/28;
  10 advertised feature-capability selectors ran non-zero; 246 documented
  selectors resolved; feature matrix, PM lint, and cargo-deny passed. Because
  the input digest drifted, these totals describe the run and not one exact
  full-candidate source tuple.
- On stable executable digest `6b258f…`, workspace selector reruns selected
  `sep23_fup`: core 23, facade 14, kill/reopen integration 3, testkit 4; all
  passed. `sep23_audit_` selected core 10 and facade 4; all passed. Newly added
  durable commit/restore-panic reconciliation tests each passed 1/1, and
  `hello_world` plus `effective_budget_report` both ran successfully.
- `cargo fmt --all -- --check`, `git diff --check`, and PM lint passed. The
  selector rerun and examples had matching before/after executable-input
  digests; this does not freeze the wider shared checkout or establish an
  extraction/release tuple.
- Five ticket mutants and two boundary mutants from the prior callback/read-
  panic pass were not re-applied here. Their recorded SHA/revert results and
  the original 15-mutant battery remain historical evidence, not fresh mutant
  results against digest `6b258f…`. Post-run owner hashes confirm Redb
  (`d7d44c1d…`) and shared lifecycle (`c69bbaec…`) still match the recorded
  mutant preimages; ledger (`3e3e5654…`), broker (`df13b546…`), and admission
  (`d936cf8e…`) do not match their recorded mutant preimages, so the FUP-004,
  FUP-005, FUP-007, and G1 mutation results are stale for this source.
- The scaling harness explicitly rejects a dirty source tree; the standalone
  extractor requires clean canonical inputs. Neither is current-source GREEN.
  The prior exact-source host/QBC tuple predates this worktree. Hosted CI,
  deployment, activation, and current-source Linux L2 remain unrun/unavailable;
  physical power-loss evidence remains unverified.

### Clean-snapshot closeout audit (2026-09-24)

The shared checkout advanced to `5cc15c39712704f325cc849a916c3d575d5eb672`
while another writer was editing it. To avoid treating a moving dirty tree as
qualification, this audit copied its exact file content into clean temporary
validation commits; these are **not** commits on the shared branch.

- Final full workspace rail: snapshot `c20acad3bd29a8b490408914f30b5144eedfb5eb`,
  tree `f6c226d4e16822e0d4a71217b13f60d9e513418c`, before/after identical,
  `scripts/check-workspace.sh` exit 0. Core 251, facade 305, testkit 82 unit
  tests passed; the seven SIGKILL/reopen integration tests, ten advertised
  feature paths, 255 documented selectors, PM lint, and cargo-deny passed.
- Full standalone extraction: prior clean snapshot
  `7f7b812805d83d980b9778239bff1744d2eb35c3` (tree
  `55ed73edf4885002ac31a255025ec8099fa98669`) produced canonical digest
  `a6ba813de538b6045ad6102679fa8b896a8d02f4c3c776abaa350eca1bacb88e`.
  The extracted workspace ran its own full `scripts/check-workspace.sh` at exit
  0 and became clean extracted commit `864813da8abcb7e1b4f8fe5ba0ce3420b07266f9`.
  Both public examples ran there. Its full scaling verifier passed 170 samples
  with no comparison baseline. That proves this local structural/cardinality
  verifier, not a host SLO or physical disk bound. An earlier full run on an
  older snapshot failed a wall-clock growth check amid concurrent Rust builds;
  the raw result is not silently promoted to a code regression or erased.
- The final snapshot differs from that extracted snapshot only in Redb
  `#[cfg(test)]` oracles and a plan document; production and benchmark inputs
  are byte-identical. The final snapshot's clean extraction pre-flight also
  passed. The stronger corruption/rollback tests are included in its 305
  facade tests; no full in-extract rail is claimed for the final test-only delta.
- Five re-applied FUP mutants (reserved-hole, foreign backend claim, class-kind
  axis, bounded provenance, live zero-byte owner) and two new resource mutants
  (retained payload byte cap and zero-byte in-flight buffer count) each produced
  a behavioral assertion RED with Cargo exit 101 on the immediately preceding
  clean source. The isolated mutant worktree was restored byte-for-byte
  (`git diff --exit-code` and tree equality). Later snapshot differences do
  not alter these production sites or selected oracles.

This addendum is a documentation-only edit **after** the frozen runs. The
shared branch still has other-writer uncommitted files, so it is not a clean
release commit. Current-source Semantica host/QBC, Linux L2 fault injection,
physical power-loss L3, hosted CI, deployment, and activation remain separate
unverified gates; historic receipts do not close them.

Postscript: after the clean-snapshot runs and this addendum, another writer
changed production `spill/backings/redb.rs` again (a capped file backend and
`max_database_file_bytes`). The frozen results above therefore do **not**
qualify the subsequently moving shared checkout. Re-freeze its final content
and rerun the affected local, extraction, scaling, and host rails before any
release/activation claim.

## 2026-09-24 committed QMP / committed Semantica local pair

This section supersedes the historical **local** stale-rail statements above
for the named pair only; it does not create a v5 tag or a deployment receipt.
QMP was clean at the start of these runs: HEAD
`7f1c2f8352e62dd8e186ba3ede1f60185a0af4b9`, tree
`567fb4d72ec5cf343bce49db9a5b24fc439f7287`, equal to `origin/main`.
Semantica was verified from a separate clean, detached worktree at
`a71e3e975dcab23aa654ea0c4ea5a66183000412`; its workspace metadata
resolved both QMP crates by path to this QMP checkout. The shared Semantica
main worktree's unrelated dirty changes are **not** part of this pair.

| Local rail | Observed result on the named pair |
| --- | --- |
| `CARGO_TARGET_DIR=<external target> scripts/check-workspace.sh` | Exit 0, `check-workspace: clean`; all-feature workspace, feature capability, seven Redb SIGKILL/reopen tests, 284 documented selectors, PM lint and cargo-deny passed. |
| `scripts/extract-to-standalone-repo.sh --dry-run <external destination>` | Exit 0; committed QMP HEAD/tree frozen; extracted-file manifest SHA-256 `87faa47955f1cf87938791e419a01ae6479a856d010642b29c88b0eae8446956`. This is pre-flight, **not** an extracted workspace build. |
| `scripts/durability/run-enospc-docker.sh` | Exit 0; five isolated Linux tmpfs modes passed, including charged spill and charged restore commit; source-before/after SHA-256 `17684892cbef438bb6cb6282ce1ccaa388d1e24710373a19eff83ea9aa93c400`, `source_stable=true`. No physical power-loss claim. |
| `scripts/run-scaling-bench.sh --smoke` | Exit 0; verifier accepted 48 rows; run-end `source_clean=true` and QMP HEAD/tree unchanged. Raw JSONL `target/scaling-results/qmp-scaling.qfGfwl/results.jsonl`, SHA-256 `3469f9543231f71673ee4bc8b0c3cc761a73bfd68d3d6e43d56b70e887d052d3`. |
| `scripts/run-scaling-bench.sh` | Exit 0; verifier accepted 220 rows; run-end `source_clean=true` and QMP HEAD/tree unchanged. Raw JSONL `target/scaling-results/qmp-scaling.71wspY/results.jsonl`, SHA-256 `cf07f5a6bffd825525e29b803b1b0fe534be581e02cd809649066f44a3514d02`. The verifier's permissive threshold is **not** a production latency SLO. |
| QBC `owner run`, `quanta-memory-host`, `all-tests`, `package-all-features` | `GREEN`, 129 passed / 0 failed; run `20260924T034719.894082Z-1fbccc48507e`. |
| QBC `owner run`, `quanta-runtime-memory-compose`, `all-tests`, `package-all-features` | `GREEN`, 48 passed / 0 failed; run `20260924T034802.136358Z-247666960b60`. |

Both QBC runs used the same host source snapshot digest
`fcacc666479fabebe606b19183b328786c69f0863c7a29e1d1027327afa7b7e0`
and the external QBC target root under `/Volumes/Extreme SSD 1/build-cache`.
The host manifest enables facade `full` and **not** `spill-redb`; optional
Redb's five-mode Linux oracle is supplemental QMP-capability evidence, **not
an active Semantica release gate**, a host Redb consumer, or host
filesystem-policy qualification. Semantica's reconstructible
revocable caches may be lost on reclaim/restart and rebuilt from their
authoritative source.

After these runs, another writer changed only `README.md` and
`README-detailed.md` in the shared QMP checkout. This receipt edit itself is
also post-run documentation. Therefore the rows prove the frozen QMP
`7f1c2f8` code candidate and committed Semantica pair, **not** a later dirty
working-tree snapshot or any subsequently published commit. Hosted CI,
deployment/activation, physical power loss and current-source Linux L2 syscall
fault injection were not run. No successor tag for this pair was created;
the older v5 tag targets its own Sep-23 source.

## Current committed base after the historical pair

The verifier change is at clean audit base
`db0b03cd3e58a53dd5439a6578bb3b3d86eac593` (tree
`388598df45c1f93a9ce4c8acc7b047fe43a30091`). There the verifier Python
contracts passed 20/20; `cross_store_hellgate` passed 2/2 with
`core-live-accounting,retained_map,revocable_store`; and replay through the
revised verifier accepted the prior 48 smoke and 220 full samples. A clean
`db0b03c` smoke artifact separately passed 48/48 with SHA-256
`502f6e70a2009f6e1c725a72a9167db8895a25117bb4facfc738fd7532b4f8a4`.

The later committed base `65f57d5` changes documentation and Cargo/Rust
comments only; it does not change executable behavior. This reconciliation
commit freezes the current qualification candidate, but does not itself prove
the gates. The exact-target `qmp-local-qualified-*` annotated tag is the
immutable receipt authority for repository canonical, fresh full benchmark,
and extraction dry-run. Resolve it and verify its target against HEAD before
claiming current local qualification. Host QBC, hosted CI, deploy and
activation remain separate. This document describes the receipt contract;
the tag annotation carries each run's source identity, command, exit, counts
and artifact hashes.
