# Verification Scenarios — quanta-memory-platform

What must be **verified** in this repo, derived from the contracts and invariants
(ledger SSOT, policy/config validation, control plane, scheduler, facade stores,
testkit conformance, extraction rails).

This is a living checklist. Correctness scenarios are **tests** (not benchmarks);
performance/allocation regressions are guarded separately (see *Test vs Bench*).

## Scope and evidence status

This document is a source-level contract catalogue, not a release or deployment
receipt. Hard pass/fail counts go stale fast and are deliberately **not** pinned
here — re-run the rail (see *How evidence was gathered / re-verify*) for current
results. Each scenario below cites the exact `#[test]` fn name and its file as
its own evidence; trust those citations over aggregate counts.

Repo-local success establishes the portable library's source, feature, and
standalone-extraction contracts only. Semantica host adoption, deployment, and
activation require exact-source evidence outside this repository.

## Legend

| Mark | Meaning |
| --- | --- |
| ✅ | covered — evidence (test name + file) listed |
| 🚧 | partially covered — some sub-cases done, residual noted |
| ⚠️ | **open** — not yet implemented (residual / next tranche) |
| 🔬 | property-based / deterministic-fuzz |
| 🧵 | concurrency |
| 🔢 | numeric / boundary |

## Test vs Bench split (how to manage these)

Correctness is managed as **tests**; benchmarks answer a different question
(speed, not correctness) and never host a correctness scenario.

- **~70% unit / module tests** (`#[cfg(test)]` in the owning module) — invariants,
  fail-closed, validation. Fast, localizing.
- **~25% e2e / conformance** (broker surface, testkit conformance, cross-crate) —
  emergent gaps only visible end-to-end (e.g. F#3 facade↔ledger).
- **~5% benchmarks** — `scripts/run-scaling-bench.sh` executes public
  store/broker paths with repeated latency/throughput distributions and a real
  tracking allocator. `scripts/verify-scaling-results.py` owns the executable
  absolute and optional same-host baseline thresholds; its negative fixtures
  prove drift and regressed results are rejected. Correctness remains in the
  owning unit/conformance suites.

---

## A. Ledger accounting invariants — ✅ (`core/src/domain/ledger.rs`)

Impl hardening landed: per-ledger **epoch** (`LEDGER_INSTANCE`) binds tickets to
their issuing ledger; snapshot totals and mutation preflights use checked
arithmetic; reserve/release use `u64` add/sub (the unsound `bytes as i64` cast
is gone). Hot admission uses exact aggregates and affected slots rather than
cloning all live owner/context maps.

- ✅ Inv 1/2 + fail-closed — covered here and at the broker surface (group C/R).
- ✅ Inv 5 global limit + headroom band — `reserve_denied_when_global_hard_limit_would_be_exceeded`, `global_ceiling_reserves_headroom_for_non_headroom_classes`.
- ✅ Inv 6 shared burst pool — `borrow_over_class_cap_is_bounded_by_shared_burst_pool`, `shared_burst_pool_is_shared_across_borrowing_classes`, `borrow_false_still_denies_over_cap_regardless_of_pool`.
- ✅ Inv 7 idempotency — `double_release_of_a_cloned_ticket_is_rejected_and_does_not_corrupt`, `commit_after_release_is_rejected`.
- ✅🔢 Zero-byte / u64 boundary / i64 cast — `zero_byte_reserve_commits_and_releases_coherently`, `snapshot_validation_rejects_unrepresentable_axis_instead_of_saturating_it`, `reserve_above_i64_max_does_not_wrap_to_negative_delta`.
- ✅ Cross-ledger ticket — `ticket_from_one_ledger_cannot_release_on_another`; id uniqueness — `ticket_ids_are_monotonic_and_unique_across_many_reserves`.
- ✅ Context axis balance — `context_axis_balances_with_owner_after_mixed_sequence`; wrong-kind release — `mixed_kind_release_preserves_charge_kind_partition`.
- ✅🔬 Deterministic fuzz — `deterministic_fuzz_preserves_invariants_and_returns_to_zero` (400-step sequence, invariants 1/2/5/6 at every step, drains to 0).

## B. Policy / config / presets — ✅ (`core/src/domain/{policy,config}.rs`)

Impl: `validate_residency` now also validates `resource_groups`, per-axis
`hard <= global`, and owner→group mappings; `ExecutionBudget` exposes admission
check helpers that the control plane enforces (see group C/D).

- ✅ validate hardening — `validate_rejects_class_hard_cap_above_global`, `validate_rejects_owner_hard_cap_above_global`, `validate_rejects_constructed_headroom_above_global`, `validate_accepts_headroom_equal_to_global`.
- ✅ resource_groups — `validate_accepts_well_formed_resource_group`, `validate_rejects_resource_group_reserved_above_soft`, `validate_rejects_resource_group_soft_above_hard`, `validate_rejects_resource_group_hard_above_global`, `validate_rejects_resource_group_burst_percent_over_100`, `validate_accepts_resource_group_burst_percent_boundaries`, `resource_group_reserved_has_an_independent_global_floor`.
- ✅ owner→group mapping validated — `validate_rejects_owner_group_mapping_to_unknown_group` (`core/src/domain/config.rs`).
- ✅ presets — `for_preset_{populates_every_mandatory_class,is_structurally_valid,class_caps_isolate_below_global,scales_caps_with_global_limit,weighted_split_never_over_allocates_distributable,headroom_class_owns_exactly_the_reserved_band,no_single_class_cap_plus_headroom_exceeds_global}`.
- ✅🔢 edges — `default_residency_global_one_does_not_underflow_and_validates`, `default_residency_small_globals_all_validate`, `borrow_flag_with_zero_shared_pool_is_valid_but_grants_no_burst`.
- ✅ override resolver/provenance — `higher_precedence_layer_wins_and_is_recorded_in_provenance`, `removing_top_layer_changes_effective_value`, `empty_layer_is_skipped_in_provenance`.
- ✅ **ExecutionBudget admission helpers** — `check_helpers_admit_within_cap_and_reject_over_cap`, `unlimited_budget_admits_everything` (`core/src/domain/policy.rs`); the cap is enforced at the control surface (see group C scratch enforcement).

## C. Control plane / live broker — ✅ (`core/src/api/{control,builder}.rs`)

- ✅ reconfigure atomicity — `denied_reconfigure_leaves_scheduler_controller_and_report_unchanged`.
- ✅ reconfigure accept rebuilds — `accepted_reconfigure_rebuilds_controller_to_new_global_limit`.
- ✅ pressure boundaries — `pressure_thresholds_are_inclusive_at_exact_boundary_fractions`.
- ✅ rebuild re-entrancy — `rebuild_is_reentrant_double_begin_single_finish_then_idle_finish_is_noop`.
- ✅ trait report not fabricated on poison — `trait_effective_report_matches_inherent_and_is_not_fabricated`.
- ✅ build() validates — `build_with_invalid_residency_returns_config_error`, `build_with_soft_above_hard_returns_config_error`, `build_without_manifest_returns_build_error`, `build_with_valid_residency_succeeds_and_reports_that_residency`.
- ✅ reconfig deny rules + pressure-from-semantic + advisory-non-mutating — existing `live_reconfig_*`, `pressure_classifies_from_semantic_usage`, `allocator_advisory_mismatch_does_not_mutate_accounting`.
- ✅ **scheduler → reserve admission (fail-closed)** — `reserve_gated_by_resource_group_admission_fail_closed`, `owner_without_group_mapping_is_not_gated_by_admission` (`core/src/api/control.rs`). Owner→group mapping now gates a `reserve`: a group-admission denial blocks the reserve and leaves accounting untouched; an unmapped owner is not gated.
- ✅ **ExecutionBudget scratch enforcement** — `execution_scratch_budget_caps_live_scratch_and_spares_other_kinds` (`core/src/api/control.rs`); the live-scratch cap admits within budget and denies over it without disturbing other charge kinds.
- ✅ **Enforced default + explicit report-only product** — core/facade `build()`
  materialize live accounting authority; `build_report_only()` returns only the
  effective report. `default_builder_over_cap_store_mutation_is_denied` and
  `default_builder_and_report_only_share_override_resolution`
  (`facade/tests/public_api_surface.rs`) pin typed cap denial and canonical
  override parity.
- ✅ **Broker-bound runtime scratch** —
  `live_runtime_scratch_charge_appears_in_ledger_snapshot`,
  `live_runtime_scratch_over_cap_is_denied_without_mutation`,
  `runtime_scratch_and_direct_broker_charge_share_one_limit`,
  `runtime_scratch_release_restores_resource_group_capacity`,
  `dropping_runtime_scratch_during_rebuild_returns_capacity`,
  `runtime_scratch_rejects_foreign_runtime_context_without_mutation`, and
  `poisoned_runtime_scratch_explicit_release_preserves_then_drop_recovers_authority`
  (`core/src/api/control.rs`). Scratch is a pending capacity permit: rebuild
  fences committed/live writes but permits exact rollback of incomplete pending
  authority so drop cannot strand ledger or group capacity.

## D. Scheduler — ✅ (`core/src/application/scheduler.rs`)

- ✅ edges — `empty_scheduler_has_zero_share_and_denies_all`, `unknown_group_denied_even_with_pool_available`, `weighted_share_distributes_remainder_work_conservingly`, `usage_release_round_trip_readmits_burst` (+ existing starvation/admission tests).
- ✅ **wired into reserve** — owner→group mapping gates a `reserve` at the broker surface; admission denial blocks the reserve fail-closed. Evidence: `reserve_gated_by_resource_group_admission_fail_closed`, `owner_without_group_mapping_is_not_gated_by_admission` (`core/src/api/control.rs`); mapping validation `validate_rejects_owner_group_mapping_to_unknown_group` (`core/src/domain/config.rs`).

## E. Pressure controller — ✅ (`core/src/application/controller.rs`)

- ✅ monotonicity enforced — `thresholds_new_accepts_monotonic_and_default_is_valid`, `thresholds_new_rejects_inverted_or_out_of_range`.
- ✅ zero-limit degenerate — `zero_limit_is_normal_when_empty_and_fatal_when_used`.
- ✅ ladder both ways + idempotent — `ladder_transitions_up_then_down_through_every_rung`, `update_from_semantic_is_idempotent`, `allocator_snapshot_observation_does_not_change_pressure`.
- ✅ reclaim ladder — `reclaim_returns_zero_under_fatal_pressure_without_spill`, `reclaim_uses_spillable_under_fatal_pressure`, `first_priority_owner_does_not_self_reclaim_below_fatal`.

## F. Facade stores — ✅ (`facade/src/*`)

- ✅ payload_store round-trip/dedup/pin/delete — `canonical_put_deduplicates_and_charges_once`, `dedup_hit_preserves_first_context_and_stores_no_second_entry`, `delete_unpinned_releases_bytes_exactly_once`, `pin_is_metadata_only_and_pinned_delete_is_rejected`.
- ✅ **F#3 for payload_store** — `charged_store_bytes_flow_through_the_ledger_and_drain_to_zero`, `charged_payload_over_cap_put_is_fail_closed`, `uncharged_store_is_explicit_non_budgeted_mode` (a "charged store" mode routes bytes through the broker ledger).
- ✅ retained_map hash-indexed point operations + accounting/order preservation — `put_get_remove_get_none_round_trip_corrects_bytes`, `equal_hash_key_duplicate_overwrites_and_does_not_double_count`, `first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1`, `overwrite_preserves_original_insertion_position_for_newest_first_sweep`, `arbitrary_remove_preserves_remaining_newest_first_sweep_order`, `charged_sweep_during_rebuild_is_fail_closed`, and `keys_lists_inserted_keys_in_deterministic_sorted_order`. The public key identity law is `Eq + Hash`; exact lookup/overwrite/remove use the canonical linked hash container while enumeration remains deterministically key-sorted.
- ✅ **F#3 for retained_map** — retained_map is ledger-wired only through the typestate builder: binding produces `BoundRetainedMapStoreBuilder`, only the bound state exposes infallible `build()`, and `SimpleRetainedMapStore` has no public late-attach transition. Evidence: the two compile-fail docs plus `retained_map_platform_builder_routes_bytes_through_the_ledger`, `charged_insert_routes_bytes_through_the_ledger`, `charged_insert_over_cap_is_denied_and_fails_closed`, `charged_overwrite_denial_leaves_existing_entry_unchanged`, and `charged_sweep_refunds_reclaimed_bytes_to_the_ledger`. Internal ticket corruption is also fail-closed before refund or container mutation (`charged_remove_rejects_a_ticketless_entry_without_mutating_state`, `charged_sweep_rejects_a_ticketless_candidate_without_partial_refund_or_eviction`). The uncharged `new` path remains distinct via `uncharged_retained_map_bytes_are_not_reflected_in_a_broker_ledger_snapshot`.
- ✅ revocable FIFO/revoke/eviction — `reclaim_one_is_fifo_and_releases_its_bytes`, `revoke_makes_entry_unretrievable_and_decrements_accounting`, `reclaim_eviction_order_is_deterministic_fifo_at_capacity_boundary`.
- ✅ publish_pipeline commit transition — `default_pipeline_runs_stage_then_commit_each_exactly_once_no_rollback`, `failed_commit_never_publishes_and_never_increments_count`.
- ✅ CanonicalKey/MeasuredValue — `cloning_measured_arc_shares_the_allocation_not_a_deep_copy`, `measured_value_charges_caller_supplied_size_not_inner_len`, `cloning_measured_value_preserves_size_and_value_context`.
- ✅ spill routing fail-closed — `routed_spill_to_available_target_succeeds_and_counts`, `spill_to_unavailable_target_fails_closed_and_does_not_count`, `routed_spill_after_target_goes_offline_fails_closed_leaving_inventory_unchanged`.
- ✅ **spill write → restore (inverse) round-trips** — the restore inverse landed: `in_memory_target_write_then_restore_round_trips_the_payload`, `in_memory_target_restore_of_missing_handle_is_none_not_an_error`, `in_memory_target_restore_after_offline_fails_closed_leaving_retained_state_unchanged` (`facade/src/spill/service.rs`).
- ✅ transcript per-session isolation + charged mode — `append_identity_is_partitioned_by_session`, `retention_stats_separate_active_evidence_and_session_costs`, and (charged) `exact_retry_charges_once_and_truncate_refunds_only_active_body`, `quota_denial_leaves_row_indexes_and_charge_unchanged` (`facade/src/transcript_store/service.rs`).
- ✅ charged facade mutators are fail-closed before store mutation — `charged_remove_during_rebuild_is_fail_closed`, `charged_sweep_during_rebuild_is_fail_closed`, `reclaim_one_during_rebuild_is_fail_closed`, `reclaim_until_during_rebuild_is_fail_closed`, `truncate_refund_failure_preserves_rows_indexes_and_accounting`.
- ✅ transcript charged breadth now matches the closeout bar — `append_identity_is_partitioned_by_session` proves session-local identity partitioning, and `truncate_refund_failure_preserves_rows_indexes_and_accounting` proves refund denial leaves both ledger and store untouched.

**publish_pipeline is intentionally not charged.** It is a non-retaining
*transition* stage (zero bytes held): it transforms a staged value into a
published one and hands it off to the store that retains it. The destination
store (payload / retained-map / transcript) is what charges those bytes; wiring
a charger into the pipeline would double-count. This is a deliberate design
choice documented in `facade/src/publish_pipeline/service.rs`, not a gap.

## G. Testkit conformance — ✅ ("the suite must bite", `testkit/`)

- ✅ ReserveFailClosed bites — `suite_rejects_target_that_mutates_accounting_on_a_denied_reserve`, `honest_fake_passes_the_strengthened_fail_closed_check`.
- ✅ commit→release both targets — `commit_then_release_returns_balance_to_zero_on_honest_fake`, `..._on_real_broker`.
- ✅ cap-deny agreement — `cap_deny_agrees_between_honest_fake_and_real_broker`.
- ✅ every break variant rejected — `suite_rejects_every_accounting_fidelity_break_variant`, `honest_fidelity_is_the_only_variant_the_suite_accepts`, `public_suite_rejects_every_break_variant_and_localizes_it`.
- ✅ faults deterministic — `reclaim_returns_zero_is_contained`, `pinned_delete_failure_is_contained_and_bytes_survive`, `rebuild_race_write_is_rejected`, `spill_failure_is_contained_without_counting`.
- ✅ alloc probe — `recorded_signals_accumulate_and_net_live_is_correct`, `dealloc_exceeding_alloc_saturates_at_zero_not_nonsense`, `signals_feed_an_invariant_assertion`.
- ✅ contract-family taxonomy is stable/unique — `every_family_has_a_unique_stable_label` (`testkit/src/conformance/invariants.rs`); the `ContractFamily` enum carries `PinDiscipline`/`ReclaimAccounting`/`SpillRouting`/`AllocatorAdvisory`.
- ✅ **`FACADE_STORE_SUITE` runner** — the declared suite now runs through the live-broker-backed `MemoryTestKit` harness (`run_facade_store_suite`, `ConformanceSuite::run_facade_store_suite`, `fixtures::tests::testkit_runs_facade_store_suite_against_live_broker_and_passes`).
- ✅ **Pin/Reclaim/Spill families are exercised by a running suite** — the facade suite now drives `PinDiscipline`, `ReclaimAccounting`, and `SpillRouting` through real portable stores (`payload_dedup_refund`, `pin_safe_delete`, `retained_map_refunds`, `transcript_truncate_refund`, `spill_round_trip`, `spill_unavailable_fail_closed` in `testkit/src/conformance/suites.rs`).
- ✅ **`AllocatorAdvisory` is now a folded conformance verdict** — the broker suite classifies inert allocator observation under `AllocatorAdvisory` (`allocator_observation_is_inert` in `testkit/src/conformance/suites.rs`), and `suite_rejects_target_that_mutates_on_allocator_observation` proves the verdict bites on dishonest targets.

## H. Extraction / portability rails — ✅ (`scripts/`, root manifests)

- ✅ static preflight: dependency direction, host-noun boundary, clean-source
  manifest binding, and an executable standalone extraction procedure.
- ✅ **Per-feature build matrix** — `scripts/verify-feature-matrix.sh` runs clippy
  `-D warnings` across default, bare, every leaf feature, `full`, and
  all-features, and rejects facade no-default graphs that activate core
  `default`/`live_control` through dependency defaults.
- ✅ **Public API surface snapshot** — re-export regression guard: `crates/quanta-memory-platform-core/tests/public_api_surface.rs`.
- ✅ **Self-hosting extracted set** — `scripts/lib/self-hosting-manifest.sh` and
  `scripts/extract-to-standalone-repo.sh` assert the canonical extracted file
  set before export and in the extracted destination.

## J. Concurrency 🧵 — ✅ (`LiveProcessMemoryBroker` = `Arc<Mutex>`, `Clone`)

- ✅ **Concurrent reserve, no TOCTOU** — `concurrent_reserves_never_exceed_global_limit_no_toctou` (`core/src/api/control.rs`): total never exceeds the global limit under N threads.
- ✅ **Clone consistency** — `broker_clones_share_one_ledger` (`core/src/api/control.rs`): cloned handles observe one shared ledger.
- ✅ **Send + Sync** — `broker_is_send_and_sync` (`core/src/api/control.rs`): the broker is safe to share across threads.
- ✅ **Lock-poison isolation under a real thread race** — `poisoned_broker_fails_closed_and_trait_report_stays_truthful` proves every public mutation/observation path returns `InvalidState` without fabricating fallback report truth.
- ✅ **Concurrent reserve + reconfigure consistency** — `concurrent_reserve_and_reconfigure_preserve_group_cap_truth` proves repeated accepted raises do not reopen overflow admission.
- ✅ **Live owner-group remap is fail-closed while bytes are live** — `remapping_live_owner_group_is_denied_fail_closed` rejects the control-plane change until those bytes drain.

## K. Numeric / safety 🔢 — ✅

- ✅ ledger numeric edges — see group A (checked aggregate folds, u64 add/sub, i64-cast regression guard).
- ✅ controller numeric coverage — `tiny_non_zero_limit_saturates_percent_without_wrapping` and the existing zero-limit / ladder tests cover tiny-limit and `u64::MAX` semantic usage edges without wrap.
- ✅ scheduler numeric coverage — `huge_pool_and_weights_compute_a_non_wrapping_fair_share` and `repeated_exact_usage_release_round_trip_never_wraps` cover huge-pool/huge-weight share math and repeated exact usage/release round-trips.

## L. Telemetry adapters — ✅ (`core/src/adapters/*`, `core/src/ports/telemetry.rs`)

- ✅ advisory correctness — `advisory_flags_mismatch_and_reports_semantic_as_authority`, `compare_reports_signed_drift_and_mismatch_boundary`, `compare_semantic_bytes_is_ssot_not_allocator`, `missing_allocator_data_is_not_treated_as_mismatch`.
- ✅ observation-only invariant — `allocator_telemetry_trait_surface_is_observation_only`, `trait_path_absent_allocator_data_never_contradicts_ssot`, `emitter_holds_no_accounting_state`.
- ✅ hdrhistogram adapter — `records_deterministic_set_and_reads_back_count_and_percentiles`, `invalid_bounds_return_error_not_panic`, `out_of_range_record_is_rejected_and_does_not_mutate_count`.

## M. Experimental unbudgeted pin registry — ✅ (`core/src/{ports,adapters}/pin.rs`)

- ✅ It is absent from default builds and available only as the explicitly named
  `UnbudgetedPinRegistry` behind the non-default `unbudgeted_pin` feature.
- ✅ poison fail-closed (no panic) — `poisoned_lock_is_fail_closed_not_panicking`.
- ✅ no over-count + round-trip — `duplicate_pin_does_not_over_count_dependent`, `distinct_dependents_each_increment_count`, `pin_invalidate_round_trip_tracks_dependent_count`, `invalidation_outcome_reports_exact_dependents`.

## N. Scratch allocator — ✅ (`core/src/ports/scratch.rs`)

- ✅ misuse resistance — `grow_after_release_is_rejected_and_fail_closed`, `double_release_is_rejected_not_silent`, `grow_beyond_reserved_is_fail_closed_capacity_error`.
- ✅ charge-kind + lifecycle — `reservation_maps_to_live_scratch_charge_kind`, `lifecycle_reserve_grow_release_succeeds_in_order`.
- ✅ broker-backed authority — live runtime coverage is listed in group C; the
  concrete port implementation lives in `core/src/adapters/scratch.rs`, not in
  the port or API module.

## R. Cross-cutting contracts — ✅ (`crates/quanta-memory-platform-core/tests/policy_contracts.rs`)

New invariants promoted to the public API boundary:
- ✅ `global_hard_limit_denies_reserve_through_public_broker_and_is_fail_closed`
- ✅ `headroom_band_is_reserved_for_headroom_class_at_public_surface`
- ✅ `shared_burst_pool_bounds_borrowing_through_public_broker`
- ✅ `non_borrowing_class_cannot_use_burst_pool_at_public_surface`
- ✅ `double_release_of_a_cloned_ticket_is_rejected_and_does_not_corrupt`
- ✅ `commit_after_release_is_rejected`
- ✅ `preset_class_caps_bite_at_public_reserve_surface`
- ✅ `preset_broker_global_limit_is_consistent_with_enforcement`

---

## Local verification status — 🚧

- Standalone implementation is locally covered by default/bare/leaf/full/all
  warning-free feature slices, non-zero runtime capability tests for every
  advertised facade feature path, runtime scratch owner tests, public surface
  compile-fail tests, examples, and performance gates. Full standalone export
  proof requires executing `scripts/extract-to-standalone-repo.sh <dest>` on
  the exact clean source under qualification; `--dry-run` establishes only the
  source-side preflight.
- `scripts/verify-doc-test-selectors.py` resolves the selectors named by this
  document against `cargo test --workspace --all-features -- --list`; stale or
  zero-test documentation is a release-gate failure.
- External Semantica host adoption, deployment, and activation remain separate
  authorities. Do not infer those closures from this file.
- Charged facade mutations are now uniformly fail-closed: charge via `reserve_committed`,
  overwrite via `recharge`, and refund-bearing delete/reclaim/truncate paths via
  fallible refund-before-mutate APIs.
- Live reconfigure preserves scheduler truth for accepted raises and rejects
  owner-group remaps while the owner still holds live bytes.
- `AccountingLedger` is intentionally non-`Clone`, so tickets stay bound to one
  live ledger epoch.

## How evidence was gathered / re-verify

```bash
scripts/check-workspace.sh
# inspect the evidence test names per file:
rg -n '^\s*fn [a-z0-9_]+\(' crates --glob '*.rs'
```
