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
For the 2026-09-24 implementation and qualification boundary, see the
[current plan index](plans/README.md) and the
[source-bound receipt](plans/sep-23-followup-misc/RECEIPT.md).

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
  prove drift and regressed results are rejected. Retained store/broker rows
  must report nonzero allocator activity, and a nonzero allocation total must
  have a consistent high-water mark. Scheduler admission may legitimately
  allocate zero bytes because its epoch starts after fixture construction.
  Correctness remains in the
  owning unit/conformance suites. The scheduler-admission row retains its
  **combined** timed-call count, throughput and `p50/p95/p99_ns`,
  and also reports `admitted_*_ns` and `denied_*_ns` percentiles from the same
  timed calls. New rows require both distributions. Historical v3 baselines
  without the split compare aggregate latency only; outcome-specific regressions
  require a baseline carrying both outcome distributions.
  The revocable fill-and-drain 1k→10k complexity gate compares the median
  per-operation p95 across repetitions (maximum 4×). Wall elapsed and throughput
  remain reported and baseline-comparable, but are not used for that complexity
  gate because host scheduling gaps between calls are not store-operation cost.
  The historical standard profile remains 48 smoke / 220 full rows. The
  opt-in `--profile=in-memory-pressure` profile uses the in-memory `full`
  feature set (no Redb) and requires 9 smoke / 40 full rows: one-byte
  cap denial→refund→readmission with separate admitted/denied tails, plus
  two charged stores writing through one broker at 1/8/32 threads. The
  pressure-cycle wall throughput includes the untimed refund between measured
  admission calls; it is not a pure reserve-only throughput figure.

  Ordinary JSONL validation proves measurement shape and absolute guards, not
  an approved performance SLO. For a release performance comparison, collect
  **three independent canonical full-process receipts per baseline and candidate** using
  `scripts/run-scaling-bench.sh --profile=in-memory-pressure` (or the unchanged
  standard profile), then invoke `scripts/verify-scaling-results.py --release`
  with three `--baseline` and three `--input` paths. Supply every
  `--release-*` limit explicitly from the consumer's approved policy. The release
  comparison requires the same `scaling.rs` benchmark-source SHA-256 on both
  sides; receipts collected before this fingerprint was recorded cannot serve
  as release baselines. The contention wall timer starts before workers leave
  the start barrier, so all worker operations are included in throughput.
  The release comparator takes each process's slowest repetition for throughput,
  highest repetition p99 (including admitted/denied tails), and highest Busy
  retry rate, then compares the median of three independent processes. A
  one-process spike below the absolute cap but beyond the run-noise bound is
  `INCONCLUSIVE`; recurrence across processes can fail the regression limit.
  The verifier rejects mixed source/toolchain/feature/machine runs, missing or
  reused run IDs, a baseline with the candidate's source tree, incomplete rows,
  duplicate JSON object keys (including nested keys), an absent/nonzero
  benchmark exit status, accounting drift, and threshold violations. Duplicate
  keys are rejected before metric validation so different JSON parsers cannot
  disagree on a receipt's SHA, exit status, or measured value.
  Recorded per-CPU host load or cross-process timing spread above policy yields
  exit 2 (`INCONCLUSIVE`), never GREEN. Exit 1 is invalid evidence or a
  measured regression; exit 0 covers only the selected local performance
  profile, not host activation or correctness E2E. Daily correctness remains
  owner-local/E2E; the release benchmark is a separate clean-source rail.

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
- ✅ **Independent management bounds** — `zero_byte_tickets_obey_independent_live_ticket_cap_fail_closed`, `owner_key_byte_limit_is_exact_and_denial_is_fail_closed`, `validate_rejects_overlong_owner_and_resource_group_identifiers`, `overlong_owner_is_denied_before_reclaim_or_accounting`, and `deferred_committed_cleanup_is_batched_and_drains_large_backlog`. Ticket cardinality is bounded independently of byte usage; deferred backlog is a disjoint subset of those live tickets and each lock-held cleanup batch is bounded.
- ✅ **Replacement at the ticket-count ceiling** — `committed_replacement_at_live_ticket_cap_is_a_net_zero_authority_swap` (`core/src/domain/ledger.rs`) denies an oversized replacement with an identical ledger postimage, then swaps a committed 0-byte ticket for a 1-byte ticket without exceeding the live-ticket ceiling. It rejects reuse of the consumed ticket and drains every peer.
- ✅ Cross-ledger ticket — `ticket_from_one_ledger_cannot_release_on_another`; id uniqueness — `ticket_ids_are_monotonic_and_unique_across_many_reserves`.
- ✅ Context axis balance — `context_axis_balances_with_owner_after_mixed_sequence`; wrong-kind release — `mixed_kind_release_preserves_charge_kind_partition`.
- ✅🔬 Deterministic fuzz — `deterministic_fuzz_preserves_invariants_and_returns_to_zero` (400-step sequence, invariants 1/2/5/6 at every step, drains to 0).
- ✅ **Per-axis kind-split validation** —
  `sep23_fup004_honest_mixed_kind_snapshot_validates`,
  `sep23_fup004_class_kind_swap_preserving_totals_is_rejected`,
  `sep23_fup004_context_kind_swap_preserving_totals_is_rejected`,
  `sep23_fup004_owner_kind_swap_preserving_totals_is_rejected`,
  `sep23_audit_admitted_recharge_moves_kind_vectors_exactly`, and
  `sep23_audit_batch_release_moves_kind_vectors_exactly`
  (`core/src/domain/ledger.rs`). Scalar totals are not enough: the
  per-kind split must agree on every axis, and recharge/batch moves it
  exactly.

## B. Policy / config / presets — ✅ (`core/src/domain/{policy,config}.rs`)

Impl: `validate_residency` now also validates `resource_groups`, per-axis
`hard <= global`, and owner→group mappings; `ExecutionBudget` exposes admission
check helpers that the control plane enforces (see group C/D).

- ✅ validate hardening — `validate_rejects_class_hard_cap_above_global`, `validate_rejects_owner_hard_cap_above_global`, `validate_rejects_constructed_headroom_above_global`, `validate_accepts_headroom_equal_to_global`.
- ✅ resource_groups — `validate_accepts_well_formed_resource_group`, `validate_accepts_resource_group_reserved_equal_to_soft_cap`, `validate_rejects_resource_group_reserved_above_soft`, `validate_rejects_resource_group_soft_above_hard`, `validate_rejects_resource_group_hard_above_global`, `validate_rejects_resource_group_burst_percent_over_100`, `validate_accepts_resource_group_burst_percent_boundaries`, `resource_group_reserved_has_an_independent_global_floor`.
- ✅ owner→group mapping validated — `validate_rejects_owner_group_mapping_to_unknown_group` (`core/src/domain/config.rs`).
- ✅ presets — `for_preset_{populates_every_mandatory_class,is_structurally_valid,class_caps_isolate_below_global,scales_caps_with_global_limit,weighted_split_never_over_allocates_distributable,headroom_class_owns_exactly_the_reserved_band,no_single_class_cap_plus_headroom_exceeds_global}`.
- ✅🔢 edges — `default_residency_global_one_does_not_underflow_and_validates`, `default_residency_small_globals_all_validate`, `borrow_flag_with_zero_shared_pool_is_valid_and_preserved`, `zero_shared_pool_cannot_extend_borrowing_class_hard_cap`.
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
- ✅ **Shared admission preflight + single reclaim plan** —
  `sep23_fup003_terminal_matrix_never_fires_callback`,
  `sep23_fup003_blocked_matrix_never_fires_callback`,
  `sep23_fup003_borrowing_class_hard_is_not_terminal_until_pool_exhausted`,
  `sep23_fup003_predicted_failure_skips_soft_and_runs_single_hard`,
  `sep23_fup003_hard_remeasure_still_denied_is_terminal_without_second_callback`,
  `sep23_fup003_ticket_exhaustion_blocks_without_callback`,
  `live_ticket_count_cap_runs_one_reclaim_then_remeasures`,
  `live_ticket_count_cap_can_recover_when_host_releases_zero_byte_charge`,
  `sep23_audit_group_denial_runs_single_hard_regardless_of_soft_crossing`,
  `sep23_audit_wrong_victim_release_does_not_admit`,
  `sep23_audit_forged_reclaimed_bytes_are_recorded_but_never_trusted`,
  `sep23_audit_panicking_reclaimer_is_contained_and_broker_survives`,
  `sep23_audit_zero_byte_reserve_fires_no_callback_even_over_soft`, and
  `sep23_audit_max_scale_plus_one_byte_denies_clean_without_callback`
  (`core/src/api/control.rs`). Terminal/blocked denials never fire the
  callback; a predicted failure runs one hard plan, never soft-first. A live
  ticket-count cap is reclaimable when the host drops a ticket: it gets one
  callback and a fresh ledger check, even if the callback reports zero bytes.
  When tickets are 0-byte authorities in a zero-hard-byte class, that class
  remains a reclaim candidate at the ticket-count ceiling.
- ✅ **Partial callback progress followed by panic** — `reclaimer_release_then_panic_uses_remeasured_ledger_not_callback_result` (`core/src/api/control.rs`) proves a host callback can release a real victim and then panic: callback failure is counted, but admission uses the remeasured ledger. One attempt, exact postimage and full drain are required.
- ✅ 🔬 **Preflight/attempt agreement** —
  `sep23_fup003_preflight_attempt_agreement_randomized`
  (`core/src/api/control.rs`): 2,000 deterministic cases pin verdict and
  byte-identical error equality plus fresh-broker terminality proofs.
- ✅ **0-byte ticket authority + deferred preservation** —
  `sep23_fup007_zero_byte_committed_blocks_group_removal_until_explicit_release`,
  `sep23_fup007_zero_byte_pending_blocks_removal_explicit_rollback_unblocks`,
  `sep23_fup007_scope_matrix_remap_cap_ungrouped_terminated`,
  `sep23_audit_failing_pending_drain_preserves_ticket_for_retry`, and
  `sep23_audit_failing_committed_drain_restores_whole_batch`
  (`core/src/api/control.rs`). Zero-byte tickets pin mappings; failing
  drains keep authority queued.
- ✅ **Managed replacement at the ticket ceiling** — `managed_recharge_at_live_ticket_cap_keeps_broker_authority_maps_exact` (`core/src/api/control.rs`) repeats the net-zero swap through a managed charge and checks final broker owner/group/deferred maps, not only ledger counters.
- ✅ **Bounded provenance + reclaim/reconfigure coupling** —
  `sep23_fup005_ten_thousand_reconfigures_keep_provenance_bounded`,
  `sep23_fup006_accepted_reconfigure_inside_callback_remeasures_under_new_policy`
  (`core/src/api/control.rs`).
- ✅ **Cross-thread callback reentry is bounded** —
  `event_sink_observation_reentry_delivers_outer_event_once`,
  `same_broker_reclaimer_reserve_reentry_is_bounded_and_typed`,
  `concurrent_reclaimer_reentry_does_not_start_a_callback_storm`, and
  `independent_outer_reserves_share_one_in_flight_reclaimer`
  (`core/src/api/control.rs`). The event sink is single-flight per broker;
  recursive or concurrent advisory events during its callback are dropped, not
  queued or replayed. Two independently initiated hard requests also share at
  most one in-flight host reclaimer; the second reaches a bounded denial while
  the first callback is deliberately held by a channel, not a sleep.
- ✅ **Panicking host callbacks are quarantined, not leaked per request** —
  `sink_panic_payloads_are_retired_once_and_callback_is_quarantined` and
  `reclaimer_panic_payloads_are_retired_once_and_admission_remeasures`
  (`core/src/api/control.rs`) exercise ordinary and panicking payload destructors.
  Later operations retain exact accounting without reinvoking the failed sink
  or reclaimer. `soft_and_hard_reclaim_plans_share_one_callback_in_both_arrival_orders`,
  `held_reclaim_remeasures_concurrent_policy_and_slot_changes`, and
  `concurrent_recharge_and_observe_never_expose_torn_axes` pin the mixed-pressure,
  changed-policy, and coherent-snapshot interleavings with exact postimages.

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
- ✅ **Payload reference Eq panic before mutation** —
  `reference_eq_panic_before_mutation_keeps_public_store_available`
  (`facade/src/payload_store/service.rs`) injects a one-shot Eq panic in
  pin/unpin/pin_count/delete, then verifies public stats, pin count, cleanup,
  and exact ledger postimage. This covers those lookup paths, not arbitrary
  slow or side-effecting caller code.
- ✅ **Payload reference Clone panic before publication** —
  `reference_clone_panic_before_publication_keeps_public_store_available`
  (`facade/src/payload_store/service.rs`) traps the stored-reference clone in
  `put_canonical`; the public store stays readable, no charge is acquired,
  and a retry plus cleanup drains exactly.
- ✅ **Retired client destructor reentry** —
  `delete_drop_reentry_is_bounded_and_peer_publication_retains_exact_charge`
  (`facade/src/payload_store/service.rs`) and
  `overwrite_drop_reentry_is_bounded_and_other_store_sees_exact_ledger`
  (`facade/src/retained_map/service.rs`) run in bounded child processes. Same-store
  reads return typed busy during the mutation lane; a peer store can publish on
  the shared broker, and final inventory and charge drain exactly.
- ✅ **F#3 for payload_store** — `charged_store_bytes_flow_through_the_ledger_and_drain_to_zero`, `charged_payload_over_cap_put_is_fail_closed`, `uncharged_store_is_explicit_non_budgeted_mode` (a "charged store" mode routes bytes through the broker ledger).
- ✅ retained_map hash-indexed point operations + accounting/order preservation — `put_get_remove_get_none_round_trip_corrects_bytes`, `equal_hash_key_duplicate_overwrites_and_does_not_double_count`, `first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1`, `overwrite_preserves_original_insertion_position_for_newest_first_sweep`, `arbitrary_remove_preserves_remaining_newest_first_sweep_order`, `charged_sweep_during_rebuild_is_fail_closed`, and `keys_lists_inserted_keys_in_deterministic_sorted_order`. The public key identity law is `Eq + Hash`; exact lookup/overwrite/remove use the canonical linked hash container while enumeration remains deterministically key-sorted.
- ✅ **Read-side client panic does not poison retained-map state** — `checked_reads_propagate_client_clone_panics_without_poisoning_owner_mutex_v1` proves a panicking value clone still propagates to the caller, then subsequent get/stats/key/entry reads remain available. This is not a callback time bound; synchronous callback latency remains caller-controlled.
- ✅ **F#3 for retained_map** — retained_map is ledger-wired only through the typestate builder: binding produces `BoundRetainedMapStoreBuilder`, only the bound state exposes infallible `build()`, and `SimpleRetainedMapStore` has no public late-attach transition. Evidence: the two compile-fail docs plus `retained_map_platform_builder_routes_bytes_through_the_ledger`, `charged_insert_routes_bytes_through_the_ledger`, `charged_insert_over_cap_is_denied_and_fails_closed`, `charged_overwrite_denial_leaves_existing_entry_unchanged`, and `charged_sweep_refunds_reclaimed_bytes_to_the_ledger`. Internal ticket corruption is also fail-closed before refund or container mutation (`charged_remove_rejects_a_ticketless_entry_without_mutating_state`, `charged_sweep_rejects_a_ticketless_candidate_without_partial_refund_or_eviction`). The uncharged `new` path remains distinct via `uncharged_retained_map_bytes_are_not_reflected_in_a_broker_ledger_snapshot`.
- ✅ **Cross-store shared broker** —
  `distinct_grouped_stores_compete_for_one_global_slot_and_handoff_after_release`
  and `store_scratch_and_zero_byte_authorities_survive_denied_remap_and_legal_raise`
  (`facade/tests/cross_store_hellgate.rs`, features `retained_map,revocable_store`)
  use independent cost constants and check exact class/owner/context/kind
  postimages and live-ticket counts, including a zero-byte authority.
- ✅ revocable FIFO/revoke/eviction — `reclaim_one_is_fifo_and_releases_its_bytes`, `revoke_makes_entry_unretrievable_and_decrements_accounting`, `reclaim_eviction_order_is_deterministic_fifo_at_capacity_boundary`.
- ✅ publish_pipeline commit transition — `default_pipeline_runs_stage_then_commit_each_exactly_once_no_rollback`, `failed_commit_never_publishes_and_never_increments_count`.
- ✅ CanonicalKey/MeasuredValue — `cloning_measured_arc_shares_the_allocation_not_a_deep_copy`, `measured_value_charges_caller_supplied_size_not_inner_len`, `cloning_measured_value_preserves_size_and_value_context`.
- ✅ spill routing fail-closed — `routed_spill_to_available_target_succeeds_and_counts`, `spill_to_unavailable_target_fails_closed_and_does_not_count`, `routed_spill_after_target_goes_offline_fails_closed_leaving_inventory_unchanged`.
- ✅ **spill write → restore (inverse) round-trips** — the restore inverse landed: `in_memory_target_write_then_restore_round_trips_the_payload`, `in_memory_target_restore_of_missing_handle_is_none_not_an_error`, `in_memory_target_restore_after_offline_fails_closed_leaving_retained_state_unchanged` (`facade/src/spill/service.rs`).
- ✅ **Closed source after in-process spill** —
  `closed_source_rejects_restored_handle_without_losing_its_charge`
  (`facade/src/revocable_store/service.rs`) proves rejected restore returns the
  still-charged handle and its later retirement refunds once, even after the
  target controller is dropped.
- ✅ transcript per-session isolation + charged mode — `append_identity_is_partitioned_by_session`, `retention_stats_separate_active_evidence_and_session_costs`, and (charged) `exact_retry_charges_once_and_truncate_refunds_only_active_body`, `quota_denial_leaves_row_indexes_and_charge_unchanged` (`facade/src/transcript_store/service.rs`).
- ✅ **Transcript read-planning panic availability** —
  `session_order_panic_before_mutation_keeps_public_transcript_available`,
  `append_preflight_order_panic_keeps_public_store_available`,
  `append_preflight_message_order_panic_keeps_public_store_available`, and
  `append_replay_canonical_key_panic_keeps_public_store_available`
  (`facade/src/transcript_store/service.rs`) exercise SessionId/MessageId Ord
  and retained CanonicalKey panics before mutation. The public load/stats/close
  path remains available, and ledger/source postimages stay exact.
- ✅ charged facade mutators are fail-closed before store mutation — `charged_remove_during_rebuild_is_fail_closed`, `charged_sweep_during_rebuild_is_fail_closed`, `reclaim_one_during_rebuild_is_fail_closed`, `reclaim_until_during_rebuild_is_fail_closed`, `truncate_refund_failure_preserves_rows_indexes_and_accounting`.
- ✅ transcript charged breadth now matches the closeout bar — `append_identity_is_partitioned_by_session` proves session-local identity partitioning, and `truncate_refund_failure_preserves_rows_indexes_and_accounting` proves refund denial leaves both ledger and store untouched.
- ✅ **Durable record exact terminal state** —
  `sep23_fup001_reserved_hole_is_not_finalized_by_later_record`,
  `sep23_fup001_reserve_only_reopen_is_not_consumed`,
  `sep23_fup001_out_of_order_commit_finalized_reinsert_rejected`,
  `sep23_fup001_same_bytes_retry_keeps_inventory_stable`,
  `sep23_fup001_uncertain_commit_keeps_source_fenced_and_blocks_wrong_retire`,
  and `sep23_fup001_legacy_watermark_file_migrates_payload_and_fails_closed`
  (`facade/src/spill/backings/redb.rs`, `facade/src/revocable_store/service.rs`).
  Reserved holes resolve `Absent`, never guessed; legacy files migrate
  payload bytes and fail closed on ambiguous history.
- ✅ **Single-owner durable recovery** —
  `sep23_fup002_concurrent_recovery_has_exactly_one_owner`,
  `sep23_fup002_stale_generation_rejected_after_claim_steal`,
  `sep23_fup002_commit_ack_loss_same_claimant_publishes_on_retry`,
  `sep23_fup002_stolen_claim_reconcile_retires_provisional_copy`,
  `sep23_fup002_foreign_backend_claim_rejected_on_every_permission_change`,
  `sep23_fup002_finalize_proof_is_per_record_not_last_writer`,
  `sep23_fup002_decode_failure_and_denied_admission_preserve_durable`,
  `sep23_fup002_backend_identity_stable_across_reopen`,
  `sep23_audit_panicking_decode_aborts_prepared_claim_and_retries_clean`,
  `sep23_audit_panicking_decode_preserves_committed_recovery_until_retry`,
  and `sep23_audit_finalize_retry_succeeds_while_unrelated_claim_is_live`.
  Exactly one recovery owner; losers keep no readable copy or charge.
- ✅ **Caller sizing panic during durable resident preparation** —
  `panicking_restore_measurement_aborts_prepared_claim_and_retries_clean` and
  `panicking_recovery_measurement_preserves_committed_claim_for_retry`
  (`facade/src/revocable_store/service.rs`) distinguish the two persisted
  restore phases. A `ByteSized` panic after successful decode propagates to
  the caller without publishing or charging a resident copy: Prepared aborts
  its claim so ordinary restore can retry; Committed retains its active
  recovery claim so recovery can retry. Both paths end with one readable
  copy and exact ledger drain.
- ✅ **Commit callback panic is an unknown durable write outcome** —
  `panicking_durable_commit_fences_source_until_exact_record_resolution`
  (`facade/src/revocable_store/service.rs`) injects panic before and after a
  target retains the record. The source stays charged but unreadable in both
  cases until exact reconciliation: `Absent` restores FIFO authority, while
  `Present` retires the source charge once. This owner-local fake tests the
  store state machine; the separate Redb kill/reopen tests establish concrete
  backing persistence boundaries.
- ✅ **Restore delete callback panic is an unknown durable claim outcome** —
  `panicking_restore_commit_fences_resident_until_redb_reconciliation`
  (`facade/src/revocable_store/service.rs`) uses real Redb with an injected
  panic before and after `commit_restore`. The provisional resident remains
  charged but unreadable until exact claim reconciliation. A pre-commit panic
  retires the provisional copy and allows ordinary restore; a post-commit
  panic publishes only that copy. Each branch yields one readable entry and
  zero final ledger charge.
- ✅ **Prepare callback panic after persisted claim** —
  `panicking_prepare_after_persisted_claim_has_explicit_recovery_path`
  (`facade/src/revocable_store/service.rs`) uses real Redb: a callback panics
  after persisting a prepared claim but before returning its token. No resident
  copy or charge is published; explicit durable recovery on the
  surviving store completes exactly once and drains the ledger. This is a
  recovery path, not an automatic plain-restore retry.
- ✅ **Real SIGKILL/reopen rail** —
  `sep23_fup006_sigkill_after_commit_reopens_with_stable_identity_and_exact_drain`,
  `sep23_fup006_sigkill_during_write_stream_converges_without_duplicates_or_tears`,
  and `sep23_fup006_reserved_hole_stays_absent_across_sigkill`
  (`facade/tests/spill_redb_kill_reopen.rs`, public API only). The child
  dies by `SIGKILL` (no destructors, no flush) and the survivor reopens
  cold. Power loss is NOT covered; mid-finalize instruction-point kills
  moved to the L1 rail below (see residual boundaries in the Sep-23 receipt).
- ✅ **Store+ledger spill process boundary** —
  `durable_spill_store_charge_and_record_survive_process_boundary`
  (`facade/tests/spill_redb_kill_reopen.rs`, features
  `spill-redb,revocable_store`) uses the public platform/store builder in the
  killed child, asserts its source charge retired after committed spill, then
  cold-reopens with a new broker/store. Exactly one whole payload is restored
  and the survivor ledger drains to zero. This does not inject an ACK loss or
  stop between commit and source retirement.
- ✅ **Finalize instruction-point kill rail (DURABILITY-001 L1)** —
  `durability_l1_finalize_before_commit_kill_recovers_present_and_drains_exactly_once`,
  and `durability_l1_finalize_after_commit_kill_resolves_finalized_without_resurrection`
  (`facade/tests/spill_redb_finalize_kill.rs`, public API only, features
  `spill-redb,durability-test-hooks`). The child blocks at a named hook on
  one side of the finalize `txn.commit()` call, dies by `SIGKILL`, and the
  survivor reopens cold. The two points bracket QMP's commit call; they do
  not stop inside Redb's `pwrite`/`fsync` and do not prove power loss.
- ✅ **Store+ledger finalize process boundary** —
  `durability_l1_store_and_ledger_recover_across_finalize_kill`
  (`facade/tests/spill_redb_finalize_kill.rs`, features
  `spill-redb,durability-test-hooks,revocable_store`) drives actual
  `RevocableStore::restore_one_durable_bounded` in the killed child and proves a live
  resident charge at each hook. A fresh platform/store restores the pre-commit
  victim exactly once; a post-commit victim is terminal and not replayed under
  the documented revocable contract. The unrelated record restores and every
  survivor charge drains. This is still process termination, not power loss.
- ✅ **Redb logical quota, file cap and restore staging** —
  `limits_reject_oversized_record_before_commit_without_consuming_reserved_id`,
  `every_invalid_limit_relation_rejects_before_creating_database_file`,
  `retained_limits_count_recovery_and_refund_only_after_finalize`,
  `retained_payload_cap_denies_next_record_without_changing_durable_authority`,
  `in_flight_restore_budget_denial_preserves_claim_and_releases_on_candidate_drop`,
  `zero_byte_restore_candidates_are_bounded_by_buffer_count`,
  `concurrent_writers_at_record_cap_admit_exactly_one_transaction`,
  `staging_denial_after_staged_remove_rolls_back_durable_payload`,
  `tighter_reopen_rejects_existing_inventory_without_erasing_it`,
  `persisted_payload_usage_mismatch_fails_closed_without_erasing_record`,
  `capped_file_backend_rejects_extension_before_file_mutation`,
  `reopening_above_database_file_cap_fails_without_erasing_payload`,
  `database_file_cap_denial_preserves_prior_durable_record_across_reopen`,
  `partial_storage_full_write_reopens_to_whole_records_only`,
  `partial_storage_full_during_restore_commit_preserves_one_whole_copy`,
  and `partial_storage_full_during_finalize_reopens_present_or_finalized`
  (`spill/backings/redb.rs`). Limits reject writes before commit, count
  recovery payloads until finalize, and reject smaller reopen limits without
  erasing the only copy. The per-file cap blocks Redb file extension and
  reports its storage-full cause as typed `Capacity`. At the
  cap, Redb can require a cold reopen after I/O error and more file headroom
  before all restores complete. This is not a whole-volume quota, actual
  ENOSPC injection or caller codec scratch bound.
  The partial-write oracles use a test-only backend that writes and reads back
  a strict prefix of one Redb I/O buffer before returning `StorageFull` at
  spill, restore-commit and finalize boundaries. Cold reopen yields only whole
  committed records or their valid finalized disposition. These are not
  actual filesystem ENOSPC or power-loss tests.
- ✅ **Redb schema, identity and recovery concurrency** —
  `unsupported_explicit_schema_versions_fail_closed_before_v2_initialization`,
  `missing_schema_marker_on_v2_shaped_database_fails_without_restamping`,
  `missing_metadata_table_on_v2_shaped_database_fails_without_recreation`,
  `incomplete_backend_identity_fails_closed_without_replacing_surviving_half`,
  `schema_v2_missing_required_authority_fails_without_recreating_it`,
  `schema_v2_payload_disposition_mismatch_fails_without_repair`,
  `concurrent_prepare_restore_issues_exactly_one_live_claim`,
  `concurrent_recovery_claims_are_generation_fenced_to_latest_owner`,
  `backend_identity_and_pending_claim_survive_cold_rename`, and
  `consistent_cold_copy_preserves_logical_lineage_identity`
  (`spill/backings/redb.rs`). Invalid persisted authority fails before
  migration or replacement; concurrent claimants cannot both publish one
  record. A quiesced rename or consistent copy preserves logical lineage,
  whereas two independently created databases remain distinct authorities.
- ✅ **Redb file-cap denial with live source authority** —
  `redb_file_cap_denial_keeps_store_source_charged_until_exact_resolution`
  (`revocable_store/service.rs`) fills the capped Redb through the charged
  store path. A refused spill keeps the source charge; an unknown commit is
  fenced until a cold reopen and exact record reconciliation. The final
  source or durable authority drains without a double refund. This injects
  QMP's file cap, not a real filesystem ENOSPC fault.
- ✅ **Oversized encoded record with live source authority** —
  `oversized_encoded_record_preserves_charged_source_and_no_durable_payload`
  (`revocable_store/service.rs`) rejects a caller-produced record above the
  configured per-record limit, preserves the exact resident entry and charge,
  and cold-reopens to zero durable payloads. The caller has already allocated
  the encoded `Vec`; this does not bound codec output before allocation or
  private encoder scratch.
- ✅ **Bounded typed durable codec** —
  `literal_v1_record_decodes_without_using_the_current_encoder`,
  `revocable_entry_cost_has_one_checked_formula`,
  `bounded_codec_spill_preflights_backend_and_broker_before_allocating`,
  `bounded_codec_cold_reopen_restore_denial_and_retry_keep_one_authority`,
  `bounded_codec_rejects_malformed_durable_record_without_publication`,
  `bounded_codec_keeps_output_charged_until_durable_commit_returns`, and
  `bounded_codec_committed_recovery_retries_after_capacity_denial`
  (`revocable_store/{cost,durable_codec,service}.rs`)
  prove literal V1 decode, shared resident/preflight cost, cap/admission
  denial, charged output overlap, cold reopen, and committed recovery retry.
  An unsupported QMR2 record is neither published nor deleted; an absent
  backend record cap is rejected before ID reservation. The closure-based
  custom codec remains explicitly unbounded.
- 🧪 **Real Linux volume ENOSPC, isolated from host data** —
  `scripts/durability/run-enospc-docker.sh` runs the
  Redb ENOSPC example on a disposable 8 MiB Docker tmpfs.
  Five fresh-volume modes cover backing spill write, restore commit, finalize,
  bounded charged spill and bounded charged restore commit. The charged spill
  mode checks that real ENOSPC retains the source ledger charge,
  proves the record-id reservation survived so the fault reached the payload
  commit, cold-reopens and reconciles any ambiguous spill, then restores or reads the
  exact payload and drains both durable records and ledger charge to zero.
  The spill and restore modes require a real `ENOSPC` to be classified as
  `Capacity`, free only the filler, then cold-reopen and drain whole records.
  The charged restore-commit mode fills the volume after a valid persisted
  claim and resident charge, before Redb's queue-to-recovery write. It checks
  the exact fenced postimage, unrelated record, cold reopen and final ledger
  drain. Its one-shot hook exists only under `durability-test-hooks`, outside
  production feature graphs.
  Finalize may succeed by reusing Redb space even when the volume is full; its
  mode checks the exact finalized or recoverable postimage after cold reopen.
  The runner fingerprints all present source files before and after. This is a
  standalone Linux volume-error oracle, not the consuming host's volume/quota
  policy, physical power-loss durability, or a caller codec scratch bound.
- 🧪 **Optional Linux syscall-error subrail (DURABILITY-001 L2 partial)** —
  `scripts/durability/run-emulated-linux.sh` calibrates the real finalize
  boundary, injects EIO into the first `pwrite64` and `fdatasync` after it, then
  cold-reopens and checks whole-state recovery/finalization plus exact drain.
  Raw traces, Redb files, source digest, and SHA-256 receipt are retained in a
  caller-selected temp artifact directory. This exercises syscall error
  handling only; it does not inject block-device failures or simulate power
  loss. Device-mapper L2 and physical L3 remain unqualified.

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
- ✅ **Per-axis kind oracle** —
  `sep23_fup004_suite_rejects_class_confined_kind_swap`,
  `sep23_fup004_suite_rejects_context_confined_kind_swap`,
  `sep23_fup004_suite_rejects_aggregate_preserving_bucket_swap`, and
  `sep23_fup004_mixed_kind_trace_agrees_on_every_axis_bucket`
  (`testkit/src/conformance/suites.rs`). Total-preserving swaps confined
  to one axis are rejected; the mixed-kind fixed script agrees on every
  axis bucket across reserve and release phases.

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

- ✅ advisory correctness — `compare_reports_signed_drift_and_mismatch_boundary`, `compare_semantic_bytes_is_ssot_not_allocator`, `missing_allocator_data_is_not_treated_as_mismatch`.
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
  owner-group remaps while the owner holds live tickets, including pending or
  committed 0-byte authority.
- `AccountingLedger` is intentionally non-`Clone`, so tickets stay bound to one
  live ledger epoch.

## How evidence was gathered / re-verify

```bash
scripts/check-workspace.sh
# inspect the evidence test names per file:
rg -n '^\s*fn [a-z0-9_]+\(' crates --glob '*.rs'
```
