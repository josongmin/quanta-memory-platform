# TO-AUDIT-structural — Structural Audit Assembly

Status: `HISTORICAL AUDIT at d370529; its gaps were subsequently resolved for the v5 source; post-v5 qualification is separate`.
Authority: [FINAL-CLOSEOUT.md](FINAL-CLOSEOUT.md) and the annotated
`qmp-final-qualified-sep23-followup-v5` receipt. The verdicts and unchecked
gap list below record this audit's earlier observation, not today's queue.
Later owner mutant proofs appear in
[TEST-OPTIMIZATION-RECEIPTS.md](TEST-OPTIMIZATION-RECEIPTS.md)
§§11–15 and the TO-01..09 ticket handoffs. The clean v5 source included the
implemented queue and passed its recorded repository, extraction, and host
rails. The verifier delta and focused evidence are at `db0b03c`; latest
committed base is `65f57d5`, with non-behavioral docs/comments after that audit.
No exact-HEAD canonical or fresh full benchmark receipt exists, and the shared
worktree has a separate uncommitted RFC edit. See the
[plan index](../docs/plans/README.md).

Current resolution: TO-01..09 implementation and ticket-local evidence are
recorded in their owner handoffs and [receipts §§11–15](TEST-OPTIMIZATION-RECEIPTS.md).
The annotated v5 tag qualifies the past source containing them. No row in the
historical verdict table below is an open TO implementation assignment.
Post-v5 changes to owner files need their own evidence and a new source-wide
qualification boundary; historical mutant proofs are not inherited by changed
behavior.

## Assembly provenance

The six delegated prior child results (refs 1–6) arrived as summaries only
(`structured result submitted`) with no inspected evidence attached. Per the
workflow reuse rule they are treated as discovery pointers, not evidence.
Every verdict below is therefore assembled from tracked sources inspected in
this session:

- `tickets/TO-01-redb-concurrency.md` through
  `tickets/TO-09-transcript-partitions.md` (spec + acceptance text).
- `tickets/TEST-OPTIMIZATION-EXECUTION.md` (leases, ordering TO-03→TO-02,
  closeout-receipt rule).
- `tickets/TEST-OPTIMIZATION-RECEIPTS.md` §§1–10 (integrator observation at
  `41faf05`, owner addendum, exact-source candidate, TO-02 smoke artifact).
- `tickets/FINAL-CLOSEOUT.md`, `tickets/FINAL-001-file-inventory.md`.
- Live implementation markers verified by grep this session: `WRITERS=4` /
  `Barrier` in `facade/src/spill/backings/redb.rs`; `begin/end_allocation_epoch`
  in `facade/benches/scaling.rs`; `classify_spill_failure` in
  `testkit/src/faults/scenarios.rs`; `assert_single_release_probe_bound(32/128)`
  in `core/src/domain/ledger.rs`;
  `zero_shared_pool_cannot_extend_borrowing_class_hard_cap` and
  `builder_reclaimer_reaches_build_live_broker`; `expected_partition_costs` /
  `ORDERED_INDEX_BYTES_ORACLE` in `facade/src/transcript_store/service.rs`;
  `compile_fail` keepers in `facade/src/retained_map/service.rs` and
  `facade/src/lib.rs`.
- Live hygiene state: `git status --short` empty, `git stash list` empty,
  `git worktree list` (main at `d370529`, detached peers preserved),
  `tickets/TO-AUDIT-structural.md` confirmed absent before creation.

Grouping: the nine TO tickets are collated into six audit tracks
(T1–T6) so the six prior findings map 1:1 to a track section.

## Historical per-ticket verdict table (at the audit snapshot)

| Ticket | Track | Owner file(s) | Implementation marker (observed) | Verdict |
|---|---|---|---|---|
| TO-01 redb concurrency | T1 timing oracles | `facade/src/spill/backings/redb.rs` | `WRITERS=4`, `WRITES_PER_WRITER=2`, `Barrier(WRITERS+1)` present | IMPLEMENTED, receipt pending; after-median closed by owner addendum (0.76s vs 6.53s, ~88%), mutant-RED excerpt still owed |
| TO-02 contract authority | T2 compiler/feature authority | payload/retained/revocable services, `facade/src/lib.rs`, facade `public_api_surface.rs`, `verify-feature-matrix.sh`, `verify-scaling-results.py`, `test_scaling_results.py`, `scaling.rs` (after TO-03) | `compile_fail` keepers present (retained-map ×2, lib ×4); 642-test green + 23 facade doctests per receipts | IMPLEMENTED, receipt pending; smoke artifact CLOSED (§10: `00ab2a6` smoke exit 0, 33 samples OK); 4 mutant REDs + zero-grep proof still owed |
| TO-03 scaling epoch | T2 compiler/feature authority | `facade/benches/scaling.rs` | `begin/end_allocation_epoch` epoch scaffolding present | IMPLEMENTED, receipt pending; `--no-run` log + revert-mutant RED still owed; smoke rows confirmed zero-alloc in §10 artifact |
| TO-04 fault negative pole | T3 fault classifier | `testkit/src/faults/scenarios.rs` | `classify_spill_failure` shared by honest/lying/unreadable poles | IMPLEMENTED, receipt pending; success-arm→`Contained` mutant RED still owed |
| TO-05 ledger cardinality | T1 timing oracles | `core/src/domain/ledger.rs` | `assert_single_release_probe_bound(32/128)`, exact-`3` probe keeper | IMPLEMENTED, receipt pending; after-median closed by owner addendum (median 0.00s vs 0.09s); full-scan mutant RED still owed |
| TO-06 zero-pool admission | T4 admission semantics | `core/src/domain/policy.rs`, `core/tests/policy_contracts.rs` | `zero_shared_pool_cannot_extend_borrowing_class_hard_cap` present; validation-only rename per spec | IMPLEMENTED, receipt pending; 1-byte-admission mutant RED still owed |
| TO-07 builder reclaimer | T5 wiring oracles | `core/src/api/builder.rs` | `builder_reclaimer_reaches_build_live_broker` present | IMPLEMENTED, receipt pending; `None`-reclaimer mutant RED still owed |
| TO-08 core test cleanup | T6 hygiene/cleanup | `core/src/ports/telemetry.rs`, `core/src/api/control.rs`, `core/tests/public_api_surface.rs` | Spec-scoped deletion only; no mutant required by ticket | IMPLEMENTED, receipt pending; keeper-implication diff proof still owed |
| TO-09 transcript partitions | T5 wiring oracles | `facade/src/transcript_store/service.rs` | `expected_partition_costs`, oracle constants, literal `(46,148,41,235)` path present | IMPLEMENTED, receipt pending; negative proof CLOSED (swap mutant RED at `service.rs:1510`, `left: 41, right: 148`; suite 47 green; production regions byte-identical after revert) |

No ticket meets the execution SSOT closeout rule (preimage/postimage HEAD,
digests, exact commands+exit codes, mutant-RED excerpts, medians recorded
in-repo). None may be flipped to `DONE` on this file's authority.

## Historical violation list (ranked at the audit snapshot)

1. **Severe — per-ticket mutant-RED evidence outstanding (TO-01…TO-08).**
   Each ticket's acceptance clause requires a temporary-mutant RED log;
   TO-09 alone has an owner-verified RED excerpt. Any `DONE` flip without
   these violates the execution SSOT. Owner action, not a rail rerun.
2. **Severe — exact-source receipt still pending for all nine.**
   Ticket headers read `IMPLEMENTED; final exact-source receipt pending`.
   Only the newest annotated `qmp-final-qualified-*` tag closes them; the
   historical bare tag and the `71f06e4` candidate observation do not.
3. **High — committed out-of-lease absorption without reviewer sign-off.**
   Receipts §2 records two dirty states absorbed by concurrent commits:
   (a) 36-file set (`.cursorrules`, `AGENTS.md`, `CHANGELOG.md`, `CLAUDE.md`,
   `tools/pm/**`, `scripts/tests/test_workspace_policy.py`,
   `scripts/extract-to-standalone-repo.sh`, …), mostly outside writer leases;
   (b) 4-file set (`CHANGELOG.md`, `docs/verification-scenarios.md`,
   `test_workspace_policy.py`, `verify-feature-matrix.sh`). Final tree is
   clean, so no pending violation — but HEAD `41faf05`-era contents were
   committed without integrator review and must be audited against leases.
4. **High — extraction-script verification removal in committed history.**
   An earlier dirty state removed the outer `cargo metadata --locked |
   check-extracted-metadata.py` verification from the non-dry-run extraction
   path. The owner addendum (§8) declares this intentional (destination
   canonical rail owns the same resolved-graph check; keeping both is
   duplicate validation). Ticket owners must confirm intent; until then this
   stays a flagged committed change, not a pending tree violation.
5. **Medium — semgrep regression-guard coverage unverified in-session.**
   The rail's semgrep stage crashed at startup (empty CA trust anchors, exit
   2) during the integrator window, so later stages were run individually
   (12/12 suites, 642 passed). Historical only: the `71f06e4` candidate
   later scanned all 66 targets with all six rules, zero production findings.
   Re-run where semgrep can start before any qualification claim.
6. **Low — `facade/src/lib.rs` writer conflict (resolved by ordering).**
   TO-02's plan notes a concurrent workspace-policy overlay moving
   `#![forbid(unsafe_code)]`. Rule: no TO-02 edit until that owner commits,
   then re-freeze the blob digest. No evidence of a violated edit; retained
   as an ordering constraint.
7. **Low — stale per-ticket status fields.**
   Receipts §6 notes all nine files still read `Status: OPEN` with spec-only
   content while implementation is landed. Cosmetic; the fix is filling the
   closeout-receipt fields per ticket, not editing status prose first.

## Historical gap list (subsequently reconciled in the receipts)

Each item names the owner and the exact artifact owed. Substitute evidence
(full-rail green, doctest pass) does not close these.

- [ ] TO-01 owner: temporary-mutant RED excerpt (record-ID outside serialized
  transaction or duplicate key → duplicate/loss/sequence RED); focused-test
  command log; record after-median in-repo (addendum has 0.76s; ticket field
  still empty).
- [ ] TO-02 owners: four mutant REDs (raw-ctor/late-attach compile-fail flip;
  `full`+`unbudgeted-stores` feature-matrix RED; linear-scan probe-bound RED;
  quadratic-drain normalized-growth RED on 1k/10k fixtures); `rg` zero-grep
  classification for residual `include_str!`/`read_to_string`.
- [ ] TO-03 owner: `cargo bench … --bench scaling --no-run` log;
  `Vec::with_capacity`-revert mutant RED (non-zero allocation assertion fires);
  panic-safety note for the scoped epoch guard if one was introduced.
- [ ] TO-04 owner: `spill_failure` focused log; success-arm→`Contained`
  mutant RED through the public driver (local-classifier RED not accepted).
- [ ] TO-05 owner: full-scan + per-visit-counter mutant RED at exact-`3`
  assertion on the large-cardinality run; record after-median in-repo
  (addendum has median 0.00s).
- [ ] TO-06 owner: zero-pool 1-byte-admission mutant RED through the public
  broker contract (validation-only RED not accepted); snapshot-equality +
  final-zero lines in the log.
- [ ] TO-07 owner: `builder_reclaimer` behavior log; `reclaimer=None` mutant
  RED (request-count assertion); `build_live`-reroute mutant outcome.
- [ ] TO-08 owner: keeper-implication diff proof, one line per deletion
  (duplicate telemetry row ⇒ keeper row; dead `assert_ne!` ⇒ exact `400_000`
  postimage; eight→nine comment ⇒ nine-element array); `git diff` showing no
  production hunk.
- [ ] TO-09 owner: none outstanding on the mutant (closed, see verdict table).
  Remaining: record the RED excerpt + post-revert green log in the ticket
  field itself rather than only in receipts §8.
- [ ] Bench smoke (shared): TO-02 smoke CLOSED by §10 artifact
  (`00ab2a6`, `run-scaling-bench.sh --smoke` exit 0, 33 samples, admission
  epoch all-zero, live bytes zero). TO-01/TO-05 medians must stay out of test
  assertions (receipt-only metrics). TO-03 `--no-run` compile gate still owed
  per above.
- [ ] Integrator: re-run the single-command full rail
  (`scripts/check-workspace.sh`) where semgrep can start, on the exact
  receipt HEAD, before binding the annotated tag.

## Hygiene verdicts on concurrent-session changes

| Check | Result | Verdict |
|---|---|---|
| Working tree (`git status --short`, `git diff --name-only`) | empty, exit 0 | CLEAN — no pending violation |
| Stash list | empty | CLEAN — nothing shelved |
| Worktrees | main `d370529`; detached peers (`/private/tmp/qmp-session-hardening`, codegraph cache, `goal-w1-qbc-current@4aff891`, `ag9-live-sibling-snapshot`) preserved per closeout disposition | PASS — no unowned cleanup performed here |
| Concurrent-commit absorption (§2 sets a/b) | committed history, not integrator changes; out-of-lease paths flagged in violation #3 | PASS with note — audit committed HEAD against leases; do not revert blindly |
| `lib.rs` overlay conflict | ordering rule respected; keepers observed live | PASS — no cross-writer edit evidence |
| New-file discipline | this file is the sole creation of this session task; no other paths written | PASS |

## What this file does not do

- Does not mark any TO ticket `DONE` and does not substitute for per-ticket
  closeout receipts or the annotated qualification tag.
- Does not re-derive the six prior summaries (they carried no evidence);
  track sections above stand on the inspected sources listed under
  provenance.
- Rollback order if needed: revert per owner file only; shared `scaling.rs`
  unwinds TO-02 before TO-03 (writer order TO-03→TO-02).

## Post-`943cf0a` review reconciliation

This section updates the historical gap list above without rewriting its
original observation. Receipts §§11, 13, and 14 supply the previously missing
owner mutant and keeper proofs; the later `qmp-final-qualified-d370529` tag
binds only its own older source. A new review found two post-tag issues:

1. TO-02 late-attach rustdoc used method-as-field expressions. A public
   `charged_against` method left retained-map doctests GREEN. The correction
   resolves method items on all five affected concrete types, and public
   mutants now make each corresponding doctest RED (receipts §15).
2. `e535f54` changed the production ledger's ticket map outside TO-05's
   test-only lease. The correction restores `BTreeMap` (receipts §15).

The command-level TO-02 matrix discrepancy is reconciled as a deduplication
of identical resolved feature sets in the ticket and final inventory. The
Semgrep CA startup failure is historical: `scripts/check-workspace.sh` passed
at `943cf0a` in the review host. The later v5 tag binds its own clean source
and host pair. It does not cover the `db0b03c` verifier delta.
