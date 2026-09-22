# Sep-23 follow-up qualification receipt

Status: `QMP QUALIFICATION CANDIDATE — final authority is annotated tag qmp-final-qualified-sep23-followup`
Scope: `full-followup` (`FUP-001` through `FUP-007`)
Date: `2026-09-23 Asia/Seoul`

This file is part of the candidate tree, so it cannot contain its own final
commit ID. Resolve the annotated tag and verify that it targets the current
candidate:

```bash
git rev-parse qmp-final-qualified-sep23-followup^{}
git show --no-patch --format=fuller qmp-final-qualified-sep23-followup
```

The tag annotation is the immutable source/result authority. Absence of that
tag, a tag/HEAD mismatch, or a dirty tree downgrades this document to planning
evidence.

## Source lineage

- Audit plan base: `028448946caa95676df034481a830c1d3e3b1f33`.
- First clean implementation candidate:
  `beba43760d76a60fbce7ccbf8ed97d3985aeaaf3`, tree
  `6c88d590f4b67c232098599ca99c1570c4d0a0e5`.
- The final candidate adds only self-audit-discovered boundary hardening,
  regression oracles, ticket state, and this receipt on top of that commit.
- No Cargo manifest, dependency edge, public builder surface, or storage
  schema was changed by the final self-audit delta.

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

## Host-pair result

Read-only Cargo metadata from both Semantica workspace roots resolved the same
QMP `0.2.0` manifests under
`/Users/songmin/Documents/code-new/quanta-memory-platform`.

- `quanta-memory-host --all-features --tests --locked`: GREEN,
  `114 + 5 + 7` passed.
- `quanta-runtime-memory-compose --tests --locked`: RED,
  `43 passed / 1 failed`.
- Failure:
  `tests::runtime_memory_bootstrap_core_owns_production_budget_and_live_dependencies`;
  actual default `override_sources=[]`, expected
  `["runtime_defaults"]`.

This is a Semantica projection defect, not a QMP broker failure. Exact owner
file, patch boundary, paired oracle, QBC commands, and dirty-repository stop
conditions are in
`tickets/HOST-PAIR-RUNTIME-DEFAULT-PROVENANCE.md`.

Therefore:

| Boundary | Verdict |
| --- | --- |
| QMP owner-local behavior | GREEN |
| QMP canonical repository rail | tag annotation authority |
| QMP standalone extraction | tag annotation authority |
| Semantica dependency identity | GREEN |
| `quanta-memory-host` raw comparison | GREEN |
| Semantica compose/QBC host-pair | RED / OPEN |
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
