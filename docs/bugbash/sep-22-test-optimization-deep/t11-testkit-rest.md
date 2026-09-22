# t11-testkit-rest — Deep audit: testkit rest (faults, fixtures, alloc_tracking, fake, loom_models, doctor, lib)

Scope (T11; disjoint from T10 `conformance/` and T12 `testkit/tests/` + hellgate):
`crates/quanta-memory-platform-testkit/src/faults/mod.rs`,
`src/faults/scenarios.rs`, `src/fixtures/mod.rs`,
`src/alloc_tracking/mod.rs`, `src/fake/broker.rs`, `src/fake/mod.rs`,
`src/fake/views.rs`, `src/loom_models.rs`, `src/doctor/mod.rs`, `src/lib.rs`.
Every `#[test]` body in scope opened and read. No fix code — audit only.

Grep census:
`grep -rn -E '#\[test\]|#\[tokio::test\]' <scope>` → **31** hits, all plain
`#[test]` (zero `#[tokio::test]`):
faults/scenarios.rs **5**, faults/mod.rs **2**, fixtures/mod.rs **5**,
alloc_tracking/mod.rs **5**, fake/broker.rs **6**, loom_models.rs **5**,
lib.rs (self_tests) **3**. `fake/mod.rs`, `fake/views.rs`, `doctor/mod.rs`
contain **zero** `#[test]` (verified by the same grep). Census table below
has **31 rows**.

Timing source: `docs/bugbash/sep-22-test-optimization-deep/timing/top40.md`
(present) + `timing/full.txt` (full per-test log, lines 152–233 cover the
testkit binary — all in-scope times visible). **No in-scope test appears in
top40.** Measured in-scope times (`--all-features`, nightly `--report-time`):
all 21 non-loom unit/self/fixture tests **0.000–0.004s**; the 5 loom models
**0.026–0.042s each** (~0.18s combined, only under `--all-features` + `loom`
feature); `fixtures::tests::testkit_runs_facade_store_suite_against_live_broker_and_passes`
0.004s. (Top40 rank 2, `property_trace_matches_independent_model` 4.973s, is
T10 `conformance/` scope — out of scope here.)

Lenses: **A** cheap-code · **B** cheap-test · **C** slow-measured ·
**D** weak-oracle · **E** duplicates. v1 =
`docs/bugbash/sep-22-test-optimization/05-testkit-integration.md` (F-1–F-17,
V-0); files 01–04 contain zero in-scope findings (grepped for
testkit/faults/fixtures/alloc/fake/loom/doctor — only unrelated hits).

## 1. Census (31 rows)

### 1a. faults/scenarios.rs (5 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| S01 | reclaim_returns_zero_is_contained | 180 | MERGE | 3rd cover of reclaim-zero with broker.rs:709 + mod.rs:121; see G01 |
| S02 | pinned_delete_failure_is_contained_and_bytes_survive | 187 | KEEP | only structural pin-delete-reject driver in scope; evidence pins bytes-survived |
| S03 | rebuild_race_write_is_rejected | 195 | MERGE | thinner twin of broker.rs:682 (no commit/release/resume legs); see G02 |
| S04 | spill_failure_is_contained_without_counting | 201 | MERGE | fail-closed leg subsumed by harness mod.rs:121; see G03 |
| S05 | spill_fault_classification_reports_escaped_for_a_lying_controller | 213 | KEEP | only negative-pole guard (Escaped must fire); harden routing, see G07 |

### 1b. faults/mod.rs (2 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| M01 | harness_covers_reclaim_pin_rebuild_and_spill | 121 | KEEP | keeper: only fan-out proof all 4 drivers run + all contained |
| M02 | empty_harness_runs_nothing | 139 | KEEP | 3-line vacuous-truth guard incl. `scenarios().is_empty()` coverage pin |

### 1c. fixtures/mod.rs (5 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| F01 | testkit_runs_conformance_against_its_fake_and_passes | 120 | MERGE | honest-fake-passes ×4 (v1 F-6); keeper suites.rs T10; see G04 |
| F02 | fake_pressure_fixture_propagates_pressure_into_the_fake_broker | 128 | KEEP | only pressure-propagation proof + Normal negative pair |
| F03 | conformance_verdict_localizes_to_contract_family_not_a_host_package | 138 | SHRINK | loop over empty vec is vacuous; harden oracle, see G08 |
| F04 | live_broker_materializes_from_fixture_manifest | 149 | KEEP | only live-broker-from-manifest proof (preset/execution/residency) |
| F05 | testkit_runs_facade_store_suite_against_live_broker_and_passes | 159 | KEEP | 0.004s; only in-scope live facade-suite entry; harden oracle, see G08 |

### 1d. alloc_tracking/mod.rs (5 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| A01 | fresh_probe_reports_zero_signals | 131 | KEEP | only zero-state + saturate-at-zero guard |
| A02 | recorded_signals_accumulate_and_net_live_is_correct | 140 | KEEP | base accumulator oracle all else builds on |
| A03 | dealloc_exceeding_alloc_saturates_at_zero_not_nonsense | 154 | KEEP | only underflow-saturation negative path; drop dead line, see G09 |
| A04 | signals_feed_an_invariant_assertion | 166 | KEEP | only probe→assertion wiring proof (conformance-gated) + drift evidence leg |
| A05 | tracker_callbacks_route_into_the_same_counters | 191 | KEEP | only AllocationTracker callback-path proof |

### 1e. fake/broker.rs (6 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| K01 | honest_reserve_keeps_class_owner_balanced | 603 | KEEP | base class/owner balance oracle |
| K02 | owner_hard_cap_denies_before_mutation | 621 | KEEP | only owner-cap deny + no-mutation; drop dead line, see G09 |
| K03 | resource_group_cap_denies_aggregate_over_owners | 647 | KEEP | only group-cap aggregate proof + smaller-fits positive pair |
| K04 | rebuild_window_rejects_writes_then_resumes | 682 | KEEP | keeper: reserve+commit+release deny + resume; absorbs S03 |
| K05 | reclaim_yield_zero_frees_nothing | 709 | KEEP | keeper: zero leg + 200-yield positive pair + invariant-1; absorbs S01 unit leg |
| K06 | broken_under_report_corrupts_only_the_reported_snapshot | 746 | KEEP | NOT a dup: documents the exact snapshot mismatch the suite must catch |

### 1f. loom_models.rs (5 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| L01 | loom_reserve_vs_reconfigure_preserves_installed_cap | 18 | KEEP | quarantined model; 0.037s, zero default-suite cost; see G10 |
| L02 | loom_ticket_transition_has_single_terminal_owner | 64 | KEEP | quarantined model; 0.037s; see G10 |
| L03 | loom_close_vs_mutation_never_mutates_after_close | 93 | KEEP | quarantined model; 0.036s; see G10 |
| L04 | loom_spill_transfer_never_has_zero_or_double_owner | 129 | KEEP | quarantined model; 0.026s; see G10 |
| L05 | loom_versioned_callback_compensation_is_exactly_once | 165 | KEEP | quarantined model; 0.042s; see G10 |

### 1g. lib.rs self_tests (3 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| T01 | conformance_suite_rejects_a_fake_that_under_reports_usage | 70 | MERGE | dup of conformance/mod.rs:128 keeper (v1 F-7); see G05 |
| T02 | conformance_suite_rejects_a_fake_with_divergent_class_totals | 85 | MERGE | dup of suites.rs keeper (v1 F-8); see G06 |
| T03 | conformance_suite_accepts_an_honest_fake | 92 | MERGE | dup ×4 honest-passes (v1 F-6); see G04 |

Row-count check: 5 + 2 + 5 + 5 + 6 + 5 + 3 = **31 = grep count 31**. Verdict
tally: KEEP 23, MERGE 7, SHRINK 1, DELETE 0.

## 2. Findings

### G01 [E/M] Reclaim-zero covered 3× in scope — fold the driver test
- Oracle 1 — `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:180`:
  `fn reclaim_returns_zero_is_contained() {`
  `assert!(outcome.evidence().contains("freed 0"));`
- Oracle 2 — `crates/quanta-memory-platform-testkit/src/fake/broker.rs:721`:
  `assert_eq!(b.reclaim_once(), 0, "reclaim-returns-zero must free nothing");`
- Mechanism: S01 re-asserts through the `FaultOutcome` evidence string what
  K05 asserts directly on the broker (plus K05's 200-yield positive pair and
  M01's harness fan-out). The evidence-substring leg is the weakest of the three.
- Action: delete S01 (`scenarios.rs:179-184`); keepers `fake/broker.rs:709`
  (unit + positive pair) and `faults/mod.rs:121` (harness coverage).
- Est. saving: ~6 lines; runtime ~0 (all legs 0.000s).

### G02 [E/M] Rebuild-race driver test is the thin twin — fold into K04
- Oracle — `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:195`:
  `fn rebuild_race_write_is_rejected() {`
  `assert!(outcome.contained(), "rebuild race must be contained: {}", outcome.evidence());`
- Mechanism: S03 asserts reserve-deny only; K04 (`fake/broker.rs:682`)
  additionally asserts commit-deny, release-deny, and post-`finish_rebuild`
  resume (`assert!(b.commit(&t).is_ok(), "writes resume after rebuild")`).
  No oracle is lost by folding.
- Action: delete S03 (`scenarios.rs:194-198`); keeper `fake/broker.rs:682`.
- Est. saving: ~5 lines; runtime ~0.

### G03 [E/S] Spill fail-closed driver test overlaps the harness — fold it
- Oracle — `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:204`:
  `assert!(outcome.evidence().contains("stayed at 0"));`
- Mechanism: S04's only distinctive leg is the `spilled_count stayed at 0`
  evidence substring; the Contained/failed-closed leg is re-proven by M01's
  harness run over `drive_spill_failure_after_reservation` and by the T10
  suite verdict. Keep the driver as the chaos-harness entry; drop its
  standalone test.
- Action: delete S04 (`scenarios.rs:200-205`); keeper `faults/mod.rs:121`.
- Est. saving: ~6 lines; runtime ~0.

### G04 [E/M] Honest-fake-passes asserted 3× in scope — keep one (T10 keeper)
- Oracles — `crates/quanta-memory-platform-testkit/src/lib.rs:92`:
  `fn conformance_suite_accepts_an_honest_fake() {`;
  `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs:120`:
  `fn testkit_runs_conformance_against_its_fake_and_passes() {`
  `assert!(verdict.violated_families().is_empty());`
- Mechanism: T03, F01, plus T10 `conformance/mod.rs` and `suites.rs`
  honest-passes tests all build an honest fake/kit and assert pass. Per v1
  F-6 the suite-level keeper (asserts every invariant held with violator
  evidence) subsumes the rest.
- Action: delete T03 (`lib.rs:91-98`) and F01 (`fixtures/mod.rs:119-125`);
  keeper T10 `suites.rs` honest-fake keeper (v1-reported `suites.rs:2374-2383`).
- Est. saving: 2 tests (~14 lines); runtime ~0 (both 0.000–0.001s).

### G05 [E/S] Under-report rejection lib copy is the dup; broker unit is not
- Oracle — `crates/quanta-memory-platform-testkit/src/lib.rs:77`:
  `verdict.violated_families().contains(&ContractFamily::ClassOwnerBalance),`
- Mechanism: T01 duplicates the T10 `conformance/mod.rs` folded-verdict +
  localization test (same fidelity, same family). K06 (`fake/broker.rs:746`)
  is a different level — it pins the raw snapshot mismatch
  (`total_owner_bytes 400 vs total_class_bytes 500`) the suite must catch —
  and must be kept.
- Action: delete T01 (`lib.rs:69-82`); keepers T10 `conformance/mod.rs:128`
  (v1-reported) and in-scope `fake/broker.rs:746`.
- Est. saving: ~13 lines; runtime ~0.

### G06 [E/S] Diverge-class rejection lib copy subsumed by exhaustive list
- Oracle — `crates/quanta-memory-platform-testkit/src/lib.rs:85`:
  `fn conformance_suite_rejects_a_fake_with_divergent_class_totals() {`
- Mechanism: same variant bitten next to the exhaustive
  `all_break_variants` list in T10 `suites.rs` (v1 F-8); the lib copy adds no
  family pin (no `violated_families` assert at all — weaker oracle too).
- Action: delete T02 (`lib.rs:84-89`); keeper T10 `suites.rs` (v1-reported
  `suites.rs:2401-2409`).
- Est. saving: ~6 lines; runtime ~0.

### G07 [D/M] `spill_fault_classification` tests its own local helper
- Oracle — `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:215`:
  `fn classify<C: SpillController<String>>(controller: &C) -> FaultOutcome {`
  with local `LyingSpillController` (`scenarios.rs:228`) that never passes
  through `drive_spill_failure_after_reservation`.
- Mechanism: a regression in the driver's `(before, result, after)` /
  `spilled_count` plumbing would not flip this test — it classifies via its
  own inline match, not the production driver path.
- Action: keep S05 (only Escaped-pole guard in scope) but route the lying
  target through the driver shape at `scenarios.rs:157-173` (or assert the
  driver's triple directly against a lying target).
- Est. saving: quality only; runtime ~0.

### G08 [D/S] Two pass-only / vacuous oracles need hardening, not deletion
- Oracle 1 — `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs:141`:
  `for family in verdict.violated_families() {`
  `let _: ContractFamily = *family;`
- Oracle 2 — `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs:159`:
  `fn testkit_runs_facade_store_suite_against_live_broker_and_passes() {`
  `assert!(verdict.passed(), "live facade suite must pass: {verdict:?}");`
- Mechanism: F03's loop iterates an empty vec on pass, so the body never
  executes — the "never names a host" pin is compile-time only and would
  still pass if the suite silently ran zero checks (same shape as v1 F-14,
  which lives in T12 `tests/`). F05 asserts `passed()` with no
  family/evidence/count spot-check.
- Action: F03 → feed a broken fake and assert the violated family localizes
  (mirror `conformance/mod.rs` pattern); F05 → also assert
  `violated_families().is_empty()` + a non-empty verdict/check count
  (mirror `conformance/mod.rs:141-145` empty-is-failed pin).
- Est. saving: quality only; runtime stays ~0.004s.

### G09 [D/S] Two dead `assert_ne!`s after exact equality
- Oracle 1 — `crates/quanta-memory-platform-testkit/src/alloc_tracking/mod.rs:161`:
  `assert_ne!(s.net_live_bytes(), u64::MAX - 898);`
  (previous line already asserted `== 0`; the `ne` is tautological and the
  `MAX - 898` constant is unexplained).
- Oracle 2 — `crates/quanta-memory-platform-testkit/src/fake/broker.rs:643`:
  `assert_ne!(after, 1_200);`
  (previous line already asserted `before == after` with `before == 800`;
  the `ne` re-states arithmetic, not a contract).
- Mechanism: both are satisfied by construction once the preceding exact
  assert holds; neither can fail independently, so they add reading cost and
  a false sense of a second oracle.
- Action: delete the two `assert_ne!` lines (`alloc_tracking/mod.rs:161`,
  `fake/broker.rs:643`); keep A03 and K02 with their exact asserts.
- Est. saving: 2 lines; runtime ~0.

### G10 [C/S] No slow-measured test in scope; loom correctly quarantined
- Evidence — `timing/full.txt:228-232`: the 5 `loom_models::` tests measure
  `0.026s / 0.036s / 0.037s / 0.037s / 0.042s`; every other in-scope test is
  `0.000–0.004s`; `top40.md` contains zero in-scope tests.
- Mechanism: loom enumeration is inherently slower than the enclosed logic,
  but `lib.rs:33-34` gates the module behind
  `#[cfg(all(test, feature = "loom"))]`, so default `cargo test` pays zero;
  the cost appears only in the `--all-features` timing run.
- Action: none — keep opt-in; do not move loom models into the default suite.
- Est. saving: n/a (informational; ~0.18s only under `--all-features`).

### G11 [A/S] `FakeBrokerFactory` is a pass-through wrapper
- Oracle — `crates/quanta-memory-platform-testkit/src/fake/broker.rs:584`:
  `impl FakeBrokerFactory {`
  `pub fn honest() -> FakeProcessBroker {` → `FakeProcessBroker::honest()`
- Mechanism: `broker.rs:582-596` re-exports one-liner constructors already on
  `FakeProcessBroker`; `fake/mod.rs:1-15` + `fake/views.rs:54-73` are thin
  re-export/factory shims. Harmless but doubles the "how do I make a fake?"
  surface (code-cheap lens, not test-cheap).
- Action: keep one constructor path in docs; deprecate the other.
- Est. saving: ~15 lines of surface; runtime n/a.

### G12 [B/S] `doctor/` has no tests and a trivial oracle
- Oracle — `crates/quanta-memory-platform-testkit/src/doctor/mod.rs:1-12`:
  whole file is a 3-vec struct; `is_clean()` checks only `errors.is_empty()`
  (warnings/notes never affect cleanliness by design).
- Mechanism: `grep #[test]` over `doctor/` = 0 (verified in census); not even
  the errors-only-cleanliness rule is pinned.
- Action: add one ~5-line `is_clean` true/false unit test (recommendation
  only — no fix code per track rules).
- Est. saving: negative (adds a test); pins an unpinned contract.

### G13 [C/S] Facade-store suite fan-out caller is cheap here — note only
- Oracle — `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs:159`:
  `fn testkit_runs_facade_store_suite_against_live_broker_and_passes() {`
  measured `0.004s` in `timing/full.txt`.
- Mechanism: F05 re-invokes the full `run_facade_store_suite` fan-out (v1
  F-2: payload + retained-map + transcript + spill + 5× lifecycle + 2
  tight-cap platforms per call, alongside the `tests/` callers in T12).
  Measured cheap in this binary, so no SHRINK verdict; the sharing fix (one
  built platform per process / `OnceLock` fixture) belongs to the suite
  owner (T10), not this caller.
- Action: none in T11; cite for the T10 owner.
- Est. saving: n/a in scope.

## 3. v1 re-verification (05-testkit-integration.md; 01–04 have no in-scope findings)

| v1 | status | note |
| --- | --- | --- |
| V-0 faults/ clean, zero sleeps | CONFIRM | re-read `scenarios.rs:1-249`, `mod.rs:1-146`: no sleep/Duration/Instant/timeout tokens; races structural |
| F-1 proptest cost | out of scope | `suites.rs` proptest is T10; no in-scope counterpart |
| F-2 suite fan-out | CONFIRM (partial) | in-scope caller `fixtures/mod.rs:159` verified; measured 0.004s, fix belongs to T10; see G13 |
| F-3 hellgate concurrency | out of scope | `hellgate_e2e.rs` is T12; no in-scope counterpart |
| F-4 loom quarantined | CONFIRM | `lib.rs:33-34` gate re-verified; 5 models 0.026–0.042s only under `--all-features`; see G10 |
| F-5 per-step view equality | out of scope | `suites.rs` deterministic trace is T10 |
| F-6 honest-passes ×4 | CONFIRM | in-scope copies `lib.rs:92`, `fixtures/mod.rs:120` re-read; fold to T10 keeper; see G04 |
| F-7 under-report ×3 | CONFIRM (nuanced) | `lib.rs:70` is the dup; `fake/broker.rs:746` is a different level (raw mismatch pin), keep; see G05 |
| F-8 diverge ×2 | CONFIRM | `lib.rs:85` has no family pin (weaker than v1 described); delete; see G06 |
| F-9 lifecycle vs hellgate | CONFIRM (partial) | in-scope side has no lifecycle suite; trim direction aims at T12 hellgate, nothing to cut here |
| F-10 pin ×3 | CONFIRM | in-scope driver `scenarios.rs:88-124` re-read; keeper side (driver + suite verdict), hellgate trim is T12 |
| F-11 spill ×3 | CONFIRM | in-scope driver `scenarios.rs:157-173` re-read; keep driver as chaos entry, fold S04 test; see G03 |
| F-12 local-helper oracle | CONFIRM | `scenarios.rs:213-248` re-read; lying controller never touches the driver; see G07 |
| F-13 assert_balanced drift | out of scope | `hellgate_e2e.rs` is T12 |
| F-14 pass-only surface test | SUPERSEDE | the cited file is T12 `tests/`; the same weak-oracle shape exists in scope at `fixtures/mod.rs:159` — folded into G08 |
| F-15 doctor untested | CONFIRM | `doctor/mod.rs:1-12` re-read, zero `#[test]`; see G12 |
| F-16 factory wrapper | CONFIRM | `fake/broker.rs:582-596` re-read; see G11 |
| F-17 cheap units keep | CONFIRM | `alloc_tracking/mod.rs:131-203` + `fixtures/mod.rs:128-135` re-read; all 0.000–0.003s; keep (minus G09 dead lines) |

## 4. Top quick wins (in scope only)

1. **Delete the 3 lib.rs meta-test dups** (G04–G06): `lib.rs:69-98` → keepers
   in T10; removes 3 tests, ~25 lines, zero oracle loss.
2. **Fold 3 thin driver tests into harness/unit keepers** (G01–G03): delete
   S01/S03/S04; keepers `fake/broker.rs:682,709` + `faults/mod.rs:121`.
3. **Harden 3 weak oracles without new tests** (G07–G09): route S05 through
   the driver; pin non-empty verdicts in F03/F05; delete 2 dead `assert_ne!`s.
4. **Do nothing, deliberately** (G10, G12–G13): loom stays opt-in; doctor
   gets one 5-line test when code is allowed; suite fan-out fix belongs to T10.
5. **Deprecate one fake-constructor path** (G11): docs-only, removes ~15 lines
   of duplicate surface.

## Unresolved

- Timing: `top40.md` present and `timing/full.txt` covers all 31 in-scope
  tests, so no timing evidence is missing for T11. (No "absent" item.)
- Keeper line numbers in T10 `suites.rs` / `conformance/mod.rs` are
  v1-reported pointers, not re-inspected here (T10 owns those files).
- Loom per-test times come from the `--all-features` run; default-feature
  runs do not execute them (by gate design) — no separate default-feature
  timing was taken.
