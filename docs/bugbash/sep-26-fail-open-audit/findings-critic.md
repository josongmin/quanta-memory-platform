# Fail-Open Test Audit — CRITIC (mutation verification)

Role: critic of the nine auditor reports (T1–T9). A previous critic attempt
stalled twice and left no report; this is a clean restart. This file is the
only new file created by the critic.

## Mission

Fail-open means "the SUT breaks and the test stays green". The only trustworthy
proof is a MUTATION: break the SUT exactly as the finding claims, run the
accused test, observe green or red. This critic performs, in priority order:
P0 = all 6 FAILOPEN mutations, P1 = 3 VACUOUS representatives, P2 = up to 3
cheap WEAK samples per report (only if P0+P1 fully written). Nothing is fixed;
everything is only measured.

## Method

For each mutation: (1) edit the named SUT file in place; (2) run the accused
test with a scoped `cargo test ... --exact` filter; (3) record GREEN/RED;
(4) restore with `git checkout -- <file>` and verify `git diff --stat` is
empty for that file; (5) append the row to the mutation table below
IMMEDIATELY (incremental report rule — a stall must never lose more than one
row). Cargo runs kept minimal: scoped `--exact` filters only, no full
workspace suite. No commits are made.

## Baseline snapshot

Captured before any mutation:

```
$ git status --porcelain
(empty)
```

Deviation from the briefed expectation (brief expected
`?? docs/bugbash/sep-26-fail-open-audit/` as the only entry): the working
tree is fully clean and all nine reports + prompts.md are committed at HEAD
(61e9c7c "Revert accidental critic mutation in control.rs ..."). Cleaner than
expected — no dirty paths under crates/ — so the safety precondition holds.
During the run the only untracked path will be this report file itself.

## Progress log

- [t0] Baseline captured (clean tree, reports committed at 61e9c7c).
- [t1] Census audit complete: all 9 reports match my grep exactly.
- [t2] P0 complete: all 6 FAILOPEN mutations run, GREEN, recorded (rows 1-6).
- [t3] P1 complete: 3 VACUOUS representatives recorded (rows 7-9).
- [t4] P2 partial by safety decision: 2 WEAK samples (rows 10-11, both
  parent-named examples) + 1 aborted for concurrency safety (row 12). Stopped
  because the integrator began applying fix-phase edits to the same working
  tree mid-run (see attestation); mission priority rule says a complete P0+P1
  beats a half-finished P2, and it is complete.
- [t5] Closeout: mutation-marker grep over crates/ = zero hits; final git
  status captured; report finalized.

## Census audit

Method: `grep -c -E '#\[test\]|#\[tokio::test\]'` per scope file, summed per
report, compared to the claimed row count (claimed == grep is each report's
own stated invariant; auditors promised row-count == grep-count, no sampling).

| report | claimed | my grep | pass/fail |
| --- | ---: | ---: | --- |
| T1 core-domain | 97 | 97 (38+14+27+3+15) | **pass** |
| T2 core-api | 145 | 145 (129+7+9) | **pass** (see note) |
| T3 app-ports | 55 | 55 (13+14+5+2+4+9+8) | **pass** |
| T4 revocable-store | 72 | 72 (68+3+1+0) | **pass** |
| T5 facade-stores | 151 | 151 (TS 60, RM 58, PS 33) | **pass** |
| T6 spill-redb | 75 | 75 (20+55) | **pass** |
| T7 crash-rails | 29 | 29 (4+3+4+18) | **pass** |
| T8 pipeline-shared | 84 | 84 (32+5+11+7+9+20) | **pass** |
| T9 testkit | 87 | 87 (39+5+5+7+2+6+5+5+5+3+4+1) | **pass** |

**T2 note (doc-comment subtlety, verified):** the 9 grep hits in
`core/tests/public_api_surface.rs` include line 11, a `//!` doc-comment
sentence containing the literal `` `#[test]` `` — I confirmed by reading the
line. Real `#[test]` fns there = 8 (lines 181–344). T2's report documents
exactly this and rows the 9th hit as the `names_every_public_reexport`
compile-pin fn (line 96, `#[allow(dead_code)]`, not executed). Cargo's own
executed count (8) independently corroborates. Census judged **pass** — the
discrepancy is disclosed in the report header, not hidden.

**Census audit verdict: 9/9 pass.** No report inflated or deflated its scope.

## Mutation table

| # | report | test fn | mutation (file:line, one line) | result | verdict | notes |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | T1 core-domain | `domain::ledger::tests::single_release_preflight_is_bounded_independent_of_owner_cardinality` | ledger.rs:914 — owner preflight loop rewritten from keyed `owner_usage.get(owner)` per refund to `for (owner, usage) in &self.owner_usage` scan × 3 kinds (O(cardinality) probing) | **GREEN** | **CONFIRMED** | Baseline green pre-mutation. Probe counter is computed from refund-map cardinality (ledger.rs:899-903), not from real lookups, so the scan is invisible at both 32 and 128 owners. Exact module path needed `domain::` prefix. Restored clean. |
| 2 | T2 core-api | `api::control::tests::pressure_classifies_from_semantic_usage` | control.rs — 3-part: `last_allocator_resident: Option<u64>` field on BrokerCore (init None); set from `snapshot.resident_bytes` in `observe_allocator`; `classify_pressure` uses `match core.last_allocator_resident { Some(r) => r, None => snapshot.checked_total_owner_bytes()? }` as `used` | **A: GREEN / B: RED** | **CONFIRMED** (see note) | Three configs run. **A** mutated SUT + current test (never calls observe_allocator): **GREEN** — the named breakage (allocator-driven classification with semantic fallback) ships green; this is the direct fail-open proof, matching the auditor's prediction verbatim. **B** mutated SUT + test augmented with preceding `observe_allocator(resident_bytes: Some(999_999))` (the auditor's minimal fix): **RED** — panics `left: Fatal, right: Soft` (999_999/1M = 99.99% ≥ 98% → Fatal vs semantic 800k/1M = 80% → Soft): the mutation is real and bites via the allocator path, and the proposed fix has teeth. **C** clean SUT + augmented test: **GREEN** — the fix is sound on a correct SUT. Decision-rule note: the brief maps "B RED → REJECTED (it bites via that path)", but B's redness is exactly what T2's report itself predicts ("with the minimal fix applied the same mutation turns it red") and is the evidence the fix closes the gap; by the mission's own criterion (SUT broken + current test green) the row is **CONFIRMED**. Both raw results reported for the parent to re-judge. Restored clean. |
| 3 | T3 app-ports | `application::controller::tests::tiny_non_zero_limit_saturates_percent_without_wrapping` | controller.rs:159-161 — u128 percent pipeline replaced with `used_bytes.wrapping_mul(100) / self.global_hard_limit_bytes` (u64); second variant `.saturating_mul(100)` | **GREEN (both)** | **CONFIRMED** | Baseline green. With limit=1 (the only controller the test builds) both mutants classify u64::MAX as ≥98% → Fatal: wrapping gives 2^64−100, saturating gives u64::MAX. The wrap under-report regression (only manifests at large limits, e.g. limit=u64::MAX → wrapping 0% Normal) passes this test, exactly as the auditor computed numerically. Both variants run and green. Restored clean. |
| 4 | T5 facade-stores | `payload_store::service::tests::canonical_digests_are_randomized_per_store_seed` | payload_store/service.rs — field `digest_seed: RandomState` → `BuildHasherDefault<DefaultHasher>`; init `RandomState::new()` → `BuildHasherDefault::default()`; digest helpers made generic `B: BuildHasher` | **GREEN** | **CONFIRMED** | Baseline green. The mutation compiles and is active in `with_accounting` (every store now shares one deterministic digest seed — the grindable-bucket regression PS:879 names), yet the test stays green because its body only compares two std `RandomState::new()` instances and never constructs a store — A1/assert-on-std, exactly as audited. Restored clean. |
| 5 | T9 testkit | `fixtures::tests::testkit_runs_conformance_against_its_fake_and_passes` | fixtures/mod.rs:59 — `run_conformance` body stubbed to `ConformanceVerdict::Passed` (no suite run; broker construction discarded) | **GREEN** | **CONFIRMED** | Baseline green. The kit's advertised host-facing conformance entry (G1) can be hollowed out to a constant `Passed` and its only driving test stays green — positive-pole-only at the kit seam, exactly as audited. Restored clean. |
| 6 | T9 testkit | `fixtures::tests::testkit_runs_facade_store_suite_against_live_broker_and_passes` | suites.rs:683-686 — the four `verdicts.extend(run_{payload_store,retained_map,transcript,spill}_slice(...))` lines deleted (lifecycle suites kept, so `from_assertions` stays non-empty) | **GREEN** | **CONFIRMED** | Baseline green. Deleting all four store slices leaves a non-empty verdict list (lifecycle only) → folded verdict still `Passed`; nothing anywhere asserts slice coverage (G2/G5). The CM:140 empty-guard only catches TOTAL removal, exactly as audited. Restored clean. |
| 7 | T3 app-ports (VACUOUS rep) | `adapters::metrics::tests::emitter_holds_no_accounting_state` | metrics.rs — both cfg variants: `MetricsEmitter` gains field `events: Arc<Mutex<u64>>`, `emit()` does `*self.events.lock().unwrap() += 1` | **GREEN** | **CONFIRMED (vacuous)** | Baseline green. The emitter now holds AND mutates accounting-ish state on every emit; the test named "holds no accounting state" stays green because its body has zero asserts (pure compile/wiring marker), exactly as audited. Necessary mechanical accommodation: the two test construction sites `MetricsEmitter` → `MetricsEmitter::default()` (unit-struct syntax breaks with a field) — syntax-only, no asserts added; both tests in the file still pass under the mutation. Restored clean. |
| 8 | T9 testkit (VACUOUS rep) | `fixtures::tests::conformance_verdict_localizes_to_contract_family_not_a_host_package` | fixtures/mod.rs:141-144 — loop body replaced with unconditional `panic!("critic: violated_families iterated")` | **GREEN** | **CONFIRMED (vacuous)** | Baseline green. An unconditional panic inside the `for family in verdict.violated_families()` loop never fires — the loop NEVER iterates on the passing verdict the test itself builds, so the named localization check is dead code and only `assert!(verdict.passed())` can fail (A2 vacuous iteration, exactly as audited). Restored clean. |
| 9 | T9 testkit (VACUOUS rep, static) | all 5 `loom_models.rs` tests (LM:17-164) | no code mutation — evidence: `grep -E 'crate::|quanta_memory_platform' loom_models.rs` returns **zero matches** | n/a (grep) | **CONFIRMED (vacuous as SUT tests)** | Zero `crate::` / `quanta_memory_platform` references in the file (grep exit 1, no output). The loom models are self-contained hand-rolled state machines (`loom::sync` only); no production type is referenced, so NO production mutation can turn any of the 5 tests red — the models verify only themselves, exactly as T9's G9/LM rows claim (module doc admits this). Verdict per the brief's rule: zero references ⇒ CONFIRMED. |
| 10 | T8 pipeline-shared (WEAK) | `public_api_surface::platform_builder_installs_reclaimer_before_live_construction` (facade tests) | shared/builder.rs:112 — facade `with_reclaimer` discards its argument (`let _ = reclaimer; self`) instead of delegating to the core builder | **GREEN** | **CONFIRMED (weak)** | Baseline green. The platform is built with NO reclaimer installed — the eviction authority is silently unwired (T8's highest-value WEAK, A6) — and the test stays green because `attempts == 0` is also true of a discarded reclaimer. Restored clean. |
| 11 | T1 core-domain (WEAK) | `domain::policy::tests::for_preset_class_caps_isolate_below_global` | policy.rs:426 — after the weighted share, `let hard_cap_bytes = hard_cap_bytes.max(global_hard_limit_bytes);` (every class cap == global; isolation nominal) | **GREEN** | **CONFIRMED (weak)** | Baseline green. The test asserts `hard_cap_bytes <= global`; with caps inflated to EQUAL the global limit the `<=` still holds, so the named "strict" isolation (the code's own doc at policy.rs:388 promises "strictly below") is unenforced — exactly the audited `<=` vs `<` gap. Note: concurrent integrator fix-phase edits to control.rs/controller.rs/config.rs/ledger.rs (not mine, see attestation) were present in the tree during this run; they do not intersect `default_residency` or this test's asserts, so the verdict stands. policy.rs itself restored clean. |
| 12 | T6 spill-redb (WEAK) | `spill::service::tests::routed_spill_rolls_back_the_count_when_accept_fails` | planned: `RoutedSpillController::spilled_count` → constant `Ok(0)` | **NOT RUN** | **SKIPPED (safety)** | Baseline green. Mutation aborted before edit: by the time this sample came up, the integrator's concurrent fix phase had made spill/service.rs a dirty, actively-edited file (75 changed lines). Running my mutation there would have required `git checkout --` against a file carrying someone else's uncommitted work — prohibited by my safety rules. Recorded as skipped, not as an unmeasured claim. |

## Final confirmed fail-open inventory (input for the fix phase)

Every row below was proven by mutation: the SUT was broken exactly as the
finding claimed, the accused test was run scoped (`--exact`), and it stayed
green. Rows 1-6 are the P0 FAILOPEN set, 7-9 the P1 VACUOUS set, 10-11 the
measured P2 WEAK set.

| # | finding | proving mutation (file) | fix direction |
| --- | --- | --- | --- |
| 1 | T1 ledger.rs:1666 `single_release_preflight_is_bounded_independent_of_owner_cardinality` — probe counter derives from refund-map cardinality, not real lookups | owner preflight loop → full `owner_usage` scan (ledger.rs:914) stays green | count real axis lookups inside the probe loops, or hook `BTreeMap::get`; add a cardinality-scaling pole |
| 2 | T2 control.rs:3434 `pressure_classifies_from_semantic_usage` — never observes an allocator snapshot, so allocator-driven classification is invisible | `last_allocator_resident` field + prefer-when-present in `classify_pressure` (control.rs) stays green; augmented test (Config B) goes RED `Fatal != Soft` and passes on clean SUT (Config C) | add the observe_allocator(999_999/0) leg asserting Soft + ledger unchanged — exactly what Config B/C validated (and what the integrator's concurrent fix implements) |
| 3 | T3 controller.rs:421 `tiny_non_zero_limit_saturates_percent_without_wrapping` — only limit=1 built; wrap/saturate under-report invisible | `wrapping_mul(100)`/`saturating_mul(100)` u64 mutants both green | add the large-limit case (`new(u64::MAX).classify_semantic(u64::MAX) == Fatal`) |
| 4 | T5 payload_store PS:879 `canonical_digests_are_randomized_per_store_seed` — asserts std `RandomState` property, never a store | `digest_seed: BuildHasherDefault<DefaultHasher>` fixed seed green | build two real stores, compare digests across seeds |
| 5 | T9 fixtures FX:119 kit conformance entry positive-only (G1) | `run_conformance` stubbed to `Passed` green | kit-level negative pole (broken fidelity through the kit facade) |
| 6 | T9 fixtures FX:158 facade-store suite slice coverage unasserted (G2/G5) | four `run_*_slice` extends deleted, suite still passes | assert the expected verdict-name set at the public entries |
| 7 | T3 metrics.rs:69 `emitter_holds_no_accounting_state` — zero-assert body (VACUOUS) | emitter holds+mutates `Arc<Mutex<u64>>` counter, green | recorder-backed behavioral oracle under `telemetry`, or replace with a type-level pin |
| 8 | T9 fixtures FX:137 localization check dead loop (A2) | `panic!` inside `violated_families()` loop never fires | drive a broken fixture so the loop iterates, or delete the loop |
| 9 | T9 loom_models.rs LM:17-164 — zero production references (G9) | grep: no `crate::`/`quanta_memory_platform` in file; no production mutation can bite | bind models to production authorities or stop counting them as SUT coverage |
| 10 | T8 surface:414 reclaimer wiring silently deletable (A6) | facade `with_reclaimer` discards argument, green | counting reclaimer + one reclaim trigger, assert counter > 0 |
| 11 | T1 policy.rs:497 strict class-cap isolation asserted as `<=` | caps inflated to equal global, green | assert `<` for non-Headroom classes |

Rows measured but NOT confirmed: none. Every executed mutation produced the
auditor-predicted fail-open (GREEN). Row 2's Config B (fixed test + broken
SUT → RED) is the only RED observed, and it is the fix-bites proof, not a
refutation (see row 2 notes for the decision-rule discussion).

## Safety attestation

1. **Baseline at start**: `git status --porcelain` was EMPTY — cleaner than
   the briefed expectation (`?? docs/bugbash/sep-26-fail-open-audit/`); the
   nine reports were already committed at HEAD 61e9c7c. Nothing under crates/
   was dirty. Precondition held.
2. **Per-mutation hygiene**: every mutation was restored with
   `git checkout -- <exactly-the-file-just-mutated>` immediately after its
   run, each verified with `git diff --stat` (empty) at that moment. No
   commit was ever made. No script wrote outside /tmp, this report, and the
   mutation targets.
3. **Mutation-leak proof**: `grep -rn "CRITIC MUTATION|CRITIC AUGMENT"
   crates/` at closeout returns ZERO matches — no critic edit survives in the
   tree.
4. **Final tree vs baseline — deviation, with cause**: the briefed end-state
   ("untracked audit dir only, now containing your report") does NOT hold.
   During my P2 phase the parent/integrator began applying its fix phase
   concurrently in this same working tree (first observed as
   `reviewed-actual-findings.md` at 20:49, then uncommitted edits matching
   the auditors' minimal fixes — e.g. the control.rs diff is precisely the
   T2/F1 fix my Config B/C validated, and the testkit fixtures diff is the
   G1 faulted-broker negative pole). Final status:

   ```
    M crates/quanta-memory-platform-core/src/api/control.rs
    M crates/quanta-memory-platform-core/src/application/controller.rs
    M crates/quanta-memory-platform-core/src/application/scheduler.rs
    M crates/quanta-memory-platform-core/src/domain/admission.rs
    M crates/quanta-memory-platform-core/src/domain/config.rs
    M crates/quanta-memory-platform-core/src/domain/ledger.rs
    M crates/quanta-memory-platform-facade/src/lib.rs
    M crates/quanta-memory-platform-facade/src/payload_store/service.rs
    M crates/quanta-memory-platform-facade/src/retained_map/service.rs
    M crates/quanta-memory-platform-facade/src/shared/lifecycle.rs
    M crates/quanta-memory-platform-facade/src/spill/service.rs
    M crates/quanta-memory-platform-facade/src/transcript_store/service.rs
    M crates/quanta-memory-platform-testkit/src/fake/broker.rs
    M crates/quanta-memory-platform-testkit/src/fixtures/mod.rs
   ?? crates/quanta-memory-platform-facade/src/test_support.rs
   ?? docs/bugbash/sep-26-fail-open-audit/findings-critic.md
   ?? docs/bugbash/sep-26-fail-open-audit/reviewed-actual-findings.md
   ```

   None of the `M` files or the two non-report `??` files were created or
   edited by me; I never ran `git checkout` against them. My entire footprint
   is exactly one new file: `findings-critic.md` (this report). This is also
   why row 12 was skipped rather than measured: mutating files carrying
   someone else's uncommitted work would have forced an unsafe restore.
5. **Verdict validity under concurrency**: all P0/P1 rows (1-9) and row 10
   were measured while `crates/` was verified clean (each restore's empty
   diff doubles as that row's clean-tree witness). Row 11 ran with early
   integrator edits present in OTHER files (none intersecting the tested
   path) — noted in-row.

*Critic, 2026-09-26. Measured, fixed nothing, committed nothing.*
