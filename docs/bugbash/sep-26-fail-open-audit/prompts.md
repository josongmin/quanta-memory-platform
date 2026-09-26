# Fail-open test audit — copy-paste prompts (sep-26 round)

11 self-contained prompts. Run 01–09 in parallel (disjoint file ownership, no
conflicts). Run 10 (critic) after the 01–09 findings exist. Run 11
(synthesis) last.

Round directory for all reports: `docs/bugbash/sep-26-fail-open-audit/`
Naming convention: every report is `findings-{범위}.md` (scope slug in
english, kebab-case). The synthesis index stays `99-index.md`.

| # | Scope slug | Report file |
|---|---|---|
| 01 | core-domain | `findings-core-domain.md` |
| 02 | core-api | `findings-core-api.md` |
| 03 | app-ports | `findings-app-ports.md` |
| 04 | revocable-store | `findings-revocable-store.md` |
| 05 | facade-stores | `findings-facade-stores.md` |
| 06 | spill-redb | `findings-spill-redb.md` |
| 07 | crash-rails | `findings-crash-rails.md` |
| 08 | pipeline-shared | `findings-pipeline-shared.md` |
| 09 | testkit | `findings-testkit.md` |
| 10 | critic | `findings-critic.md` |
| 11 | synthesis | `99-index.md` |

---

## Prompt 01 — T1 core-domain (ledger/policy/config/admission)

```
You are auditor T1 of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

What this repo is (30 seconds): a portable memory budgeting/accounting library.
Three crates: quanta-memory-platform-core (domain/application/ports/api),
-facade (stores, spill, pipelines), -testkit (conformance, faults, fakes).
The ledger is fail-closed: every mutating accounting op fully succeeds or
leaves state untouched. Allocator telemetry is advisory only. Tests exist to
catch real regressions, not merely to import modules.

GROUND RULES (hard):
- READ-ONLY for code. You may create exactly ONE new file, your report:
  docs/bugbash/sep-26-fail-open-audit/findings-core-domain.md
  No other edits, no `pm sync`, no commits.
- The worktree has unrelated dirty changes (README.md, README-detailed.md,
  facade/tests/spill_redb_kill_reopen.rs, deleted
  docs/bugbash/sep-22-test-optimization-deep/**). Leave them untouched.
- Read first: AGENTS.md, tools/pm/sources/rules/tests.md,
  tools/pm/sources/rules/cheatsheet.md, and the ledger/conformance sections of
  tools/pm/sources/rules/determinism.md.
- You may run scoped read-only cargo test to observe behavior, e.g.:
  cargo test -p quanta-memory-platform-core --all-features --lib
- NEVER mutate SUT code. A separate critic agent does mutation verification.
  Your job is to name the mutation; theirs is to apply and measure it.

YOUR SCOPE (you own these files; no other auditor touches them):
- crates/quanta-memory-platform-core/src/domain/ledger.rs (~38 test fns)
- crates/quanta-memory-platform-core/src/domain/policy.rs (~14)
- crates/quanta-memory-platform-core/src/domain/config.rs (~27)
- crates/quanta-memory-platform-core/src/domain/admission.rs
- every other *.rs under core/src/domain/ containing #[cfg(test)]
- crates/quanta-memory-platform-core/tests/policy_contracts.rs (~15)

CENSUS REQUIREMENT: grep your scope for #[test] and #[tokio::test] yourself
(the counts above are advisory). Produce exactly one table row per test fn.
Row count MUST equal your grep count. Sampling is a defect; the critic will
reject your report.

VERDICTS:
- STRONG  — breaking the behavior the test names would turn it red; oracle is
  tied to the real authority (ledger byte totals, exact error variants,
  restored content).
- WEAK    — asserts something real but strictly weaker than the name claims
  (shape-not-content, count-not-order, is_err() without variant check).
- FAILOPEN — the failure mode the test names can occur while the test stays
  green (swallowed Result on the op under test, vacuous loop, assert only on
  test-constructed data, unreachable error path).
- VACUOUS — the named path cannot be exercised by this body at all (dead
  setup, condition never true, helper chain that ends without asserting).

FAIL-OPEN ARCHETYPES — check every test against all eight:
A1 assert-on-input: the asserted values were produced by the test/fixture
   itself, not by the system under test.
A2 vacuous iteration: a for/while over a collection that can be empty; zero
   iterations = unconditional pass.
A3 advisory-vs-authority: asserts telemetry (advisory) while the ledger
   (fail-closed authority) goes unasserted.
A4 `let _ =` on the op under test (distinguish from benign cleanup like
   fs::remove_file or Drop).
A5 catch_unwind that passes regardless of where/why the panic happened.
A6 fault never lands: a fault-injection coordinate that never intersects the
   real I/O, silently converting the test into a happy-path test.
A7 round-trip that checks only Ok/Some shape, not content equality.
A8 invariant claim with no negative pole (no broken variant proving the check
   can fail).

TRACK-SPECIFIC QUESTION (your area's defining risk): fail-closed atomicity.
For every test that names abort/fail/partial/uncertain/rollback: does it
actually prove the ledger was left UNTOUCHED (byte-for-byte snapshot compare
or equivalent), or only that an Err came back? An Err-with-corrupted-state is
exactly the bug class this repo must never ship. Also check: are error paths
asserted with exact variants, or just is_err()?

REPORT FORMAT — write your findings to
docs/bugbash/sep-26-fail-open-audit/findings-core-domain.md:
- Header: scope file list, your grep counts per file, your row count.
- Census table: | test fn (file:line) | verdict | mechanism / reason |
  minimal fix | critic mutation |
- For WEAK/STRONG rows keep the reason to one line. Spend your words on
  FAILOPEN/VACUOUS rows: quote the oracle (1-3 lines), state the mechanism
  (why it stays green when the named breakage happens — mechanism, not vibe),
  and give the critic a one-line mutation that should turn the test red.
- Footer: 3-line risk summary for your area.

FINISH CHECKLIST:
- Row count == grep count; both numbers stated in the header.
- Every FAILOPEN/VACUOUS row has a concrete critic mutation.
- `git status --porcelain` shows no changes except your report file
  (docs/bugbash/sep-26-fail-open-audit/findings-core-domain.md).
```

---

## Prompt 02 — T2 core-api (control.rs + builder + public surface)

```
You are auditor T2 of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

What this repo is (30 seconds): a portable memory budgeting/accounting library.
Three crates: quanta-memory-platform-core (domain/application/ports/api),
-facade (stores, spill, pipelines), -testkit (conformance, faults, fakes).
The ledger is fail-closed: every mutating accounting op fully succeeds or
leaves state untouched. Allocator telemetry is advisory only. Tests exist to
catch real regressions, not merely to import modules.

GROUND RULES (hard):
- READ-ONLY for code. You may create exactly ONE new file, your report:
  docs/bugbash/sep-26-fail-open-audit/findings-core-api.md
  No other edits, no `pm sync`, no commits.
- The worktree has unrelated dirty changes (README.md, README-detailed.md,
  facade/tests/spill_redb_kill_reopen.rs, deleted
  docs/bugbash/sep-22-test-optimization-deep/**). Leave them untouched.
- Read first: AGENTS.md, tools/pm/sources/rules/tests.md,
  tools/pm/sources/rules/cheatsheet.md.
- You may run scoped read-only cargo test to observe behavior, e.g.:
  cargo test -p quanta-memory-platform-core --all-features --lib api::
- NEVER mutate SUT code. A separate critic agent does mutation verification.
  Your job is to name the mutation; theirs is to apply and measure it.

YOUR SCOPE (you own these files; no other auditor touches them):
- crates/quanta-memory-platform-core/src/api/control.rs (~129 test fns — the
  single largest chunk of this audit)
- crates/quanta-memory-platform-core/src/api/builder.rs (~7)
- every other *.rs under core/src/api/ containing #[cfg(test)]
- crates/quanta-memory-platform-core/tests/public_api_surface.rs (~9)

DEPTH GUIDANCE: census ALL test fns (row per fn, no sampling), but spend your
deep-read budget on tests whose names mention fail/abort/error/uncertain/
kill/fence/atomic/partial. For long mechanically-similar happy-path groups,
one representative deep read + a grouped row is acceptable if the group is
truly homogeneous — say so explicitly and count the group members.

CENSUS REQUIREMENT: grep your scope for #[test] and #[tokio::test] yourself.
Row count MUST equal your grep count. Sampling is a defect.

VERDICTS:
- STRONG  — breaking the behavior the test names would turn it red; oracle is
  tied to the real authority (ledger byte totals, exact error variants,
  restored content).
- WEAK    — asserts something real but strictly weaker than the name claims.
- FAILOPEN — the failure mode the test names can occur while the test stays
  green.
- VACUOUS — the named path cannot be exercised by this body at all.

FAIL-OPEN ARCHETYPES — check every test against all eight:
A1 assert-on-input: asserted values were produced by the test/fixture, not by
   the system under test.
A2 vacuous iteration: loop over a possibly-empty collection; zero iterations =
   unconditional pass.
A3 advisory-vs-authority: asserts telemetry (advisory) while the ledger
   (fail-closed authority) goes unasserted. THIS IS YOUR AREA'S SIGNATURE
   RISK: control.rs drives reserve→commit→snapshot→release→rebuild; a test
   that asserts only advisory gauges/metrics while ledger totals go unchecked
   is fail-open by construction.
A4 `let _ =` on the op under test (there are ~2 known in control.rs — verify
   each; distinguish from benign cleanup).
A5 catch_unwind that passes regardless of where/why the panic happened.
A6 fault never lands: fault coordinate that never intersects real work.
A7 round-trip that checks only Ok/Some shape, not content equality.
A8 invariant claim with no negative pole.

TRACK-SPECIFIC QUESTION: does each control.rs test that claims a state machine
guarantee (fence authority, epoch exhaustion, rebuild ordering) assert the
POST-state precisely, or merely that the call returned Ok? For
public_api_surface.rs: it is legitimate for surface tests to lock paths and
signatures (extraction contract — see EXTRACTION.md), but flag any that only
"use" an item without locking its shape, and any behavior claim hiding inside
a surface test.

REPORT FORMAT — write your findings to
docs/bugbash/sep-26-fail-open-audit/findings-core-api.md:
- Header: scope file list, your grep counts per file, your row count.
- Census table: | test fn (file:line) | verdict | mechanism / reason |
  minimal fix | critic mutation |
- For WEAK/STRONG rows keep the reason to one line. Spend your words on
  FAILOPEN/VACUOUS rows: quote the oracle (1-3 lines), state the mechanism,
  and give the critic a one-line mutation that should turn the test red.
- Footer: 3-line risk summary for your area.

FINISH CHECKLIST:
- Row count == grep count; both numbers stated in the header.
- Every FAILOPEN/VACUOUS row has a concrete critic mutation.
- `git status --porcelain` shows no changes except your report file
  (docs/bugbash/sep-26-fail-open-audit/findings-core-api.md).
```

---

## Prompt 03 — T3 core application/adapters/ports

```
You are auditor T3 of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

What this repo is (30 seconds): a portable memory budgeting/accounting library.
Three crates: quanta-memory-platform-core (domain/application/ports/api),
-facade (stores, spill, pipelines), -testkit (conformance, faults, fakes).
The ledger is fail-closed: every mutating accounting op fully succeeds or
leaves state untouched. Allocator telemetry is advisory only. Tests exist to
catch real regressions, not merely to import modules.

GROUND RULES (hard):
- READ-ONLY for code. You may create exactly ONE new file, your report:
  docs/bugbash/sep-26-fail-open-audit/findings-app-ports.md
  No other edits, no `pm sync`, no commits.
- The worktree has unrelated dirty changes (README.md, README-detailed.md,
  facade/tests/spill_redb_kill_reopen.rs, deleted
  docs/bugbash/sep-22-test-optimization-deep/**). Leave them untouched.
- Read first: AGENTS.md, tools/pm/sources/rules/tests.md,
  tools/pm/sources/rules/cheatsheet.md.
- You may run scoped read-only cargo test to observe behavior, e.g.:
  cargo test -p quanta-memory-platform-core --all-features --lib application::
- NEVER mutate SUT code. A separate critic agent does mutation verification.

YOUR SCOPE (you own these files; no other auditor touches them):
- crates/quanta-memory-platform-core/src/application/controller.rs (~13)
- crates/quanta-memory-platform-core/src/application/scheduler.rs (~14)
- every other *.rs under core/src/application/ containing #[cfg(test)]
- crates/quanta-memory-platform-core/src/adapters/pin.rs, metrics.rs,
  hdrhistogram.rs (feature-gated), and any other adapters with test fns
- crates/quanta-memory-platform-core/src/ports/scratch.rs (~9),
  telemetry.rs (~8), and any other ports with test fns

CENSUS REQUIREMENT: grep your scope for #[test] and #[tokio::test] yourself.
Row count MUST equal your grep count. Sampling is a defect.

VERDICTS:
- STRONG  — breaking the behavior the test names would turn it red.
- WEAK    — asserts something real but strictly weaker than the name claims.
- FAILOPEN — the named failure mode can occur while the test stays green.
- VACUOUS — the named path cannot be exercised by this body at all.

FAIL-OPEN ARCHETYPES — check every test against all eight:
A1 assert-on-input: asserted values were produced by the test/fixture, not by
   the system under test.
A2 vacuous iteration: loop over a possibly-empty collection.
A3 advisory-vs-authority: asserts telemetry while the ledger authority goes
   unasserted.
A4 `let _ =` on the op under test (1 known in adapters/pin.rs — verify).
A5 catch_unwind that passes regardless of panic location.
A6 fault never lands: fault coordinate that never intersects real work.
A7 round-trip that checks only Ok/Some shape, not content.
A8 invariant claim with no negative pole.

TRACK-SPECIFIC QUESTION: controller/scheduler tests must assert DECISIONS
(which victim was chosen, in what order, under what pressure), not merely that
calls return Ok. Flag any test where the scheduler could select arbitrarily
and the test would still pass. KNOWN LEADS to verify:
- adapters/metrics.rs `emitter_holds_no_accounting_state` has no assert,
  expect, or panic — determine whether it can fail at all (if it is a
  compile-time/marker test, say so and judge whether that is sufficient).
- ports/telemetry.rs: are the trait-level tests asserting behavior or just
  wiring?

REPORT FORMAT — write your findings to
docs/bugbash/sep-26-fail-open-audit/findings-app-ports.md:
- Header: scope file list, your grep counts per file, your row count.
- Census table: | test fn (file:line) | verdict | mechanism / reason |
  minimal fix | critic mutation |
- For WEAK/STRONG rows keep the reason to one line. Spend your words on
  FAILOPEN/VACUOUS rows: quote the oracle (1-3 lines), state the mechanism,
  and give the critic a one-line mutation that should turn the test red.
- Footer: 3-line risk summary for your area.

FINISH CHECKLIST:
- Row count == grep count; both numbers stated in the header.
- Every FAILOPEN/VACUOUS row has a concrete critic mutation.
- `git status --porcelain` shows no changes except your report file
  (docs/bugbash/sep-26-fail-open-audit/findings-app-ports.md).
```

---

## Prompt 04 — T4 facade revocable_store

```
You are auditor T4 of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

What this repo is (30 seconds): a portable memory budgeting/accounting library.
Three crates: quanta-memory-platform-core (domain/application/ports/api),
-facade (stores, spill, pipelines), -testkit (conformance, faults, fakes).
The ledger is fail-closed: every mutating accounting op fully succeeds or
leaves state untouched. Allocator telemetry is advisory only. Tests exist to
catch real regressions, not merely to import modules.

GROUND RULES (hard):
- READ-ONLY for code. You may create exactly ONE new file, your report:
  docs/bugbash/sep-26-fail-open-audit/findings-revocable-store.md
  No other edits, no `pm sync`, no commits.
- The worktree has unrelated dirty changes (README.md, README-detailed.md,
  facade/tests/spill_redb_kill_reopen.rs, deleted
  docs/bugbash/sep-22-test-optimization-deep/**). Leave them untouched.
- Read first: AGENTS.md, tools/pm/sources/rules/tests.md,
  tools/pm/sources/rules/cheatsheet.md.
- You may run scoped read-only cargo test to observe behavior, e.g.:
  cargo test -p quanta-memory-platform-facade --all-features --lib revocable_store
- NEVER mutate SUT code. A separate critic agent does mutation verification.

YOUR SCOPE (you own these files; no other auditor touches them):
- crates/quanta-memory-platform-facade/src/revocable_store/service.rs (~68)
- crates/quanta-memory-platform-facade/src/revocable_store/durable_codec.rs
- crates/quanta-memory-platform-facade/src/revocable_store/cost.rs
- every other *.rs under facade/src/revocable_store/ with test fns

CENSUS REQUIREMENT: grep your scope for #[test] and #[tokio::test] yourself.
Row count MUST equal your grep count. Sampling is a defect.

VERDICTS:
- STRONG / WEAK / FAILOPEN / VACUOUS (standard definitions):
  STRONG = breaking the named behavior turns the test red, oracle tied to the
  real authority; WEAK = asserts something strictly weaker than the name;
  FAILOPEN = the named failure mode can occur while the test stays green;
  VACUOUS = the named path cannot be exercised by this body at all.

FAIL-OPEN ARCHETYPES — check every test against all eight:
A1 assert-on-input. A2 vacuous iteration. A3 advisory-vs-authority.
A4 `let _ =` on the op under test. A5 catch_unwind location-insensitive.
A6 fault never lands. A7 shape-only round-trip. A8 no negative pole.

TRACK-SPECIFIC QUESTION — your area is the `let _ =` capital of this audit
(~47 occurrences inside test code). Build a CLASSIFICATION TABLE of every
`let _ =` in a test body: CLEANUP (fs remove, deliberate drop, teardown),
DOC-EXAMPLE (in doc comments — exclude from the count but note), or
SWALLOWED-VERIFICATION (the ignored Result belongs to the operation the test
claims to verify — a fail-open). Quote each ambiguous one. KNOWN LEADS:
- service.rs ~line 1052: `let _ = controller.abort_restore(&candidate.claim)`
  inside a test with a `reason = "panic unwind must preserve the original
  payload..."` annotation — determine whether the abort outcome is verified
  elsewhere in that test or silently dropped.
- service.rs `sep23_fup001_uncertain_commit_keeps_source_fenced_and_blocks_
  wrong_retire` delegates to helpers with no top-level assert — follow the
  helper chain and verify it ends in real assertions tied to ledger state.
- catch_unwind sites ~2452 and ~2530 (`spill_one_durable_unbounded` panic
  preservation): does the test assert WHERE the panic came from and that the
  payload survived intact, or only that some panic happened?

REPORT FORMAT — write your findings to
docs/bugbash/sep-26-fail-open-audit/findings-revocable-store.md:
- Header: scope file list, your grep counts per file, your row count.
- Census table: | test fn (file:line) | verdict | mechanism / reason |
  minimal fix | critic mutation |
- The `let _ =` classification table (separate section).
- Footer: 3-line risk summary for your area.

FINISH CHECKLIST:
- Row count == grep count; both numbers stated in the header.
- Every FAILOPEN/VACUOUS row has a concrete critic mutation.
- `git status --porcelain` shows no changes except your report file
  (docs/bugbash/sep-26-fail-open-audit/findings-revocable-store.md).
```

---

## Prompt 05 — T5 facade transcript/retained/payload stores

```
You are auditor T5 of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

What this repo is (30 seconds): a portable memory budgeting/accounting library.
Three crates: quanta-memory-platform-core (domain/application/ports/api),
-facade (stores, spill, pipelines), -testkit (conformance, faults, fakes).
The ledger is fail-closed: every mutating accounting op fully succeeds or
leaves state untouched. Allocator telemetry is advisory only. Tests exist to
catch real regressions, not merely to import modules.

GROUND RULES (hard):
- READ-ONLY for code. You may create exactly ONE new file, your report:
  docs/bugbash/sep-26-fail-open-audit/findings-facade-stores.md
  No other edits, no `pm sync`, no commits.
- The worktree has unrelated dirty changes (README.md, README-detailed.md,
  facade/tests/spill_redb_kill_reopen.rs, deleted
  docs/bugbash/sep-22-test-optimization-deep/**). Leave them untouched.
- Read first: AGENTS.md, tools/pm/sources/rules/tests.md,
  tools/pm/sources/rules/cheatsheet.md.
- You may run scoped read-only cargo test to observe behavior, e.g.:
  cargo test -p quanta-memory-platform-facade --all-features --lib transcript_store
- NEVER mutate SUT code. A separate critic agent does mutation verification.

YOUR SCOPE (you own these files; no other auditor touches them):
- crates/quanta-memory-platform-facade/src/transcript_store/service.rs (~60)
- crates/quanta-memory-platform-facade/src/retained_map/service.rs (~58)
- crates/quanta-memory-platform-facade/src/payload_store/service.rs (~33)
- every other *.rs under those three modules with test fns

CENSUS REQUIREMENT: grep your scope for #[test] and #[tokio::test] yourself.
Row count MUST equal your grep count. Sampling is a defect.

VERDICTS:
- STRONG / WEAK / FAILOPEN / VACUOUS (standard definitions):
  STRONG = breaking the named behavior turns the test red, oracle tied to the
  real authority; WEAK = asserts something strictly weaker than the name;
  FAILOPEN = the named failure mode can occur while the test stays green;
  VACUOUS = the named path cannot be exercised by this body at all.

FAIL-OPEN ARCHETYPES — check every test against all eight:
A1 assert-on-input. A2 vacuous iteration. A3 advisory-vs-authority.
A4 `let _ =` on the op under test (~11 in transcript_store, ~6 in
   payload_store, ~5 in retained_map — verify each).
A5 catch_unwind location-insensitive.
A6 fault never lands. A7 shape-only round-trip. A8 no negative pole.

TRACK-SPECIFIC QUESTION — round-trip content equality (A7 is your signature
risk). For every test whose name contains round/restore/recover/replay/
rebuild/durable: does it compare the RESTORED content byte-for-byte (or an
equivalent strong equality) against the original, or only that the operation
returned Ok/Some? List each such test with its comparison operator.
KNOWN LEADS:
- payload_store/service.rs has 4 catch_unwind sites (~935, ~1191, ~1255,
  ~1383): for each, does the test pin the panic to the intended operation and
  assert the post-panic state, or would any panic anywhere pass?
- retained_map `first_and_last_key_point_operations_share_one_n_independent_
  probe_bound_v1` has no top-level assert — follow the helper
  (exercise_endpoint_point_operations_without_linear_probe_growth) and verify
  its end asserts are reachable for the arguments passed.

REPORT FORMAT — write your findings to
docs/bugbash/sep-26-fail-open-audit/findings-facade-stores.md:
- Header: scope file list, your grep counts per file, your row count.
- Census table: | test fn (file:line) | verdict | mechanism / reason |
  minimal fix | critic mutation |
- Round-trip comparison table (separate section): test → equality strength.
- Footer: 3-line risk summary for your area.

FINISH CHECKLIST:
- Row count == grep count; both numbers stated in the header.
- Every FAILOPEN/VACUOUS row has a concrete critic mutation.
- `git status --porcelain` shows no changes except your report file
  (docs/bugbash/sep-26-fail-open-audit/findings-facade-stores.md).
```

---

## Prompt 06 — T6 spill + redb backing

```
You are auditor T6 of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

What this repo is (30 seconds): a portable memory budgeting/accounting library.
Three crates: quanta-memory-platform-core (domain/application/ports/api),
-facade (stores, spill, pipelines), -testkit (conformance, faults, fakes).
The ledger is fail-closed: every mutating accounting op fully succeeds or
leaves state untouched. Allocator telemetry is advisory only. Tests exist to
catch real regressions, not merely to import modules.

GROUND RULES (hard):
- READ-ONLY for code. You may create exactly ONE new file, your report:
  docs/bugbash/sep-26-fail-open-audit/findings-spill-redb.md
  No other edits, no `pm sync`, no commits.
- The worktree has unrelated dirty changes (README.md, README-detailed.md,
  facade/tests/spill_redb_kill_reopen.rs, deleted
  docs/bugbash/sep-22-test-optimization-deep/**). Leave them untouched.
- Read first: AGENTS.md, tools/pm/sources/rules/tests.md,
  tools/pm/sources/rules/cheatsheet.md, and the backing round-trip sections
  of tools/pm/sources/rules/determinism.md.
- You may run scoped read-only cargo test to observe behavior, e.g.:
  cargo test -p quanta-memory-platform-facade --all-features --lib spill
- NEVER mutate SUT code. A separate critic agent does mutation verification.

YOUR SCOPE (you own these files; no other auditor touches them):
- crates/quanta-memory-platform-facade/src/spill/service.rs (~20)
- crates/quanta-memory-platform-facade/src/spill/backings/redb.rs (~55)
- crates/quanta-memory-platform-facade/src/spill/mod.rs (feature-gated test fns)
- every other *.rs under facade/src/spill/ with test fns

CENSUS REQUIREMENT: grep your scope for #[test] and #[tokio::test] yourself.
Row count MUST equal your grep count. Sampling is a defect.

VERDICTS:
- STRONG / WEAK / FAILOPEN / VACUOUS (standard definitions):
  STRONG = breaking the named behavior turns the test red, oracle tied to the
  real authority; WEAK = asserts something strictly weaker than the name;
  FAILOPEN = the named failure mode can occur while the test stays green;
  VACUOUS = the named path cannot be exercised by this body at all.

FAIL-OPEN ARCHETYPES — check every test against all eight:
A1 assert-on-input. A2 vacuous iteration. A3 advisory-vs-authority.
A4 `let _ =` on the op under test (~18 in backings/redb.rs — verify each;
   distinguish fs::remove_file cleanup from swallowed verification).
A5 catch_unwind location-insensitive.
A6 fault never lands — YOUR SIGNATURE RISK (see below).
A7 shape-only round-trip. A8 no negative pole.

TRACK-SPECIFIC QUESTION: in-process fault injection at write boundaries.
The spill tier's job is durable round-trip under faults; a fault test whose
fault coordinate never intersects an actual write is a happy-path test
wearing a fault-test name. For every test that injects a fault (encoder
panic, failing closure, limit violation): trace whether the faulted closure
is GUARANTEED to be invoked on the path taken, and whether the test asserts
the fault actually fired (not just that an Err came back — an Err can come
from an earlier, unrelated check). Also: redb round-trip tests — byte-for-byte
content equality on restore, and does any test verify the record content
after reopen rather than only key existence?

REPORT FORMAT — write your findings to
docs/bugbash/sep-26-fail-open-audit/findings-spill-redb.md:
- Header: scope file list, your grep counts per file, your row count.
- Census table: | test fn (file:line) | verdict | mechanism / reason |
  minimal fix | critic mutation |
- Fault-injection table (separate section): test → fault kind → guaranteed to
  fire? → firing asserted?
- Footer: 3-line risk summary for your area.

FINISH CHECKLIST:
- Row count == grep count; both numbers stated in the header.
- Every FAILOPEN/VACUOUS row has a concrete critic mutation.
- `git status --porcelain` shows no changes except your report file
  (docs/bugbash/sep-26-fail-open-audit/findings-spill-redb.md).
```

---

## Prompt 07 — T7 crash rails (kill/reopen integration tests)

```
You are auditor T7 of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

What this repo is (30 seconds): a portable memory budgeting/accounting library.
Three crates: quanta-memory-platform-core (domain/application/ports/api),
-facade (stores, spill, pipelines), -testkit (conformance, faults, fakes).
The ledger is fail-closed: every mutating accounting op fully succeeds or
leaves state untouched. Allocator telemetry is advisory only. Tests exist to
catch real regressions, not merely to import modules.

GROUND RULES (hard):
- READ-ONLY for code. You may create exactly ONE new file, your report:
  docs/bugbash/sep-26-fail-open-audit/findings-crash-rails.md
  No other edits, no `pm sync`, no commits.
- The worktree has unrelated dirty changes (README.md, README-detailed.md,
  facade/tests/spill_redb_kill_reopen.rs — which is IN YOUR SCOPE in its
  current working-tree form — and deleted
  docs/bugbash/sep-22-test-optimization-deep/**). Leave them untouched.
- Read first: AGENTS.md, tools/pm/sources/rules/tests.md,
  tools/pm/sources/rules/cheatsheet.md, and the crash/durability sections of
  tools/pm/sources/rules/determinism.md.
- These tests spawn and kill real child processes and are slow: READING is
  your primary method. Run one only if a question cannot be answered from
  the body and the child example source, e.g.:
  cargo test -p quanta-memory-platform-facade --all-features --test spill_redb_kill_reopen -- --list
- NEVER mutate SUT code. A separate critic agent does mutation verification.

YOUR SCOPE (you own these files; no other auditor touches them):
- crates/quanta-memory-platform-facade/tests/spill_redb_kill_reopen.rs
- crates/quanta-memory-platform-facade/tests/spill_redb_finalize_kill.rs
- crates/quanta-memory-platform-facade/tests/cross_store_hellgate.rs
- crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs
- the child-process example sources these rails build and kill (find them:
  grep the rail files for the example/binary names they spawn; typically
  under facade/examples/ or similar — those sources are READ scope too)

CENSUS REQUIREMENT: grep your scope for #[test] and #[tokio::test] yourself.
Row count MUST equal your grep count. Sampling is a defect.

VERDICTS:
- STRONG / WEAK / FAILOPEN / VACUOUS (standard definitions):
  STRONG = breaking the named behavior turns the test red; WEAK = asserts
  something strictly weaker than the name; FAILOPEN = the named failure mode
  can occur while the test stays green; VACUOUS = the named path cannot be
  exercised by this body at all.

FAIL-OPEN ARCHETYPES — check every test against all eight:
A1 assert-on-input. A2 vacuous iteration. A3 advisory-vs-authority.
A4 `let _ =` on the op under test. A5 catch_unwind location-insensitive.
A6 fault never lands — YOUR SIGNATURE RISK (see below).
A7 shape-only round-trip. A8 no negative pole.

TRACK-SPECIFIC QUESTION — this is the audit's highest-stakes area: does the
crash actually land where the test claims? For every kill/crash rail:
1. COORDINATE PROOF: does the rail assert evidence that the child reached the
   intended crash point BEFORE being killed (log line, handshake, exit
   status, partial-write marker)? If the child can be killed before any write,
   the "recovery" under test is just a clean open — a fail-open.
2. PARTIAL-STATE DETECTOR: for rails claiming "no partial/poisoned state
   survives", is there an assertion that would actually catch partial state
   (e.g., content equality of every record, ledger total equality), or does
   the rail only assert reopen succeeded?
3. BOUNDARY HONESTY: the rails document that SIGKILL cannot prove power loss.
   Check no assertion overstates what the rail proves (docs vs body mismatch
   is itself a finding).
4. For cross_store_hellgate/hellgate_e2e: same questions plus — are the
   asserted post-crash invariants ledger-level (fail-closed) or merely
   process-level ("it didn't hang")?

REPORT FORMAT — write your findings to
docs/bugbash/sep-26-fail-open-audit/findings-crash-rails.md:
- Header: scope file list, your grep counts per file, your row count.
- Census table: | test fn (file:line) | verdict | mechanism / reason |
  minimal fix | critic mutation |
- Crash-coordinate table (separate section): rail → intended crash point →
  proof child reached it → partial-state detector strength.
- Footer: 3-line risk summary for your area.

FINISH CHECKLIST:
- Row count == grep count; both numbers stated in the header.
- Every FAILOPEN/VACUOUS row has a concrete critic mutation (for child-process
  rails the mutation may target the child example source — name the file).
- `git status --porcelain` shows no changes except your report file
  (docs/bugbash/sep-26-fail-open-audit/findings-crash-rails.md).
```

---

## Prompt 08 — T8 publish_pipeline + shared + facade public surface

```
You are auditor T8 of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

What this repo is (30 seconds): a portable memory budgeting/accounting library.
Three crates: quanta-memory-platform-core (domain/application/ports/api),
-facade (stores, spill, pipelines), -testkit (conformance, faults, fakes).
The ledger is fail-closed: every mutating accounting op fully succeeds or
leaves state untouched. Allocator telemetry is advisory only. Tests exist to
catch real regressions, not merely to import modules.

GROUND RULES (hard):
- READ-ONLY for code. You may create exactly ONE new file, your report:
  docs/bugbash/sep-26-fail-open-audit/findings-pipeline-shared.md
  No other edits, no `pm sync`, no commits.
- The worktree has unrelated dirty changes (README.md, README-detailed.md,
  facade/tests/spill_redb_kill_reopen.rs, deleted
  docs/bugbash/sep-22-test-optimization-deep/**). Leave them untouched.
- Read first: AGENTS.md, tools/pm/sources/rules/tests.md,
  tools/pm/sources/rules/cheatsheet.md.
- You may run scoped read-only cargo test to observe behavior, e.g.:
  cargo test -p quanta-memory-platform-facade --all-features --lib publish_pipeline
- NEVER mutate SUT code. A separate critic agent does mutation verification.

YOUR SCOPE (you own these files; no other auditor touches them):
- crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs (~32)
- crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs
- every other *.rs under facade/src/publish_pipeline/ with test fns
- crates/quanta-memory-platform-facade/src/presets/ with test fns
- crates/quanta-memory-platform-facade/src/shared/builder.rs (~11),
  lifecycle.rs (~7), measure.rs (~9), and other shared/*.rs with test fns
- crates/quanta-memory-platform-facade/tests/public_api_surface.rs (~20)

CENSUS REQUIREMENT: grep your scope for #[test] and #[tokio::test] yourself.
Row count MUST equal your grep count. Sampling is a defect.

VERDICTS:
- STRONG / WEAK / FAILOPEN / VACUOUS (standard definitions):
  STRONG = breaking the named behavior turns the test red; WEAK = asserts
  something strictly weaker than the name; FAILOPEN = the named failure mode
  can occur while the test stays green; VACUOUS = the named path cannot be
  exercised by this body at all.

FAIL-OPEN ARCHETYPES — check every test against all eight:
A1 assert-on-input. A2 vacuous iteration. A3 advisory-vs-authority.
A4 `let _ =` on the op under test (~3 in prepared_publication.rs — verify;
   note ~2270 is `let _ = catch_unwind(...)` which may be deliberate
   panic-swallowing: determine whether the panic is asserted elsewhere).
A5 catch_unwind location-insensitive — YOUR SIGNATURE RISK: prepared_publication.rs
   has 5 catch_unwind sites (~608, ~1923, ~1973, ~2202, ~2270). For each:
   does the test pin the panic to the intended closure/phase and assert the
   post-panic publication state, or would any panic anywhere pass?
A6 fault never lands. A7 shape-only round-trip. A8 no negative pole.

TRACK-SPECIFIC QUESTION: publication is a state-machine authority — for every
test naming publish/abort/rollback/stage/fence, check the assertion target is
the resulting STATE (what is visible/durable after), not the return value
alone. For facade/tests/public_api_surface.rs: surface tests legitimately
lock paths and signatures (extraction contract — EXTRACTION.md), but flag any
item that is merely "used" without its shape being locked, and any behavior
claim hiding inside a surface test.

REPORT FORMAT — write your findings to
docs/bugbash/sep-26-fail-open-audit/findings-pipeline-shared.md:
- Header: scope file list, your grep counts per file, your row count.
- Census table: | test fn (file:line) | verdict | mechanism / reason |
  minimal fix | critic mutation |
- catch_unwind table (separate section): site → panic pinned? → post-state
  asserted?
- Footer: 3-line risk summary for your area.

FINISH CHECKLIST:
- Row count == grep count; both numbers stated in the header.
- Every FAILOPEN/VACUOUS row has a concrete critic mutation.
- `git status --porcelain` shows no changes except your report file
  (docs/bugbash/sep-26-fail-open-audit/findings-pipeline-shared.md).
```

---

## Prompt 09 — T9 testkit (conformance gap map + faults/fakes/fixtures)

```
You are auditor T9 of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

What this repo is (30 seconds): a portable memory budgeting/accounting library.
Three crates: quanta-memory-platform-core (domain/application/ports/api),
-facade (stores, spill, pipelines), -testkit (conformance, faults, fakes).
The ledger is fail-closed: every mutating accounting op fully succeeds or
leaves state untouched. Allocator telemetry is advisory only. Conformance
must reject a broken fake, not only accept honest implementations.

GROUND RULES (hard):
- READ-ONLY for code. You may create exactly ONE new file, your report:
  docs/bugbash/sep-26-fail-open-audit/findings-testkit.md
  No other edits, no `pm sync`, no commits.
- The worktree has unrelated dirty changes (README.md, README-detailed.md,
  facade/tests/spill_redb_kill_reopen.rs, deleted
  docs/bugbash/sep-22-test-optimization-deep/**). Leave them untouched.
- Read first: AGENTS.md, tools/pm/sources/rules/tests.md,
  tools/pm/sources/rules/cheatsheet.md, and the conformance negative-pole
  requirement in tools/pm/sources/rules/determinism.md.
- You may run scoped read-only cargo test to observe behavior, e.g.:
  cargo test -p quanta-memory-platform-testkit --all-features
  (If the loom feature needs a special toolchain, note it and move on —
  do not force it.)
- NEVER mutate SUT code. A separate critic agent does mutation verification.

YOUR SCOPE (you own these files; no other auditor touches them):
- crates/quanta-memory-platform-testkit/src/conformance/suites.rs (~39),
  invariants.rs (~5), mod.rs (~5), and other conformance/*.rs
- crates/quanta-memory-platform-testkit/src/faults/scenarios.rs (~7),
  faults/mod.rs
- crates/quanta-memory-platform-testkit/src/fake/broker.rs (~6)
- crates/quanta-memory-platform-testkit/src/fixtures/mod.rs (~5),
  alloc_tracking/mod.rs, loom_models.rs (~5), and remaining testkit src
  with test fns
- crates/quanta-memory-platform-testkit/tests/facade_conformance_bite.rs
- crates/quanta-memory-platform-testkit/tests/public_api_surface.rs

CENSUS REQUIREMENT: grep your scope for #[test] and #[tokio::test] yourself.
Row count MUST equal your grep count. Sampling is a defect.

VERDICTS:
- STRONG / WEAK / FAILOPEN / VACUOUS (standard definitions):
  STRONG = breaking the named behavior turns the test red; WEAK = asserts
  something strictly weaker than the name; FAILOPEN = the named failure mode
  can occur while the test stays green; VACUOUS = the named path cannot be
  exercised by this body at all.

FAIL-OPEN ARCHETYPES — check every test against all eight:
A1 assert-on-input. A2 vacuous iteration. A3 advisory-vs-authority.
A4 `let _ =` on the op under test (~1 in suites.rs — verify).
A5 catch_unwind location-insensitive. A6 fault never lands — for every
   faults/scenarios.rs test: is the fault GUARANTEED to fire on the exercised
   path, and is its firing asserted?
A7 shape-only round-trip.
A8 no negative pole — YOUR SIGNATURE RISK (see below).

TRACK-SPECIFIC QUESTION — THE GAP MAP (this is your primary deliverable):
The conformance suite claims a set of invariants. For EACH invariant the
suites check, list whether a BROKEN implementation is proven to be rejected:
invariant → checking site (file:line) → negative pole exists? (which broken
fake / self-test fails it?) → if missing, what broken fake would slip through.
Known good poles to build from: facade_conformance_bite.rs rejects
leaking-close, state-losing-close, double-refund fakes. The question is what
is NOT covered: e.g., are there invariants checked only via honest fakes
passing (positive pole only)? Is there any invariant whose "check" cannot
fail because the oracle itself computes the expected value from the SUT
(A1 at suite scale)?
Also: fake/broker.rs — if the fake used by all conformance runs has a bug,
which suite test would catch it? If none, that is a finding.

REPORT FORMAT — write your findings to
docs/bugbash/sep-26-fail-open-audit/findings-testkit.md:
- Header: scope file list, your grep counts per file, your row count.
- Census table: | test fn (file:line) | verdict | mechanism / reason |
  minimal fix | critic mutation |
- The GAP MAP table (separate section, primary deliverable).
- Footer: 3-line risk summary for your area.

FINISH CHECKLIST:
- Row count == grep count; both numbers stated in the header.
- Every FAILOPEN/VACUOUS row has a concrete critic mutation.
- `git status --porcelain` shows no changes except your report file
  (docs/bugbash/sep-26-fail-open-audit/findings-testkit.md).
```

---

## Prompt 10 — C critic (mutation verification; run AFTER 01–09)

```
You are the CRITIC of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

Your job: the auditor findings live under docs/bugbash/sep-26-fail-open-audit/
as findings-core-domain.md, findings-core-api.md, findings-app-ports.md,
findings-revocable-store.md, findings-facade-stores.md,
findings-spill-redb.md, findings-crash-rails.md,
findings-pipeline-shared.md, findings-testkit.md. If any is missing, state
which and audit only what exists.

WHY YOU EXIST: fail-open means "the SUT breaks and the test stays green".
The only trustworthy proof is a MUTATION: break the SUT exactly as the
finding claims, run the accused test, and observe green or red. Auditor
judgment alone is not acceptable evidence.

SAFETY (hard rules):
- Before anything else, snapshot the worktree state:
  git status --porcelain > /tmp/qmp-critic-baseline.txt
  The worktree ALREADY has unrelated dirty changes (README.md,
  README-detailed.md, facade/tests/spill_redb_kill_reopen.rs, deleted
  docs/bugbash/sep-22-test-optimization-deep/**). You must not touch or
  revert those.
- For EVERY mutation you apply: run the scoped test, then IMMEDIATELY restore
  with `git checkout -- <file>` (only ever files YOU mutated). Never commit.
  Never run scripts that write outside /tmp and your report file.
- You may create exactly ONE new file, your report:
  docs/bugbash/sep-26-fail-open-audit/findings-critic.md
- At the end: git status --porcelain must equal the baseline snapshot except
  for your report file. State this comparison in your report.

TASKS:
1. CENSUS AUDIT: for each findings-{scope}.md report, re-count the test fns
   in its scope files (grep #[test] and #[tokio::test]) and compare with the
   report's claimed row count. Mismatch = reject the report (list it).
2. MUTATION VERIFICATION: for EVERY row verdicted FAILOPEN or VACUOUS (and a
   random sample of at least 5 WEAK rows per report):
   a. Read the finding's critic-mutation suggestion. Apply the minimal SUT
      mutation that breaks exactly the behavior the finding names (one line
      where possible; for child-process rails the mutation may target the
      child example source the rail spawns).
   b. Run exactly the accused test, scoped, e.g.:
      cargo test -p quanta-memory-platform-core --all-features --lib api::control::tests::<fn>
      cargo test -p quanta-memory-platform-facade --all-features --lib revocable_store::service::tests::<fn>
      (use --test <name> for integration tests; use --exact where useful)
   c. Verdict:
      CONFIRMED   — test stays GREEN under a mutation that breaks the named
                    behavior → real fail-open.
      REJECTED    — test goes RED → the auditor overclaimed; the test bites.
      INCONCLUSIVE — mutation inapplicable, build broke for unrelated
                    reasons, or environment (child-process/flaky). Say why.
   d. Restore the mutated file immediately (see SAFETY).
3. SPOT CHECK: open bodies of 5 random census rows per report and judge
   whether the verdict is plausible; flag clear misverdicts.

REPORT FORMAT — write your results to
docs/bugbash/sep-26-fail-open-audit/findings-critic.md:
- Summary: reports checked, census pass/fail per report.
- Per finding: | findings-{scope} | test fn | auditor verdict | mutation
  applied (file:line, one line) | test result | critic verdict |.
- Final confirmed fail-open inventory (CONFIRMED rows only), each with the
  mutation that proved it — this inventory is the input for the fix phase.
- Safety attestation: baseline comparison result.

CONSTRAINTS: you fix NOTHING. You only measure. Suggestions for fixes belong
to the auditors' reports and the synthesis step.
```

---

## Prompt 11 — S synthesis (run LAST)

```
You are the SYNTHESIS step of a parallel fail-open test audit of the repo at
/Users/songmin/Documents/code-new/quanta-memory-platform (cd there first; verify with pwd).

Inputs (all under docs/bugbash/sep-26-fail-open-audit/): the nine
findings-{scope}.md census reports (core-domain, core-api, app-ports,
revocable-store, facade-stores, spill-redb, crash-rails, pipeline-shared,
testkit) plus findings-critic.md. If some are missing, synthesize what exists
and say so up front.

TASK: produce docs/bugbash/sep-26-fail-open-audit/99-index.md — the round's
single decision document. NO code changes; this is the only file you create.

Contents (in this order):
1. Executive summary: how many tests censused, how many confirmed fail-open
   (critic CONFIRMED only — REJECTED findings must NOT appear as problems;
   list them separately as auditor overclaims), how many vacuous, how many
   weak.
2. Confirmed fail-open inventory ranked by risk tier:
   Tier 1 — ledger fail-closed violations & conformance authority gaps
   (testkit gap-map holes, missing negative poles)
   Tier 2 — public API surface and control.rs state-machine claims
   Tier 3 — store/spill/pipeline behavior (round-trips, catch_unwind sites,
   let _ = swallowed verification)
   Tier 4 — crash-rail coordinate proofs (crash-rails findings)
   Each row: test, file:line, broken-by mutation (from critic), one-line fix.
3. Fix phase plan: group confirmed findings by the ORIGINAL track ownership
   (the nine scope boundaries stay the fix owners — disjoint by
   construction), with a suggested order (Tier 1 first) and per-track
   estimated effort (S/M/L based on finding count and fix depth).
4. The conformance gap map from findings-testkit.md reproduced verbatim (or
   linked) with a priority ordering of missing negative poles.
5. Process notes: census quality per report, auditor overclaim rate (REJECTED
   / mutated), anything the next round should do differently.

Rules: cite file:line for everything; no vibes; do not invent findings the
critic did not confirm; do not modify any other file; no commits.
```
