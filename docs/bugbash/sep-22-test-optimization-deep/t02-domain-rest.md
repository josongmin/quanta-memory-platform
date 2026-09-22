# T02 — domain rest deep audit (policy, config, context, stats, recharge, errors + policy_contracts)

Scope files (workspace-relative under `crates/quanta-memory-platform-core/`):

- `src/domain/policy.rs` (690 lines)
- `src/domain/config.rs` (733 lines)
- `src/domain/context.rs` (32 lines, zero tests)
- `src/domain/stats.rs` (103 lines, zero tests)
- `src/domain/recharge.rs` (15 lines, zero tests)
- `src/domain/errors.rs` (76 lines, zero tests)
- `tests/policy_contracts.rs` (441 lines)

Grep census command:

```
grep -rn -A1 '#\[test\]\|#\[tokio::test\]' \
  crates/quanta-memory-platform-core/src/domain/policy.rs \
  crates/quanta-memory-platform-core/src/domain/config.rs \
  crates/quanta-memory-platform-core/src/domain/context.rs \
  crates/quanta-memory-platform-core/src/domain/stats.rs \
  crates/quanta-memory-platform-core/src/domain/recharge.rs \
  crates/quanta-memory-platform-core/src/domain/errors.rs \
  crates/quanta-memory-platform-core/tests/policy_contracts.rs
```

Grep count: `policy.rs` 14 + `config.rs` 21 + `context.rs` 0 + `stats.rs` 0
+ `recharge.rs` 0 + `errors.rs` 0 + `policy_contracts.rs` 14 = **49**.
Census rows below: 49. No `#[tokio::test]` in scope (all plain `#[test]`).

Timing: `timing/top40.md` is present. **No in-scope test appears in the top-40**
(all ranks are spill/redb, conformance, retained_map, or doc-test compile-fail
entries). `timing/full.txt` in this repo state holds only the last ~300 log
lines (facade binaries), so per-test times for the 49 in-scope tests are not
visible in the committed log; lens C verdicts below are therefore
code-cost reasoning, not measured times. See Unresolved.

## Census (49 rows)

### policy.rs — execution_budget_tests (2)

| # | Test (file:line) | Verdict | Reason |
|---|------------------|---------|--------|
| P1 | `check_helpers_admit_within_cap_and_reject_over_cap` (policy.rs:95) | KEEP | Only multi-axis `check_*` coverage; failure names no axis (see D1) but coverage is load-bearing |
| P2 | `unlimited_budget_admits_everything` (policy.rs:115) | MERGE | Same 5-axis matrix as P1 with `unlimited()`; fold both into one per-axis table (see D1) |

### policy.rs — preset / residency tests (12)

| # | Test (file:line) | Verdict | Reason |
|---|------------------|---------|--------|
| P3 | `for_preset_populates_every_mandatory_class` (policy.rs:470) | KEEP | Sole totality assertion at unit level; also absorbs contracts-totality loop (see E2) |
| P4 | `for_preset_is_structurally_valid` (policy.rs:488) | MERGE | Validates all 4 presets; merge with `every_preset_…_is_total…` (policy_contracts.rs:38) into one preset loop — one 4-preset construction saved |
| P5 | `for_preset_class_caps_isolate_below_global` (policy.rs:497) | KEEP | Only test pinning per-class `reserved<=soft<=hard` + burst-pool + headroom existence per preset |
| P6 | `for_preset_scales_caps_with_global_limit` (policy.rs:524) | KEEP | Claim samples 1 of 13 classes (see D3); extend loop in place, no new test |
| P7 | `for_preset_weighted_split_never_over_allocates_distributable` (policy.rs:537) | KEEP | Unique sum-band property; distinct from P8/P9 (see E1) |
| P8 | `for_preset_headroom_class_owns_exactly_the_reserved_band` (policy.rs:560) | KEEP | Unique exact-equality property (headroom cap == band + reconstruction) |
| P9 | `for_preset_no_single_class_cap_plus_headroom_exceeds_global` (policy.rs:581) | KEEP | Unique max-class + headroom property; keep all three band tests (P7/P8/P9 test different predicates) |
| P10 | `default_residency_global_one_does_not_underflow_and_validates` (policy.rs:605) | MERGE | Fold its extra per-class asserts into the P11 sweep body (which already visits global=1); keeps the asserts, drops the second construction |
| P11 | `default_residency_small_globals_all_validate` (policy.rs:626) | KEEP | Sweep keeper for degenerate globals after absorbing P10's asserts |
| P12 | `validate_rejects_constructed_headroom_above_global` (policy.rs:643) | MERGE | Same floor mechanism as C6 (`config.rs:466`); keeper is C6 (adds class-reserved term). Delete here |
| P13 | `validate_accepts_headroom_equal_to_global` (policy.rs:655) | KEEP | Sole exact-floor boundary pin (`==` accepted, `>` rejected); 3 lines, no saving worth chasing |
| P14 | `borrow_flag_with_zero_shared_pool_is_valid_but_grants_no_burst` (policy.rs:663) | SHRINK | Last 6 lines are tautological (see D2); keep validate + flag asserts, delete `burstable` tail (or drive a real reserve) |

### config.rs — resolver / provenance (3)

| # | Test (file:line) | Verdict | Reason |
|---|------------------|---------|--------|
| C1 | `higher_precedence_layer_wins_and_is_recorded_in_provenance` (config.rs:367) | KEEP | Sole precedence-order + provenance-order pin; keeper for the pair |
| C2 | `removing_top_layer_changes_effective_value` (config.rs:408) | MERGE | Negative pair of C1 over the same resolver; fold into C1 as a second resolve block (~20 lines saved) |
| C3 | `empty_layer_is_skipped_in_provenance` (config.rs:438) | KEEP | Sole empty-layer skip pin; strengthen with a resolved-value assert (see D4), no row change |

### config.rs — residency validation (8)

| # | Test (file:line) | Verdict | Reason |
|---|------------------|---------|--------|
| C4 | `validate_rejects_reserved_above_soft` (config.rs:451) | KEEP | Sole class `reserved>soft` rejection pin |
| C5 | `validate_rejects_global_below_reserved_plus_headroom_floor` (config.rs:466) | KEEP | Keeper for the floor mechanism; absorbs P12 |
| C6 | `validate_rejects_class_hard_cap_above_global` (config.rs:485) | KEEP | Sole class `hard>global` rejection pin |
| C7 | `validate_rejects_owner_hard_cap_above_global` (config.rs:500) | KEEP | Sole owner-quota `hard>global` rejection pin |
| C8 | `validate_accepts_well_formed_policy` (config.rs:509) | KEEP | Sole class-level accept pin; strengthen with a floor-value assert (see D4) |
| C9 | `validate_rejects_missing_each_mandatory_class` (config.rs:526) | KEEP | Sole per-class totality rejection loop (14 classes); load-bearing |
| C10 | `validated_policy_materializes_no_implicit_class_holes` (config.rs:540) | MERGE | Overlaps P3/P4 + contracts-totality; fold its `try_from` materialization assert into P3's loop (P3 iterates the same mandatory set) |
| C11 | `reserved_floor_checked_add_overflow_is_config_error` (config.rs:553) | KEEP | Sole `u64::MAX` overflow-floor pin; cannot merge (unique trap path) |

### config.rs — resource-group / owner-group validation (10)

| # | Test (file:line) | Verdict | Reason |
|---|------------------|---------|--------|
| C12 | `validate_accepts_well_formed_resource_group` (config.rs:594) | KEEP | Baseline accept pin + `good_group()` keeper fixture; absorbs nothing |
| C13 | `validate_rejects_resource_group_reserved_above_soft` (config.rs:604) | MERGE | One-field mutation of `good_group()`; fold C13/C14/C15/C16 into one table-driven rejection test over field-mutator closures (see E3) |
| C14 | `validate_rejects_resource_group_soft_above_hard` (config.rs:615) | MERGE | Same table as C13 |
| C15 | `validate_rejects_resource_group_hard_above_global` (config.rs:626) | MERGE | Same table as C13 |
| C16 | `validate_rejects_resource_group_burst_percent_over_100` (config.rs:638) | MERGE | Same table as C13 |
| C17 | `validate_accepts_resource_group_burst_percent_boundaries` (config.rs:649) | KEEP | Sole inclusive-boundary (0/100) pin; pairs with the merged rejection table |
| C18 | `resource_group_reserved_has_an_independent_global_floor` (config.rs:663) | KEEP | Sole pin of the independent-axes semantic (group reserved NOT summed with class reserved); documents a decision, not a dup |
| C19 | `validate_rejects_group_reserved_plus_headroom_above_global` (config.rs:680) | KEEP | Sole multi-group floor-sum rejection; distinct from C5 (class axis) |
| C20 | `validate_rejects_owner_group_mapping_to_unknown_group` (config.rs:710) | MERGE | Positive/negative pair with C21 over one builder; fold into a single test with two resolves |
| C21 | `validate_accepts_owner_group_mapping_to_declared_group` (config.rs:717) | KEEP | Pair keeper with C20 absorbed |

### policy_contracts.rs — unit-level contracts (6)

| # | Test (file:line) | Verdict | Reason |
|---|------------------|---------|--------|
| I1 | `execution_budget_default_matches_runtime_budget_truth` (policy_contracts.rs:10) | KEEP | Public-surface pin of default cap values; cheap, guards host-visible truth |
| I2 | `memory_class_taxonomy_keeps_mandatory_and_future_classes_split` (policy_contracts.rs:21) | KEEP | Sole taxonomy-count + disjointness pin (14/4); cheap |
| I3 | `every_preset_and_custom_validated_policy_is_total_over_mandatory_classes` (policy_contracts.rs:38) | MERGE | Third copy of the preset-totality loop (with P3/P4/C10); fold into P3 |
| I4 | `residency_policy_tracks_class_quota_and_resource_group_vocabulary` (policy_contracts.rs:59) | SHRINK | Field-echo asserts (see D5); keep builder-vocabulary wiring, drop value round-trip lines |
| I5 | `builder_emits_effective_budget_report_with_override_provenance` (policy_contracts.rs:100) | KEEP | Sole `build_report_only` path pin; only indirect cover of `from_resolved` (see D6) |
| I6 | `memory_error_has_typed_config_and_report_variants` (policy_contracts.rs:115) | KEEP | Expand in place to all 9 variants (see D7); no new test file |

### policy_contracts.rs — live_control boundary contracts (8, `#[cfg(feature = "live_control")]`)

| # | Test (file:line) | Verdict | Reason |
|---|------------------|---------|--------|
| L1 | `global_hard_limit_denies_reserve_through_public_broker_and_is_fail_closed` (policy_contracts.rs:172) | KEEP | Behavioral keeper of the global-limit pair (public wiring + fail-closed snapshots); ledger unit is the stronger-oracle side — keep both (see E4) |
| L2 | `headroom_band_is_reserved_for_headroom_class_at_public_surface` (policy_contracts.rs:208) | KEEP | Same: behavioral keeper |
| L3 | `shared_burst_pool_bounds_borrowing_through_public_broker` (policy_contracts.rs:241) | KEEP | Same: behavioral keeper; cap+borrow+overdraw triple is unique |
| L4 | `non_borrowing_class_cannot_use_burst_pool_at_public_surface` (policy_contracts.rs:285) | KEEP | Same: behavioral keeper; sole `borrow=false` denial at the public surface |
| L5 | `managed_charge_double_release_is_rejected_without_counter_drift` (policy_contracts.rs:310) | KEEP | Sole double-release InvalidState pin at public surface |
| L6 | `dropping_pending_public_authority_rolls_reservation_back` (policy_contracts.rs:346) | KEEP | Sole pending-authority Drop-refund pin |
| L7 | `preset_class_caps_bite_at_public_reserve_surface` (policy_contracts.rs:367) | KEEP | Sole per-preset behavioral isolation loop (exact-cap admit + cap+1 deny × 4 presets) |
| L8 | `preset_broker_global_limit_is_consistent_with_enforcement` (policy_contracts.rs:416) | KEEP | Sole report-vs-enforcement consistency pin (loops all 14 mandatory classes) |

Zero-test files (deliberate, verified by grep — no `#[test]`, no `mod tests`):
`context.rs` (3-type vocabulary + trivial constructor), `stats.rs`
(report/observation structs, no logic tests), `recharge.rs` (2 private types),
`errors.rs` (constructors + Display; Display gap noted in D7).

## Findings

### D1 — [B cheap-test, E duplicates] P1+P2 cover 5 axes in 2 bodies with no axis attribution

- Location: `crates/quanta-memory-platform-core/src/domain/policy.rs:95` (P1 body
  lines 95–112) and `:115` (P2 body 115–122).
- Oracle (quoted):
  ```
  budget.check_output_bytes(50 * 1024 * 1024).expect("under output cap");
  budget.check_subqueries(1_000).expect("exactly at subquery cap is allowed");
  ```
  plus `budget.check_output_bytes(usize::MAX).expect("unlimited output");`
  (policy.rs:117).
- Mechanism: P1 asserts 5 axes (`output`, `subqueries`, `wall_time`, `trace_edges`,
  `scratch`) in one body — a failure reports only the line, and P2 repeats the
  same 5-axis matrix for `unlimited()`. The `as u64` casts in
  `check_subqueries`/`check_trace_edges` (policy.rs:67,72) also hide a 32-bit
  truncation risk that a single `usize::MAX` probe cannot attribute per axis.
- Action: MERGE P1+P2 into one per-axis table: for each of the 5 axes,
  `(at-cap admits, over-cap denies as Capacity, unlimited admits MAX)`.
  Keeper: `policy.rs:51` (`fn check`, the canonical helper under test).
- Est. saving: ~10 lines net; main gain is failure attribution, not time
  (both tests are sub-millisecond).

### D2 — [B cheap-test, D weak-oracle] P14's `burstable` tail proves nothing

- Location: `crates/quanta-memory-platform-core/src/domain/policy.rs:687`.
- Oracle (quoted):
  ```
  let burstable = r.shared_burst_pool_bytes;
  assert_eq!(burstable, 0, "an empty shared pool grants no burst capacity");
  ```
- Mechanism: reads back the field the test itself set via
  `.with_shared_burst_pool_bytes(0)` (policy.rs:676). Tautological: any struct
  echo passes. The real claim in the comment ("can never actually
  oversubscribe — effective ceiling stays its hard cap") is never exercised
  with a reserve, because unit scope has no ledger.
- Action: SHRINK — delete lines 685–688, or (stronger) convert to a ledger
  reserve-over-cap denial. Keeper for the valid-policy half:
  `policy.rs:679` (`validate_residency(&r).expect(...)`).
- Est. saving: 6 lines; converts a vacuous test into a real one.

### D3 — [E duplicates] P6 samples 1 of 13 non-headroom classes for a universal claim

- Location: `crates/quanta-memory-platform-core/src/domain/policy.rs:529`.
- Oracle (quoted):
  ```
  let dev_cap = dev.classes[&MemoryClass::ArtifactCache].hard_cap_bytes;
  ```
- Mechanism: comment claims "the split is a deterministic function of the
  global limit" but only `ArtifactCache` is compared dev-vs-prod. A regression
  skewing any other class passes silently. Sibling tests (P5, P7–P9) already
  loop all classes — this one is the outlier.
- Action: extend the loop in place (`for class in mandatory, skip Headroom:
  prod >= dev`). Keeper: `policy.rs:524` itself.
- Est. saving: none (adds ~4 lines); closes a 12/13-class hole for free
  (sub-millisecond loop).

### D4 — [D weak-oracle] accept-path tests assert only `.is_ok()` / absence

- Locations: `crates/quanta-memory-platform-core/src/domain/config.rs:509`
  (C8), `:594` (C12), `:649` (C17), `:438` (C3), `:717` (C21).
- Oracle (quoted):
  ```
  assert!(validate_residency(&residency).is_ok());
  ```
  (config.rs:522; identical shape at 596–599, 653–657, 731) and
  ```
  !resolved.provenance.iter().any(|r| r.source == OverrideSource::Cli),
  ```
  (config.rs:444).
- Mechanism: a validator returning `Ok(())` unconditionally passes C8/C12/C17/C21;
  C3 never checks the resolved value equals the manifest default, so a resolver
  that drops the manifest base also passes. Five accept tests, zero value pins.
- Action: add one value assert per test: C8 — floor math
  (`reserved + headroom <= global` recomputed); C12 — `hard<=global` echo;
  C3 — `resolved.residency.global_hard_limit_bytes == 4 GiB` (the
  `manifest()` base at config.rs:361); C21 — group mapping recorded.
  Keeper values live at `config.rs:353` (`manifest()`) and `config.rs:577`
  (`good_group()`).
- Est. saving: none (adds lines); closes the "unconditionally-Ok passes" hole.

### D5 — [B cheap-test, D weak-oracle] I4 echoes builder inputs back

- Location: `crates/quanta-memory-platform-core/tests/policy_contracts.rs:90`.
- Oracle (quoted):
  ```
  assert_eq!(policy.global_hard_limit_bytes, 512 * 1024 * 1024);
  assert_eq!(policy.headroom_bytes, 64 * 1024 * 1024);
  ```
- Mechanism: every assert reads back a literal the test just wrote
  (lines 60–88). commodity struct-echo: fails only if the builder forgets a
  field assignment — i.e. it tests the test's own literals. The load-bearing
  half (vocabulary wiring: class/owner/group maps accept entries) is 3 lines
  (94–96).
- Action: SHRINK to the wiring asserts (94–96) + one `validate_residency`
  call proving the composed policy is coherent. Keeper:
  `policy_contracts.rs:94` (`classes.contains_key` block).
- Est. saving: ~8 lines; negligible time (microseconds).

### D6 — [D weak-oracle, A cheap-code] `stats.rs::from_resolved` dedup + default asymmetry untested

- Location: `crates/quanta-memory-platform-core/src/domain/stats.rs:54`
  (O(n²) `contains`-dedup) and `:37` vs `:61` (`"manifest"` vs `"preset"`
  empty-provenance default).
- Oracle (quoted):
  ```
  if !override_sources.contains(&label) {
  ```
  (stats.rs:56) and
  ```
  override_sources.last().cloned().unwrap_or_else(|| "manifest".to_string());
  ```
  (stats.rs:37) vs `"preset"` at stats.rs:61.
- Mechanism: `from_resolved` has zero direct tests; only indirect cover via I5
  (`policy_contracts.rs:100`), which builds via `build_report_only` and never
  touches `from_resolved`. A dedup-order regression or a default-label swap
  passes the whole scope. The `"manifest"`/`"preset"` asymmetry looks
  intentional (bare-manifest vs resolved paths) but no test pins either value.
- Action: add 2 unit tests in `stats.rs` (new `#[cfg(test)] mod`, file has none):
  multi-field provenance dedup ordering, and empty-provenance default source
  for both constructors. Keeper: `stats.rs:52` (`from_resolved`).
- Est. saving: negative (new lines); covers real logic (the only production
  code in scope with zero tests).

### D7 — [D weak-oracle] `MemoryError` Display covered for 2 of 9 variants

- Location: `crates/quanta-memory-platform-core/tests/policy_contracts.rs:115`.
- Oracle (quoted):
  ```
  assert_eq!(MemoryError::config("bad preset").to_string(), "config error: bad preset");
  ```
- Mechanism: covers `Config` + `Report` Display only (`errors.rs:12-26` lists 9
  variants). `Capacity`, `Closed`, `OperationInProgress`, `Poisoned`,
  `InvalidState`, `Unsupported`, `Build` Display arms have no assertion
  anywhere in scope (grep-clean for `to_string()` on those variants).
- Action: table-driven test over all 9 variants asserting prefix strings,
  in place at `policy_contracts.rs:115`. Keeper: `errors.rs:58`
  (`impl Display for MemoryError`).
- Est. saving: adds ~15 lines; covers 7 untested Display arms.

### E1 — [E duplicates] band-arithmetic trio P7/P8/P9: keep all three, shared fixture

- Locations: `crates/quanta-memory-platform-core/src/domain/policy.rs:537`,
  `:560`, `:581`.
- Mechanism: all three iterate `presets()` and assert global/headroom
  arithmetic, but the predicates differ (sum≤distributable, exact headroom
  equality + reconstruction, max-class + headroom ≤ global). Folding them loses
  a distinct predicate per fold. Real cost is 3× `for_preset` construction
  (~microseconds each — presets are pure arithmetic, no I/O).
- Action: KEEP all; optionally hoist `for preset in presets()` + residency
  construction into one helper returning `[ResidencyPolicy; 4]`.
  Keeper: the three bodies themselves.
- Est. saving: ~6 lines if hoisted; no time saving worth claiming.

### E2 — [E duplicates] preset-totality asserted 3× (P3, P4-adjacent, I3, C10)

- Locations: `crates/quanta-memory-platform-core/src/domain/policy.rs:470`
  (P3), `policy_contracts.rs:38` (I3), `config.rs:540` (C10).
- Mechanism: P3 (mandatory-set equality per preset), I3 (validated-totality per
  preset + custom), C10 (`try_from` materialization, custom only) overlap on
  "every preset is total over mandatory classes". I3's only unique content is
  the 5th element (`ResidencyPolicy::new(123_456)`), C10's is the `try_from`
  path on a zero policy.
- Action: MERGE — P3 absorbs I3's loop and C10's `try_from` assert (P3 already
  iterates `presets()` + `mandatory()`). Keepers: `policy.rs:470` (loop),
  `config.rs:541` (`try_from` line, moved not deleted).
- Est. saving: one 4-preset construction + ~15 lines.

### E3 — [E duplicates, A cheap-code] four group-rejection tests are one table

- Locations: `crates/quanta-memory-platform-core/src/domain/config.rs:604`,
  `:615`, `:626`, `:638` (C13–C16), sharing `good_group()` (`:577`) +
  `residency_with_group()` (`:588`).
- Oracle (quoted):
  ```
  let mut g = good_group();
  g.reserved_bytes = 250; // > soft 200
  ```
  (config.rs:605–606; same mutate-one-field shape ×4).
- Mechanism: each test mutates exactly one field and asserts
  `Err(Config(_))`. Four near-identical 8-line bodies; adding a 5th group
  invariant copies the shape again.
- Action: MERGE into one `#[test]` looping `[(mutator, case-name); 4]`.
  Keeper: `config.rs:577` (`good_group()`).
- Est. saving: ~20 lines; time unchanged (microseconds).

### E4 — [E duplicates] unit-vs-boundary enforcement pairs: keep both layers

- Locations: boundary L1–L5 (`policy_contracts.rs:172,208,241,285,310`);
  unit counterparts live in `ledger.rs` (T01 scope, e.g. global/headroom/burst
  triples with byte values 600/600, 900/200, 600+50/60 — confirmed by v1
  finding F1-cluster text; ledger bodies not re-opened here, out of scope).
- Mechanism (inspected on the boundary side): L1–L4 drive the same
  (global, headroom, burst) scenarios through `reserve_managed_charge` and
  assert `total_owner_bytes()` + typed `Capacity`; the ledger units assert
  whole-map `snapshot == before` equality, which is strictly stronger on
  mutation-freedom. Different oracles ⇒ not deletable dups.
- Action: KEEP both; extract shared (global, bytes-a, bytes-b) triples into
  one table if T01 agrees. Keeper (boundary side): `policy_contracts.rs:172`
  (L1, the fail-closed snapshot pattern).
- Est. saving: 0 (deliberate keep).

## v1 re-verification (from `docs/bugbash/sep-22-test-optimization/01-core-domain.md`)

Only v1 file `01-core-domain.md` touches this scope (v1 track 1 owned the
whole `domain/` + `policy_contracts.rs`; files 02–05 verified grep-clean for
`policy.rs|config.rs|context.rs|stats.rs|recharge.rs|errors.rs|policy_contracts`).
Findings F1–F3/F11 are ledger-only → out of scope for T02 (left to T01);
their cross-file clusters are addressed via E4 above.

| v1 | Verdict | Evidence |
|----|---------|----------|
| F4 (`class_budget` helper dup ledger↔contracts) | SUPERSEDE | Narrowed: in-scope side (`policy_contracts.rs:137`) is a 6-line local helper used by 8 live_control tests; no in-scope second copy (ledger side belongs to T01). No action inside T02 beyond noting the drift risk |
| F5 (`ledger_from_policy` vs `permissive_policy` semantic trap) | SUPERSEDE | Same narrowing: in-scope `permissive_policy` (`policy_contracts.rs:147`) + `broker_from_residency` (`:153`) fully opened; semantics (zero-fill-to-global per mandatory class) confirmed load-bearing for L1–L6. Unification needs the ledger side (T01) |
| F6 (P14 tautological `burstable` tail, `policy.rs:687-688`) | CONFIRM | Re-opened body lines 663–689; `let burstable = r.shared_burst_pool_bytes; assert_eq!(burstable, 0)` still present → D2 |
| F7 (accept-path `.is_ok()`-only tests) | CONFIRM | Re-opened `config.rs:508-523,593-600,648-658,716-732` + `:437-447`; all still value-free → D4 (adds C3 to the list) |
| F8 (`MemoryError` Display 2/9 variants) | CONFIRM | Re-opened `policy_contracts.rs:114-118` + `errors.rs:12-26`; still Config+Report only → D7 |
| F9 (group enforcement: validated but never reserve-tested) | CONFIRM | `config.rs:571-732` still structural-only; no in-scope reserve-through-group test (ledger grep for `resource_group` left to T01; none of L1–L8 exercises groups). Correctness-gap question stands |
| F10 (`from_resolved` untested, default asymmetry) | CONFIRM | Re-opened `stats.rs:33-71`; still zero unit tests, asymmetry `:37` vs `:61` intact → D6 |
| F12 (P1/P2 5-axes-in-2-bodies + 32-bit `as u64`) | CONFIRM | Re-opened `policy.rs:94-122` + `:66-77`; casts and single-body matrix unchanged → D1 |
| F13 (P6 single-class sample) | CONFIRM | Re-opened `policy.rs:523-532`; still `ArtifactCache`-only → D3 |
| F14 (context/recharge zero tests deliberate) | CONFIRM | Grep + full reads: `context.rs` (32 lines), `recharge.rs` (15 lines, `pub(crate)`), plus `stats.rs`/`errors.rs` zero-`#[test]` confirmed. Verdict stands, extended: `stats.rs` SHOULD gain tests (D6), `errors.rs` Display table (D7) |
| Cluster 1 (unit vs boundary pairs, keep both) | CONFIRM | Boundary side re-opened (L1–L5); oracle-strength argument holds → E4 |
| Cluster 2 (preset isolation 3 depths, fold contracts totality into unit) | CONFIRM | Re-opened P3/P4/P7–P9 + I3 + I7; fold plan stands → E2 |

## Lenses

- **A. cheap-code**: production code in scope is thin. Only logic worth naming:
  `ExecutionBudget::check` (`policy.rs:51`, 6 lines, fully covered per-axis by
  P1/P2), `validate_residency` floor/overflow paths (covered incl. `u64::MAX`
  trap C11), `stats.rs::from_resolved` dedup (UNCOVERED → D6), group-validation
  branch fan-out (covered structurally, table-mergeable → E3). No dead-code
  deletion candidate in scope.
- **B. cheap-test**: everything in scope is sub-millisecond pure arithmetic /
  struct assembly except L1–L8 (broker + ledger Carson, still microseconds —
  no I/O, no sleeps, no threads). No test is slow enough to justify deletion
  on time grounds (see timing note). Optimization currency here is lines and
  oracle strength, not wall time: SHRINKs (P14 tail, I4 echo) and MERGEs
  (C2, P10–P11, C13–C16, C20–C21, P4/I3/C10) save ~70–90 lines total.
- **C. slow-measured**: `timing/top40.md` present; zero in-scope tests in the
  top-40. `timing/full.txt` holds only the trailing ~300 lines (facade
  binaries), so no measured per-test time exists in the committed log for any
  of the 49 tests — all "fast" verdicts are by construction (no I/O/sleep),
  not by measurement. Biggest in-scope cost by reasoning: L7 (4 preset broker
  builds + ~12 reserves) and C9 (14 validate passes) — both still
  sub-millisecond scale.
- **D. weak-oracle**: D2 (tautology), D4 (5 accept tests, no value pins), D5
  (struct echo), D6 (untested dedup/defaults), D7 (2/9 Display arms), D1/D3
  (unattributed / single-sample oracles). Strong-oracle models to copy:
  L1 (`policy_contracts.rs:197-204`: byte-for-byte snapshot equality +
  invariants 1+2 after denial) and C9 (per-class rejection messages).
- **E. duplicates**: E1 (band trio — keep, distinct predicates), E2 (totality
  ×3 — merge into P3), E3 (group rejections ×4 — merge into table), E4
  (unit/boundary pairs — keep both, stronger-oracle split). Net duplicate
  saving ≈ 35–45 lines + one 4-preset construction.

## Top-5 quick wins (ordered by value)

1. E2: fold I3 + C10 totality loops into P3 (one 4-preset construction + ~15 lines saved; kills the 3× totality).
2. D2: delete or convert P14's tautological `burstable` tail (`policy.rs:685-688`) — only vacuous oracle in scope.
3. E3: table-drive C13–C16 group rejections (~20 lines saved; stops the copy-paste shape).
4. D7: table-driven 9-variant `MemoryError` Display test at `policy_contracts.rs:115` (7 untested arms).
5. D6: two `from_resolved` unit tests (dedup order + default source) — only production logic in scope with zero tests.

## Unresolved

- Per-test measured times for the 49 in-scope tests: `timing/top40.md` is
  present and contains none of them (all sub-top-40 by construction), but
  `timing/full.txt` is tail-truncated (~300 lines, facade only), so no
  committed per-test `<time>s` line was inspectable for any in-scope test.
  Lens C claims rest on code inspection (no I/O/sleep/threads), not measurement.
- F9 correctness question (is group admission enforced anywhere at reserve
  time?) needs cross-track evidence from broker/scheduler scope — outside T02.
- Ledger-side counterparts of E4 pairs not re-opened (T01 scope); shared-triple
  table proposal needs T01 agreement.
