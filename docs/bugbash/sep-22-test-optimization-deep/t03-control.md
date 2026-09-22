# Deep audit t03 — `api/control.rs` (76 tests)

Scope: `crates/quanta-memory-platform-core/src/api/control.rs`, test module
`control.rs:1544`–`control.rs:4489`. Every `#[test]` body opened and read.
No fix code (audit only).

Grep count: `grep -c -E '#\[(tokio::)?test' control.rs` = **76**. All 76 are
plain `#[test]` (no `#[tokio::test]` in scope). Census rows below: **76**.

Timing: `docs/bugbash/sep-22-test-optimization-deep/timing/top40.md` present and
inspected. No `api/control` unit test appears in the top-40 slowest; the only
control hit is the `ManagedCharge` doc-test compile-fail line (0.273s).
`timing/full.txt` holds only the last 300 log lines, so per-test control timings
are unresolved (see Unresolved). Lens C verdicts below therefore rest on static
cost (thread spawns, iteration counts, broker builds), not measured times.

Helpers in scope (read): `report_with` (`control.rs:1555`), `class_budget`
(`control.rs:1580`), `base_residency` (`control.rs:1590`), `broker`
(`control.rs:1600`), `managed_charge` (`control.rs:1775`),
`runtime_scratch_broker` (`control.rs:2977`), `poison_broker_lock`
(`control.rs:3786`), `RecordingEventSink` (`control.rs:3920`),
`RecordingReclaimer` (`control.rs:4127`).

## Census (76 rows)

| # | line | test fn | verdict | rationale |
|---|------|---------|---------|-----------|
| 1 | 1605 | scratch_reserve_cap_overflow_is_denied_without_mutation | KEEP | unique reserve-entry overflow oracle; extract shared setup (T03-D01) |
| 2 | 1637 | scratch_recharge_cap_overflow_preserves_original_ticket | KEEP | unique recharge-entry overflow oracle; extract shared setup (T03-D01) |
| 3 | 1681 | dropping_pending_reservation_rolls_back_all_axes | KEEP | base pending-drop dimension (v1 F-22 keeper) |
| 4 | 1702 | dropping_pending_reservation_during_rebuild_does_not_strand_capacity | MERGE | rebuild-fence drop overlaps #8 (`control.rs:1802`); merge into one fence-drop test |
| 5 | 1724 | failed_pending_commit_during_rebuild_rolls_back_capacity | KEEP | commit-fails path is distinct from drop path |
| 6 | 1743 | pending_drop_restores_resource_group_capacity | KEEP | group-axis pending drop (v1 F-22 keeper) |
| 7 | 1787 | managed_charge_drop_on_healthy_broker_releases_exact_capacity | KEEP | healthy Drop baseline + accessor pins |
| 8 | 1802 | managed_charge_drop_during_rebuild_defers_until_finish | KEEP | fence-drop merge target for #4 |
| 9 | 1820 | managed_charge_explicit_release_failure_retains_authority | KEEP | explicit-release-fails vs drop-defers are different contracts |
| 10 | 1839 | managed_charge_batch_failure_retains_every_lease | KEEP | batch atomicity oracle (ids + totals) |
| 11 | 1857 | managed_charge_batch_rejects_foreign_broker_before_mutation | KEEP | cross-broker fail-closed, both ledgers pinned |
| 12 | 1876 | managed_charge_recharge_denial_restores_old_authority | DELETE | subsumed by #55 (`control.rs:3482`, stronger oracle) + #13 (entry point); v1 F-20 fold |
| 13 | 1911 | replace_denial_returns_original_live_charge | KEEP | sole `recharge_owned` denial cover (v1 F-20 keeper) |
| 14 | 1940 | owned_charge_sequence_matches_exact_reference_model | KEEP | only model test; optional sampling (T03-D05) |
| 15 | 2014 | reserve_commit_keeps_class_owner_and_partition_balanced | KEEP | happy-path invariant anchor |
| 16 | 2056 | reserve_denied_owner_hard_cap_leaves_ledger_untouched | SHRINK | keep; delete dead `assert_ne!` at `control.rs:2090` (T03-D02) |
| 17 | 2094 | reserve_denied_class_hard_cap_no_borrow | KEEP | class-cap axis + borrow-enabled positive pair |
| 18 | 2137 | reserve_borrow_beyond_shared_pool_denied | KEEP | burst-pool-boundary denial; complements #17's accept pair |
| 19 | 2158 | release_returns_bytes_and_preserves_invariants | KEEP | release-subtracts invariant |
| 20 | 2181 | rebuild_in_progress_rejects_reserve_commit_release | KEEP | 3-op fence + resume pair in one test |
| 21 | 2218 | live_reconfig_shrink_below_usage_denied_and_noop | KEEP | sextet axis: global usage (table-drive, keep axis; T03-D06) |
| 22 | 2249 | live_reconfig_class_hard_cap_below_class_usage_denied | KEEP | sextet axis: class usage |
| 23 | 2275 | live_reconfig_dropping_a_class_with_live_usage_is_denied | KEEP | sextet axis: dropped class (fail-open guard) |
| 24 | 2310 | live_reconfig_owner_quota_below_owner_usage_is_denied | KEEP | sextet axis: owner quota |
| 25 | 2339 | live_reconfig_raising_headroom_over_non_headroom_usage_is_denied | KEEP | sextet axis: headroom band + legal-accept pair |
| 26 | 2384 | live_reconfig_lowering_group_cap_below_live_group_usage_is_denied | KEEP | sextet axis: group cap + raise-accept pair |
| 27 | 2437 | live_reconfig_raise_caps_accepted_and_preserves_usage | KEEP | sole accept-path usage-preservation pin |
| 28 | 2472 | allocator_advisory_mismatch_does_not_mutate_accounting | KEEP | semantic-SSOT oracle (drift math + no-mutation) |
| 29 | 2503 | pressure_classifies_from_semantic_usage | KEEP | semantic (not allocator) classification pin |
| 30 | 2541 | denied_reconfigure_leaves_scheduler_controller_and_report_unchanged | KEEP | strongest denied-reconfig oracle (4-surface atomicity) |
| 31 | 2619 | accepted_reconfigure_rebuilds_controller_to_new_global_limit | KEEP | controller-rebuild proof (Soft→Normal) |
| 32 | 2672 | reconfigure_rejects_policy_missing_drained_mandatory_class | DELETE | subsumed by #33 (`control.rs:2686`); v1 F-19 |
| 33 | 2686 | reconfigure_missing_class_denial_preserves_policy_scheduler_and_usage | KEEP | missing-class keeper (usage + admission preservation) |
| 34 | 2719 | pressure_thresholds_are_inclusive_at_exact_boundary_fractions | SHRINK | keep 7-row table; reuse one broker (T03-D04) |
| 35 | 2765 | rebuild_is_reentrant_double_begin_single_finish_then_idle_finish_is_noop | KEEP | 3-behavior re-entrancy pin in one test |
| 36 | 2812 | trait_effective_report_matches_inherent_and_is_not_fabricated | KEEP | fabrication-guard oracle (4GiB + marker) |
| 37 | 2837 | reserve_gated_by_resource_group_admission_fail_closed | KEEP | group-admission deny + release-readmit pair |
| 38 | 2896 | owner_without_group_mapping_is_not_gated_by_admission | KEEP | unmapped-owner negative of #37 |
| 39 | 2924 | execution_scratch_budget_caps_live_scratch_and_spares_other_kinds | KEEP | scratch-budget scoping (payload spared) |
| 40 | 2988 | live_runtime_scratch_charge_appears_in_ledger_snapshot | KEEP | scratch-happy-path 5-axis oracle |
| 41 | 3018 | live_runtime_scratch_over_cap_is_denied_without_mutation | KEEP | scratch-cap deny + drop-drain pair |
| 42 | 3040 | runtime_scratch_and_direct_broker_charge_share_one_limit | KEEP | shared-limit proof across two entry points |
| 43 | 3065 | dropping_runtime_scratch_during_rebuild_returns_capacity | KEEP | scratch-axis rebuild drop (v1 F-22 keeper) |
| 44 | 3086 | runtime_scratch_rejects_foreign_runtime_context_without_mutation | KEEP | sole adapter-seam guard pin (v1 F-05) |
| 45 | 3105 | runtime_scratch_release_restores_resource_group_capacity | KEEP | scratch×group interaction |
| 46 | 3150 | recharge_denied_by_scratch_budget_preserves_old_scratch_charge | KEEP | distinct denial source (budget, not cap); v1 F-20 stay |
| 47 | 3191 | concurrent_reserves_never_exceed_global_limit_no_toctou | SHRINK | keep invariant; 16×100→8×50 + hoist `format!` (T03-D03) |
| 48 | 3231 | broker_clones_share_one_ledger | KEEP | shared-ledger identity pin |
| 49 | 3258 | broker_is_send_and_sync | KEEP | compile-only by design (v1 F-14) |
| 50 | 3266 | accepted_reconfigure_preserves_scheduler_group_usage | KEEP | scheduler-usage-survives-accept guard |
| 51 | 3316 | remapping_live_owner_group_is_denied_fail_closed | KEEP | remap-deny + post-drain-accept pair |
| 52 | 3407 | reserve_committed_during_rebuild_is_fail_closed_no_strand | KEEP | atomic-charge fence pin |
| 53 | 3431 | reserve_committed_charges_exactly_once | KEEP | atomic-charge happy path |
| 54 | 3450 | recharge_swaps_committed_charge_atomically | KEEP | recharge happy path + old-consumed pin |
| 55 | 3482 | recharge_denied_preserves_original_old_ticket | KEEP | strongest recharge oracle; v1 F-20 keeper |
| 56 | 3527 | denied_recharge_preserves_the_old_axis_not_the_new_one | KEEP | wrong-axis restore pin; v1 F-20 stay |
| 57 | 3572 | recharge_during_rebuild_leaves_old_intact | KEEP | rebuild-denial + post-finish validity |
| 58 | 3608 | recharge_does_not_transiently_double_count_group_admission | KEEP | atomicity-vs-naive-charge proof |
| 59 | 3665 | denied_recharge_on_grouped_owner_preserves_group_usage | KEEP | grouped-denial + exact-70k admission bounds |
| 60 | 3722 | recharge_cannot_consume_peer_group_reserved_floor | KEEP | recharge entry of floor pair (twin #72 kept, T03-D07) |
| 61 | 3796 | managed_charge_drop_recovers_poisoned_lock_for_exact_release | KEEP | poison-recovery path: managed drop |
| 62 | 3811 | poisoned_runtime_scratch_explicit_release_preserves_then_drop_recovers_authority | KEEP | poison-recovery path: scratch release→drop |
| 63 | 3836 | poisoned_pending_commit_retains_ticket_until_drop_rollback | KEEP | poison-recovery path: pending commit→drop |
| 64 | 3865 | poisoned_broker_fails_closed_and_trait_report_stays_truthful | KEEP | poison surface sweep + truthful-report pin |
| 65 | 3937 | bounded_events_execute_outside_lock_and_cover_major_broker_outcomes | KEEP | outside-lock proof + 4-event cover |
| 66 | 4000 | telemetry_panic_cannot_change_or_abort_accounting | KEEP | panic-containment pin |
| 67 | 4021 | forged_snapshot_has_fallible_totals_without_panicking | KEEP | overflow-fallibility pin (no broker needed) |
| 68 | 4043 | concurrent_reserve_and_reconfigure_preserve_group_cap_truth | SHRINK | keep interlock; 128→32 iters (T03-D05) |
| 69 | 4152 | soft_cap_crossing_triggers_reclaim_without_immediate_hard_deny | KEEP | soft-lane baseline (class) |
| 70 | 4182 | owner_and_group_soft_caps_each_trigger_the_same_reclaim_lane | KEEP | owner-lane + group-lane pins (distinct axes) |
| 71 | 4239 | hard_cap_denial_reclaims_outside_lock_remeasures_and_retries_once | KEEP | hard-retry + re-entry proof |
| 72 | 4282 | group_reserved_floor_is_unavailable_to_peer_group | KEEP | reserve entry of floor pair (twin #60 kept, T03-D07) |
| 73 | 4340 | accepted_reconfigure_updates_revision_source_and_matches_headroom_predicate | KEEP | revision/source/provenance oracle |
| 74 | 4379 | identity_reconfigure_succeeds_during_legal_class_borrow | KEEP | borrow-legality survivor (table-drive candidate, T03-D08) |
| 75 | 4399 | identity_reconfigure_succeeds_during_legal_group_burst | KEEP | burst-legality survivor (table-drive candidate, T03-D08) |
| 76 | 4429 | reclaim_priority_orders_victims_deterministically | KEEP | victim-ordering oracle (class + group) |

Verdict totals: KEEP 69 · SHRINK 4 (#16, #34, #47, #68) · MERGE 1 (#4) ·
DELETE 2 (#12, #32). 69+4+1+2 = 76 = grep count.

## Findings (new + deepened)

### T03-D01 (A/S) — scratch-overflow twins share a 30-line `u64::MAX` setup
File: `crates/quanta-memory-platform-core/src/api/control.rs:1606`
(and `:1637`).
Oracle quoted (`control.rs:1628`–`control.rs:1632`):

```
assert!(
    matches!(second, Err(MemoryError::Capacity(_))),
    "scratch cap overflow must deny: {second:?}"
);
assert_eq!(b.ledger_snapshot().expect("after denial"), before);
```

Mechanism: both tests build `ResidencyPolicy::new(u64::MAX)` +
`max_execution_scratch_bytes = Some(usize::MAX)` and seed a `u64::MAX-10/-11`
charge; only the second half (reserve-deny vs recharge-deny) differs.
Action: extract `fn scratch_overflow_broker() -> (LiveProcessMemoryBroker, Ticket)`
next to `runtime_scratch_broker` (`control.rs:2977`); keep both tests
(keepers: `control.rs:1606`, `control.rs:1637`). Est saving: ~25 lines, zero
oracle risk. (Deepens v1 F-02 with exact lines.)

### T03-D02 (D/S) — dead `assert_ne!` after exact equality
File: `crates/quanta-memory-platform-core/src/api/control.rs:2090`, inside #16.
Oracle quoted (`control.rs:2088`–`control.rs:2090`):

```
assert_eq!(after.total_owner_bytes(), 400_000);
...
assert_ne!(after.total_owner_bytes(), 600_000);
```

Mechanism: `== 400_000` entails `!= 600_000`; the second assert can never add
information. Action: delete `control.rs:2090` (keeper: `control.rs:2088`).
Est saving: 1 line + reader attention. (Confirms v1 F-17 at exact line.)

### T03-D03 (C/M) — `format!("o{i}")` inside the 1600-iteration hot loop
File: `crates/quanta-memory-platform-core/src/api/control.rs:3203`, inside #47.
Oracle quoted (`control.rs:3218`–`control.rs:3227`):

```
assert!(
    snap.total_owner_bytes() <= 1_000_000,
    "global limit must hold under concurrent reserves (no TOCTOU): {}",
```

Mechanism: 16 threads × 100 iterations each allocate an owner string inside the
contended reserve section, so the critical section measures the allocator as
well as the ledger. Action: hoist to one pre-built `Vec<String>` per thread
before spawning (keeper: `control.rs:3191`), and halve to 8×50 — TOCTOU is
binary (lock serializes or not). Est saving: ~1/3 wall time of the heaviest
unit test. (Confirms v1 F-03 + F-09 with exact lines.)

### T03-D04 (C/M) — 7 full broker builds for pure threshold classification
File: `crates/quanta-memory-platform-core/src/api/control.rs:2737`–`control.rs:2741`,
inside #34. Oracle quoted (`control.rs:2752`–`control.rs:2756`):

```
assert_eq!(
    b.classify_pressure().expect("classify"),
    expected,
    "usage {used} / 1_000_000 must classify as {expected:?} (>= boundary)"
```

Mechanism: each of the 7 boundary rows runs `from_report` (full validation)
although only the controller's `>=` classification is pinned — and that logic
already has a cheap unit home (`controller.rs:251` per v1 F-11). Action: reuse
one roomy broker across all 7 cases (also a nicer idempotence proof) or keep a
single broker spot check (keeper: `control.rs:2719`). Est saving: 6 validations.
(Confirms v1 F-11.)

### T03-D05 (C/S) — iteration-count shrinks, same invariants
Files: `crates/quanta-memory-platform-core/src/api/control.rs:4043` (#68) and
`control.rs:1940` (#14).
Oracle quoted (`control.rs:4119`–`control.rs:4120`):

```
assert_eq!(admitted, 0, "group overflow reserve must never be admitted");
assert_eq!(denied, 128, "every concurrent overflow reserve must be denied");
```

Mechanism: #68 runs 128 full-policy `try_live_reconfigure` (each re-validates
residency and rebuilds scheduler+controller) against 128 concurrent reserves;
32 iterations exercise the same interlock. #14 takes 3 balance snapshots × 100
steps (300 snapshots); totals-every-step + balances-every-10th keeps the model
strength. Action: 128→32 in #68 (keeper: `control.rs:4043`); optional sampling
in #14 (keeper: `control.rs:1940`). Est saving: ~3/4 of #68 wall time.
(Confirms v1 F-10 + F-13.)

### T03-D06 (E/S) — live-reconfig denied sextet: table-drive setup, keep all six
Files: `crates/quanta-memory-platform-core/src/api/control.rs:2218`,
`:2249`, `:2275`, `:2310`, `:2339`, `:2384` (#21–#26).
Oracle quoted (`control.rs:2243`–`control.rs:2246`):

```
let after = b.effective_report().expect("report").residency.global_hard_limit_bytes;
assert_eq!(before, after, "denied reconfig must leave active policy untouched");
```

Mechanism: all six are reserve→commit→denied-reconfig→untouched; only the
denial axis differs (global / class / dropped-class / owner / headroom /
group). Each axis is a distinct fail-closed guard — no deletion. Action: add a
`deny_case` helper (or table-drive policy construction) cutting ~150 lines of
setup (keepers: all six lines above). Est saving: ~150 lines. (Confirms v1 F-26;
verdict stays "keep all six", new saving estimate.)

### T03-D07 (E/S, noted, no action) — reserved-floor twins across entry points
Files: `crates/quanta-memory-platform-core/src/api/control.rs:3722` (#60,
`recharge`) vs `control.rs:4282` (#72, `reserve`).
Oracle quoted (`control.rs:3767`–`control.rs:3770`):

```
.expect_err("recharge must preserve the other group's 400-byte floor");
assert!(matches!(denied.error, MemoryError::Capacity(_)));
```

Mechanism: same 400-byte floor protected against the peer group through two
different entry points; neither subsumes the other. Action: none — keep both
(same intentional double-coverage shape as v1 F-25). New observation (not in v1).

### T03-D08 (E/S) — identity-reconfigure twins share one shape
Files: `crates/quanta-memory-platform-core/src/api/control.rs:4379` (#74,
class borrow) vs `control.rs:4399` (#75, group burst).
Oracle quoted (`control.rs:4395`):

```
broker.try_live_reconfigure(policy).expect("identity reconfigure preserves legal borrow");
```

Mechanism: both are reserve→identity-reconfigure→release with only the
legality axis differing (shared-pool borrow vs group burst). Action:
parametrize over a `policy_with_legal_overage()` constructor or keep both (12 +
28 lines; cheap). Keepers: `control.rs:4379`, `control.rs:4399`. Est saving if
folded: ~20 lines. New observation (not in v1).

## v1 re-verification (scope = control.rs findings in 01..05)

Only `02-core-api-application.md` carries control-scope findings; `01`, `03`,
`04`, `05` contain no control.rs finding (01 mentions `live_control`
integration tests only in passing — nothing to re-verify). Result per finding:

| v1 | claim | result | evidence this session |
|----|-------|--------|----------------------|
| F-01 (S) | 6-field `ResourceGroup` literal pasted ~10× | CONFIRM (count now ~15 sites incl. `control.rs:4401`, `control.rs:4438` closure, `control.rs:3320` double) | opened literals at `control.rs:1745`, `control.rs:2543`, `control.rs:2840`, `control.rs:2899`, `control.rs:3107`, `control.rs:3269`, `control.rs:3320`, `control.rs:3611`, `control.rs:3668`, `control.rs:3725`, `control.rs:4047`, `control.rs:4205`, `control.rs:4284`, `control.rs:4401`, `control.rs:4438` |
| F-02 (S) | scratch-overflow twins dup setup, keep both | CONFIRM | opened `control.rs:1606`, `control.rs:1637`; see T03-D01 |
| F-03 (M) | `format!` in hot loop | CONFIRM | opened `control.rs:3203`; see T03-D03 |
| F-05 (M) | adapter seam pinned only by `control.rs:3086` | CONFIRM | opened `control.rs:3086`–`control.rs:3103` (foreign-runtime guard + no-mutation oracle intact) |
| F-08 (S) | trait surface covered via 2812/3865, no action | CONFIRM | opened `control.rs:2812`, `control.rs:3865`; both oracles strong |
| F-09 (M) | 16×100 heaviest unit test, halve | CONFIRM | opened `control.rs:3191`; see T03-D03 |
| F-10 (M) | 128 reconfigures, cut to 32 | CONFIRM | opened `control.rs:4043`; see T03-D05 |
| F-11 (M) | 7 broker builds, reuse one | CONFIRM | opened `control.rs:2719`; see T03-D04 |
| F-13 (S) | 300 snapshots, optional sampling | CONFIRM | opened `control.rs:1940` (per-step `total_owner_bytes` + 3 balance asserts at `control.rs:1998`–`control.rs:2002`) |
| F-14 (S) | `Send+Sync` compile-only, keep | CONFIRM | opened `control.rs:3258` (`fn assert_send_sync<T: Send + Sync>() {}`) |
| F-17 (S) | dead `assert_ne!` | CONFIRM | opened `control.rs:2090`; see T03-D02 |
| F-19 (M) | delete 2672, keep 2686 | CONFIRM | opened `control.rs:2672` (no usage, report-only) vs `control.rs:2686` (live usage + admission preservation at `control.rs:2701`–`control.rs:2710`) |
| F-20 (M) | fold 1876, keep 3482 + 1911; scratch/wrong-axis/grouped variants stay | CONFIRM | opened `control.rs:1876`, `control.rs:1911`, `control.rs:3482` (partition+kind oracle at `control.rs:3510`–`control.rs:3518`), `control.rs:3150`, `control.rs:3527`, `control.rs:3665` — each pins a distinct axis |
| F-22 (M) | pending-drop ×4: keep 3, merge 1702 into 1802 theme | CONFIRM | opened `control.rs:1681`, `control.rs:1702`, `control.rs:1743`, `control.rs:1802`, `control.rs:3065` — three distinct axes + one fence overlap |
| F-26 (M) | denied sextet table-drive, keep six | CONFIRM | opened all six (#21–#26); see T03-D06 |

REFUTE / SUPERSEDE: none. No v1 control finding was contradicted.

## Lenses

- A (cheap code): T03-D01 (scratch setup helper ~25 lines), F-01 `group()`
  helper (~60+ lines across ~15 sites), T03-D06 `deny_case` (~150 lines),
  T03-D08 (~20 lines).
- B (cheap tests / gaps): no new gap inside scope; scope is deny/allow-complete
  (positive pairs at #17/#18, #25/#26, #37/#38, #51, #60/#72). The adapter-seam
  gap (v1 F-05) is out of scope to fix here.
- C (slow, measured): no control unit test in top40 (see header); static-cost
  shrinks are T03-D03 (#47), T03-D04 (#34), T03-D05 (#68, #14).
- D (weak oracle): codebase oracles are strong (exact totals + balance
  predicates + typed errors). Only weakness found is the dead assert (T03-D02);
  compile-only #49 is by design (v1 F-14).
- E (duplicates): DELETE #12 (→ #55 + #13), DELETE #32 (→ #33), MERGE #4
  (→ #8); keep-noted pairs T03-D07, T03-D08, #69/#70 (class vs owner/group
  soft lanes).

## Unresolved

- Per-test wall times for the 76 control unit tests: `timing/full.txt` holds
  only the last 300 log lines (no `api::control` unit rows) and `top40.md`
  lists only the one control doc-test line. Lens C savings are therefore
  static estimates, not measured.
