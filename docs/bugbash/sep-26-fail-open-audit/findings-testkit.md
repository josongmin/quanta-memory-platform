# Fail-Open Test Audit — T9: quanta-memory-platform-testkit

## Header

Scope (12 files, all under `crates/quanta-memory-platform-testkit/`):

| File | `#[test]`/`#[tokio::test]` grep count |
| --- | --- |
| `src/conformance/suites.rs` | 39 |
| `src/conformance/invariants.rs` | 5 |
| `src/conformance/mod.rs` | 5 |
| `src/conformance/target.rs` | 0 (no test fns) |
| `src/faults/scenarios.rs` | 7 |
| `src/faults/mod.rs` | 2 |
| `src/fake/broker.rs` | 6 |
| `src/fixtures/mod.rs` | 5 |
| `src/alloc_tracking/mod.rs` | 5 |
| `src/loom_models.rs` | 5 |
| `src/lib.rs` | 3 |
| `tests/facade_conformance_bite.rs` | 4 |
| `tests/public_api_surface.rs` | 1 |

- **Grep count: 87** (`grep -rE '^\s*#\[(tokio::)?test\]'` over the crate; no `#[tokio::test]` exists — all synchronous).
- **Census row count: 87** (below). Row count == grep count; no sampling.
- Cross-check: `cargo test -p quanta-memory-platform-testkit --all-features` → 82 unit + 4 (`facade_conformance_bite`) + 1 (`public_api_surface`) = **87 passed, 0 failed**. The `loom` feature is NOT in `full` but IS picked up by `--all-features`; all 5 loom models compiled and passed with the standard toolchain (no special toolchain needed).
- Zero-test scope leftovers (not in census, coverage noted in gap map): `src/fake/mod.rs`, `src/fake/views.rs`, `src/doctor/mod.rs`, `src/conformance/target.rs` — no `#[test]` fns.

Verdict tallies: **78 STRONG, 1 WEAK, 2 FAILOPEN, 6 VACUOUS.**

## Census table

Legend: verdicts per standard definitions. "pole" = negative pole (a proven-broken input that turns the row red). File prefix `S` = `src/conformance/suites.rs`, `INV` = `src/conformance/invariants.rs`, `CM` = `src/conformance/mod.rs`, `FS` = `src/faults/scenarios.rs`, `FM` = `src/faults/mod.rs`, `FB` = `src/fake/broker.rs`, `FX` = `src/fixtures/mod.rs`, `AT` = `src/alloc_tracking/mod.rs`, `LM` = `src/loom_models.rs`, `LIB` = `src/lib.rs`, `BITE` = `tests/facade_conformance_bite.rs`, `PAS` = `tests/public_api_surface.rs`.

### conformance/suites.rs (39)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `deterministic_trace_matches_independent_model` S:1686 | STRONG | 256-step trace replayed against an `IndependentAccountingModel` computed from the issued ops (never from the target); runs BOTH fake and real broker; inline stale-commit/release rejection asserts. Guards the fake against silent accounting drift. | — | Make `FakeProcessBroker::release` skip the owner-axis subtraction → model/view divergence at S:1668 goes red. |
| `property_trace_matches_independent_model` S:1906 | STRONG | proptest (128 cases) random op sequences incl. `u64::MAX` reserve denial, rebuild windows, advisory observes; grouped model checks group axis too; both targets. | — | Make fake `reserve` ignore `rebuild_in_progress` → `assert!(result.is_err())` at S:1746 goes red. |
| `reentrant_sink_is_contained_outside_owner_lock` S:1934 | STRONG | Event sink re-enters `broker.observe()` from `emit`; asserts exactly 1 reentry and exact semantic bytes (8 → 0). If emission happened under the owner lock the test deadlocks/fails; if never emitted, reentries==0 fails. | — | Remove sink dispatch from reserve → `reentries==1` assert goes red. |
| `suite_rejects_black_hole_accounting` S:1959 | STRONG | Negative pole: view always `AccountingView::default()`; asserts rejection AND exact violated-family set `[OwnerPartition, AllocatorAdvisory]`. | — | Weaken `independent_exact_accounting_model` verdict to `true` → family-set assert fails. |
| `suite_rejects_commit_swallowing_target` S:1967 | STRONG | Pole: `do_commit` returns Ok without committing; localization to `TicketLifecycle`. | — | Make verdict `every_commit_result_checked` always hold → assert_eq on families goes red. |
| `suite_rejects_release_swallowing_target` S:1975 | STRONG | Pole: `do_release` Ok without releasing; final-zero verdict fires. | — | Same pattern, `terminal_releases`/final-view verdict. |
| `suite_rejects_duplicate_commit_accepting_target` S:1983 | STRONG | Pole: second commit accepted (`or(Ok(()))`); "second commit must reject" verdict S:491 fires. | — | Drop the `duplicate_commit` verdict → family set no longer matches. |
| `suite_rejects_wrong_context_axis` S:1991 | STRONG | Pole: context bytes moved to axis 1; exact-model comparison catches. | — | — |
| `suite_rejects_wrong_charge_kind_axis` S:1999 | STRONG | Pole: payload folded into metadata scalar; exact model + partition verdicts catch. | — | — |
| `sep23_fup004_suite_rejects_class_confined_kind_swap` S:2013 | STRONG | Pole confined to `class_kind_bytes[0]`; additionally asserts the NAMED verdict `observed_kind_split_agrees_across_axes` is the one that violates (meta-bite: dropping the per-axis check goes red). | — | Delete the kinds verdict S:486 → the `report.iter().any(... && !verdict.held)` assert fails. |
| `sep23_fup004_suite_rejects_context_confined_kind_swap` S:2033 | STRONG | Same as S:2013 for `context_kind_bytes[0]`. | — | — |
| `sep23_fup004_suite_rejects_aggregate_preserving_bucket_swap` S:2050 | STRONG | Pre-verifies the fault preserves every aggregate (asserts the lie is invisible to old checks) then asserts rejection via bucket×kind model. Proves the new check adds discriminating power. | — | Swap check back to aggregate-only comparison → `assert_broken_target_rejected` fails. |
| `sep23_fup004_mixed_kind_trace_agrees_on_every_axis_bucket` S:2081 | STRONG | Fixed mixed-kind script asserting FULL kind matrices on fake and live broker across reserve AND release phases; model applied per-op. | — | Corrupt `apply_release` kind-bucket decrement → view/model assert goes red. |
| `suite_rejects_stale_ticket_accepting_target` S:2144 | STRONG | Pole: stale release accepted; "second release must reject" fires. | — | — |
| `snapshot_error_is_failed_verdict_not_zero` S:2152 | STRONG | Pole: `view()` returns Err — must surface as VIOLATED verdicts, never as a zero-read (anti-A3). | — | Map view Err to default view → family set changes → red. |
| `suite_rejects_transfer_double_owner` S:2165 | STRONG | Pole: owner total + axis inflated by 1_000; balance + exact model fire. | — | — |
| `prepared_publication_suite_accepts_honest_service` S:2178 | STRONG | Positive pole, correctly scoped: real service through the suite; any billing/replay drift fails verdicts (paired with S:2246/2332/2409). | — | Break prepared replay idempotency → red. |
| `prepared_suite_rejects_correct_replay_with_unbilled_identity` S:2246 | STRONG | Pole: honest behavior but `stats()` zeroed — a target with correct semantics but lying accounting must still fail; also asserts the conflict verdict still HOLDS (localization precision). | — | — |
| `prepared_publication_suite_rejects_duplicate_charging_fake` S:2332 | STRONG | Pole: every prepare re-billed (charged=7×n); replay verdict + fail-closed localization. | — | — |
| `prepared_publication_suite_rejects_root_blind_gc_fake` S:2409 | STRONG | Pole: `orphan_tokens`/`gc_orphans` ignore the root set; root-aware-gc verdict fires, localizes to ReclaimAccounting. | — | — |
| `transcript_append_suite_accepts_honest_service` S:2420 | STRONG | Positive pole with exact byte oracle (independent literal formula at S:303-307). | — | — |
| `transcript_append_suite_rejects_non_idempotent_fake` S:2486 | STRONG | Pole: append-every-time fake — replay, collisions, retention verdicts all fire. | — | — |
| `transcript_suite_rejects_target_that_ignores_declared_horizon` S:2497 | STRONG | Pole: retention horizon 3 instead of 2 — ONLY the saturation verdict may fail, all others must hold (proves the verdict isolates the horizon contract). | — | — |
| `transcript_suite_rejects_correct_replay_with_unbilled_identity` S:2554 | STRONG | Pole: `retained_bytes()==0` wrapper; replay verdict fires, collision verdict holds. | — | — |
| `honest_fake_passes_every_accounting_invariant` S:2580 | STRONG | Positive pole; empty-suite guard via `!verdicts.is_empty()`. | — | — |
| `suite_rejects_fake_that_under_reports_owner_bytes` S:2592 | STRONG | Pole via `AccountingFidelity::UnderReportOwnerBytes`; localization to ClassOwnerBalance. | — | — |
| `suite_rejects_fake_whose_class_diverges_from_owner` S:2608 | STRONG | Pole via `DivergeClassFromOwner(64)`. | — | — |
| `all_break_variants_are_distinct_non_honest_and_family_mapped` S:2647 | STRONG | Enumeration hygiene: count assert + compile-time exhaustive match (S:2630) forces new `AccountingFidelity` variants into the list. Note: `let _ = expected_family(*v)` (S:2654) is an A4-flavored call-only proof; the real mapping proof lives in S:2674. | — | Add a 4th enum variant without updating → compile error; change count → red. |
| `suite_rejects_every_accounting_fidelity_break_variant` S:2674 | STRONG | Meta-enumeration: every break variant rejected AND localized to the SPECIFIC family. | — | — |
| `owner_partition_break_localizes_to_owner_partition_not_balance` S:2696 | STRONG | Localization precision pole: invariant-2 break must NOT masquerade as invariant-1. | — | — |
| `honest_fidelity_is_the_only_variant_the_suite_accepts` S:2713 | STRONG | Paired positive pole (a suite rejecting everything is also worthless). | — | — |
| `suite_rejects_target_that_mutates_accounting_on_a_denied_reserve` S:2838 | STRONG | Pole: denied reserve returns Err but moved counters; full-view before==after check (S:516) fires; localizes to ReserveFailClosed. | — | Weaken before/after compare to scalar total → family set assert still catches via exact model; remove both → red. |
| `honest_fake_passes_the_strengthened_fail_closed_check` S:2854 | STRONG | Positive pole with anti-vacuity guard: asserts the ReserveFailClosed verdict list is NON-EMPTY before asserting all held. | — | Delete the fail-closed verdict → `!fail_closed_checks.is_empty()` goes red. |
| `suite_rejects_target_that_mutates_on_allocator_observation` S:2956 | STRONG | Pole: `observe_allocator` bumps a counter; advisory-inertness verdict (before==after view, S:562/577) fires; localizes to AllocatorAdvisory. | — | — |
| `commit_then_release_returns_balance_to_zero_on_honest_fake` S:2972 | STRONG | Exact `AccountingView::default()` equality after full lifecycle plus invariants 1/2 mid-flight. | — | — |
| `commit_then_release_returns_balance_to_zero_on_real_broker` S:2999 | STRONG | Same on the REAL broker through the neutral adapter. | — | — |
| `cap_deny_agrees_between_honest_fake_and_real_broker` S:3058 | STRONG | Differential oracle: fake and real must agree on (1) typed `Capacity` deny, (2) post-deny inertness (full-view before==after on both), (3) subsequent in-cap success with identical totals. Catches a fake that drifts from real deny semantics. | — | Make fake cap check post-mutate → inertness assert goes red. |
| `both_targets_still_pass_the_portable_suite_after_strengthening` S:3133 | STRONG | Regression positive pole over both targets. | — | — |
| `spill_routing_verdict_holds_for_honest_and_violates_for_a_dropping_target` S:3182 | STRONG | Both poles for the suite's spill helper: honest holds, handle-dropping target violates. Inventory captured BETWEEN spill and restore (1→0), so an after-only check cannot pass a dropping target. | — | Weaken helper to `after==0` only → dropping-target assert fails. |

### conformance/invariants.rs (5)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `equal_bytes_hold_unequal_violate_with_drift_evidence` INV:143 | STRONG | Verdict-machinery unit test with in-body negative pair (1_000 vs 900 → VIOLATED, drift=100 in evidence). | — | Make `assert_eq_bytes` always hold → `!bad.held()` goes red. |
| `predicate_assertion_carries_supplied_evidence` INV:156 | STRONG | False predicate → VIOLATED with supplied evidence and family attribution. | — | — |
| `every_family_has_a_unique_stable_label` INV:180 | STRONG | Dedup + non-empty + Display==label for the full taxonomy; prevents ambiguous localization. | — | Duplicate a label → dedup count assert fails. |
| `drift_evidence_sign_distinguishes_over_from_under_reporting` INV:197 | STRONG | Positive/negative drift evidence (±500) and u64::MAX boundary pair. | — | — |
| `family_attribution_is_preserved_through_both_assertion_constructors` INV:217 | STRONG | Both constructors attribute every family correctly. | — | — |

### conformance/mod.rs (5)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `honest_fake_yields_passed_verdict` CM:119 | STRONG | Folded-verdict positive pole (paired with CM:127). | — | — |
| `broken_fake_yields_failed_verdict_localized_to_balance_family` CM:127 | STRONG | Negative pole at the PUBLIC folded surface (`ConformanceSuite::run_broker_accounting`), localized. | — | — |
| `empty_assertion_list_is_failed_not_silently_passed` CM:140 | STRONG | Anti-vacuity guard (A2 countermeasure): `from_assertions(&[])` is `Failed`, so a suite that runs nothing cannot pass. NOTE: this only guards the fully-empty case — a partially-emptied verdict list still passes (see gap map G5). | — | Make empty → Passed → red. |
| `public_suite_rejects_every_break_variant_and_localizes_it` CM:179 | STRONG | Exhaustive-match-tied enumeration of break variants at the folded surface + honest positive pole. | — | — |
| `real_core_broker_passes_the_same_portable_suite` CM:223 | STRONG | The reusable suite holds against the real `LiveProcessMemoryBroker` (proves the suite is not fake-only shape). | — | — |

### faults/scenarios.rs (7)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `reclaim_returns_zero_is_contained` FS:206 | STRONG | Fault guaranteed to land: fake configured `with_reclaim_yield_bytes(0)`, `reclaim_once()` invoked; asserts Contained + "freed 0" evidence. If the fake freed bytes or drifted usage → Escaped → red. Caveat: the DRIVER's Escaped branch has no negative pole (gap map G3); `let _ = broker.commit(&t)` (FS:69) is an A4 instance on a setup op (harmless: reserve already moved counters). | Add a rubber-stamp pole (fake whose reclaim lies) asserting the driver reports Escaped. | Make `drive_reclaim_returns_zero` unconditionally return Contained → this test stays GREEN (finding G3); make fake `reclaim_once` return 50 while subtracting 0 → test goes red (SUT pole works). |
| `pinned_delete_failure_is_contained_and_bytes_survive` FS:213 | STRONG | Real facade store; pin success checked, delete attempted while pinned; Contained requires Err AND charged_before==charged_after>0 (an Err from `charged_bytes()` is conservatively treated as drift via `unwrap_or(0)` → cannot pass silently). Same driver-pole caveat (G3). | — | Make store delete pinned payloads → Ok arm → Escaped → red. |
| `rebuild_race_write_is_rejected` FS:221 | STRONG | Structural race: window opened, racing reserve issued, full before/after usage compare. Same driver-pole caveat (G3). | — | Make fake accept reserves during rebuild → Ok arm → Escaped → red. |
| `spill_failure_is_contained_without_counting` FS:227 | STRONG | Honest offline controller through the production classifier; paired with FS:239/282/317 poles. | — | — |
| `spill_fault_classification_reports_escaped_for_a_lying_controller` FS:239 | STRONG | Negative pole: lying controller (Ok on unavailable target) → Escaped with "reported success"; honest pole re-run through the SAME helper. Proves the driver is not a rubber stamp. | — | Make `classify_spill_failure` always Contained → `!escaped.contained()` goes red. |
| `spill_fault_classification_reports_escaped_when_count_drifts` FS:282 | STRONG | Pole: fails closed but inventory moved (before≠after) → Escaped "inventory proof". | — | — |
| `spill_fault_classification_reports_escaped_when_count_is_unreadable` FS:317 | STRONG | Pole: inventory unreadable (Err) → Escaped; the proof cannot be silently skipped. | — | — |

### faults/mod.rs (2)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `harness_covers_reclaim_pin_rebuild_and_spill` FM:120 | STRONG | Coverage enumeration: dropping any scenario from `all()` fails the `contains` asserts; all-contained assert prints uncontained reports. | — | Remove a scenario from `FaultScenario::all()` → red. |
| `empty_harness_runs_nothing` FM:138 | STRONG | Anti-vacuity: an empty harness must not claim coverage. | — | — |

### lib.rs (3)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `conformance_suite_rejects_a_fake_that_under_reports_usage` LIB:69 | STRONG | Crate-level self-test negative pole + localization. | — | — |
| `conformance_suite_rejects_a_fake_with_divergent_class_totals` LIB:84 | STRONG | Negative pole. | — | — |
| `conformance_suite_accepts_an_honest_fake` LIB:91 | STRONG | Paired positive pole. | — | — |

### alloc_tracking/mod.rs (5)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `fresh_probe_reports_zero_signals` AT:130 | STRONG | Zero-state + underflow guard (net_live==0, not huge). | — | — |
| `recorded_signals_accumulate_and_net_live_is_correct` AT:139 | STRONG | Exact counters after deterministic record calls. | — | — |
| `dealloc_exceeding_alloc_saturates_at_zero_not_nonsense` AT:153 | STRONG | Negative path 5: saturation, never u64 wrap. | — | Use wrapping sub → `!= u64::MAX-898` goes red. |
| `signals_feed_an_invariant_assertion` AT:165 | STRONG | Probe feeds a real `InvariantAssertion` (balanced → held) WITH in-body negative pair (imbalance → VIOLATED, drift=2048). Gated `#[cfg(feature = "conformance")]`; runs under `--all-features`. | — | — |
| `tracker_callbacks_route_into_the_same_counters` AT:190 | STRONG | `AllocationTracker` callbacks (the global-allocator path) accumulate into the same counters as the deterministic API. | — | — |

### fixtures/mod.rs (5)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `testkit_runs_conformance_against_its_fake_and_passes` FX:119 | **FAILOPEN** | Positive-pole-only at the KIT seam. `MemoryTestKit::run_conformance()` (FX:59) is the only conformance entry a host consumer is pointed at, and no test drives it (or `fake_broker()`) with a broken fake — the kit offers no fidelity knob. If the kit entry stops actually running the suite, every test stays green. | Add a kit-level negative pole: a `MemoryFixtureBuilder::with_broken_fidelity(...)` (or a test-only hook) so `run_conformance` can be driven to `Failed` through the kit facade; assert it. | Replace `MemoryTestKit::run_conformance` body with `ConformanceVerdict::Passed` → all 87 tests stay green. |
| `fake_pressure_fixture_propagates_pressure_into_the_fake_broker` FX:127 | STRONG | Positive + negative pair (Soft vs Normal) on the fixture→fake plumbing. | — | — |
| `conformance_verdict_localizes_to_contract_family_not_a_host_package` FX:137 | **VACUOUS** (A2) | The named check — "verdict localizes to a contract family, never a host package" — is a `for family in verdict.violated_families()` loop over a PASSING verdict: `violated_families()` is empty, the loop body NEVER executes, and the body itself (`let _: ContractFamily = *family;`) asserts nothing. Only the trailing `assert!(verdict.passed())` can fail, which tests nothing about localization. | Drive a broken kit fixture (see FX:119 fix) and assert every violated family round-trips through `label()`/`Display` (the INV:180 machinery) — or delete the dead loop. | Replace the loop body with `panic!("host noun leaked")` → test stays green. |
| `live_broker_materializes_from_fixture_manifest` FX:148 | STRONG | Manifest→broker→effective_report round-trip with full structural equality (preset, execution, residency). | — | Drop manifest wiring → equality fails. |
| `testkit_runs_facade_store_suite_against_live_broker_and_passes` FX:158 | **FAILOPEN** | Positive-pole-only for the FULL facade-store suite. `run_facade_store_suite` (S:681-689) folds payload/retained/transcript/spill slices + lifecycle suites; nothing asserts the slice list is complete. Removing some slice sub-suites leaves a non-empty verdict list → `from_assertions` still `Passed` (the CM:140 guard only catches removal of ALL checks). The kit seam (FX:67) is never driven negative. | Assert the emitted verdict NAMES (or count) of `run_facade_store_suite` — e.g. all of `payload_dedup_refund`, `pin_safe_delete`, `retained_map_refunds`, `retained_atomic_recharge_overwrite`, `transcript_truncate_refund`, `transcript_over_cap_fail_closed`, `spill_round_trip`, `spill_unavailable_fail_closed` plus 5×4 lifecycle names are present. | Delete the four `verdicts.extend(run_*_slice(...))` lines in S:683-686 → all 87 tests stay green. |

### fake/broker.rs (6)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `honest_reserve_keeps_class_owner_balanced` FB:602 | STRONG | Exact totals + invariant 1 on the honest fake. | — | — |
| `owner_hard_cap_denies_before_mutation` FB:620 | STRONG | Typed `Capacity` deny + before==after inertness + `assert_ne!(1_200)`. | — | Mutate before deny check → inertness assert fails. |
| `resource_group_cap_denies_aggregate_over_owners` FB:646 | STRONG | Aggregate-across-owners deny + in-cap positive pair. | — | — |
| `rebuild_window_rejects_writes_then_resumes` FB:681 | STRONG | Reserve, commit, AND release all rejected during the window; writes resume after finish. | — | — |
| `reclaim_yield_zero_frees_nothing` FB:708 | STRONG | Negative path (yield 0 frees nothing, usage unchanged) + positive pair (yield 200: freed==200, usage 300, invariant 1 preserved). Scope limit: single class, single owner — multi-class reclaim cross-axis consistency is NOT covered (gap map G6). | Add a two-class/two-owner reclaim case asserting `total_class_bytes()==total_owner_bytes()` after `reclaim_once()`. | Make `reclaim_once` skip the class-axis subtraction → single-class test stays GREEN (G6); with a multi-class case it goes red. |
| `broken_under_report_corrupts_only_the_reported_snapshot` FB:745 | STRONG | Proves the fault-injection machinery produces exactly the intended lie (owner 400 vs class 500) — a meta-pole ensuring the poles used everywhere else are real. | — | — |

### loom_models.rs (5)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `loom_reserve_vs_reconfigure_preserves_installed_cap` LM:17 | **VACUOUS** (as SUT test) | The body implements a hand-rolled `BudgetState` behind `loom::sync::Mutex` — production code is never referenced (module doc admits this: "they do not claim to execute production mutex or storage implementations"). The named production behavior (broker reserve vs reconfigure) cannot be exercised by this body; the model verifies only itself. | Bind the model to the production authority (e.g. drive `LiveProcessMemoryBroker` reserve/reconfigure under Loom-cfg double-gating), or rename/mark the module as protocol-shape documentation so it is not counted as SUT coverage. | Remove the rebuild/reconfigure gate in the production broker (or break `RebuildFence`) → all 5 loom tests stay green. |
| `loom_ticket_transition_has_single_terminal_owner` LM:63 | **VACUOUS** (as SUT test) | Same: hand-rolled `TicketState` model; no production ticket type. (Model itself is STRONG — Loom enumerates the interleavings and the postconditions are real.) | Same as LM:17. | Break production duplicate-commit rejection → stays green. |
| `loom_close_vs_mutation_never_mutates_after_close` LM:92 | **VACUOUS** (as SUT test) | Hand-rolled `(closed, counter)` tuple; never touches a facade store. | Same as LM:17. | Break store `Closed` gating → stays green. |
| `loom_spill_transfer_never_has_zero_or_double_owner` LM:128 | **VACUOUS** (as SUT test) | Hand-rolled `PayloadOwner` enum; never touches `RoutedSpillController`. | Same as LM:17. | Break spill routing → stays green. |
| `loom_versioned_callback_compensation_is_exactly_once` LM:164 | **VACUOUS** (as SUT test) | Hand-rolled `CallbackState`; the exactly-once assertion (`assert_ne!(callback_won, release_won)`) is real for the model only. | Same as LM:17. | Break production exactly-once compensation → stays green. |

### tests/facade_conformance_bite.rs (4)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `honest_fake_and_all_five_real_stores_pass_lifecycle_oracle` BITE:108 | STRONG | Positive pole for the destructive lifecycle oracle across all five real stores (paired with the three poles below). | — | — |
| `lifecycle_oracle_rejects_leaking_close_fake` BITE:115 | STRONG | Pole: close/drop refunds nothing → ledger≠0 → rejected, localized to ReclaimAccounting. | — | Weaken `close_exactly_once` verdict (S:735) to ignore ledger → red. |
| `lifecycle_oracle_rejects_state_losing_close_fake` BITE:122 | STRONG | Pole: FAILED close discards live state → rejected. | — | — |
| `lifecycle_oracle_rejects_double_refund_fake` BITE:129 | STRONG | Pole: second close refunds again → negative ledger → rejected. | — | — |

### tests/public_api_surface.rs (1)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `fixture_mode_exposes_the_facade_suite_end_to_end` PAS:110 | WEAK | Asserts only that the kit facade suite is runnable and passes (the compile-time `names_*` fns pin the surface). As a behavior oracle it asserts strictly less than the suite it names — no verdict-name/count/coverage assert — so it adds a third positive-only green to the FX:158 seam rather than biting power. | Assert the emitted verdict set (see FX:110 fix) so the surface test also pins coverage. | Remove `run_facade_store_suite` slices → still green (same mutation as FX:158). |

## GAP MAP — invariant → checking site → negative pole? → what slips through

"A1 at suite scale" answer first: **no invariant's oracle computes its expected value from the SUT.** The accounting oracle is a hand-written literal `AccountingView` (S:439-457); the trace/property oracle is an `IndependentAccountingModel` fed by the issued operations (S:1541-1599); prepared/transcript byte costs are independent literal formulas (S:120-126, S:303-307, S:1000-1003, S:1186-1191); the lifecycle oracle compares two INDEPENDENT sources (store-side counter vs broker-side ledger, S:727-732). The one self-referential spot — the advisory check derives `resident_bytes` from the target's own view (S:548-551) — is intentional (differential drift test) and is pole'd by `ObserveMutatesTarget` (S:2956).

### Broker accounting suite (`run_broker_accounting_suite`)

| # | Invariant (verdict name) | Checking site | Negative pole exists? | If missing — what broken fake slips through |
| --- | --- | --- | --- | --- |
| 1 | In-cap reserves succeed (`in_cap_reserves_succeed`) | S:411-421 | **NO** — no test feeds a target that denies the 1_750-byte workload (no over-denying fault exists) | A verifier-weakening of this verdict (predicate → `true`) is caught by nothing. A host broker with caps so tight it denies everything would fail HERE in real runs, but the meta-property "this verdict can fire" is unproven. |
| 2 | Commit results all Ok (`every_commit_result_checked`) | S:432-437 | YES — `CommitSwallowing` (S:1967) fires the family via downstream verdicts | — |
| 3 | Exact independent accounting model (`independent_exact_accounting_model`) | S:439-466 | YES — richest row: BlackHole, WrongContextAxis, WrongChargeKind, WrongClassKind, WrongContextKind, SwapKindAcrossClassBuckets, TransferDoubleOwner, ObservationError + all 3 `AccountingFidelity` variants | — |
| 4 | class == owner (`observed_class_equals_owner`) | S:467-472 | YES — UnderReportOwnerBytes / DivergeClassFromOwner (S:2592, S:2608, CM:127, LIB:69/84) | — |
| 5 | owner == partition sum == context sum (`observed_owner_partitions`) | S:473-478 | YES — `DivergeOwnerPartition(48)` (S:2696) | — |
| 6 | Per-axis kind split agrees (`observed_kind_split_agrees_across_axes`) | S:482-489 | YES — WrongClassKindAxis / WrongContextKindAxis poles assert the verdict BY NAME (S:2013, S:2033) | — |
| 7 | Duplicate commit rejected | S:491-495 | YES — `DuplicateCommitAccepting` (S:1983) | — |
| 8 | Rebuild fences writes (begin ok / during denied / finish ok / post accepted) | S:497-541 | **NO suite-level pole** — no broken target accepts writes during rebuild (`MutateOnDeny` still returns Err; no fault touches `do_begin_rebuild`) | The `RebuildWriteFence` family is NEVER violated in any self-test. A host broker that accepts reserves during rebuild WOULD be caught by this verdict in a real run, but nothing proves the verdict can fire — weakening it (or the whole family) to `true` flips no test. In-trace denial asserts exist only inside S:1745-1747 for the two honest targets. Fix: a `RebuildAcceptingTarget` fake asserting the family fires. |
| 9 | Denied reserve is inert (full view) | S:509-521 | YES — `MutateOnDenyTarget` (S:2838); positive pole S:2854 with non-empty guard | — |
| 10 | Allocator observation inert + drift/mismatch math | S:543-581 | YES — `ObserveMutatesTarget` (S:2956), `ObservationError` (S:2152) | — |
| 11 | Terminal releases Ok / stale release rejected / final exact zero | S:583-606 | YES — `ReleaseSwallowing` (S:1975), `StaleTicketAccepting` (S:2144) | — |

### Prepared-publication suite (`run_prepared_publication_suite`) — fully pole'd

| Invariant | Checking site | Negative pole | Gap |
| --- | --- | --- | --- |
| Replay inert + exact billing | S:127-213 | `UnderbillingPreparedTarget` (S:2246), `BrokenPreparedTarget` (S:2332) | none |
| Payload drift conflicts without stats change | S:214-220 | asserted HELD under the underbilling fault (S:2266) — localization precision; the fired-pole is via `BrokenPreparedTarget` appends | none |
| Abort reclaims exactly once (resolve→abort→replay) | S:221-237 | `BrokenPreparedTarget` (abort always Aborted → replay disposition wrong) | none |
| Root-aware GC preserves references | S:139-203 | `RootBlindPreparedTarget` (S:2409) | none |

### Transcript-append suite (`run_transcript_append_suite`) — fully pole'd

| Invariant | Checking site | Negative pole | Gap |
| --- | --- | --- | --- |
| Exact replay inert + byte oracle | S:343-351 | `UnderbillingTranscriptTarget` (S:2554), `BrokenTranscriptTarget` (S:2486) | none |
| Three collision axes typed | S:352-366 | `BrokenTranscriptTarget` (Ok where Err required) | none |
| Truncate preserves retry tombstone | S:367-376 | `BrokenTranscriptTarget` (truncate loses tombstone → late retry Applied) | none |
| New identity fails closed at retention bound | S:377-390 | wrong-horizon target (S:2497), with all-other-verdicts-held isolation | none |

### Store-lifecycle suite (`run_store_lifecycle_suite`) — pole'd via BITE fakes

| Invariant | Checking site | Negative pole | Gap |
| --- | --- | --- | --- |
| Close refunds exactly once (store==ledger>0 → 0) | S:735-738 | LeakingClose (BITE:115), DoubleRefund (BITE:129) | none |
| Failed close is fail-closed (state kept) | S:752-756 | StateLosingClose (BITE:122) | none |
| Last-owner drop releases | S:765-768 | LeakingClose drop path (ledger ≠ 0) | none |
| Drop during rebuild defers until finish | S:780-784 | LeakingClose (deferred refund skipped → after ≠ 0) | none |

### Facade store slices (`run_facade_store_suite`) — **the big gap**

| Invariant | Checking site | Negative pole exists? | What slips through |
| --- | --- | --- | --- |
| Payload dedup + full refund (`payload_dedup_refund`) | S:1004-1063 | **NO** broken-store pole (honest real store only) | As an SUT test it is strong (independent literal oracle 32+9+64+16=121, ledger AND store counters, final zero). But as conformance infrastructure nothing proves the verdict can fire: a verifier-weakening mutation (predicate → `true`, or swapping the oracle to `store.charged_bytes()`) flips no test. A host store whose dedup double-bills would be caught in a real run, but the meta bite is unproven. |
| Pin-safe delete (`pin_safe_delete`) | S:1064-1086 | PARTIAL — `faults::drive_pin_invalidation_failure` (FS:213) exercises pinned-delete containment on a real store, but through a DIFFERENT driver, not this verdict | Weakening this verdict is caught by nothing in the suite; only the faults test covers the behavior, and it does not run the verdict. |
| Retained-map overwrite refund + sweep (`retained_map_refunds`) | S:1163-1179 | **NO** | Same class: real-store positive only. A sweep that reports `reclaimed_bytes` without freeing (or frees without reporting) IS caught here in a real run (ledger+committed compared, surviving keys checked), but the verdict's firing is unproven. |
| Atomic recharge overwrite (`retained_atomic_recharge_overwrite`) | S:1134-1161 | **NO** | The tight-cap construction is exactly right for localizing a non-atomic charge-then-refund regression, but no broken store is fed through it. Verifier-weakening undetected. |
| Transcript truncate refund (`transcript_truncate_refund`) | S:1264-1273 | **NO** at slice level (the trait-level `run_transcript_append_suite` has poles, but this slice drives the real store with its own literal oracle) | Same class. |
| Transcript over-cap fail-closed (`transcript_over_cap_fail_closed`) | S:1274-1286 | **NO** at slice level (denial + zero ledger + zero retained + item_count 0 all asserted positively) | Same class. |
| Spill round-trip 1→0 (`spill_round_trip`) | S:1295-1314 | **YES** — `DroppingSpillTarget` (S:3182), including the between-spill-and-restore inventory capture | none |
| Spill unavailable fails closed (`spill_unavailable_fail_closed`) | S:1317-1337 | PARTIAL — FS:227/239/282/317 exercise the same fail-closed path through the faults classifier | Slice-verdict firing unproven. |

### Cross-cutting seams (G-list)

| # | Gap | Evidence | Consequence |
| --- | --- | --- | --- |
| G1 | Kit conformance entry positive-only | `MemoryTestKit::run_conformance` (FX:59) exercised only by FX:119/FX:137, both positive; no fidelity knob at kit level | Stubbing the kit entry to `Passed` keeps all 87 tests green (FAILOPEN FX:119). |
| G2 | Kit facade-store entry positive-only + slice coverage unasserted | FX:67 driven by FX:158, BITE:111, PAS:110 — all `assert!(passed())`; no verdict-name/count assert | Deleting slice sub-suites from `run_facade_store_suite` (S:683-686) keeps everything green (FAILOPEN FX:158). `from_assertions` empty-guard (CM:140) catches only total removal. |
| G3 | Fault-driver Escaped branches: only SPILL is self-verified | spill has 3 poles (FS:239/282/317); reclaim (S:58-82), pin (S:88-124), rebuild (S:130-152) drivers have no rubber-stamp pole | A mutation making any of those three drivers unconditionally `Contained` is caught by nothing. The spill driver shows the intended pattern — replicate it. |
| G4 | `RebuildWriteFence` family never violated in any self-test | no fault accepts writes during rebuild (see broker-suite row 8) | Fence-verdict weakening undetected. |
| G5 | Partial suite-emptying passes the fold | `ConformanceVerdict::from_assertions` only fails on TOTAL emptiness (CM:38-44) | Combined with G2: any conformance runner can silently drop checks. Assert the expected verdict-name set at the public entries. |
| G6 | Fake `reclaim_once` multi-class cross-axis consistency untested | FB:708 single class/owner; S:433-512 subtracts `freed` from class buckets in BTreeMap order regardless of which class the largest owner's bytes sat in | A fake bug that desyncs `total_class_bytes` from `total_owner_bytes` after reclaim with ≥2 classes slips through every test (the conformance suites never call `reclaim_once`). |
| G7 | `fake/views.rs` (`FakeRuntimeViewFactory`, `RejectingScratchAllocator`) — ZERO tests anywhere in the crate | its documented honesty contract ("deliberately rejects scratch allocation") is asserted nowhere | A regression that makes the fake runtime view GRANT scratch authority (the exact authority-leak the module exists to prevent) is caught by nothing. |
| G8 | `doctor/mod.rs` — zero tests | `MemoryDoctorReport::is_clean` untested | trivial data model; low risk, but the `doctor` feature ships untested code. |
| G9 | Loom models exercise no production code | LM:11-6 doc admits model-only; `loom` also not in `full` feature set | Production concurrency regressions (reserve-vs-reconfigure, exactly-once compensation, close-vs-mutation) have NO loom-level SUT coverage; the 5 "concurrency tests" green-light broken production code (VACUOUS rows). |

### fake/broker.rs — "if the fake has a bug, which suite test catches it?"

Caught (reserve/commit/release/view/snapshot semantics, the paths every conformance run uses):
- `deterministic_trace_matches_independent_model` (S:1686) — fake and real broker both replayed against an op-trace model; any drift in the fake's accounting axes goes red.
- `property_trace_matches_independent_model` (S:1906) — 128 random traces, grouped axis included.
- `cap_deny_agrees_between_honest_fake_and_real_broker` (S:3058) — differential deny/inertness agreement.
- `both_targets_still_pass_the_portable_suite_after_strengthening` (S:3133) + all `honest_fake_passes_*` positives.
- The broken-snapshot machinery itself is verified by FB:745 (the lie is exactly as intended).

NOT caught (fake bugs with no witness):
- Multi-class `reclaim_once` class/owner desync (G6).
- `pressure()` beyond the fixtures plumbing pair (FX:127) — pressure never affects any accounting assertion.
- `FakeReservation` accessor surface — trivial.
- Nothing catches a fake bug in code the suites never call (`reclaim_once` non-zero yields, group release preflight is covered by the grouped proptest).

## Footer — 3-line risk summary

1. The broker-accounting, prepared-publication, transcript, and store-lifecycle suites have genuine, localized negative poles (23+ distinct broken fakes); their oracles are independent of the SUT — the core "conformance must bite" contract is real for those four families.
2. The residual fail-open surface is concentrated at the seams: kit entries are positive-only (G1/G2), 6 of 8 facade-slice verdicts and the `RebuildWriteFence` family are never proven able to fire (G4/G5), 3 of 4 fault drivers lack rubber-stamp poles (G3), and the 5 loom "concurrency" tests verify hand-rolled models, not production code (G9).
3. Cheapest high-value fixes: assert the expected verdict-name set at `run_facade_store_suite`/`run_conformance` entries, add a `RebuildAcceptingTarget` + reclaim/pin/rebuild driver poles (mirror the spill pattern), and bind loom models to production authorities or stop counting them as coverage.

---
*Auditor T9, 2025-09-26. Read-only audit; no SUT code was modified. Verification run: `cargo test -p quanta-memory-platform-testkit --all-features` → 87 passed / 0 failed.*
