# T3 Findings — application / adapters / ports (fail-open audit, sep-26)

Auditor: T3. Read-only audit; the only file created by this audit is this report.

## Header — scope, census, method

Scope files (grep `#[test]` + `#[tokio::test]`, `grep -c '#\[test\]\|#\[tokio::test\]'`):

| file | grep count |
| --- | --- |
| crates/quanta-memory-platform-core/src/application/controller.rs | 13 |
| crates/quanta-memory-platform-core/src/application/scheduler.rs | 14 |
| crates/quanta-memory-platform-core/src/adapters/pin.rs | 5 |
| crates/quanta-memory-platform-core/src/adapters/metrics.rs | 2 |
| crates/quanta-memory-platform-core/src/adapters/hdrhistogram.rs | 4 |
| crates/quanta-memory-platform-core/src/ports/scratch.rs | 9 |
| crates/quanta-memory-platform-core/src/ports/telemetry.rs | 8 |

- **Grep total: 55. Row count in census below: 55. Zero sampling.**
- All 55 are plain `#[test]`; `#[tokio::test]` count is **0** across scope.
- No other `*.rs` under `core/src/application/`, `core/src/adapters/`, `core/src/ports/`
  contains test fns (checked `adapters/scratch.rs`, `adapters/mod.rs`, `ports/pin.rs`,
  `ports/reclaim.rs`, `ports/store.rs`, `ports/mod.rs`, `application/mod.rs` — none).
- Baseline observed (read-only runs, all green): `--all-features --lib application::` 27,
  `adapters::` 11, `ports::` 17 → 55 total.
- Feature note: hdrhistogram rows are gated `#[cfg(all(test, feature = "telemetry"))]`;
  they run only under `--features telemetry`/`--all-features` (verified) and are skipped by
  plain `cargo test` (default = `live_control`). The `not(telemetry)` no-op stubs
  (`LatencyRecorder::record → Ok`, `percentile → 0`; `MetricsEmitter::emit → {}`) have
  **zero** tests in either configuration.

Verdict tally: **49 STRONG, 4 WEAK, 1 FAILOPEN, 1 VACUOUS.**

## Census

Verdicts: STRONG / WEAK / FAILOPEN / VACUOUS. "Critic mutation" = one-line SUT mutation
for the critic agent; expected result stated. Line numbers are the `fn` lines.

### application/controller.rs (13)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| classification_is_deterministic_across_the_ladder (controller.rs:279) | STRONG | Exact classification at every boundary (740/750/899/900/980) plus re-run; pure SUT output. | — | change any `>=` to `>` in `classify_semantic` → red |
| update_caches_current_pressure (controller.rs:292) | STRONG | Asserts cached state via trait read after update(950) → Hard. | — | delete the `pressure.store` in `update_from_semantic` → red (stays Normal) |
| ladder_gets_more_aggressive_with_pressure (controller.rs:300) | WEAK | Soft ladder asserted exactly, but Fatal only via `contains(Spill)` + length; Fatal rung **order** and the entire Hard ladder unasserted — reclaim-walk order (spill last) is the decision. | assert exact `vec![…]` for Hard and Fatal. | swap Spill to first in the Fatal ladder (or drop `LowerPriorityClasses` from Hard) → stays **green** today; red after exact-vec fix |
| reclaim_returns_zero_under_fatal_pressure_without_spill (controller.rs:314) | STRONG | Fatal + spillable=0 + general=10_000 → exactly 0. | — | delete the `if spillable == 0 { return 0 }` early return → red (min→500) |
| reclaim_uses_spillable_under_fatal_pressure (controller.rs:328) | STRONG | Fatal, spillable 800 vs target 500 → exactly 500. | — | `return 0` unconditionally under Fatal → red |
| first_priority_owner_does_not_self_reclaim_below_fatal (controller.rs:341) | STRONG | First→0 vs Normal→500 contrast at same pool; negative pole present. | — | remove the `ReclaimPriority::First` guard → red |
| thresholds_new_accepts_monotonic_and_default_is_valid (controller.rs:354) | STRONG | Getter echoes are A1-adjacent but the equal-rung classification oracle (49→Normal, 50→Fatal) and Default validation carry real behavior. | — | reject equal rungs in `new` (`<` instead of `<=` in `is_monotonic`) → red |
| thresholds_new_rejects_inverted_or_out_of_range (controller.rs:384) | STRONG | 5-case hardcoded table (not A2), variant-asserted `Err(Config)`. | — | make `validate` return `Ok(())` always → red |
| zero_limit_is_normal_when_empty_and_fatal_when_used (controller.rs:406) | STRONG | 0→Normal / 1→Fatal / u64::MAX→Fatal on zero limit is real. Note: trailing `assert_eq!(c.classify_semantic(0), c.classify_semantic(0))` (line 417) is an `assert_eq!(x,x)` placeholder (cheatsheet row) — cannot fail. | drop line 417 or assert against a literal. | `return PressureState::Fatal` unconditionally when limit==0 → red |
| tiny_non_zero_limit_saturates_percent_without_wrapping (controller.rs:421) | **FAILOPEN** | see below | add large-limit case | see below |
| ladder_transitions_up_then_down_through_every_rung (controller.rs:436) | STRONG | Full ascent+descent asserting both return value and cached state per rung. | — | introduce hysteresis (keep last Hard on descend) → red |
| update_from_semantic_is_idempotent (controller.rs:461) | STRONG | Two separate stateful invocations compared (not `x==x`); catches state-perturbing updates. | — | make `update_from_semantic` store `Soft` on every second call → red |
| allocator_snapshot_observation_does_not_change_pressure (controller.rs:476) | STRONG | The A3 negative pole done right: semantic Hard held while allocator snapshot claims 0 resident. | — | store a decode of snapshot bytes in `observe_allocator_snapshot` → red |

**FAILOPEN — tiny_non_zero_limit_saturates_percent_without_wrapping (controller.rs:421).**
Oracle:
```rust
assert_eq!(c.classify_semantic(u64::MAX), PressureState::Fatal,
    "used_bytes * 100 must saturate, never wrap below fatal");
```
with `limit = 1` (the only controller the test builds). Mechanism: with `limit == 1` the
division is by 1, so a **wrapping** u64 multiply lands at `2^64 − 100`, still ≥ 98 → Fatal;
a **saturating** multiply lands at `u64::MAX` → Fatal. Verified numerically
(python): wrapping(1) = 18446744073709551516, saturating(1) = u64::MAX — both Fatal, both
**green**. The named failure mode (wrap under-reporting pressure) can be fully present in
`classify_semantic` while this test passes; the under-report only manifests at large limits
(limit=u64::MAX, used=u64::MAX → correct 100% Fatal; wrapping 0% Normal; saturating 1%
Normal — the cheatsheet `qmp-saturating-mul-then-divide` regression has **no** red test in
scope). The test does catch overflow→default-to-zero mutations, so not vacuous.
Minimal fix: add
`LadderPressureController::new(u64::MAX, PressureThresholds::default(), Normal).classify_semantic(u64::MAX) == Fatal`
(wrapping/saturating impls both return Normal there → red).
Critic mutation: replace the u128 pipeline with
`let percent = used_bytes.wrapping_mul(100) / self.global_hard_limit_bytes;` → test stays
**green** today (proves fail-open); turns red once the large-limit case is added.

### application/scheduler.rs (14)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| within_hard_cap_is_always_admitted (scheduler.rs:298) | STRONG | Admit decisions at cap for both groups; "always" bounded by burst tests below. | — | deny when `usage + delta > hard_cap` even inside cap → red |
| unknown_group_is_denied_fail_closed (scheduler.rs:305) | STRONG | Deny + typed `InvalidState` on record_usage/release + `usage.len()==0` no-state-growth. | — | default-fill unknown groups in `admits` → red |
| noisy_group_cannot_borrow_beyond_weighted_share (scheduler.rs:321) | STRONG | Exact byte boundaries (300 vs 301; bg 100 vs 101) after noise — decision + victim fairness, not just Ok. | — | drop the fair-share cap in `check_group_admission` → red |
| release_returns_burst_capacity (scheduler.rs:348) | STRONG | Deny → release 300 → admit 200 transition asserted. | — | make `record_release` not decrement usage → red |
| prospective_floor_aggregate_matches_committed_growth_and_release (scheduler.rs:357) | STRONG | Prospective Err at 801 vs Ok at 800, then committed `floor_fits` must match; catches aggregate drift in `record_usage`. | — | skip the `unused_reserved_floor_bytes` update in `record_usage` → red (800+stale floor > band) |
| headroom_usage_never_consumes_a_non_headroom_group_floor (scheduler.rs:382) | STRONG | Exact aggregate values (100/0/0/100) across headroom/non-headroom mix + negative pole (peer 800/800 → Err). | — | count Headroom growth in `non_headroom` → red |
| empty_scheduler_has_zero_share_and_denies_all (scheduler.rs:421) | STRONG | Zero-weight registry: deny even 0-byte request, share==0, no panic. | — | return `true` when total weight == 0 → red |
| unknown_group_denied_even_with_pool_available (scheduler.rs:434) | STRONG | Registered admitted vs unregistered denied with pool funded. | — | treat unknown group as weight-1 → red |
| weighted_share_distributes_remainder_work_conservingly (scheduler.rs:445) | STRONG | Sum==400 AND exact tuple (134,133,133) pins the deterministic tie-break; admit 134/deny 135. | — | distribute remainder in reverse name order → red |
| burst_percent_changes_admission_outcome (scheduler.rs:462) | STRONG | 10% burst: admit 1_100 / deny 1_101 — outcome-changing boundary. | — | ignore `burst_percent` in admission → red |
| usage_release_round_trip_readmits_burst (scheduler.rs:476) | STRONG | Not A7: exact usage content (1_300→1_000), re-admission, and over-release leaves usage byte-identical. | — | clamp instead of erroring on over-release (mutate usage) → red |
| huge_pool_and_weights_compute_a_non_wrapping_fair_share (scheduler.rs:502) | STRONG | Expected mirrors impl formula (mild A1 echo) but admit-at-exact-share / deny-at-+1 are SUT decisions; wrapping impl ≠ u128 expected → red. | — | compute share with `wrapping_mul` → red |
| admission_reuses_policy_static_fair_share_table_at_high_group_cardinality (scheduler.rs:520) | WEAK | Decisions inside are real (admit 1 / deny 2, table len/sum), but the named claim — *reuses* the static table — is a work/alloc claim; `fair_burst_shares == allocation` cannot fail against a deterministic rebuild (values identical). | expose a test-only precompute counter, or move the reuse claim to a bench. | inline `precompute_fair_burst_shares(&self.groups, …)` into `admits()` → stays **green** (rebuild is value-identical) |
| repeated_exact_usage_release_round_trip_never_wraps (scheduler.rs:544) | STRONG | u64::MAX charge, +1 → typed `Capacity` with usage unchanged, deny, full release to 0; ×8 repetition redundant but harmless. | — | use wrapping add in `usage_after_growth` → red |

### adapters/pin.rs (5)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| pin_invalidate_round_trip_tracks_dependent_count (pin.rs:87) | STRONG | Exact counts 0→1→2→0→1 across the lifecycle (count len only; content covered next row). | — | drop the dedup/insert in `pin` → red |
| invalidation_outcome_reports_exact_dependents (pin.rs:117) | STRONG | Not A7: exact vec content + order, unrelated payload untouched, unknown → default. | — | `invalidate` clears the whole map → red (beta count 0) |
| duplicate_pin_does_not_over_count_dependent (pin.rs:143) | STRONG | Triple identical pin → count 1, invalidated exactly once. | — | remove the `iter().any` dedup guard → red |
| distinct_dependents_each_increment_count (pin.rs:166) | STRONG | Three distinct dependents → 3. | — | replace insert with overwrite → red |
| poisoned_lock_is_fail_closed_not_panicking (pin.rs:182) | STRONG | A5 done right: real poison, read path recovers count 1, pin+invalidate → typed `Poisoned`, no panic anywhere. **A4 lead refuted**: the `let _ =` (pin.rs:190) is on the poisoning thread's `join()` — fixture discard of an *intentional* panic, not the op under test; the ops under test all have assertions. Optional hardening: `assert!(handle.join().is_err())` proves poisoning happened. | (optional) assert the join errored | make `locked()` use `unwrap_or_else(PoisonError::into_inner)` → red (pin succeeds, no `Err(Poisoned)`) |

### adapters/metrics.rs (2)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| emitter_surface_is_emit_only_and_takes_shared_ref (metrics.rs:47) | WEAK | Compile-marker: `&self`-only calls and `let returned: () = …` are enforced by rustc, not the body; the sole runtime assert is `assert_eq!(returned, ())` — comparing unit to unit, always true (an `assert_eq!(x,x)` placeholder). Can only fail if `emit` panics. Signature claim is compiler-guaranteed (sufficient for that narrow claim); emission *content* (labels/values reaching `metrics::counter!`/`histogram!`) is asserted nowhere. | under `telemetry`, install a metrics-util test recorder and assert the recorded name + label pairs. | swap the label keys (`"outcome" => event.operation.label()`) → stays **green** |
| emitter_holds_no_accounting_state (metrics.rs:69) | **VACUOUS** | see below | see below | see below |

**VACUOUS — emitter_holds_no_accounting_state (metrics.rs:69).**
Body (complete):
```rust
let emitter = MetricsEmitter;
let cloned = emitter.clone();
emitter.emit(MemoryEvent::new(crate::MemoryOperation::Reserve, …, Some(1)));
cloned.emit(MemoryEvent::new(crate::MemoryOperation::Release, …, Some(2)));
```
Mechanism: **zero** `assert*`/`expect*`/`panic!`/`unwrap` in the body. The test can only
fail if `emit` (or `clone`) panics — under `not(feature = "telemetry")` (the default test
run) `emit` is a literal no-op, and under `telemetry` the macros write to the global no-op
recorder; neither can panic. This is a pure compile/wiring marker: it witnesses
Default+Clone+`&self` callability (all compiler-guaranteed) but the named property —
"holds no accounting state" — is unobservable from this body. It is a runtime test in name
with no runtime oracle; a doc-comment or `const fn` type assertion would state the same
contract without pretending to execute it.
Critic mutation (proves vacuity): add `state: std::sync::Arc<std::sync::Mutex<u64>>` to
`MetricsEmitter` (Default stays derivable) and `*self.state.lock().unwrap() += 1;` inside
`emit` — an emitter that *does* hold mutable accounting-ish state — test compiles and
stays **green**.
Minimal fix with teeth: under `feature = "telemetry"`, install a
`metrics_util::debugging::DebuggingRecorder`/`MockRecorder` as the global recorder, emit a
known `MemoryEvent`, and assert exactly one counter ("quanta_memory_operation_total" with
the right operation/outcome labels) and one histogram value — plus assert nothing else was
recorded. That converts the advisory-lane claim into a behavioral oracle and subsumes both
metrics.rs rows.

### adapters/hdrhistogram.rs (4) — `#[cfg(all(test, feature = "telemetry"))]`

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| records_deterministic_set_and_reads_back_count_and_percentiles (hdrhistogram.rs:90) | STRONG | len==100, p50∈49..=51, p99∈98..=100, p100∈100..=101 from 1..=100 — content, not shape. | — | make `percentile` return `self.histogram.len()` → red |
| record_is_monotonic_and_lossless_in_count (hdrhistogram.rs:118) | STRONG | Count checked after each of 4 records. | — | make `record` drop every other sample → red |
| invalid_bounds_return_error_not_panic (hdrhistogram.rs:129) | STRONG | low>high → `Err`, message preserves source text. | — | `Ok(Self)` unconditionally in `new` → red |
| out_of_range_record_is_rejected_and_does_not_mutate_count (hdrhistogram.rs:140) | STRONG | Negative pole with fail-closed state check: rejected record leaves `len()` unchanged. | — | clamp out-of-range values in `record` → red |

### ports/scratch.rs (9)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| reservation_maps_to_live_scratch_charge_kind (scratch.rs:203) | STRONG | `charge_kind()==LiveScratch` is a real mapping decision; `reserved_bytes()==1024` is fixture echo but subordinate. Fixture uses the test-only `unenforced` ctor (authority covered by rows 8–9). | — | return `ChargeKind::Headroom` → red |
| grow_to_observed_updates_within_reserved_ceiling (scratch.rs:219) | STRONG | 400→400 and exact-ceiling 1000→1000 asserted. | — | ignore the write on success → red |
| grow_beyond_reserved_is_fail_closed_capacity_error (scratch.rs:231) | STRONG | A8 satisfied: typed `Capacity` denial **and** observed-bytes unchanged after. | — | clamp `observed_bytes = reserved` on overflow → red |
| scratch_peak_never_decreases_on_smaller_observation (scratch.rs:250) | STRONG | Full peak semantics: smaller/equal/larger, denied over-ceiling, post-release rejection — each with peak intact. | — | `self.observed_bytes = observed_bytes` (no max) → red |
| grow_after_release_is_rejected_and_fail_closed (scratch.rs:284) | STRONG | Typed `InvalidState` + observed unchanged post-release. | — | allow grow after release → red |
| double_release_is_rejected_not_silent (scratch.rs:305) | STRONG | Second release → `InvalidState`, `is_released()` still true. | — | make `release` idempotently return Ok → red |
| lifecycle_reserve_grow_release_succeeds_in_order (scratch.rs:321) | STRONG | Happy path with real intermediate values (observed 1024, released flag). | — | set `released = true` in `grow_to_observed` → red |
| failed_release_preserves_authority_for_retry (scratch.rs:346) | STRONG | Injected authority failure: Err, still enforced, retry Ok, exactly 2 authority calls — fail-closed retry semantics via a fake that can bite. | — | clear `authority` before calling `authority.release()` → red (is_enforced false, retry no-ops, calls==1) |
| successful_release_prevents_drop_from_releasing_twice (scratch.rs:364) | STRONG | Drop after explicit release → exactly 1 authority call. | — | make `Drop` call `release()` unconditionally → red (calls==2) |

### ports/telemetry.rs (8)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| operational_labels_are_closed_bounded_sets (telemetry.rs:203) | WEAK | `ops.map(label).len() == 13` asserts the fixture array's own length (A1 echo — `.len()` of a built array cannot differ); real part is non-empty labels. "Closed set" is compile-enforced by the enum + exhaustive `match`; a *new* variant or a **duplicate label collision** is invisible to this body. | assert label uniqueness: sort+dedup labels, assert count 13 / 11. | make `Recharge => "release"` (duplicate label) → stays **green**; red after dedup assert |
| advisory_agreement_has_no_mismatch (telemetry.rs:239) | STRONG | Equal bytes → mismatch false, drift Some(0). | — | `mismatch = resident != 0` → red |
| missing_allocator_data_is_not_treated_as_mismatch (telemetry.rs:248) | STRONG | A8 pole for the advisory lane: `None` → no mismatch, drift None. | — | `mismatch = true` when `None` → red |
| static_telemetry_returns_its_snapshot_verbatim (telemetry.rs:256) | STRONG | Not A7: full-struct equality of the round-tripped snapshot. | — | zero out `resident_bytes` in `snapshot()` → red |
| compare_reports_signed_drift_and_mismatch_boundary (telemetry.rs:266) | STRONG | 7-case table, both signs, exact boundary, semantic SSOT echo checked per case. | — | `mismatch = resident > semantic` only → red (999<1000 case) |
| compare_semantic_bytes_is_ssot_not_allocator (telemetry.rs:308) | STRONG | u64::MAX allocator vs semantic 7: authority stays 7, mismatch set, exact i128 drift (no overflow). | — | `semantic_bytes: resident.unwrap_or_default()` → red |
| allocator_telemetry_trait_surface_is_observation_only (telemetry.rs:328) | STRONG | Answer to the track lead: **behavior, not just wiring** — generic trait-bound fn drives `compare`, asserts mismatch/drift values, read idempotence (`advisory == again`), and source-unchanged. The "no mutating seam" half of the name is compiler-enforced (trait has only `snapshot()`); the body's assertions are real. | — | make `StaticAllocatorTelemetry::snapshot` clear `resident_bytes` after first read → red |
| trait_path_absent_allocator_data_never_contradicts_ssot (telemetry.rs:351) | STRONG | Custom blind impl through the trait path: no mismatch, None drift, semantic carried. | — | `BlindTelemetry` returning `resident_bytes: Some(0)`… (fixture) — SUT-side: `compare` defaulting `None` to `Some(0)` → drift Some(-9_999) → red |

## Track-specific question (controller/scheduler decisions)

Controller/scheduler tests overwhelmingly assert **decisions**, not bare `Ok`: exact
pressure classifications at byte boundaries, exact admit/deny at ±1 byte, exact fair-share
tuples, exact floor aggregates. No test passes while the scheduler "selects arbitrarily":
tie-breaks (name-order remainder), share math, and denial variants are all pinned to exact
values. Two named-claim gaps, both filed above: (1) the **Fatal/Hard ladder order** is not
asserted — the controller could order spill first or drop `LowerPriorityClasses` from Hard
and `ladder_gets_more_aggressive_with_pressure` stays green (WEAK); (2) the scheduler's
"reuses the policy-static table" claim is untestable by value equality since a rebuild is
value-identical (WEAK). Victim *selection* itself lives in the facade slow path, outside
this scope; here the ladder order is the decision surface and it is undertested at
Fatal/Hard.

## Footer — risk summary

1. Highest risk: `classify_semantic`'s u128 overflow guard (the pressure-percent
   under-report regression) has **no red test** — wrapping or saturating math passes every
   controller test today (FAILOPEN, controller.rs:421); one added large-limit assertion
   closes it.
2. The telemetry advisory lane is behaviorally sound where asserted (ports/telemetry.rs is
   strong), but `adapters/metrics.rs` contributes zero runtime oracle: one vacuous test
   and one compile-marker; emission content and "no accounting state" are unverifiable as
   written, and the `not(telemetry)` stubs are entirely untested.
3. Everything else in scope is genuinely decision-asserting; residual nits — Fatal/Hard
   ladder order, label uniqueness, one `assert_eq!(x,x)` placeholder line
   (controller.rs:417) — are cheap fixes with proposed one-line mutations above.
