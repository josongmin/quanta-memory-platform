# t10-conformance — Deep audit: testkit/src/conformance/

Scope: `crates/quanta-memory-platform-testkit/src/conformance/` —
`suites.rs` (2975 lines), `target.rs` (439), `mod.rs` (232),
`invariants.rs` (233). Every `#[test]` body opened and read.
No fix code — audit only.

Grep census: `grep -rn -E '#\[(test|tokio::test)' crates/quanta-memory-platform-testkit/src/conformance/` →
mod.rs **5**, invariants.rs **5**, suites.rs **35**, target.rs **0**, total **45**.
Census table below has **45 rows**. No `#[tokio::test]` in scope (all plain
`#[test]`). Note: `suites.rs:1836` (`property_trace_matches_independent_model`)
is declared inside the `proptest!` macro body rather than a plain `mod tests`
item, but it expands to exactly one test fn and appears once in timing —
counted once. `target.rs` contains zero test fns (pure adapter impls).

Timing source: `docs/bugbash/sep-22-test-optimization-deep/timing/top40.md`
(**present**). Only one in-scope test appears in the top-40: rank 2
`conformance::suites::tests::property_trace_matches_independent_model`
(4.973s). No other in-scope test is timing-visible; the rest are
sub-0.2s by absence (see Unresolved for the `full.txt` caveat).

Lenses: **A** cheap-code · **B** cheap-test · **C** slow-measured ·
**D** weak-oracle · **E** duplicates. v1 =
`docs/bugbash/sep-22-test-optimization/05-testkit-integration.md`
(findings F-1–F-17); files 01/02/03/04 contain zero in-scope findings
(grepped for conformance/suites.rs/invariants.rs/testkit — one hit in
01 is a `class_budget` helper dup in `ledger.rs` vs `policy_contracts.rs`,
unrelated to this scope).

## 1. Census (45 rows)

### 1a. mod.rs (5 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| M01 | honest_fake_yields_passed_verdict | 120 | MERGE | honest-pass pole subsumed by M04's honest pole (216-220) + S21 keeper; see G03 |
| M02 | broken_fake_yields_failed_verdict_localized_to_balance_family | 128 | KEEP | keeper: only public folded-verdict UnderReport→ClassOwnerBalance proof; see G04 |
| M03 | empty_assertion_list_is_failed_not_silently_passed | 141 | KEEP | only empty-fold fail-closed proof; guards zero-check rubber stamp |
| M04 | public_suite_rejects_every_break_variant_and_localizes_it | 180 | KEEP | keeper: exhaustive folded-surface enumeration + compile-time guard |
| M05 | real_core_broker_passes_the_same_portable_suite | 224 | KEEP | only folded-surface proof on the real broker (S34 runs raw verdicts) |

### 1b. invariants.rs (5 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| I01 | equal_bytes_hold_unequal_violate_with_drift_evidence | 144 | MERGE | hold/violate/evidence subsumed by I04 + attribution by I05; see G05 |
| I02 | predicate_assertion_carries_supplied_evidence | 157 | KEEP | only Display/VIOLATED formatting proof for assert_that |
| I03 | every_family_has_a_unique_stable_label | 181 | KEEP | only label-uniqueness + Display==label taxonomy proof |
| I04 | drift_evidence_sign_distinguishes_over_from_under_reporting | 198 | KEEP | keeper: direction-of-lie evidence + u64::MAX boundary |
| I05 | family_attribution_is_preserved_through_both_assertion_constructors | 218 | KEEP | only per-family attribution proof over both constructors |

### 1c. suites.rs (35 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| S01 | deterministic_trace_matches_independent_model | 1623 | SHRINK | 256 steps ×2 backends × full view; stride the oracle; see G02 |
| S02 | property_trace_matches_independent_model | 1837 | SHRINK | 4.973s, top40 #2; cut cases / subsample live backend; see G01 |
| S03 | reentrant_sink_is_contained_outside_owner_lock | 1865 | KEEP | only deadlock-containment (reentrant observe) proof |
| S04 | suite_rejects_black_hole_accounting | 1890 | KEEP | distinct fault; 6-line wrapper, name-is-the-diagnosis |
| S05 | suite_rejects_commit_swallowing_target | 1898 | KEEP | distinct TicketLifecycle fault leg; see G07 |
| S06 | suite_rejects_release_swallowing_target | 1906 | KEEP | distinct TicketLifecycle fault leg; see G07 |
| S07 | suite_rejects_duplicate_commit_accepting_target | 1914 | KEEP | distinct TicketLifecycle fault leg; see G07 |
| S08 | suite_rejects_wrong_context_axis | 1922 | KEEP | only context-axis fault proof |
| S09 | suite_rejects_wrong_charge_kind_axis | 1930 | KEEP | only charge-kind-axis fault proof |
| S10 | suite_rejects_stale_ticket_accepting_target | 1938 | KEEP | distinct TicketLifecycle fault leg; see G07 |
| S11 | snapshot_error_is_failed_verdict_not_zero | 1946 | KEEP | only injected-observation-error family-set proof |
| S12 | suite_rejects_transfer_double_owner | 1959 | KEEP | only double-owner family-set proof |
| S13 | prepared_publication_suite_accepts_honest_service | 1972 | KEEP | only honest prepared-publication acceptance |
| S14 | prepared_suite_rejects_correct_replay_with_unbilled_identity | 2040 | KEEP | only underbilling oracle with partial-hold (conflict still HELD) shape |
| S15 | prepared_publication_suite_rejects_duplicate_charging_fake | 2126 | KEEP | only duplicate-charging bite + fail-closed localization |
| S16 | prepared_publication_suite_rejects_root_blind_gc_fake | 2203 | KEEP | only root-blind-GC bite + reclaim localization |
| S17 | transcript_append_suite_accepts_honest_service | 2214 | KEEP | only honest transcript acceptance |
| S18 | transcript_append_suite_rejects_non_idempotent_fake | 2280 | KEEP | only non-idempotent-append bite proof |
| S19 | transcript_suite_rejects_target_that_ignores_declared_horizon | 2291 | KEEP | only horizon-mismatch single-verdict localization |
| S20 | transcript_suite_rejects_correct_replay_with_unbilled_identity | 2348 | KEEP | transcript-side underbilling oracle (different suite than S14) |
| S21 | honest_fake_passes_every_accounting_invariant | 2374 | KEEP | keeper: raw-suite honest acceptance with violator evidence; see G03 |
| S22 | suite_rejects_fake_that_under_reports_owner_bytes | 2386 | MERGE | dup of M02 keeper; see G04 |
| S23 | suite_rejects_fake_whose_class_diverges_from_owner | 2402 | KEEP | keeper per F-8 (lives by the exhaustive list); see G04 |
| S24 | all_break_variants_are_distinct_non_honest_and_family_mapped | 2441 | KEEP | meta guard: list completeness + family mapping |
| S25 | suite_rejects_every_accounting_fidelity_break_variant | 2468 | KEEP | keeper: negative enumeration over all break variants |
| S26 | owner_partition_break_localizes_to_owner_partition_not_balance | 2490 | KEEP | only negative-localization (NOT balance) proof; S25 asserts contains only |
| S27 | honest_fidelity_is_the_only_variant_the_suite_accepts | 2507 | MERGE | broken(Honest) accept == S21 via other constructor; see G03 |
| S28 | suite_rejects_target_that_mutates_accounting_on_a_denied_reserve | 2622 | KEEP | only ReserveFailClosed bite proof (mutate-on-deny liar) |
| S29 | honest_fake_passes_the_strengthened_fail_closed_check | 2638 | MERGE | fail-closed pole subsumed by S21 all-held; fold non-empty assert into S21; see G03 |
| S30 | suite_rejects_target_that_mutates_on_allocator_observation | 2736 | KEEP | only AllocatorAdvisory bite proof |
| S31 | commit_then_release_returns_balance_to_zero_on_honest_fake | 2752 | KEEP | keeper: cycle-to-zero lifecycle oracle |
| S32 | commit_then_release_returns_balance_to_zero_on_real_broker | 2779 | MERGE | same lifecycle as S31 on second backend; genericize over target; see G06 |
| S33 | cap_deny_agrees_between_honest_fake_and_real_broker | 2838 | KEEP | only fake/real deny-agreement (typed Capacity + inert + resume) proof |
| S34 | both_targets_still_pass_the_portable_suite_after_strengthening | 2913 | MERGE | strengthened-suite pass == S21 + M05 re-run; see G03 |
| S35 | spill_routing_verdict_holds_for_honest_and_violates_for_a_dropping_target | 2962 | KEEP | only SpillRouting bite proof via suite helper (fixtures-gated) |

Verdict totals: KEEP 36 · SHRINK 2 (S01, S02) · MERGE 7 (M01, I01, S22, S27, S29, S32, S34) · DELETE 0. Total 45 = grep count 45.
Net proposed: −7 tests folded into keepers, 2 shrunk in place.

## 2. Findings

### G01 [C] Proptest trace is the slowest in-scope test (4.973s) — SHRINK S02
- Location: `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1829-1843`
- Oracle quoted:
  ```
  cases: 128,
  max_shrink_iters: 8_192,
  ```
  ```
  run_generated_trace(&mut grouped_fake(), &operations);
  run_generated_trace(&mut grouped_live_broker(), &operations);
  ```
- Mechanism: 128 cases × up-to-96 ops × **two** backends, with a full
  `target.view() == Ok(model.view())` equality after every op
  (`suites.rs:1749-1753`), each `view()` cloning ledger maps; failure
  shrinking replays up to 8k iters. Timing confirms: top40 rank 2, 4.973s —
  the only in-scope test in the top-40.
- Action: in `suites.rs:1829-1843`, cut `cases: 128 → 32`, keep
  `max_shrink_iters`, and run the live broker on every 4th case (fake every
  case). Keeper stays `suites.rs:1837` (S02) itself, shrunk in place.
- Est. saving: ~3–3.5s per full-workspace run (dominant in-scope cost).

### G02 [C] Deterministic trace asserts a full ledger view after each of 512 steps — SHRINK S01
- Location: `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1604-1608`
- Oracle quoted:
  ```
  assert_eq!(
      target.view(),
      Ok(model.view()),
  ```
- Mechanism: `run_independent_trace` (`suites.rs:1545-1620`) compares the
  full `AccountingView` after each of 256 steps × 2 backends (fake + live),
  each `view()` cloning full ledger maps (`target.rs:353-414`). Not
  timing-visible (sub-0.2s), but the same shape as G01 at small scale.
- Action: in `suites.rs:1604-1608`, assert affected axes per step and keep
  the full-view assert every 16th step + the final drain
  (`suites.rs:1618-1619`). Keeper `suites.rs:1623` (S01), shrunk in place.
- Est. saving: ~50–100ms + ~450 fewer full-snapshot clones per run.

### G03 [E] Honest-acceptance asserted in five places — keep S21 (+M04/M05 poles), merge three
- Locations:
  - `crates/quanta-memory-platform-testkit/src/conformance/mod.rs:119-125` (M01)
  - `crates/quanta-memory-platform-testkit/src/conformance/mod.rs:215-220` (M04 honest pole)
  - `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2373-2383` (S21)
  - `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2506-2514` (S27)
  - `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2637-2655` (S29)
  - `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2912-2924` (S34)
- Oracle quoted (`suites.rs:2378-2382`):
  ```
  assert!(
      all_held(&verdicts),
      "honest fake must satisfy every invariant: {:?}",
  ```
- Mechanism: M01, S21, S27 all drive an honest fake through the same suite
  and assert all-held; S29 re-asserts the fail-closed subset of the same
  verdict list; S34 re-runs S21's fake leg plus M05's real leg. Six
  suite-runs (five fake) prove one fact. (v1 F-6's other two copies,
  `lib.rs:92-98` and `fixtures/mod.rs:120-125`, are T11 scope — out of
  census, same recommendation.)
- Action: keeper `suites.rs:2374` (S21, only one asserting every invariant
  with violator evidence) + keep M04's honest pole (folded surface) and M05
  (real broker). MERGE M01 into M04's pole, S27 into S21 (constructor
  difference `broken(Honest)` vs `honest()` proves nothing — both go through
  the same fidelity path), S29 into S21 by adding a fail-closed-family
  non-empty assert, S34 deleted as a re-run of S21+M05.
- Est. saving: ~4 redundant full-suite runs on fakes + 1 on a live broker
  (~100–200ms) and ~60 lines.

### G04 [E] Under-report / class-diverge rejection duplicated across surfaces — keep M02 + S23, merge S22
- Locations:
  - `crates/quanta-memory-platform-testkit/src/conformance/mod.rs:127-138` (M02, keeper)
  - `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2385-2399` (S22)
  - `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2401-2409` (S23, keeper)
- Oracle quoted (`mod.rs:130-137`):
  ```
  let mut broken = FakeProcessBroker::broken(AccountingFidelity::UnderReportOwnerBytes(100));
  let verdict = ConformanceSuite::new().run_broker_accounting(&mut broken);
  ```
- Mechanism: M02 and S22 feed the identical
  `UnderReportOwnerBytes(100)` variant and assert the identical
  `ClassOwnerBalance` localization — once via the folded verdict, once via
  raw verdicts. The folded-surface copy (M02) is the stronger oracle (it
  also proves the fold preserves localization); the raw copy adds nothing
  beyond what S25's enumeration already covers. S23 stays: it is the keeper
  next to the exhaustive `all_break_variants` list (`suites.rs:2418-2436`).
  (v1 F-8's `lib.rs:85-89` copy is T11 scope — delete per F-8, out of census.)
- Action: MERGE S22 into M02; keep M02 + S23 + S25 enumeration.
- Est. saving: 1 redundant suite run + ~15 lines.

### G05 [B/E] I01 subsumed by I04 + I05 — merge the byte-equality smoke into the drift keeper
- Location: `crates/quanta-memory-platform-testkit/src/conformance/invariants.rs:143-154`
- Oracle quoted:
  ```
  let bad = a.assert_eq_bytes("class_vs_owner", 1_000, 900);
  assert!(bad.evidence.contains("drift=100"), "evidence must expose the drift: {bad}");
  ```
- Mechanism: I04 (`invariants.rs:197-215`) already proves hold (u64::MAX
  boundary), violate in both directions, and drift evidence with sign; I05
  (`invariants.rs:217-232`) already proves family attribution for
  `assert_eq_bytes` across all nine families — everything I01 asserts except
  the `"balance"` name string. Microsecond-cheap either way (lens B: keep
  verdict is also defensible); merge is lines-only hygiene.
- Action: MERGE I01 into I04 (keeper `invariants.rs:198`); carry the
  `drift=100` magnitude spot-check as one extra assert in I04.
- Est. saving: ~12 lines, ~0 runtime.

### G06 [E] Commit→release cycle proven twice, once per backend — genericize S31/S32
- Locations:
  - `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2751-2776` (S31, fake)
  - `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2778-2799` (S32, real)
- Oracle quoted (`suites.rs:2773-2775`):
  ```
  assert_eq!(end.total_owner_bytes, 0, "release returns owner usage to zero");
  assert_eq!(end.total_class_bytes, 0, "release returns class usage to zero");
  ```
- Mechanism: both tests speak only through the shared
  `BrokerConformanceTarget` trait (`do_reserve`/`do_commit`/`do_release`/`view`)
  — the bodies differ only in target construction. Two ~25-line bodies prove
  one backend-neutral lifecycle.
- Action: MERGE S32 into S31 via a `fn cycle_returns_to_zero<T:
  BrokerConformanceTarget>(target: &mut T)` helper in `suites.rs` tests mod,
  called with `FakeProcessBroker::honest()` and `live_broker()`
  (`suites.rs:2805-2835`). Keeper S31.
- Est. saving: ~25 lines, ~0 runtime (both sub-ms).

### G07 [D] Nine fault-rejection tests have no exhaustiveness tie to BrokerFault — add guard, keep tests
- Location: `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1349-1360` (fault enum) vs `suites.rs:1889-1969` (S04–S12)
- Oracle quoted:
  ```
  enum BrokerFault {
      BlackHoleAccounting,
  ```
- Mechanism: unlike `AccountingFidelity`, which has a compile-time
  exhaustiveness guard (`suites.rs:2423-2430` + `mod.rs:188-195`) forcing
  each variant into the enumeration test, nothing fails to compile when a
  tenth `BrokerFault` is added and left without a rejection test — the nine
  S04–S12 wrappers are a hand-maintained list with no tie. The tests
  themselves stay (cheap one-liners whose names are the diagnosis).
- Action: add a `_exhaustiveness_guard` match (no wildcard) over
  `BrokerFault` next to `assert_broken_target_rejected`
  (`suites.rs:1449-1458`) mirroring `suites.rs:2423-2430`. No test deleted.
- Est. saving: 0 runtime; prevents a silent rubber-stamp hole on the next
  fault variant.

### G08 [C] Every prepared/transcript test builds its own full platform — share one per test binary
- Locations: `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1973-1985` (S13), `suites.rs:2041-2053` (S14), `suites.rs:2143-2158` (S16), `suites.rs:2215-2228` (S17), `suites.rs:2293-2305` (S19), `suites.rs:2349-2362` (S20); suite fan-out `suites.rs:664-672` + tight-cap rebuilds `suites.rs:1116`, `suites.rs:1209`
- Oracle quoted:
  ```
  let platform = MemoryPlatformBuilder::new()
      .with_preset(MemoryPreset::Development)
      .build()
      .expect("platform");
  ```
  (repeated verbatim in six test bodies)
- Mechanism: six tests each pay a full `MemoryPlatformBuilder::build()`
  (broker + residency + groups) to reach an identical Development-preset
  platform before exercising small prepared/transcript scripts. None is
  timing-visible individually, but the pattern is the in-scope instance of
  v1 F-2's fan-out.
- Action: share one lazily-built platform per test process (e.g. helper
  returning a fresh store handle off a `OnceLock` platform, or a
  `fn dev_platform() -> LiveMemoryPlatform` used by S13/S14/S16/S17/S19/S20
  at `suites.rs:1973` et al.). Keepers unchanged.
- Est. saving: ~5 redundant platform builds per run (~100–300ms est.;
  unmeasured — no per-test timing below top-40 granularity).

## 3. v1 re-verification (in-scope findings only)

| v1 | status | evidence |
| --- | --- | --- |
| F-1 proptest cost | CONFIRM | timing rank 2, 4.973s (`timing/top40.md:18`); body read `suites.rs:1829-1843`; see G01 (this report supersedes the "likely dominates" hedge with a measured rank) |
| F-2 suite fan-out | CONFIRM | `run_facade_store_suite` fan-out re-read (`suites.rs:664-672`, extra platforms at `suites.rs:1116`/`1209`); in-scope instance is per-test platform builds, see G08 |
| F-5 per-step full-view equality | CONFIRM | re-read `suites.rs:1604-1608`; deterministic leg confirmed cheap-but-same-shape; see G02 |
| F-6 honest-fake dupes | CONFIRM + SUPERSEDE keeper set | keeper `suites.rs:2374` confirmed best oracle (violator evidence); in-scope dup count revised to 6 bodies (M01, M04-pole, S21, S27, S29, S34); see G03 |
| F-7 under-report dupes | CONFIRM | M02 vs S22 re-read, identical variant + family; keeper M02 (folded surface); see G04 |
| F-8 DivergeClass dup | CONFIRM | S23 re-read as keeper-by-list-proximity; `lib.rs` copy out of scope (T11); see G04 |
| F-17 cheap invariant/fixture units | CONFIRM | all five `invariants.rs:143-232` bodies re-read, microsecond-cheap, real contracts (drift sign, label uniqueness, attribution); KEEP per census, I01 merged per G05 |
| F-3, F-4, F-9, F-10, F-11, F-12, F-13, F-14, F-15, F-16 | OUT OF SCOPE for T10 | touch `hellgate_e2e.rs` (T12), `loom_models.rs`/`faults/`/`fixtures/`/`fake/`/`alloc_tracking/`/`doctor/`/`lib.rs`/`tests/` (T11/T12); no verdict offered here |

## 4. Lens summary

- A (cheap-code): `target.rs:1-439` is adapter/mapping code with zero tests —
  correctly so (no branching logic beyond `map_class` range check at
  `target.rs:165-172`); no finding. `AccountingView::checked_*_sum`
  (`target.rs:64-78`) overflow-safe helpers are covered transitively by every
  suite run; no dedicated unit needed.
- B (cheap-test): the five invariants tests run in microseconds; four KEEP,
  one MERGE (lines-only). No deletion of cheap-but-valuable pins.
- C (slow-measured): one measured hotspot (S02, 4.973s) + two structural
  shrinks (S01 per-step views, per-test platform builds). Everything else in
  scope is timing-invisible.
- D (weak-oracle): strongest file in the workspace for oracles
  (independent models, literal V1 cost oracles at `suites.rs:120-126` and
  `suites.rs:303-307`, bite-proven fakes). One gap: S04–S12 lack the
  exhaustiveness tie S25 enjoys (G07).
- E (duplicates): 7 merges into named keepers (M01→M04, I01→I04, S22→M02,
  S27→S21, S29→S21, S32→S31, S34→S21+M05); cross-track dupes
  (`lib.rs`, `fixtures/`, `tests/`, hellgate) cited but left to T11/T12.

## Unresolved

- Per-test times for the 44 non-top-40 in-scope tests are not visible:
  `timing/full.txt` holds only the last 300 of 823 log lines, so absence
  from top40 bounds them only above (<0.2s-scale by rank-40 cutoff 0.225s,
  but exact values unconfirmed). Est. savings in G02/G03/G08 are
  engineering estimates, not measurements.
- Whether `#[test]` count (45) equals cargo's reported test count for the
  module was not executed (audit-only; no test run performed). The
  `proptest!`-nested S02 expands to one test per timing rank 2 — consistent.
- Cross-track keeper decisions (v1 F-6/F-8 `lib.rs` copies, F-9/F-10/F-11
  hellgate overlaps) are pointers for T11/T12, not verdicts here.
