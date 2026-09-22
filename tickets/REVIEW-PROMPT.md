# Review prompt — test-optimization wave (TO-01 … TO-09)

Paste everything below the line into a fresh review session (repo root,
branch `main`).

---

Review the completed test-optimization wave on `main`. Base `4aff891`
(all 22 owner digests frozen in `tickets/TEST-OPTIMIZATION-EXECUTION.md`);
implementation commits `415e659`, `3713dcc`, `41faf05`; current HEAD
`943cf0a` (includes follow-up qualification/docs commits — note which
HEAD you actually reviewed).

## Scope (read-only first; no commits, no ticket edits)

1. `tickets/TO-01-redb-concurrency.md` … `TO-09-transcript-partitions.md`
   — spec + acceptance + `## Closeout receipt` in each file.
2. `tickets/TEST-OPTIMIZATION-EXECUTION.md` — write leases, TO-03→TO-02
   order, non-negotiable rules.
3. `tickets/TEST-OPTIMIZATION-RECEIPTS.md` §§1–14 — integrator log,
   owner medians/mutants, smoke artifact, final round.
4. `tickets/TO-AUDIT-structural.md` — prior structural audit (7 violations,
   gap list). Confirm each item is closed or still open.
5. Code diff `4aff891..HEAD` for every owner file in the lease table.

## Verify (evidence, not vibes — cite file:line or command output)

- Per ticket: implementation matches the ticket's file-by-file plan;
  keeper oracles intact (compile_fail snippets, probe-bound test, FIFO
  tests, exact sequences/multisets, snapshot equality, partition
  literals); no assertion was loosened to pass — check the RED direction.
- Independently reproduce at least 3 mutant-RED proofs yourself
  (suggested: TO-05 full-scan → `left: 35, right: 3`; TO-09 cost swap →
  `left: 41, right: 148`; TO-02d O(N) rescan per reclaim → verifier
  `6.96x vs 4.0x`). Revert each mutant and confirm
  `git diff HEAD -- <file>` is empty afterwards.
- Confirm no leftover mutants anywhere:
  `rg -n "TEMPORARY MUTANT|MUTANT" crates/ scripts/` must be empty.
- Confirm no production weakening: TO-08's three files show no
  production hunk; TO-03 emit/schema lines unchanged; TO-09 production
  untouched.
- Confirm lease discipline: every non-ticket change outside the lease
  table (redundant_clone rollout, extraction-contracts.sh,
  semgrep fixtures, doc rewrites) is either justified in the receipts
  or flagged.
- Re-run and report: `cargo fmt --all -- --check`,
  `cargo test --locked --workspace --all-features` (expect 12/12, 642),
  `scripts/verify-feature-matrix.sh`,
  `cargo test -p quanta-memory-platform-facade --doc --all-features`.
  Note: `scripts/check-workspace.sh` halts at the semgrep stage on hosts
  with empty CA trust anchors (environmental, pre-existing) — report
  whether it runs on your host or reproduces the blocker.

## Output

1. Verdict table: per ticket RESOLVED / NOT RESOLVED with one-line reason.
2. Findings ranked S/M/L with file:line and suggested fix (do not fix).
3. Explicit statement: is anything still owed before an annotated
   `qmp-final-qualified-*` tag may be issued? If yes, list it exactly.
