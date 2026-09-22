# T05 deep audit — application + adapters + ports test fns

Scope: `crates/quanta-memory-platform-core/src/application/controller.rs`,
`scheduler.rs`, `src/adapters/` (all), `src/ports/` (all test fns).
No fix code — audit only.

Grep census (must match row count):

```
grep -rn -c -E '#\[test\]|#\[tokio::test\]' <scope files>
controller.rs:13  scheduler.rs:13  adapters/pin.rs:5  adapters/hdrhistogram.rs:4
adapters/metrics.rs:2  adapters/scratch.rs:0  adapters/mod.rs:0
ports/telemetry.rs:9  ports/scratch.rs:8  ports/{mod,pin,reclaim,store}.rs:0
TOTAL = 54
```

Timing: `timing/top40.md` present and read. No in-scope test appears in the
top-40 (top ranks are all redb/conformance/ledger/doc-tests). `timing/full.txt`
holds only the last 300 of 823 log lines, so in-scope unit timings are
unmeasured (all sub-ms, CPU-only, except the thread-spawning poison test and
the 10k-group build). Lens C verdicts below are therefore structural, not
measured — stated per finding.

## Census (54 rows)

Verdicts: KEEP / SHRINK (same fn, fewer/weaker-cost steps) / MERGE (delete fn,
fold unique lines into keeper) / DELETE (pure subsumption).

### controller.rs (13)

| line | test fn | verdict | reason |
|---|---|---|---|
| 251 | classification_is_deterministic_across_the_ladder | KEEP | unique just-below-boundary neighbors 740/899; v1 F-23 keeper |
| 264 | update_caches_current_pressure | MERGE → `ladder_transitions_up_then_down_through_every_rung` (:394) | 394 asserts update+cache at 7 points incl. Hard; only unique line is the initial-Normal readback — fold 1 line into 394 |
| 272 | ladder_gets_more_aggressive_with_pressure | KEEP | only `ladder_for` shape test (Soft vs Fatal+Spill) |
| 286 | reclaim_returns_zero_under_fatal_pressure_without_spill | KEEP | fail-closed zero leg of the fatal pair |
| 300 | reclaim_uses_spillable_under_fatal_pressure | KEEP | positive leg of the fatal pair (spillable rescues) |
| 313 | first_priority_owner_does_not_self_reclaim_below_fatal | KEEP | only priority-axis test; Normal-pair assert inside is the control, not dup |
| 326 | thresholds_new_accepts_monotonic_and_default_is_valid | SHRINK | 4 asserts: equal-rungs case + double Default check are belt-and-braces; keep new(75,90,98)+validate, drop one |
| 342 | thresholds_new_rejects_inverted_or_out_of_range | KEEP | 5-row deny table, strongest ctor oracle |
| 364 | zero_limit_is_normal_when_empty_and_fatal_when_used | KEEP | only degenerate-limit test; keeper for :379 merge |
| 379 | tiny_non_zero_limit_saturates_percent_without_wrapping | MERGE → :364 | same degenerate-limit axis (limit 1 vs limit 0); add `classify(1-limit)` section to :364, delete fn |
| 394 | ladder_transitions_up_then_down_through_every_rung | KEEP | strongest oracle in file (7 update+cache pairs, no hysteresis) |
| 419 | update_from_semantic_is_idempotent | SHRINK | cached-equality assert follows from pure function; keep first==second, drop cache-compare |
| 434 | allocator_snapshot_observation_does_not_change_pressure | KEEP | only advisory-seam test (semantic SSOT vs allocator) |

### scheduler.rs (13)

| line | test fn | verdict | reason |
|---|---|---|---|
| 299 | within_hard_cap_is_always_admitted | SHRINK (extend) | two interior asserts only; add exact-cap and share+1 boundary (v1 F-18) |
| 306 | unknown_group_is_denied_fail_closed | KEEP | only state-growth check (`usage.len()==0`); error-variant asserts unique |
| 316 | noisy_group_cannot_borrow_beyond_weighted_share | KEEP | strongest starvation oracle (300/301 + bg-not-starved legs) |
| 337 | release_returns_burst_capacity | DELETE → :429 | subsumed by :429 which adds exact-usage restore + over-release fail-closed (v1 F-27) |
| 346 | prospective_floor_aggregate_matches_committed_growth_and_release | KEEP | only floor-aggregate preview-vs-commit test |
| 374 | empty_scheduler_has_zero_share_and_denies_all | MERGE → :387 | zero-share assert is the only unique line; move `fair_burst_share==0` into :387, delete fn |
| 387 | unknown_group_denied_even_with_pool_available | KEEP | keeper: registered-admitted + unregistered-denied with pool headroom |
| 398 | weighted_share_distributes_remainder_work_conservingly | KEEP | only small-scale Hamilton-remainder proof (134,133,133) |
| 415 | burst_percent_changes_admission_outcome | KEEP | only burst_percent axis test |
| 429 | usage_release_round_trip_readmits_burst | KEEP | keeper of F-27 (round-trip + over-release fail-closed :440-442) |
| 446 | huge_pool_and_weights_compute_a_non_wrapping_fair_share | KEEP | only u64::MAX exact-share test |
| 464 | admission_reuses_policy_static_fair_share_table_at_high_group_cardinality | SHRINK | 10_000 groups + map clone + 2000 admits; 100 groups prove "no rebuild" identically (v1 F-12) |
| 486 | repeated_exact_usage_release_round_trip_never_wraps | SHRINK | 8 identical MAX-charge/release rounds; 2 rounds prove never-wraps |

### adapters/pin.rs (5)

| line | test fn | verdict | reason |
|---|---|---|---|
| 87 | pin_invalidate_round_trip_tracks_dependent_count | KEEP | only full lifecycle (0→1→2→0→1) with count at each step |
| 117 | invalidation_outcome_reports_exact_dependents | KEEP | only pin-order + cross-payload-isolation + empty-outcome asserts |
| 143 | duplicate_pin_does_not_over_count_dependent | KEEP | only dedup/idempotency axis |
| 166 | distinct_dependents_each_increment_count | KEEP | 12-line count>2 fan-out pin (v1 F-28, keep both) |
| 182 | poisoned_lock_is_fail_closed_not_panicking | KEEP | only poison-path test; thread-spawn cost is the price of the axis |

### adapters/hdrhistogram.rs (4, telemetry-gated)

| line | test fn | verdict | reason |
|---|---|---|---|
| 90 | records_deterministic_set_and_reads_back_count_and_percentiles | SHRINK | tighten p50/p99 to exact per v1 F-16; absorb one duplicate-value record from :118 |
| 118 | record_is_monotonic_and_lossless_in_count | MERGE → :90 | :90 already asserts len 0→100 + empty transitions; only unique bit is duplicate-value counting — one line in :90 |
| 129 | invalid_bounds_return_error_not_panic | KEEP | only ctor-rejection test |
| 140 | out_of_range_record_is_rejected_and_does_not_mutate_count | KEEP | only rejected-record-leaves-count check |

### adapters/metrics.rs (2)

| line | test fn | verdict | reason |
|---|---|---|---|
| 47 | emitter_surface_is_emit_only_and_takes_shared_ref | KEEP (+ comment fix) | keeper; weakest oracle in scope by structure — admit it pins the signature (v1 F-15) |
| 69 | emitter_holds_no_accounting_state | MERGE → :47 | same signature-only shape (clone + shared-ref emit); one combined emit-only test suffices |

### ports/telemetry.rs (9)

| line | test fn | verdict | reason |
|---|---|---|---|
| 200 | operational_labels_are_closed_bounded_sets | SHRINK | `.map(label).len()==13/11` is a tautology (array len == itself); keep non-empty check, drop count asserts |
| 236 | advisory_flags_mismatch_and_reports_semantic_as_authority | DELETE → :273 | row 1 of the :273 table (2000v1000) (v1 F-21) |
| 246 | advisory_agreement_has_no_mismatch | DELETE → :273 | equality rows (1000,1000)/(0,0) + drift Some(0) all inside :273 table |
| 255 | missing_allocator_data_is_not_treated_as_mismatch | MERGE → :358 | None-resident axis fully subsumed by BlindTelemetry :358 (mismatch+drift+semantic); :273 table has no None row so :358 is the keeper |
| 263 | static_telemetry_returns_its_snapshot_verbatim | KEEP | only snapshot-verbatim test |
| 273 | compare_reports_signed_drift_and_mismatch_boundary | KEEP | keeper table (7 rows, both drift directions + boundary) |
| 315 | compare_semantic_bytes_is_ssot_not_allocator | KEEP | only u64::MAX drift-overflow (i128 headroom) test |
| 335 | allocator_telemetry_trait_surface_is_observation_only | KEEP | only generic-trait-contract + idempotency proof |
| 358 | trait_path_absent_allocator_data_never_contradicts_ssot | KEEP | keeper of the None-resident axis |

### ports/scratch.rs (8)

| line | test fn | verdict | reason |
|---|---|---|---|
| 198 | reservation_maps_to_live_scratch_charge_kind | KEEP | only charge_kind mapping + fixture baseline |
| 214 | grow_to_observed_updates_within_reserved_ceiling | KEEP | keeper for :280 merge (grow axis) |
| 226 | grow_beyond_reserved_is_fail_closed_capacity_error | KEEP | only Capacity axis + observed-unchanged |
| 243 | grow_after_release_is_rejected_and_fail_closed | KEEP | only post-release InvalidState axis |
| 264 | double_release_is_rejected_not_silent | KEEP | only double-release axis |
| 280 | lifecycle_reserve_grow_release_succeeds_in_order | MERGE → :214 | every step covered by :214 (grow) + :264 (release); append 2 release lines to :214 |
| 305 | failed_release_preserves_authority_for_retry | KEEP | only authority-failure/retry path |
| 323 | successful_release_prevents_drop_from_releasing_twice | KEEP | only Drop-guard test |

Row count 13+13+5+4+2+9+8 = **54**, equals the grep total above.

## Findings (lens prefix; no fix code)

### D-1 (v1 F-15 CONFIRM) — metrics emitter tests assert nothing observable
- `crates/quanta-memory-platform-core/src/adapters/metrics.rs:57`
- Oracle: `assert_eq!(returned, (), "emit is observation-only: it yields nothing to fold back");`
- Mechanism: no metrics facade is wired in tests, so `emit` is called for its
  `()` return and cloneability only; a broken label mapping would still pass.
- Action: MERGE :69 into :47 (keeper) + add comment admitting the survivor pins
  the `&self → ()` signature, not emission; optional recording-sink follow-up
  mirroring `control.rs:3920`. Est. saving: ~15 lines.

### D-2 (v1 F-16 CONFIRM) — loose HDR percentile windows hide off-by-one
- `crates/quanta-memory-platform-core/src/adapters/hdrhistogram.rs:103`
- Oracle: `assert!((49..=51).contains(&p50), "p50 of 1..=100 should be ~50, got {p50}");`
- Mechanism: deterministic 1..=100 set through a 3-sigfig histogram has an exact
  median; the ±1 window passes a fencepost regression.
- Action: tighten to `== 50` / `== 99` in :90 (keeper `hdrhistogram.rs:90`).
  Est. saving: 0 lines (oracle strength, not size).

### D-3 (new) — label-count asserts are tautological
- `crates/quanta-memory-platform-core/src/ports/telemetry.rs:229`
- Oracle: `assert_eq!(operations.map(MemoryOperation::label).len(), 13);`
- Mechanism: mapping an N-array always yields length N; the assert re-states the
  literal. Only the non-empty check on :231 carries signal.
- Action: SHRINK `telemetry.rs:200` — drop the two count asserts, keep
  non-empty; a new variant without a label still passes either way (accepted).
  Est. saving: 2 lines.

### E-1 (v1 F-27 CONFIRM) — scheduler burst round-trip overlap
- `crates/quanta-memory-platform-core/src/application/scheduler.rs:337`
- Oracle: `assert!(!s.admits("fg", 1), "no more burst available");`
- Mechanism: `release_returns_burst_capacity` (:337) is a strict prefix of
  `usage_release_round_trip_readmits_burst` (:429), which adds exact-usage
  restoration (:437) and over-release fail-closed (:441-442).
- Action: DELETE :337, keeper `scheduler.rs:429`. Est. saving: ~9 lines.

### E-2 (v1 F-21 CONFIRM) — telemetry single-case subsumed by table
- `crates/quanta-memory-platform-core/src/ports/telemetry.rs:240`
- Oracle: `assert!(advisory.mismatch, "2000 allocator vs 1000 semantic is a mismatch");`
- Mechanism: exact row 1 `(2_000, 1_000, 1_000, true)` of the :273 table.
- Action: DELETE :236, keeper `telemetry.rs:273`. Est. saving: ~9 lines.

### E-3 (new, extends F-21) — agreement single-case also inside the table
- `crates/quanta-memory-platform-core/src/ports/telemetry.rs:250`
- Oracle: `assert!(!advisory.mismatch);`
- Mechanism: :273 covers equality from both sides plus the exact crossing
  (`(1_000,1_000,0,false)`, `(0,0,0,false)`) and `drift==Some(0)`; :246 adds no row.
- Action: DELETE :246, keeper `telemetry.rs:273`. Est. saving: ~8 lines.

### E-4 (new) — missing-data direct test subsumed via trait path
- `crates/quanta-memory-platform-core/src/ports/scratch.rs:197` context — telemetry file:
  `crates/quanta-memory-platform-core/src/ports/telemetry.rs:258`
- Oracle: `assert!(!advisory.mismatch, "absent allocator data cannot contradict semantic SSOT");`
- Mechanism: `trait_path_absent_allocator_data_never_contradicts_ssot` (:358)
  asserts mismatch + drift-None + semantic through a stricter seam
  (custom `BlindTelemetry` impl); the :273 table has no None-resident row, so
  :358 — not the table — is the keeper.
- Action: MERGE :255 into `telemetry.rs:358`, delete :255. Est. saving: ~7 lines.

### E-5 (new) — HDR monotonic-count test folds into the deterministic set
- `crates/quanta-memory-platform-core/src/adapters/hdrhistogram.rs:122`
- Oracle: `assert_eq!(rec.len(), expected_len, "each record bumps the observed count by one");`
- Mechanism: :90 already walks len 0→100 with empty/non-empty transitions;
  :118's only unique bit is duplicate-value counting (`(2, 5)` records 5 twice).
- Action: MERGE :118 into `hdrhistogram.rs:90` (record one duplicate value),
  delete :118. Est. saving: ~8 lines.

### E-6 (new, extends F-15) — metrics clone test merges into emit-only test
- `crates/quanta-memory-platform-core/src/adapters/metrics.rs:74`
- Oracle: `emitter.emit(MemoryEvent::new(` (shared-ref emit, no observable assert)
- Mechanism: same signature-only shape as :47; cloneability + an extra emit fit
  in the keeper's body.
- Action: MERGE :69 into `metrics.rs:47`. Est. saving: ~15 lines (with D-1).

### E-7 (new) — scratch happy-path lifecycle merges into grow test
- `crates/quanta-memory-platform-core/src/ports/scratch.rs:284`
- Oracle: `res.grow_to_observed(1024).expect("grow while live");`
- Mechanism: grow leg covered by :214, release leg by :264; the sequencing
  value is kept by appending 2 release lines to :214.
- Action: MERGE :280 into `scratch.rs:214`, delete :280. Est. saving: ~9 lines.

### E-8 (new) — tiny-limit test merges into zero-limit test
- `crates/quanta-memory-platform-core/src/application/controller.rs:382`
- Oracle: `assert_eq!(c.classify_semantic(1), PressureState::Fatal, "1/1 is already fatal");`
- Mechanism: same degenerate-limit axis as :364 (limit 0 vs limit 1); the
  u128-saturation comment applies to both.
- Action: MERGE :379 into `controller.rs:364` (second section), delete :379.
  Est. saving: ~11 lines.

### E-9 (new) — empty-scheduler test merges into pool-available denial test
- `crates/quanta-memory-platform-core/src/application/scheduler.rs:381`
- Oracle: `assert_eq!(s.fair_burst_share("fg"), 0, "unknown group => zero share");`
- Mechanism: unknown-group denial is pinned twice (:374, :387); the zero-share
  no-divide-by-zero assert is :374's only unique line.
- Action: MERGE :374 into `scheduler.rs:387`, delete :374. Est. saving: ~10 lines.

### C-1 (v1 F-12 CONFIRM) — 10k-group scheduler test, structural cost
- `crates/quanta-memory-platform-core/src/application/scheduler.rs:470`
- Oracle: `assert_eq!(scheduler.fair_burst_shares.len(), 10_000);`
- Mechanism: builds 10_000 groups, clones the 10k-entry share map (:473), runs
  2000 admits (:475-478). The Hamilton property is pinned by 3-group :398; the
  "no per-admission rebuild" assert needs only ~100 groups. Not in top-40 and
  absent from the 300-line `full.txt` tail, so no measured time — structural.
- Action: SHRINK `scheduler.rs:464` to 100 groups / 20 admits. Est. saving:
  ~99% of this test's build+clone cost.

### C-2 (new) — 8× MAX round-trip loop + poison-thread spawn
- `crates/quanta-memory-platform-core/src/application/scheduler.rs:488`
- Oracle: `assert_eq!(s.record_usage("fg", u64::MAX).expect("first charge"), u64::MAX);`
- Mechanism: 8 identical MAX-charge/overflow/release rounds; rounds 3-8 re-prove
  the same checked-arithmetic path. Separately, `pin.rs:182` spawns a thread
  purely to poison a mutex — necessary for its axis, just the costliest shape
  in an otherwise sub-ms scope (neither in top-40; unmeasured in `full.txt` tail).
- Action: SHRINK :486 to 2 rounds; keep `pin.rs:182` as-is (KEEP). Est. saving:
  ~75% of :486's iterations.

### B-1 (v1 F-07 CONFIRM) — validate precedence + initial-state gaps still open
- `crates/quanta-memory-platform-core/src/application/controller.rs:67`
- Oracle (gap): `controller.rs:342` feeds one violation class per case — never
  both (e.g. `(200, 50, 50)`), so `>100`-before-monotonicity precedence is
  unpinned; no test reads back the `new` `initial` param (all 13 use Normal).
- Mechanism: cheap-code gap, not dup — needs a new 3-line test, not a deletion.
- Action: add `(200, 50, 50)` row to :342 + one `current_pressure()==initial`
  assert at construction. Est. cost: +5 lines.

### B-2 (v1 F-05 CONFIRM) — `adapters/scratch.rs` still has zero direct tests
- `crates/quanta-memory-platform-core/src/adapters/scratch.rs:40`
- Mechanism: `BrokerBoundScratchAllocator` is pinned only indirectly via
  broker-level tests; the foreign-runtime guard (:46-51) has no adapter-seam test.
- Action: add 3-case direct test (mismatched runtime, delegation, drop-release)
  against a stub authority. Est. cost: +~40 lines (new coverage, not saving).

## v1 re-verification (in-scope findings from `02-core-api-application.md`)

| v1 | status | note |
|---|---|---|
| F-05 adapters/scratch zero tests | CONFIRM | grep still 0 (`adapters/scratch.rs:0`) |
| F-07 precedence + initial state | CONFIRM | no dual-violation row; no initial readback (all bodies re-opened) |
| F-12 10k groups | CONFIRM | body unchanged; see C-1 |
| F-15 metrics weak oracle | CONFIRM | bodies unchanged; see D-1/E-6 |
| F-16 HDR ±1 windows | CONFIRM | `49..=51` / `98..=100` still at :103-108; see D-2 |
| F-18 thin happy-path asserts | CONFIRM | :299-303 still two interior points; census SHRINK |
| F-21 telemetry row-1 dup | CONFIRM | see E-2 |
| F-23 ladder overlap keep-both | CONFIRM | :251 neighbors 740/899 not in :393; keep both, no action |
| F-24 charge_kind 3× | CONFIRM | :197, :220 (`still live scratch`), :282; harmless one-liners, no churn |
| F-27 burst round-trip overlap | CONFIRM | see E-1 |
| F-28 pin 2-vs-3 keep-both | CONFIRM | :166 is 12 lines pinning count>2 fan-out; keep both, no action |

All other v1 findings (F-01–F-04, F-06, F-08–F-11, F-13, F-14, F-17, F-19,
F-20, F-22, F-25, F-26; all of 01/03/04/05) touch files outside this track's
scope — not re-verified here.

## Quick wins (top 5)

1. Delete 3 subsumed fns (:337→:429, :236→:273, :246→:273) — pure deletion,
   ~26 lines (E-1, E-2, E-3).
2. Merge 6 micro-dups into keepers (:118→:90, :69→:47, :280→:214, :379→:364,
   :374→:387, :255→:358) — ~60 lines (E-4–E-9, D-1).
3. Shrink the 10k-group test to 100 groups (C-1) — largest structural cost in scope.
4. Tighten HDR + label oracles (D-2, D-3) — zero-line strength gains.
5. Close F-07 with a 5-line addition (B-1) — only item that adds lines.
