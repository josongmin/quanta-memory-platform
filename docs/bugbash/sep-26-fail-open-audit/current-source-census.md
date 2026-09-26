# Current-source test census — sep-26 fail-open audit

- Immutable audited base commit: `61e9c7c47fa7294f9a926fd867e4af0b116c429b`.
- Captured UTC: `2026-09-26T13:03:37.623382+00:00`.
- Source state at capture: **dirty working tree**; this census inventories the source bytes listed below, not a committed-source qualification receipt.
- Source digest SHA-256: `adb9867a8059156d0dcf8d891fda6b532171a5ba87e61e7760e2703914223621`.
- Actual attributed test functions: **801**; independently anchored `rg` matches: **801**.
- Scope ownership comes from the nine historical reports. The new `facade/src/test_support.rs` belongs to `pipeline-shared`; documented zero-test leftovers and crash-child examples remain inventoried.
- Comments and string literals are excluded before pairing an attribute with its actual function. Each row names the function line and also records its attribute line. Conditional feature gates remain lexical entries; these counts do not assert runtime execution.
- The immutable audited-base source was independently parsed and matches all historical actual per-scope counts. The historical raw count was 795. One doc-comment pseudo-test in `core/tests/public_api_surface.rs` and its non-test compile-pin census row are excluded here, yielding **794 historical actual tests**. Historical reports are preserved.
- Doctests, compile-only helper functions and macro-generated property cases without these attributes are outside this census. Loom model tests are inventoried as source tests, without claiming production concurrency coverage.

## Scope totals

| Scope | Historical actual tests | Current rows | Anchored rg | Delta | Match |
| --- | ---: | ---: | ---: | ---: | --- |
| core-domain | 97 | 98 | 98 | +1 | yes |
| core-api | 144 | 144 | 144 | +0 | yes |
| app-ports | 55 | 55 | 55 | +0 | yes |
| revocable-store | 72 | 72 | 72 | +0 | yes |
| facade-stores | 151 | 151 | 151 | +0 | yes |
| spill-redb | 75 | 75 | 75 | +0 | yes |
| crash-rails | 29 | 29 | 29 | +0 | yes |
| pipeline-shared | 84 | 86 | 86 | +2 | yes |
| testkit | 87 | 91 | 91 | +4 | yes |
| **Total** | **794** | **801** | **801** | **+7** | **yes** |

## Source file inventory

| Scope | Source file | Actual rows | Anchored rg |
| --- | --- | ---: | ---: |
| core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs` | 39 | 39 |
| core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs` | 14 | 14 |
| core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs` | 27 | 27 |
| core-domain | `crates/quanta-memory-platform-core/src/domain/admission.rs` | 3 | 3 |
| core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs` | 15 | 15 |
| core-domain | `crates/quanta-memory-platform-core/src/domain/context.rs` | 0 | 0 |
| core-domain | `crates/quanta-memory-platform-core/src/domain/errors.rs` | 0 | 0 |
| core-domain | `crates/quanta-memory-platform-core/src/domain/management_limits.rs` | 0 | 0 |
| core-domain | `crates/quanta-memory-platform-core/src/domain/mod.rs` | 0 | 0 |
| core-domain | `crates/quanta-memory-platform-core/src/domain/recharge.rs` | 0 | 0 |
| core-domain | `crates/quanta-memory-platform-core/src/domain/stats.rs` | 0 | 0 |
| core-api | `crates/quanta-memory-platform-core/src/api/control.rs` | 129 | 129 |
| core-api | `crates/quanta-memory-platform-core/src/api/builder.rs` | 7 | 7 |
| core-api | `crates/quanta-memory-platform-core/tests/public_api_surface.rs` | 8 | 8 |
| core-api | `crates/quanta-memory-platform-core/src/api/broker.rs` | 0 | 0 |
| core-api | `crates/quanta-memory-platform-core/src/api/mod.rs` | 0 | 0 |
| app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs` | 13 | 13 |
| app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs` | 14 | 14 |
| app-ports | `crates/quanta-memory-platform-core/src/adapters/pin.rs` | 5 | 5 |
| app-ports | `crates/quanta-memory-platform-core/src/adapters/metrics.rs` | 2 | 2 |
| app-ports | `crates/quanta-memory-platform-core/src/adapters/hdrhistogram.rs` | 4 | 4 |
| app-ports | `crates/quanta-memory-platform-core/src/ports/scratch.rs` | 9 | 9 |
| app-ports | `crates/quanta-memory-platform-core/src/ports/telemetry.rs` | 8 | 8 |
| app-ports | `crates/quanta-memory-platform-core/src/adapters/scratch.rs` | 0 | 0 |
| app-ports | `crates/quanta-memory-platform-core/src/adapters/mod.rs` | 0 | 0 |
| app-ports | `crates/quanta-memory-platform-core/src/ports/pin.rs` | 0 | 0 |
| app-ports | `crates/quanta-memory-platform-core/src/ports/reclaim.rs` | 0 | 0 |
| app-ports | `crates/quanta-memory-platform-core/src/ports/store.rs` | 0 | 0 |
| app-ports | `crates/quanta-memory-platform-core/src/ports/mod.rs` | 0 | 0 |
| app-ports | `crates/quanta-memory-platform-core/src/application/mod.rs` | 0 | 0 |
| revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs` | 68 | 68 |
| revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/durable_codec.rs` | 3 | 3 |
| revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/cost.rs` | 1 | 1 |
| revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/mod.rs` | 0 | 0 |
| facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs` | 60 | 60 |
| facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/mod.rs` | 0 | 0 |
| facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs` | 58 | 58 |
| facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/mod.rs` | 0 | 0 |
| facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs` | 33 | 33 |
| facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/mod.rs` | 0 | 0 |
| spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs` | 20 | 20 |
| spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs` | 55 | 55 |
| spill-redb | `crates/quanta-memory-platform-facade/src/spill/mod.rs` | 0 | 0 |
| spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/mod.rs` | 0 | 0 |
| crash-rails | `crates/quanta-memory-platform-facade/tests/spill_redb_kill_reopen.rs` | 4 | 4 |
| crash-rails | `crates/quanta-memory-platform-facade/tests/spill_redb_finalize_kill.rs` | 3 | 3 |
| crash-rails | `crates/quanta-memory-platform-facade/tests/cross_store_hellgate.rs` | 4 | 4 |
| crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs` | 18 | 18 |
| crash-rails | `crates/quanta-memory-platform-facade/examples/spill_redb_crash_child.rs` | 0 | 0 |
| crash-rails | `crates/quanta-memory-platform-facade/examples/spill_redb_finalize_kill_child.rs` | 0 | 0 |
| pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs` | 32 | 32 |
| pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs` | 5 | 5 |
| pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/mod.rs` | 0 | 0 |
| pipeline-shared | `crates/quanta-memory-platform-facade/src/presets/mod.rs` | 0 | 0 |
| pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs` | 11 | 11 |
| pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/lifecycle.rs` | 7 | 7 |
| pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/measure.rs` | 9 | 9 |
| pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/accounting.rs` | 0 | 0 |
| pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/mod.rs` | 0 | 0 |
| pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs` | 20 | 20 |
| pipeline-shared | `crates/quanta-memory-platform-facade/src/test_support.rs` | 2 | 2 |
| testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs` | 39 | 39 |
| testkit | `crates/quanta-memory-platform-testkit/src/conformance/invariants.rs` | 5 | 5 |
| testkit | `crates/quanta-memory-platform-testkit/src/conformance/mod.rs` | 5 | 5 |
| testkit | `crates/quanta-memory-platform-testkit/src/conformance/target.rs` | 0 | 0 |
| testkit | `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs` | 8 | 8 |
| testkit | `crates/quanta-memory-platform-testkit/src/faults/mod.rs` | 2 | 2 |
| testkit | `crates/quanta-memory-platform-testkit/src/fake/broker.rs` | 9 | 9 |
| testkit | `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs` | 5 | 5 |
| testkit | `crates/quanta-memory-platform-testkit/src/alloc_tracking/mod.rs` | 5 | 5 |
| testkit | `crates/quanta-memory-platform-testkit/src/loom_models.rs` | 5 | 5 |
| testkit | `crates/quanta-memory-platform-testkit/src/lib.rs` | 3 | 3 |
| testkit | `crates/quanta-memory-platform-testkit/tests/facade_conformance_bite.rs` | 4 | 4 |
| testkit | `crates/quanta-memory-platform-testkit/tests/public_api_surface.rs` | 1 | 1 |
| testkit | `crates/quanta-memory-platform-testkit/src/fake/mod.rs` | 0 | 0 |
| testkit | `crates/quanta-memory-platform-testkit/src/fake/views.rs` | 0 | 0 |
| testkit | `crates/quanta-memory-platform-testkit/src/doctor/mod.rs` | 0 | 0 |

## Historical report completeness

The nine historical census tables contain **795 rows**, equal to their historical raw grep claims. All nine findings reports, `findings-critic.md`, and `99-index.md` exist, are nonempty, have concluding sections, and contain no standalone unfinished placeholders. Report hashes are recorded in the JSON and checked by the verifier.

This is structural completeness, not acceptance of every finding or a claim that all mutations were executed. The critic explicitly records one safety-skipped WEAK sample (row 12) and one static Loom-reference check (row 9); the synthesis describes the historical audit and its fix checklist, rather than current-source qualification.

| Historical report | Census table rows | Raw count claim | Bytes |
| --- | ---: | ---: | ---: |
| `findings-core-domain.md` | 97 | 97 | 30622 |
| `findings-core-api.md` | 145 | 145 | 42290 |
| `findings-app-ports.md` | 55 | 55 | 22614 |
| `findings-revocable-store.md` | 72 | 72 | 30905 |
| `findings-facade-stores.md` | 151 | 151 | 46787 |
| `findings-spill-redb.md` | 75 | 75 | 27870 |
| `findings-crash-rails.md` | 29 | 29 | 22386 |
| `findings-pipeline-shared.md` | 84 | 84 | 30619 |
| `findings-testkit.md` | 87 | 87 | 39631 |
| `findings-critic.md` | n/a | n/a | 19503 |
| `99-index.md` | n/a | n/a | 37047 |

## Test functions

<!-- TEST_CENSUS_START -->
| # | Scope | Source function location | Function name | Attribute | Attribute line |
| ---: | --- | --- | --- | --- | ---: |
| 1 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1268` | `zero_byte_tickets_obey_independent_live_ticket_cap_fail_closed` | `#[test]` | 1267 |
| 2 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1297` | `committed_replacement_at_live_ticket_cap_is_a_net_zero_authority_swap` | `#[test]` | 1296 |
| 3 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1350` | `owner_key_byte_limit_is_exact_and_denial_is_fail_closed` | `#[test]` | 1349 |
| 4 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1375` | `reserve_denied_when_global_hard_limit_would_be_exceeded` | `#[test]` | 1374 |
| 5 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1392` | `reserve_for_unconfigured_class_is_denied_without_mutation` | `#[test]` | 1391 |
| 6 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1405` | `global_ceiling_reserves_headroom_for_non_headroom_classes` | `#[test]` | 1404 |
| 7 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1424` | `headroom_usage_does_not_consume_protected_non_headroom_band_twice` | `#[test]` | 1423 |
| 8 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1445` | `reserved_floor_protects_class_from_peer_consumption` | `#[test]` | 1444 |
| 9 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1483` | `double_release_of_a_cloned_ticket_is_rejected_and_does_not_corrupt` | `#[test]` | 1482 |
| 10 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1508` | `commit_after_release_is_rejected` | `#[test]` | 1507 |
| 11 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1521` | `second_commit_of_same_reservation_is_rejected` | `#[test]` | 1520 |
| 12 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1536` | `ticket_id_exhaustion_fails_without_reuse` | `#[test]` | 1535 |
| 13 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1558` | `borrow_over_class_cap_is_bounded_by_shared_burst_pool` | `#[test]` | 1557 |
| 14 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1580` | `shared_burst_pool_is_shared_across_borrowing_classes` | `#[test]` | 1579 |
| 15 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1600` | `borrow_false_still_denies_over_cap_regardless_of_pool` | `#[test]` | 1599 |
| 16 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1615` | `owner_hard_cap_still_enforced` | `#[test]` | 1614 |
| 17 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1631` | `empty_release_batch_is_a_noop_even_during_rebuild` | `#[test]` | 1630 |
| 18 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1687` | `zero_byte_reserve_commits_and_releases_coherently` | `#[test]` | 1686 |
| 19 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1725` | `overflow_denial_preserves_exact_live_ticket_accounting` | `#[test]` | 1724 |
| 20 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1740` | `exact_boundary_ticket_survives_first_release` | `#[test]` | 1739 |
| 21 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1759` | `unrepresentable_sum_is_denied_without_mutating_any_axis` | `#[test]` | 1758 |
| 22 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1774` | `batch_release_preflights_every_charge_kind_before_first_ticket_mutates` | `#[test]` | 1773 |
| 23 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1830` | `owner_probe_meter_observes_keyed_reads_and_every_scanned_key` | `#[test]` | 1829 |
| 24 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1855` | `single_release_preflight_is_bounded_independent_of_owner_cardinality` | `#[test]` | 1854 |
| 25 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1861` | `aggregate_usage_tracks_charge_kinds_and_headroom` | `#[test]` | 1860 |
| 26 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1905` | `snapshot_validation_rejects_unrepresentable_axis_instead_of_saturating_it` | `#[test]` | 1904 |
| 27 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1924` | `forged_snapshot_total_owner_bytes_panics_as_documented` | `#[test]` | 1922 |
| 28 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1943` | `reserve_above_i64_max_does_not_wrap_to_negative_delta` | `#[test]` | 1942 |
| 29 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:1978` | `ticket_from_one_ledger_cannot_release_on_another` | `#[test]` | 1977 |
| 30 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:2006` | `ticket_ids_are_monotonic_and_unique_across_many_reserves` | `#[test]` | 2005 |
| 31 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:2027` | `context_axis_balances_with_owner_after_mixed_sequence` | `#[test]` | 2026 |
| 32 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:2086` | `mixed_kind_release_preserves_charge_kind_partition` | `#[test]` | 2085 |
| 33 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:2137` | `deterministic_fuzz_preserves_invariants_and_returns_to_zero` | `#[test]` | 2136 |
| 34 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:2258` | `sep23_fup004_honest_mixed_kind_snapshot_validates` | `#[test]` | 2257 |
| 35 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:2322` | `sep23_audit_admitted_recharge_moves_kind_vectors_exactly` | `#[test]` | 2321 |
| 36 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:2347` | `sep23_audit_batch_release_moves_kind_vectors_exactly` | `#[test]` | 2346 |
| 37 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:2364` | `sep23_fup004_class_kind_swap_preserving_totals_is_rejected` | `#[test]` | 2363 |
| 38 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:2383` | `sep23_fup004_context_kind_swap_preserving_totals_is_rejected` | `#[test]` | 2382 |
| 39 | core-domain | `crates/quanta-memory-platform-core/src/domain/ledger.rs:2400` | `sep23_fup004_owner_kind_swap_preserving_totals_is_rejected` | `#[test]` | 2399 |
| 40 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:95` | `check_helpers_admit_within_cap_and_reject_over_cap` | `#[test]` | 94 |
| 41 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:115` | `unlimited_budget_admits_everything` | `#[test]` | 114 |
| 42 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:470` | `for_preset_populates_every_mandatory_class` | `#[test]` | 469 |
| 43 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:488` | `for_preset_is_structurally_valid` | `#[test]` | 487 |
| 44 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:497` | `for_preset_class_caps_isolate_below_global` | `#[test]` | 496 |
| 45 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:524` | `for_preset_scales_caps_with_global_limit` | `#[test]` | 523 |
| 46 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:537` | `for_preset_weighted_split_never_over_allocates_distributable` | `#[test]` | 536 |
| 47 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:560` | `for_preset_headroom_class_owns_exactly_the_reserved_band` | `#[test]` | 559 |
| 48 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:581` | `for_preset_no_single_class_cap_plus_headroom_exceeds_global` | `#[test]` | 580 |
| 49 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:605` | `default_residency_global_one_does_not_underflow_and_validates` | `#[test]` | 604 |
| 50 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:626` | `default_residency_small_globals_all_validate` | `#[test]` | 625 |
| 51 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:643` | `validate_rejects_constructed_headroom_above_global` | `#[test]` | 642 |
| 52 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:655` | `validate_accepts_headroom_equal_to_global` | `#[test]` | 654 |
| 53 | core-domain | `crates/quanta-memory-platform-core/src/domain/policy.rs:663` | `borrow_flag_with_zero_shared_pool_is_valid_and_preserved` | `#[test]` | 662 |
| 54 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:464` | `higher_precedence_layer_wins_and_is_recorded_in_provenance` | `#[test]` | 463 |
| 55 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:505` | `removing_top_layer_changes_effective_value` | `#[test]` | 504 |
| 56 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:535` | `empty_layer_is_skipped_in_provenance` | `#[test]` | 534 |
| 57 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:548` | `validate_rejects_reserved_above_soft` | `#[test]` | 547 |
| 58 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:563` | `validate_rejects_global_below_reserved_plus_headroom_floor` | `#[test]` | 562 |
| 59 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:582` | `validate_rejects_class_hard_cap_above_global` | `#[test]` | 581 |
| 60 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:597` | `validate_rejects_owner_hard_cap_above_global` | `#[test]` | 596 |
| 61 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:606` | `validate_accepts_well_formed_policy` | `#[test]` | 605 |
| 62 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:628` | `validate_rejects_policy_map_cardinality_above_the_caps` | `#[test]` | 627 |
| 63 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:708` | `runtime_reconfigure_layers_are_rejected_at_build_time` | `#[test]` | 707 |
| 64 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:737` | `validate_rejects_shared_burst_pool_above_global_hard_limit` | `#[test]` | 736 |
| 65 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:751` | `validate_rejects_zero_admission_weight_with_burst_demand` | `#[test]` | 750 |
| 66 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:774` | `validate_rejects_missing_each_mandatory_class` | `#[test]` | 773 |
| 67 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:788` | `validated_policy_materializes_no_implicit_class_holes` | `#[test]` | 787 |
| 68 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:801` | `reserved_floor_checked_add_overflow_is_config_error` | `#[test]` | 800 |
| 69 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:842` | `validate_accepts_well_formed_resource_group` | `#[test]` | 841 |
| 70 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:851` | `validate_accepts_resource_group_reserved_equal_to_soft_cap` | `#[test]` | 850 |
| 71 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:863` | `validate_rejects_resource_group_reserved_above_soft` | `#[test]` | 862 |
| 72 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:874` | `validate_rejects_resource_group_soft_above_hard` | `#[test]` | 873 |
| 73 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:885` | `validate_rejects_resource_group_hard_above_global` | `#[test]` | 884 |
| 74 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:897` | `validate_rejects_resource_group_burst_percent_over_100` | `#[test]` | 896 |
| 75 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:908` | `validate_accepts_resource_group_burst_percent_boundaries` | `#[test]` | 907 |
| 76 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:922` | `resource_group_reserved_has_an_independent_global_floor` | `#[test]` | 921 |
| 77 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:950` | `validate_rejects_group_reserved_plus_headroom_above_global` | `#[test]` | 949 |
| 78 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:980` | `validate_rejects_owner_group_mapping_to_unknown_group` | `#[test]` | 979 |
| 79 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:987` | `validate_accepts_owner_group_mapping_to_declared_group` | `#[test]` | 986 |
| 80 | core-domain | `crates/quanta-memory-platform-core/src/domain/config.rs:1005` | `validate_rejects_overlong_owner_and_resource_group_identifiers` | `#[test]` | 1004 |
| 81 | core-domain | `crates/quanta-memory-platform-core/src/domain/admission.rs:823` | `deficit_saturates_for_unrepresentable_postimages` | `#[test]` | 822 |
| 82 | core-domain | `crates/quanta-memory-platform-core/src/domain/admission.rs:839` | `unrepresentable_global_aggregate_blocks_without_panic` | `#[test]` | 838 |
| 83 | core-domain | `crates/quanta-memory-platform-core/src/domain/admission.rs:885` | `unknown_group_denies_terminally_with_legacy_message` | `#[test]` | 884 |
| 84 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:10` | `execution_budget_default_matches_runtime_budget_truth` | `#[test]` | 9 |
| 85 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:21` | `memory_class_taxonomy_keeps_mandatory_and_future_classes_split` | `#[test]` | 20 |
| 86 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:38` | `every_preset_and_custom_validated_policy_is_total_over_mandatory_classes` | `#[test]` | 37 |
| 87 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:59` | `residency_policy_tracks_class_quota_and_resource_group_vocabulary` | `#[test]` | 58 |
| 88 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:100` | `builder_emits_effective_budget_report_with_override_provenance` | `#[test]` | 99 |
| 89 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:115` | `memory_error_has_typed_config_and_report_variants` | `#[test]` | 114 |
| 90 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:172` | `global_hard_limit_denies_reserve_through_public_broker_and_is_fail_closed` | `#[test]` | 171 |
| 91 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:208` | `headroom_band_is_reserved_for_headroom_class_at_public_surface` | `#[test]` | 207 |
| 92 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:241` | `shared_burst_pool_bounds_borrowing_through_public_broker` | `#[test]` | 240 |
| 93 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:285` | `zero_shared_pool_cannot_extend_borrowing_class_hard_cap` | `#[test]` | 284 |
| 94 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:327` | `non_borrowing_class_cannot_use_burst_pool_at_public_surface` | `#[test]` | 326 |
| 95 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:352` | `managed_charge_double_release_is_rejected_without_counter_drift` | `#[test]` | 351 |
| 96 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:388` | `dropping_pending_public_authority_rolls_reservation_back` | `#[test]` | 387 |
| 97 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:409` | `preset_class_caps_bite_at_public_reserve_surface` | `#[test]` | 408 |
| 98 | core-domain | `crates/quanta-memory-platform-core/tests/policy_contracts.rs:458` | `preset_broker_global_limit_is_consistent_with_enforcement` | `#[test]` | 457 |
| 99 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2274` | `overlong_owner_is_denied_before_reclaim_or_accounting` | `#[test]` | 2273 |
| 100 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2291` | `scratch_reserve_cap_overflow_is_denied_without_mutation` | `#[test]` | 2290 |
| 101 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2323` | `scratch_recharge_cap_overflow_preserves_original_ticket` | `#[test]` | 2322 |
| 102 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2367` | `dropping_pending_reservation_rolls_back_all_axes` | `#[test]` | 2366 |
| 103 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2388` | `dropping_pending_reservation_during_rebuild_does_not_strand_capacity` | `#[test]` | 2387 |
| 104 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2410` | `failed_pending_commit_during_rebuild_rolls_back_capacity` | `#[test]` | 2409 |
| 105 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2429` | `pending_drop_restores_resource_group_capacity` | `#[test]` | 2428 |
| 106 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2473` | `deferred_committed_cleanup_is_batched_and_drains_large_backlog` | `#[test]` | 2472 |
| 107 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2508` | `managed_charge_drop_on_healthy_broker_releases_exact_capacity` | `#[test]` | 2507 |
| 108 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2523` | `managed_charge_drop_during_rebuild_defers_until_finish` | `#[test]` | 2522 |
| 109 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2541` | `managed_charge_explicit_release_failure_retains_authority` | `#[test]` | 2540 |
| 110 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2560` | `managed_charge_batch_failure_retains_every_lease` | `#[test]` | 2559 |
| 111 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2578` | `managed_charge_batch_rejects_foreign_broker_before_mutation` | `#[test]` | 2577 |
| 112 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2597` | `managed_batch_invalid_lease_rejects_before_ticket_collection` | `#[test]` | 2596 |
| 113 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2767` | `managed_charge_recharge_denial_restores_old_authority` | `#[test]` | 2766 |
| 114 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2802` | `managed_recharge_at_live_ticket_cap_keeps_broker_authority_maps_exact` | `#[test]` | 2801 |
| 115 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2844` | `replace_denial_returns_original_live_charge` | `#[test]` | 2843 |
| 116 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2873` | `owned_charge_sequence_matches_exact_reference_model` | `#[test]` | 2872 |
| 117 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2947` | `reserve_commit_keeps_class_owner_and_partition_balanced` | `#[test]` | 2946 |
| 118 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:2989` | `reserve_denied_owner_hard_cap_leaves_ledger_untouched` | `#[test]` | 2988 |
| 119 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3025` | `reserve_denied_class_hard_cap_no_borrow` | `#[test]` | 3024 |
| 120 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3068` | `reserve_borrow_beyond_shared_pool_denied` | `#[test]` | 3067 |
| 121 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3089` | `release_returns_bytes_and_preserves_invariants` | `#[test]` | 3088 |
| 122 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3112` | `rebuild_in_progress_rejects_reserve_commit_release` | `#[test]` | 3111 |
| 123 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3149` | `live_reconfig_shrink_below_usage_denied_and_noop` | `#[test]` | 3148 |
| 124 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3180` | `live_reconfig_class_hard_cap_below_class_usage_denied` | `#[test]` | 3179 |
| 125 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3206` | `live_reconfig_dropping_a_class_with_live_usage_is_denied` | `#[test]` | 3205 |
| 126 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3241` | `live_reconfig_owner_quota_below_owner_usage_is_denied` | `#[test]` | 3240 |
| 127 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3270` | `live_reconfig_raising_headroom_over_non_headroom_usage_is_denied` | `#[test]` | 3269 |
| 128 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3315` | `live_reconfig_lowering_group_cap_below_live_group_usage_is_denied` | `#[test]` | 3314 |
| 129 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3368` | `live_reconfig_raise_caps_accepted_and_preserves_usage` | `#[test]` | 3367 |
| 130 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3403` | `allocator_advisory_mismatch_does_not_mutate_accounting` | `#[test]` | 3402 |
| 131 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3434` | `pressure_classifies_from_semantic_usage` | `#[test]` | 3433 |
| 132 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3496` | `denied_reconfigure_leaves_scheduler_controller_and_report_unchanged` | `#[test]` | 3495 |
| 133 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3574` | `accepted_reconfigure_rebuilds_controller_to_new_global_limit` | `#[test]` | 3573 |
| 134 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3632` | `reconfigure_rejects_policy_missing_drained_mandatory_class` | `#[test]` | 3631 |
| 135 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3646` | `reconfigure_missing_class_denial_preserves_policy_scheduler_and_usage` | `#[test]` | 3645 |
| 136 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3679` | `pressure_thresholds_are_inclusive_at_exact_boundary_fractions` | `#[test]` | 3678 |
| 137 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3725` | `rebuild_fence_is_single_owner_finishes_once_and_recovers_after_abandon` | `#[test]` | 3724 |
| 138 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3799` | `begin_rebuild_epoch_exhaustion_leaves_fence_state_untouched` | `#[test]` | 3798 |
| 139 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3827` | `rebuild_fence_rejects_foreign_clone_finish_and_second_live_begin` | `#[test]` | 3826 |
| 140 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3855` | `rebuild_fence_gates_reclaim_slow_path_before_capacity_breaches` | `#[test]` | 3854 |
| 141 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:3941` | `live_reconfig_denies_removing_a_quota_while_the_owner_holds_authority` | `#[test]` | 3940 |
| 142 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4003` | `reentrant_recharge_from_reclaimer_callback_is_bounded_and_typed` | `#[test]` | 4002 |
| 143 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4098` | `trait_effective_report_matches_inherent_and_is_not_fabricated` | `#[test]` | 4097 |
| 144 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4123` | `reserve_gated_by_resource_group_admission_fail_closed` | `#[test]` | 4122 |
| 145 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4182` | `owner_without_group_mapping_is_not_gated_by_admission` | `#[test]` | 4181 |
| 146 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4210` | `execution_scratch_budget_caps_live_scratch_and_spares_other_kinds` | `#[test]` | 4209 |
| 147 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4263` | `execution_scratch_budget_accepts_exact_cap_and_denies_one_more` | `#[test]` | 4262 |
| 148 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4310` | `live_runtime_scratch_charge_appears_in_ledger_snapshot` | `#[test]` | 4309 |
| 149 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4340` | `scratch_peak_denial_release_and_drop_keep_authority_exact` | `#[test]` | 4339 |
| 150 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4401` | `live_runtime_scratch_over_cap_is_denied_without_mutation` | `#[test]` | 4400 |
| 151 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4423` | `runtime_scratch_and_direct_broker_charge_share_one_limit` | `#[test]` | 4422 |
| 152 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4448` | `dropping_runtime_scratch_during_rebuild_returns_capacity` | `#[test]` | 4447 |
| 153 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4469` | `runtime_scratch_rejects_foreign_runtime_context_without_mutation` | `#[test]` | 4468 |
| 154 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4488` | `runtime_scratch_release_restores_resource_group_capacity` | `#[test]` | 4487 |
| 155 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4533` | `recharge_denied_by_scratch_budget_preserves_old_scratch_charge` | `#[test]` | 4532 |
| 156 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4574` | `concurrent_reserves_never_exceed_global_limit_no_toctou` | `#[test]` | 4573 |
| 157 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4614` | `broker_clones_share_one_ledger` | `#[test]` | 4613 |
| 158 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4641` | `broker_is_send_and_sync` | `#[test]` | 4640 |
| 159 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4649` | `accepted_reconfigure_preserves_scheduler_group_usage` | `#[test]` | 4648 |
| 160 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4699` | `remapping_live_owner_group_is_denied_fail_closed` | `#[test]` | 4698 |
| 161 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4790` | `reserve_committed_during_rebuild_is_fail_closed_no_strand` | `#[test]` | 4789 |
| 162 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4814` | `reserve_committed_charges_exactly_once` | `#[test]` | 4813 |
| 163 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4833` | `recharge_swaps_committed_charge_atomically` | `#[test]` | 4832 |
| 164 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4865` | `recharge_denied_preserves_original_old_ticket` | `#[test]` | 4864 |
| 165 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4910` | `denied_recharge_preserves_the_old_axis_not_the_new_one` | `#[test]` | 4909 |
| 166 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4955` | `recharge_during_rebuild_leaves_old_intact` | `#[test]` | 4954 |
| 167 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:4991` | `recharge_does_not_transiently_double_count_group_admission` | `#[test]` | 4990 |
| 168 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5048` | `denied_recharge_on_grouped_owner_preserves_group_usage` | `#[test]` | 5047 |
| 169 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5105` | `recharge_cannot_consume_peer_group_reserved_floor` | `#[test]` | 5104 |
| 170 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5179` | `managed_charge_drop_recovers_poisoned_lock_for_exact_release` | `#[test]` | 5178 |
| 171 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5194` | `poisoned_runtime_scratch_explicit_release_preserves_then_drop_recovers_authority` | `#[test]` | 5193 |
| 172 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5219` | `poisoned_pending_commit_retains_ticket_until_drop_rollback` | `#[test]` | 5218 |
| 173 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5249` | `poisoned_broker_fails_closed_and_trait_report_stays_truthful` | `#[test]` | 5248 |
| 174 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5327` | `bounded_events_execute_outside_lock_and_cover_major_broker_outcomes` | `#[test]` | 5326 |
| 175 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5390` | `telemetry_panic_cannot_change_or_abort_accounting` | `#[test]` | 5389 |
| 176 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5441` | `sink_panic_payloads_are_retired_once_and_callback_is_quarantined` | `#[test]` | 5440 |
| 177 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5492` | `reclaimer_panic_payloads_are_retired_once_and_admission_remeasures` | `#[test]` | 5491 |
| 178 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5557` | `event_sink_observation_reentry_delivers_outer_event_once` | `#[test]` | 5556 |
| 179 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5611` | `same_broker_reclaimer_reserve_reentry_is_bounded_and_typed` | `#[test]` | 5610 |
| 180 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5685` | `concurrent_reclaimer_reentry_does_not_start_a_callback_storm` | `#[test]` | 5684 |
| 181 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5728` | `independent_outer_reserves_share_one_in_flight_reclaimer` | `#[test]` | 5727 |
| 182 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5814` | `soft_and_hard_reclaim_plans_share_one_callback_in_both_arrival_orders` | `#[test]` | 5813 |
| 183 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:5934` | `held_reclaim_remeasures_concurrent_policy_and_slot_changes` | `#[test]` | 5933 |
| 184 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6023` | `concurrent_recharge_and_observe_never_expose_torn_axes` | `#[test]` | 6022 |
| 185 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6118` | `forged_snapshot_has_fallible_totals_without_panicking` | `#[test]` | 6117 |
| 186 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6140` | `concurrent_reserve_and_reconfigure_preserve_group_cap_truth` | `#[test]` | 6139 |
| 187 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6276` | `soft_cap_crossing_triggers_reclaim_without_immediate_hard_deny` | `#[test]` | 6275 |
| 188 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6306` | `owner_and_group_soft_caps_each_trigger_the_same_reclaim_lane` | `#[test]` | 6305 |
| 189 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6363` | `hard_cap_denial_reclaims_outside_lock_remeasures_and_retries_once` | `#[test]` | 6362 |
| 190 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6406` | `group_reserved_floor_is_unavailable_to_peer_group` | `#[test]` | 6405 |
| 191 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6464` | `grouped_headroom_preserves_protected_floor_across_reconfigure_and_release` | `#[test]` | 6463 |
| 192 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6565` | `accepted_reconfigure_updates_revision_source_and_matches_headroom_predicate` | `#[test]` | 6564 |
| 193 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6604` | `identity_reconfigure_succeeds_during_legal_class_borrow` | `#[test]` | 6603 |
| 194 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6624` | `identity_reconfigure_succeeds_during_legal_group_burst` | `#[test]` | 6623 |
| 195 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6654` | `reclaim_priority_orders_victims_deterministically` | `#[test]` | 6653 |
| 196 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6770` | `sep23_fup007_zero_byte_committed_blocks_group_removal_until_explicit_release` | `#[test]` | 6769 |
| 197 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6794` | `sep23_fup007_zero_byte_committed_drop_after_denied_reconfigure_releases` | `#[test]` | 6793 |
| 198 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6807` | `sep23_fup007_zero_byte_committed_rebuild_fence_drop_drains_on_finish` | `#[test]` | 6806 |
| 199 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6823` | `sep23_fup007_zero_byte_pending_blocks_removal_explicit_rollback_unblocks` | `#[test]` | 6822 |
| 200 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6847` | `sep23_fup007_zero_byte_pending_drop_rolls_back_without_stranding` | `#[test]` | 6846 |
| 201 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6867` | `sep23_fup007_denied_reconfigure_preserves_policy_revision_scheduler_ledger` | `#[test]` | 6866 |
| 202 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6887` | `sep23_fup007_mixed_zero_and_nonzero_partial_termination` | `#[test]` | 6886 |
| 203 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:6919` | `sep23_fup007_scope_matrix_remap_cap_ungrouped_terminated` | `#[test]` | 6918 |
| 204 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7012` | `sep23_fup005_ten_thousand_reconfigures_keep_provenance_bounded` | `#[test]` | 7011 |
| 205 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7055` | `sep23_fup005_denied_reconfigure_preserves_provenance_byte_for_byte` | `#[test]` | 7054 |
| 206 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7119` | `sep23_fup003_terminal_matrix_never_fires_callback` | `#[test]` | 7118 |
| 207 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7245` | `sep23_fup003_borrowing_class_hard_is_not_terminal_until_pool_exhausted` | `#[test]` | 7244 |
| 208 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7296` | `sep23_fup003_blocked_matrix_never_fires_callback` | `#[test]` | 7295 |
| 209 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7337` | `sep23_fup003_ticket_exhaustion_blocks_without_callback` | `#[test]` | 7336 |
| 210 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7401` | `live_ticket_count_cap_runs_one_reclaim_then_remeasures` | `#[test]` | 7400 |
| 211 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7491` | `live_ticket_count_cap_can_recover_when_host_releases_zero_byte_charge` | `#[test]` | 7490 |
| 212 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7558` | `sep23_fup003_predicted_failure_skips_soft_and_runs_single_hard` | `#[test]` | 7557 |
| 213 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7613` | `sep23_fup003_hard_remeasure_still_denied_is_terminal_without_second_callback` | `#[test]` | 7612 |
| 214 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7678` | `sep23_fup006_accepted_reconfigure_inside_callback_remeasures_under_new_policy` | `#[test]` | 7677 |
| 215 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7753` | `sep23_audit_max_scale_plus_one_byte_denies_clean_without_callback` | `#[test]` | 7752 |
| 216 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7828` | `sep23_audit_failing_pending_drain_preserves_ticket_for_retry` | `#[test]` | 7827 |
| 217 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7884` | `sep23_audit_failing_committed_drain_restores_whole_batch` | `#[test]` | 7883 |
| 218 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7909` | `competing_rebuild_fence_preserves_deferred_authority_and_original_error` | `#[test]` | 7908 |
| 219 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:7946` | `sep23_fup006_denied_reconfigure_inside_callback_preserves_receipt_and_admits` | `#[test]` | 7945 |
| 220 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:8032` | `sep23_audit_group_denial_runs_single_hard_regardless_of_soft_crossing` | `#[test]` | 8031 |
| 221 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:8080` | `sep23_audit_wrong_victim_release_does_not_admit` | `#[test]` | 8079 |
| 222 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:8132` | `sep23_audit_forged_reclaimed_bytes_are_recorded_but_never_trusted` | `#[test]` | 8131 |
| 223 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:8170` | `sep23_audit_panicking_reclaimer_is_contained_and_broker_survives` | `#[test]` | 8169 |
| 224 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:8233` | `reclaimer_release_then_panic_uses_remeasured_ledger_not_callback_result` | `#[test]` | 8232 |
| 225 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:8296` | `sep23_audit_zero_byte_reserve_fires_no_callback_even_over_soft` | `#[test]` | 8295 |
| 226 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:8337` | `terminal_preflight_preserves_the_live_first_breach_when_zero_axis_differs` | `#[test]` | 8336 |
| 227 | core-api | `crates/quanta-memory-platform-core/src/api/control.rs:8471` | `sep23_fup003_preflight_attempt_agreement_randomized` | `#[test]` | 8470 |
| 228 | core-api | `crates/quanta-memory-platform-core/src/api/builder.rs:171` | `build_without_manifest_returns_build_error` | `#[test]` | 170 |
| 229 | core-api | `crates/quanta-memory-platform-core/src/api/builder.rs:183` | `build_with_invalid_residency_returns_config_error` | `#[test]` | 182 |
| 230 | core-api | `crates/quanta-memory-platform-core/src/api/builder.rs:203` | `build_with_soft_above_hard_returns_config_error` | `#[test]` | 202 |
| 231 | core-api | `crates/quanta-memory-platform-core/src/api/builder.rs:225` | `build_with_valid_residency_succeeds_and_reports_that_residency` | `#[test]` | 224 |
| 232 | core-api | `crates/quanta-memory-platform-core/src/api/builder.rs:249` | `builder_rejects_manifest_missing_mandatory_class` | `#[test]` | 248 |
| 233 | core-api | `crates/quanta-memory-platform-core/src/api/builder.rs:266` | `builder_reclaimer_reaches_build_live_broker` | `#[test]` | 265 |
| 234 | core-api | `crates/quanta-memory-platform-core/src/api/builder.rs:332` | `report_only_builder_applies_override_layers` | `#[test]` | 331 |
| 235 | core-api | `crates/quanta-memory-platform-core/tests/public_api_surface.rs:182` | `memory_error_has_exactly_the_nine_documented_variants` | `#[test]` | 181 |
| 236 | core-api | `crates/quanta-memory-platform-core/tests/public_api_surface.rs:231` | `memory_class_taxonomy_is_stable` | `#[test]` | 230 |
| 237 | core-api | `crates/quanta-memory-platform-core/tests/public_api_surface.rs:256` | `charge_kind_has_exactly_three_partitions` | `#[test]` | 255 |
| 238 | core-api | `crates/quanta-memory-platform-core/tests/public_api_surface.rs:274` | `memory_preset_has_exactly_the_four_documented_variants` | `#[test]` | 273 |
| 239 | core-api | `crates/quanta-memory-platform-core/tests/public_api_surface.rs:298` | `builder_first_surface_produces_a_broker_from_a_preset` | `#[test]` | 296 |
| 240 | core-api | `crates/quanta-memory-platform-core/tests/public_api_surface.rs:322` | `builder_without_manifest_fails_closed_with_build_variant` | `#[test]` | 321 |
| 241 | core-api | `crates/quanta-memory-platform-core/tests/public_api_surface.rs:331` | `report_only_builder_returns_data_without_runtime_authority` | `#[test]` | 330 |
| 242 | core-api | `crates/quanta-memory-platform-core/tests/public_api_surface.rs:346` | `pressure_thresholds_construct_and_getters_reach_the_external_path` | `#[test]` | 344 |
| 243 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:279` | `classification_is_deterministic_across_the_ladder` | `#[test]` | 278 |
| 244 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:292` | `update_caches_current_pressure` | `#[test]` | 291 |
| 245 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:300` | `ladder_gets_more_aggressive_with_pressure` | `#[test]` | 299 |
| 246 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:314` | `reclaim_returns_zero_under_fatal_pressure_without_spill` | `#[test]` | 313 |
| 247 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:328` | `reclaim_uses_spillable_under_fatal_pressure` | `#[test]` | 327 |
| 248 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:341` | `first_priority_owner_does_not_self_reclaim_below_fatal` | `#[test]` | 340 |
| 249 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:354` | `thresholds_new_accepts_monotonic_and_default_is_valid` | `#[test]` | 353 |
| 250 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:384` | `thresholds_new_rejects_inverted_or_out_of_range` | `#[test]` | 383 |
| 251 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:406` | `zero_limit_is_normal_when_empty_and_fatal_when_used` | `#[test]` | 405 |
| 252 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:421` | `extreme_limits_classify_percent_without_wrapping_or_saturating_product` | `#[test]` | 420 |
| 253 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:448` | `ladder_transitions_up_then_down_through_every_rung` | `#[test]` | 447 |
| 254 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:473` | `update_from_semantic_is_idempotent` | `#[test]` | 472 |
| 255 | app-ports | `crates/quanta-memory-platform-core/src/application/controller.rs:488` | `allocator_snapshot_observation_does_not_change_pressure` | `#[test]` | 487 |
| 256 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:298` | `within_hard_cap_is_always_admitted` | `#[test]` | 297 |
| 257 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:305` | `unknown_group_is_denied_fail_closed` | `#[test]` | 304 |
| 258 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:321` | `noisy_group_cannot_borrow_beyond_weighted_share` | `#[test]` | 320 |
| 259 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:348` | `release_returns_burst_capacity` | `#[test]` | 347 |
| 260 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:357` | `prospective_floor_aggregate_matches_committed_growth_and_release` | `#[test]` | 356 |
| 261 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:382` | `headroom_usage_never_consumes_a_non_headroom_group_floor` | `#[test]` | 381 |
| 262 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:421` | `empty_scheduler_has_zero_share_and_denies_all` | `#[test]` | 420 |
| 263 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:434` | `unknown_group_denied_even_with_pool_available` | `#[test]` | 433 |
| 264 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:445` | `weighted_share_distributes_remainder_work_conservingly` | `#[test]` | 444 |
| 265 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:462` | `burst_percent_changes_admission_outcome` | `#[test]` | 461 |
| 266 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:476` | `usage_release_round_trip_readmits_burst` | `#[test]` | 475 |
| 267 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:502` | `huge_pool_and_weights_compute_a_non_wrapping_fair_share` | `#[test]` | 501 |
| 268 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:520` | `admission_reuses_policy_static_fair_share_table_at_high_group_cardinality` | `#[test]` | 519 |
| 269 | app-ports | `crates/quanta-memory-platform-core/src/application/scheduler.rs:557` | `repeated_exact_usage_release_round_trip_never_wraps` | `#[test]` | 556 |
| 270 | app-ports | `crates/quanta-memory-platform-core/src/adapters/pin.rs:87` | `pin_invalidate_round_trip_tracks_dependent_count` | `#[test]` | 86 |
| 271 | app-ports | `crates/quanta-memory-platform-core/src/adapters/pin.rs:117` | `invalidation_outcome_reports_exact_dependents` | `#[test]` | 116 |
| 272 | app-ports | `crates/quanta-memory-platform-core/src/adapters/pin.rs:143` | `duplicate_pin_does_not_over_count_dependent` | `#[test]` | 142 |
| 273 | app-ports | `crates/quanta-memory-platform-core/src/adapters/pin.rs:166` | `distinct_dependents_each_increment_count` | `#[test]` | 165 |
| 274 | app-ports | `crates/quanta-memory-platform-core/src/adapters/pin.rs:182` | `poisoned_lock_is_fail_closed_not_panicking` | `#[test]` | 181 |
| 275 | app-ports | `crates/quanta-memory-platform-core/src/adapters/metrics.rs:47` | `emitter_surface_is_emit_only_and_takes_shared_ref` | `#[test]` | 46 |
| 276 | app-ports | `crates/quanta-memory-platform-core/src/adapters/metrics.rs:69` | `emitter_holds_no_accounting_state` | `#[test]` | 68 |
| 277 | app-ports | `crates/quanta-memory-platform-core/src/adapters/hdrhistogram.rs:90` | `records_deterministic_set_and_reads_back_count_and_percentiles` | `#[test]` | 89 |
| 278 | app-ports | `crates/quanta-memory-platform-core/src/adapters/hdrhistogram.rs:118` | `record_is_monotonic_and_lossless_in_count` | `#[test]` | 117 |
| 279 | app-ports | `crates/quanta-memory-platform-core/src/adapters/hdrhistogram.rs:129` | `invalid_bounds_return_error_not_panic` | `#[test]` | 128 |
| 280 | app-ports | `crates/quanta-memory-platform-core/src/adapters/hdrhistogram.rs:140` | `out_of_range_record_is_rejected_and_does_not_mutate_count` | `#[test]` | 139 |
| 281 | app-ports | `crates/quanta-memory-platform-core/src/ports/scratch.rs:203` | `reservation_maps_to_live_scratch_charge_kind` | `#[test]` | 202 |
| 282 | app-ports | `crates/quanta-memory-platform-core/src/ports/scratch.rs:219` | `grow_to_observed_updates_within_reserved_ceiling` | `#[test]` | 218 |
| 283 | app-ports | `crates/quanta-memory-platform-core/src/ports/scratch.rs:231` | `grow_beyond_reserved_is_fail_closed_capacity_error` | `#[test]` | 230 |
| 284 | app-ports | `crates/quanta-memory-platform-core/src/ports/scratch.rs:250` | `scratch_peak_never_decreases_on_smaller_observation` | `#[test]` | 249 |
| 285 | app-ports | `crates/quanta-memory-platform-core/src/ports/scratch.rs:284` | `grow_after_release_is_rejected_and_fail_closed` | `#[test]` | 283 |
| 286 | app-ports | `crates/quanta-memory-platform-core/src/ports/scratch.rs:305` | `double_release_is_rejected_not_silent` | `#[test]` | 304 |
| 287 | app-ports | `crates/quanta-memory-platform-core/src/ports/scratch.rs:321` | `lifecycle_reserve_grow_release_succeeds_in_order` | `#[test]` | 320 |
| 288 | app-ports | `crates/quanta-memory-platform-core/src/ports/scratch.rs:346` | `failed_release_preserves_authority_for_retry` | `#[test]` | 345 |
| 289 | app-ports | `crates/quanta-memory-platform-core/src/ports/scratch.rs:364` | `successful_release_prevents_drop_from_releasing_twice` | `#[test]` | 363 |
| 290 | app-ports | `crates/quanta-memory-platform-core/src/ports/telemetry.rs:203` | `operational_labels_are_closed_bounded_sets` | `#[test]` | 202 |
| 291 | app-ports | `crates/quanta-memory-platform-core/src/ports/telemetry.rs:239` | `advisory_agreement_has_no_mismatch` | `#[test]` | 238 |
| 292 | app-ports | `crates/quanta-memory-platform-core/src/ports/telemetry.rs:248` | `missing_allocator_data_is_not_treated_as_mismatch` | `#[test]` | 247 |
| 293 | app-ports | `crates/quanta-memory-platform-core/src/ports/telemetry.rs:256` | `static_telemetry_returns_its_snapshot_verbatim` | `#[test]` | 255 |
| 294 | app-ports | `crates/quanta-memory-platform-core/src/ports/telemetry.rs:266` | `compare_reports_signed_drift_and_mismatch_boundary` | `#[test]` | 265 |
| 295 | app-ports | `crates/quanta-memory-platform-core/src/ports/telemetry.rs:308` | `compare_semantic_bytes_is_ssot_not_allocator` | `#[test]` | 307 |
| 296 | app-ports | `crates/quanta-memory-platform-core/src/ports/telemetry.rs:328` | `allocator_telemetry_trait_surface_is_observation_only` | `#[test]` | 327 |
| 297 | app-ports | `crates/quanta-memory-platform-core/src/ports/telemetry.rs:351` | `trait_path_absent_allocator_data_never_contradicts_ssot` | `#[test]` | 350 |
| 298 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1446` | `sparse_reclaim_partition_and_rollback_preserve_large_fifo` | `#[test]` | 1445 |
| 299 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1556` | `compound_measurement_and_drop_panic_does_not_abort_insert` | `#[test]` | 1555 |
| 300 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1610` | `insert_tracks_reclaimable_bytes` | `#[test]` | 1609 |
| 301 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1623` | `reclaim_one_is_fifo_and_releases_its_bytes` | `#[test]` | 1622 |
| 302 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1642` | `same_store_reclaimer_returns_busy_without_losing_fifo` | `#[test]` | 1641 |
| 303 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1738` | `reclaim_to_in_memory_tier_keeps_bytes_charged` | `#[test]` | 1737 |
| 304 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1773` | `many_spills_share_large_binding_identity_without_unbilled_clones` | `#[test]` | 1772 |
| 305 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1847` | `spill_and_close_emit_bounded_facade_events` | `#[test]` | 1846 |
| 306 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1872` | `failed_destination_write_preserves_source_entry_and_charge` | `#[test]` | 1871 |
| 307 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1906` | `noop_spill_does_not_discard_source` | `#[test]` | 1905 |
| 308 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1922` | `foreign_store_cannot_adopt_spill_charge` | `#[test]` | 1921 |
| 309 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1943` | `source_close_does_not_refund_live_in_process_tier` | `#[test]` | 1942 |
| 310 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1959` | `closed_source_rejects_restored_handle_without_losing_its_charge` | `#[test]` | 1958 |
| 311 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1997` | `durable_spill_releases_source_only_after_commit` | `#[test]` | 1996 |
| 312 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2085` | `failed_durable_write_and_encoder_error_restore_source_fifo_and_charge` | `#[test]` | 2084 |
| 313 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2170` | `bounded_codec_spill_preflights_backend_and_broker_before_allocating` | `#[test]` | 2169 |
| 314 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2216` | `bounded_codec_cold_reopen_restore_denial_and_retry_keep_one_authority` | `#[test]` | 2215 |
| 315 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2265` | `bounded_codec_rejects_malformed_durable_record_without_publication` | `#[test]` | 2264 |
| 316 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2296` | `bounded_codec_keeps_output_charged_until_durable_commit_returns` | `#[test]` | 2295 |
| 317 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2366` | `bounded_codec_committed_recovery_retries_after_capacity_denial` | `#[test]` | 2365 |
| 318 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2411` | `panicking_durable_encoder_restores_source_before_unwind` | `#[test]` | 2410 |
| 319 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2469` | `panicking_durable_commit_fences_source_until_exact_record_resolution` | `#[test]` | 2468 |
| 320 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2569` | `durable_redb_round_trip_transfers_charge_exactly_once` | `#[test]` | 2568 |
| 321 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2601` | `oversized_encoded_record_preserves_charged_source_and_no_durable_payload` | `#[test]` | 2600 |
| 322 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2645` | `redb_file_cap_denial_keeps_store_source_charged_until_exact_resolution` | `#[test]` | 2644 |
| 323 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2725` | `durable_redb_committed_restore_recovers_after_process_restart` | `#[test]` | 2724 |
| 324 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2778` | `durable_redb_prepared_restore_recovers_after_process_restart` | `#[test]` | 2777 |
| 325 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:2834` | `sep23_fup001_uncertain_commit_keeps_source_fenced_and_blocks_wrong_retire` | `#[test]` | 2833 |
| 326 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3038` | `sep23_fup002_concurrent_recovery_has_exactly_one_owner` | `#[test]` | 3037 |
| 327 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3144` | `sep23_audit_panicking_decode_aborts_prepared_claim_and_retries_clean` | `#[test]` | 3143 |
| 328 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3214` | `panicking_restore_measurement_aborts_prepared_claim_and_retries_clean` | `#[test]` | 3213 |
| 329 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3270` | `panicking_recovery_measurement_preserves_committed_claim_for_retry` | `#[test]` | 3269 |
| 330 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3326` | `panicking_restore_commit_fences_resident_until_redb_reconciliation` | `#[test]` | 3325 |
| 331 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3432` | `panicking_prepare_after_persisted_claim_has_explicit_recovery_path` | `#[test]` | 3431 |
| 332 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3519` | `sep23_audit_panicking_decode_preserves_committed_recovery_until_retry` | `#[test]` | 3518 |
| 333 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3565` | `sep23_fup002_stolen_claim_reconcile_retires_provisional_copy` | `#[test]` | 3564 |
| 334 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3646` | `sep23_fup002_decode_failure_and_denied_admission_preserve_durable` | `#[test]` | 3645 |
| 335 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3703` | `sep23_fup002_commit_ack_loss_same_claimant_publishes_on_retry` | `#[test]` | 3702 |
| 336 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3764` | `denied_durable_restore_leaves_redb_record_authoritative` | `#[test]` | 3763 |
| 337 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3795` | `failed_durable_delete_rolls_back_provisional_source_charge` | `#[test]` | 3794 |
| 338 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3872` | `ambiguous_durable_delete_fences_source_until_absence_is_reconciled` | `#[test]` | 3871 |
| 339 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:3995` | `panicking_finalize_fences_published_source_until_reconciliation` | `#[test]` | 3994 |
| 340 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4125` | `reclaim_one_on_empty_returns_none` | `#[test]` | 4124 |
| 341 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4132` | `reclaim_until_frees_at_least_target_and_reports_reclaimed_bytes` | `#[test]` | 4131 |
| 342 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4146` | `reclaim_until_on_empty_store_reports_zero` | `#[test]` | 4145 |
| 343 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4154` | `reclaim_until_charges_and_evicts_zero_payload_keys` | `#[test]` | 4153 |
| 344 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4172` | `reclaim_until_of_zero_payload_entries_reclaims_their_key_and_index_cost` | `#[test]` | 4171 |
| 345 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4185` | `revoke_makes_entry_unretrievable_and_decrements_accounting` | `#[test]` | 4184 |
| 346 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4195` | `empty_reclaim_leaves_state_byte_for_byte_unchanged` | `#[test]` | 4194 |
| 347 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4203` | `insert_over_cap_is_denied_and_fails_closed` | `#[test]` | 4202 |
| 348 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4213` | `large_key_with_zero_value_is_charged_and_denied_without_state_change` | `#[test]` | 4212 |
| 349 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4223` | `cost_component_sum_overflow_is_fail_closed_before_charge` | `#[test]` | 4222 |
| 350 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4244` | `insert_charges_and_reclaim_refunds_the_ledger` | `#[test]` | 4243 |
| 351 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4263` | `close_refunds_every_live_entry_exactly_once_and_stays_closed` | `#[test]` | 4262 |
| 352 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4281` | `close_release_failure_preserves_entries_fifo_and_ledger_authority` | `#[test]` | 4280 |
| 353 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4301` | `reclaim_batch_failure_preserves_every_entry_and_charge` | `#[test]` | 4300 |
| 354 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4318` | `reclaim_one_during_rebuild_is_fail_closed` | `#[test]` | 4317 |
| 355 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4333` | `reclaim_until_during_rebuild_is_fail_closed` | `#[test]` | 4332 |
| 356 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4349` | `drop_without_explicit_close_releases_capacity` | `#[test]` | 4348 |
| 357 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4363` | `drop_during_rebuild_defers_exact_release_until_finish` | `#[test]` | 4362 |
| 358 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4378` | `drop_during_outer_unwind_contains_two_client_panics_and_refunds_all_charges` | `#[test]` | 4377 |
| 359 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4410` | `concurrent_insert_and_close_have_one_linearized_outcome` | `#[test]` | 4409 |
| 360 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4443` | `counter_overflow_is_rejected_before_broker_charge_or_mutation` | `#[test]` | 4442 |
| 361 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4455` | `poisoned_stats_is_error_not_empty_and_drop_still_releases` | `#[test]` | 4454 |
| 362 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4473` | `reentrant_read_and_close_under_state_lock_return_typed_busy` | `#[test]` | 4472 |
| 363 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4490` | `constructed_budgeted_store_exposes_its_single_broker_authority` | `#[test]` | 4489 |
| 364 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4497` | `reclaim_eviction_order_is_deterministic_fifo_at_capacity_boundary` | `#[test]` | 4496 |
| 365 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:4511` | `unbudgeted_store_is_a_distinct_explicit_product_type` | `#[test]` | 4510 |
| 366 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/durable_codec.rs:114` | `literal_v1_record_decodes_without_using_the_current_encoder` | `#[test]` | 113 |
| 367 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/durable_codec.rs:127` | `versioned_codec_round_trip_and_exact_cost` | `#[test]` | 126 |
| 368 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/durable_codec.rs:138` | `malformed_or_forged_lengths_never_reach_allocation` | `#[test]` | 137 |
| 369 | revocable-store | `crates/quanta-memory-platform-facade/src/revocable_store/cost.rs:26` | `revocable_entry_cost_has_one_checked_formula` | `#[test]` | 25 |
| 370 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1372` | `reentrant_load_stats_and_close_under_read_lock_are_typed_busy` | `#[test]` | 1371 |
| 371 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1385` | `partial_snapshot_clone_and_drop_panics_cannot_abort` | `#[test]` | 1384 |
| 372 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1479` | `partial_load_clone_and_drop_panics_cannot_abort` | `#[test]` | 1478 |
| 373 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1575` | `new_session_clone_and_drop_panics_cannot_abort` | `#[test]` | 1574 |
| 374 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1651` | `owned_session_key_order_and_drop_panics_cannot_abort` | `#[test]` | 1650 |
| 375 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1743` | `partial_append_identity_clone_and_drop_panics_cannot_abort` | `#[test]` | 1742 |
| 376 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1843` | `client_message_clone_during_load_reenters_as_typed_busy_without_ledger_drift` | `#[test]` | 1842 |
| 377 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1958` | `exact_request_retried_n_times_applies_once` | `#[test]` | 1957 |
| 378 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1975` | `append_truncate_cycles_cannot_grow_past_fail_closed_evidence_bound` | `#[test]` | 1974 |
| 379 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1998` | `exact_retry_and_conflicts_remain_deterministic_at_saturation` | `#[test]` | 1997 |
| 380 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2019` | `tombstone_byte_limit_counts_large_message_ids` | `#[test]` | 2018 |
| 381 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2041` | `retention_reconfigure_below_live_evidence_is_rejected` | `#[test]` | 2040 |
| 382 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2072` | `retention_stats_separate_active_evidence_and_session_costs` | `#[test]` | 2071 |
| 383 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2110` | `same_identity_with_body_drift_is_payload_conflict` | `#[test]` | 2109 |
| 384 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2119` | `reused_sequence_with_different_message_id_is_typed_conflict` | `#[test]` | 2118 |
| 385 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2127` | `reused_message_id_at_different_sequence_is_typed_conflict` | `#[test]` | 2126 |
| 386 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2135` | `append_identity_is_partitioned_by_session` | `#[test]` | 2134 |
| 387 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2150` | `concurrent_identical_appends_have_one_applied_winner` | `#[test]` | 2149 |
| 388 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2182` | `concurrent_conflicting_appends_have_one_applied_winner` | `#[test]` | 2181 |
| 389 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2226` | `truncate_retains_tombstone_so_late_retry_cannot_reappend` | `#[test]` | 2225 |
| 390 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2244` | `truncate_to_zero_retains_all_identity_tombstones` | `#[test]` | 2243 |
| 391 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2267` | `zero_body_with_large_ids_and_session_retains_semantic_evidence_cost` | `#[test]` | 2266 |
| 392 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2288` | `selective_retain_preserves_sparse_rows_and_evicted_identity_tombstones` | `#[test]` | 2287 |
| 393 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2310` | `selective_retain_rejects_invalid_selector_without_mutation` | `#[test]` | 2309 |
| 394 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2329` | `versioned_snapshot_retain_rejects_concurrent_append_without_dropping_rows` | `#[test]` | 2328 |
| 395 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2371` | `checked_load_reports_poisoned_storage_instead_of_empty_success` | `#[test]` | 2370 |
| 396 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2384` | `ordered_load_sorts_by_caller_sequence_and_allows_declared_gaps` | `#[test]` | 2383 |
| 397 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2630` | `append_message_id_clone_and_order_reentry_is_typed_busy` | `#[test]` | 2629 |
| 398 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2709` | `panicking_message_id_clone_at_each_publication_point_leaves_append_atomic` | `#[test]` | 2708 |
| 399 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2749` | `panicking_message_id_clone_cannot_publish_an_empty_new_session` | `#[test]` | 2748 |
| 400 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2796` | `panicking_new_session_order_after_charge_preserves_old_session_and_ledger` | `#[test]` | 2795 |
| 401 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2846` | `append_preflight_order_panic_keeps_public_store_available` | `#[test]` | 2845 |
| 402 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2890` | `append_preflight_message_order_panic_keeps_public_store_available` | `#[test]` | 2889 |
| 403 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2926` | `append_replay_canonical_key_panic_keeps_public_store_available` | `#[test]` | 2925 |
| 404 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:2982` | `panicking_message_id_order_at_publication_leaves_append_atomic` | `#[test]` | 2981 |
| 405 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3058` | `message_id_keyed_insert_panic_rolls_back_publication_completely` | `#[test]` | 3057 |
| 406 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3205` | `semantic_identity_component_overflow_denies_before_ledger_or_index_mutation` | `#[test]` | 3204 |
| 407 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3239` | `zero_body_large_identity_and_session_are_charged_until_evidence_retirement` | `#[test]` | 3238 |
| 408 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3266` | `retain_rollback_and_commit_do_not_compare_client_session_id_after_detach` | `#[test]` | 3265 |
| 409 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3324` | `panicking_session_order_before_detach_preserves_rows_and_ledger` | `#[test]` | 3323 |
| 410 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3377` | `active_row_and_session_evidence_own_separate_kind_buckets` | `#[test]` | 3376 |
| 411 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3427` | `compound_hostile_session_ord_panic_cannot_abort_append` | `#[test]` | 3426 |
| 412 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3551` | `session_order_panic_before_mutation_keeps_public_transcript_available` | `#[test]` | 3550 |
| 413 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3606` | `session_bytes_ord_panic_preserves_store_and_ledger` | `#[test]` | 3605 |
| 414 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3663` | `drop_contains_multiple_client_destructor_panics_during_outer_unwind` | `#[test]` | 3662 |
| 415 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3721` | `retain_retires_multiple_panicking_client_bodies_after_ledger_postimage` | `#[test]` | 3720 |
| 416 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3780` | `quota_denial_leaves_row_indexes_and_charge_unchanged` | `#[test]` | 3779 |
| 417 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3798` | `retention_saturation_denies_before_any_new_ledger_charge` | `#[test]` | 3797 |
| 418 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3816` | `retention_saturation_emits_one_bounded_tombstone_event` | `#[test]` | 3815 |
| 419 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3854` | `every_conflict_axis_preserves_rows_indexes_and_accounting` | `#[test]` | 3853 |
| 420 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3880` | `exact_retry_charges_once_and_truncate_refunds_only_active_body` | `#[test]` | 3879 |
| 421 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3901` | `truncate_refund_failure_preserves_rows_indexes_and_accounting` | `#[test]` | 3900 |
| 422 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3917` | `close_refunds_all_sessions_exactly_once_and_exposes_terminal_empty_reads` | `#[test]` | 3916 |
| 423 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3952` | `failed_close_preserves_rows_versions_tombstones_and_all_lease_authority` | `#[test]` | 3951 |
| 424 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:3981` | `drop_releases_all_live_rows_even_after_storage_poison` | `#[test]` | 3980 |
| 425 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:4003` | `drop_during_rebuild_defers_exact_release_until_finish` | `#[test]` | 4002 |
| 426 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:4019` | `mutation_lane_rejects_reentry_and_reads_without_exposing_partial_images` | `#[test]` | 4018 |
| 427 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:4043` | `checked_byte_and_version_overflow_denials_leave_ledger_and_indexes_intact` | `#[test]` | 4042 |
| 428 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:4074` | `failed_multirow_retain_preserves_positions_tombstones_and_version` | `#[test]` | 4073 |
| 429 | facade-stores | `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:4121` | `racing_close_and_append_have_one_linearizable_terminal_image` | `#[test]` | 4120 |
| 430 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:824` | `insert_commits_envelope_bytes` | `#[test]` | 823 |
| 431 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:837` | `committed_byte_overflow_rejects_before_payload_or_accounting_mutation_v1` | `#[test]` | 836 |
| 432 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:875` | `concurrent_insert_if_absent_has_one_creator_and_never_overwrites_v1` | `#[test]` | 874 |
| 433 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:923` | `concurrent_checked_read_observes_only_complete_old_or_new_payloads_v1` | `#[test]` | 922 |
| 434 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1205` | `compound_hostile_remove_panic_cannot_abort_process` | `#[test]` | 1204 |
| 435 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1227` | `compound_hostile_insert_panic_cannot_abort_process` | `#[test]` | 1226 |
| 436 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1249` | `read_hash_callback_reentry_is_typed_busy_and_preserves_exact_postimage` | `#[test]` | 1248 |
| 437 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1295` | `sweep_hash_callback_reentry_is_typed_busy_before_refund` | `#[test]` | 1294 |
| 438 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1333` | `hash_unwind_during_detached_mutation_restores_owned_state_and_charge` | `#[test]` | 1332 |
| 439 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1396` | `checked_reads_propagate_client_clone_panics_without_poisoning_owner_mutex_v1` | `#[test]` | 1395 |
| 440 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1450` | `partial_entry_read_clone_and_drop_panics_cannot_abort` | `#[test]` | 1449 |
| 441 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1518` | `put_get_remove_get_none_round_trip_corrects_bytes` | `#[test]` | 1517 |
| 442 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1574` | `equal_hash_key_duplicate_overwrites_and_does_not_double_count` | `#[test]` | 1573 |
| 443 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1707` | `first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1` | `#[test]` | 1706 |
| 444 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1713` | `reinsert_same_key_recharges_to_new_size_not_doubled` | `#[test]` | 1712 |
| 445 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1774` | `overwrite_drop_reentry_is_bounded_and_other_store_sees_exact_ledger` | `#[test]` | 1773 |
| 446 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1849` | `remove_releases_committed_bytes` | `#[test]` | 1848 |
| 447 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1863` | `sweep_reclaims_at_least_target_bytes_and_releases_them` | `#[test]` | 1862 |
| 448 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1880` | `overwrite_preserves_original_insertion_position_for_newest_first_sweep` | `#[test]` | 1879 |
| 449 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1900` | `arbitrary_remove_preserves_remaining_newest_first_sweep_order` | `#[test]` | 1899 |
| 450 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1917` | `byte_targeted_sweep_reclaims_full_cost_of_newest_entry` | `#[test]` | 1916 |
| 451 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1940` | `byte_targeted_sweep_of_empty_values_reclaims_key_and_index_cost` | `#[test]` | 1939 |
| 452 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1958` | `get_missing_key_is_none` | `#[test]` | 1957 |
| 453 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1964` | `keys_lists_inserted_keys_in_deterministic_sorted_order` | `#[test]` | 1963 |
| 454 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1979` | `entries_lists_pairs_in_same_sorted_order_as_keys` | `#[test]` | 1978 |
| 455 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1995` | `keys_after_remove_drops_the_removed_key` | `#[test]` | 1994 |
| 456 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2006` | `keys_on_empty_store_is_empty` | `#[test]` | 2005 |
| 457 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2033` | `uncharged_retained_map_bytes_are_not_reflected_in_a_broker_ledger_snapshot` | `#[test]` | 2032 |
| 458 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2068` | `typed_value_round_trips_and_accounts_its_own_byte_size` | `#[test]` | 2067 |
| 459 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2089` | `measured_any_handle_round_trips_and_accounts_caller_size` | `#[test]` | 2088 |
| 460 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2168` | `empty_value_with_huge_retained_key_is_denied_without_ledger_or_map_mutation` | `#[test]` | 2167 |
| 461 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2191` | `key_measurement_panic_precedes_accounting_and_preserves_state` | `#[test]` | 2190 |
| 462 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2209` | `counter_reads_show_open_busy_and_closed_images` | `#[test]` | 2208 |
| 463 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2260` | `overwrite_preserves_retained_original_key_cost_and_exact_ledger` | `#[test]` | 2259 |
| 464 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2281` | `aggregate_cost_overflow_is_denied_before_new_entry_or_ledger_charge` | `#[test]` | 2280 |
| 465 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2319` | `panic_in_old_value_drop_cannot_split_committed_bytes_from_ledger` | `#[test]` | 2318 |
| 466 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2357` | `close_contains_client_destructor_panic_and_keeps_terminal_image` | `#[test]` | 2356 |
| 467 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2388` | `drop_during_unwind_contains_two_client_destructor_panics_and_releases_all` | `#[test]` | 2387 |
| 468 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2485` | `sweep_never_hashes_client_key_after_refund` | `#[test]` | 2484 |
| 469 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2550` | `sweep_hash_panic_before_refund_preserves_charged_preimage` | `#[test]` | 2549 |
| 470 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2587` | `panic_in_sweep_value_drop_is_contained_after_exact_refund` | `#[test]` | 2586 |
| 471 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2615` | `sweep_multiple_panicking_destructors_does_not_abort_process` | `#[test]` | 2614 |
| 472 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2704` | `rebuild_denied_recharge_preserves_charged_preimage_v1` | `#[test]` | 2703 |
| 473 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2731` | `charged_insert_routes_bytes_through_the_ledger` | `#[test]` | 2730 |
| 474 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2764` | `atomic_insert_if_absent_never_recharges_or_overwrites_an_existing_entry_v1` | `#[test]` | 2763 |
| 475 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2795` | `denied_atomic_insert_if_absent_publishes_neither_payload_nor_charge_v1` | `#[test]` | 2794 |
| 476 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2821` | `charged_insert_over_cap_is_denied_and_fails_closed` | `#[test]` | 2820 |
| 477 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2853` | `charged_overwrite_denial_leaves_existing_entry_unchanged` | `#[test]` | 2852 |
| 478 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2881` | `entry_remains_refundable_after_a_denied_overwrite` | `#[test]` | 2880 |
| 479 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2908` | `charged_overwrite_during_rebuild_is_fail_closed_and_leaves_entry_intact` | `#[test]` | 2907 |
| 480 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2945` | `charged_sweep_refunds_reclaimed_bytes_to_the_ledger` | `#[test]` | 2944 |
| 481 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2968` | `charged_full_eviction_leaves_no_entry_key_ticket_or_accounting_residue_v1` | `#[test]` | 2967 |
| 482 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2991` | `charged_remove_during_rebuild_is_fail_closed` | `#[test]` | 2990 |
| 483 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:3015` | `charged_remove_rejects_a_ticketless_entry_without_mutating_state` | `#[test]` | 3014 |
| 484 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:3044` | `charged_sweep_during_rebuild_is_fail_closed` | `#[test]` | 3043 |
| 485 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:3084` | `charged_sweep_rejects_a_ticketless_candidate_without_partial_refund_or_eviction` | `#[test]` | 3083 |
| 486 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:3125` | `uncharged_store_is_explicit_non_budgeted_mode` | `#[test]` | 3124 |
| 487 | facade-stores | `crates/quanta-memory-platform-facade/src/retained_map/service.rs:3136` | `remove_contains_panicking_key_and_returns_value_after_exact_refund` | `#[test]` | 3135 |
| 488 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:710` | `canonical_put_deduplicates_and_charges_once` | `#[test]` | 709 |
| 489 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:724` | `payload_lookup_probe_count_is_n_independent` | `#[test]` | 723 |
| 490 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:777` | `payload_delete_reuses_vacant_slots_without_stale_index_alias` | `#[test]` | 776 |
| 491 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:814` | `transient_canonical_key_is_not_charged_but_retained_identity_is` | `#[test]` | 813 |
| 492 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:837` | `cost_component_sum_overflow_denies_without_publishing_or_charging` | `#[test]` | 836 |
| 493 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:853` | `composite_entry_charges_one_payload_bucket` | `#[test]` | 852 |
| 494 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:881` | `canonical_digests_are_randomized_per_store_seed` | `#[test]` | 880 |
| 495 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:899` | `counter_reads_show_open_busy_and_closed_images` | `#[test]` | 898 |
| 496 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:931` | `measurement_panic_precedes_accounting_and_does_not_poison_state` | `#[test]` | 930 |
| 497 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:947` | `metadata_equality_reentry_under_read_lock_is_typed_busy_and_preserves_charge` | `#[test]` | 946 |
| 498 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:972` | `duplicate_canonical_callback_reentry_under_lock_is_typed_busy_and_charges_once` | `#[test]` | 971 |
| 499 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1011` | `dedup_hit_preserves_first_context_and_stores_no_second_entry` | `#[test]` | 1010 |
| 500 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1047` | `distinct_payloads_each_charge_once` | `#[test]` | 1046 |
| 501 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1078` | `non_injective_public_reference_conversion_is_rejected_before_charge` | `#[test]` | 1077 |
| 502 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1098` | `delete_unpinned_releases_bytes_exactly_once` | `#[test]` | 1097 |
| 503 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1112` | `over_cap_put_is_fail_closed_and_does_not_consume_ref` | `#[test]` | 1111 |
| 504 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1126` | `pin_is_metadata_only_and_pinned_delete_is_rejected` | `#[test]` | 1125 |
| 505 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1147` | `reference_eq_panic_before_mutation_keeps_public_store_available` | `#[test]` | 1146 |
| 506 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1225` | `reference_clone_panic_before_publication_keeps_public_store_available` | `#[test]` | 1224 |
| 507 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1272` | `missing_delete_and_unpin_without_pin_are_invalid_state` | `#[test]` | 1271 |
| 508 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1283` | `put_and_delete_during_rebuild_are_fail_closed_then_resume` | `#[test]` | 1282 |
| 509 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1310` | `explicit_unbudgeted_store_is_a_distinct_non_ledger_mode` | `#[test]` | 1309 |
| 510 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1320` | `close_refunds_every_live_entry_exactly_once` | `#[test]` | 1319 |
| 511 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1334` | `close_refund_failure_preserves_entries_and_charge_ownership` | `#[test]` | 1333 |
| 512 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1351` | `drop_during_rebuild_defers_then_finish_drains_capacity` | `#[test]` | 1350 |
| 513 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1365` | `drop_during_outer_unwind_contains_two_client_panics_and_refunds_all_charges` | `#[test]` | 1364 |
| 514 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1398` | `mutation_after_close_is_typed_closed` | `#[test]` | 1397 |
| 515 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1412` | `double_close_is_idempotent_without_double_refund` | `#[test]` | 1411 |
| 516 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1421` | `concurrent_mutation_and_close_has_one_typed_linearized_outcome` | `#[test]` | 1420 |
| 517 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1454` | `counter_overflow_is_rejected_without_accounting_or_entry_mutation` | `#[test]` | 1453 |
| 518 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1466` | `poisoned_stats_is_error_not_empty` | `#[test]` | 1465 |
| 519 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1526` | `delete_drop_reentry_is_bounded_and_peer_publication_retains_exact_charge` | `#[test]` | 1525 |
| 520 | facade-stores | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1593` | `delete_contains_two_panicking_client_destructors_after_exact_refund` | `#[test]` | 1592 |
| 521 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:514` | `poisoned_spill_inventory_returns_error_not_zero` | `#[test]` | 513 |
| 522 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:529` | `unavailable_spill_returns_the_only_handle_without_dropping_it` | `#[test]` | 528 |
| 523 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:550` | `noop_spill_rejects_and_returns_the_handle` | `#[test]` | 549 |
| 524 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:559` | `routed_spill_to_available_target_succeeds_and_counts` | `#[test]` | 558 |
| 525 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:589` | `routed_spill_rolls_back_the_count_when_accept_fails` | `#[test]` | 588 |
| 526 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:605` | `spill_to_unavailable_target_fails_closed_and_does_not_count` | `#[test]` | 604 |
| 527 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:622` | `available_controller_succeeds_while_unavailable_controller_rejects` | `#[test]` | 621 |
| 528 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:637` | `noop_controller_rejects_without_a_target_and_returns_handle` | `#[test]` | 636 |
| 529 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:646` | `routed_spill_after_target_goes_offline_fails_closed_leaving_inventory_unchanged` | `#[test]` | 645 |
| 530 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:670` | `in_memory_target_accept_after_toggle_offline_is_rejected` | `#[test]` | 669 |
| 531 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:692` | `in_memory_target_write_then_restore_round_trips_the_payload` | `#[test]` | 691 |
| 532 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:709` | `in_memory_target_restore_of_missing_handle_is_none_not_an_error` | `#[test]` | 708 |
| 533 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:725` | `in_memory_target_restore_after_offline_fails_closed_leaving_retained_state_unchanged` | `#[test]` | 724 |
| 534 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:759` | `retaining_target_round_trips_handles_fifo` | `#[test]` | 758 |
| 535 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:773` | `retaining_target_offline_rejects_accept_but_state_unchanged` | `#[test]` | 772 |
| 536 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:794` | `retaining_target_offline_restore_fails_closed_like_in_memory` | `#[test]` | 793 |
| 537 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:824` | `both_spill_targets_share_the_offline_restore_contract` | `#[test]` | 823 |
| 538 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:834` | `routed_controller_spill_then_restore_balances_the_spilled_count` | `#[test]` | 833 |
| 539 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:851` | `concurrent_spills_keep_spilled_count_consistent_then_drain_to_zero` | `#[test]` | 850 |
| 540 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/service.rs:882` | `noop_controller_restore_is_none` | `#[test]` | 881 |
| 541 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2436` | `create_on_an_invalid_path_is_a_neutral_memory_error` | `#[test]` | 2435 |
| 542 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2443` | `live_backing_excludes_second_open_and_drop_releases_persisted_file` | `#[test]` | 2442 |
| 543 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2461` | `durable_spill_target_round_trips_fifo_through_the_trait` | `#[test]` | 2460 |
| 544 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2479` | `reopened_backing_resumes_sequence_and_restores_persisted_handles` | `#[test]` | 2478 |
| 545 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2494` | `drained_queue_never_reuses_sequence_or_accepts_stale_restore_token` | `#[test]` | 2493 |
| 546 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2523` | `persisted_restore_claim_blocks_duplicate_prepare_and_recovers_after_reopen` | `#[test]` | 2522 |
| 547 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2552` | `committed_restore_payload_survives_reopen_until_resident_ack` | `#[test]` | 2551 |
| 548 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2583` | `injected_write_crash_points_converge_without_duplicate_records` | `#[test]` | 2582 |
| 549 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2609` | `injected_restore_and_finalize_ack_loss_preserve_recovery_authority` | `#[test]` | 2608 |
| 550 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2638` | `generic_restore_is_non_destructive_and_requires_owned_two_phase_path` | `#[test]` | 2637 |
| 551 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2654` | `generic_write_is_non_mutating_and_returns_the_original_payload` | `#[test]` | 2653 |
| 552 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2666` | `reopened_redb_reconciles_controller_count_from_persisted_inventory` | `#[test]` | 2665 |
| 553 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2694` | `concurrent_accepts_lose_no_handle_and_assign_distinct_keys` | `#[test]` | 2693 |
| 554 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2763` | `sep23_fup001_reserved_hole_is_not_finalized_by_later_record` | `#[test]` | 2762 |
| 555 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2808` | `sep23_fup001_reserve_only_reopen_is_not_consumed` | `#[test]` | 2807 |
| 556 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2830` | `sep23_fup001_out_of_order_commit_finalized_reinsert_rejected` | `#[test]` | 2829 |
| 557 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2882` | `sep23_fup001_same_bytes_retry_keeps_inventory_stable` | `#[test]` | 2881 |
| 558 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2906` | `limits_reject_oversized_record_before_commit_without_consuming_reserved_id` | `#[test]` | 2905 |
| 559 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2921` | `retained_limits_count_recovery_and_refund_only_after_finalize` | `#[test]` | 2920 |
| 560 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2946` | `retained_payload_cap_denies_next_record_without_changing_durable_authority` | `#[test]` | 2945 |
| 561 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2965` | `spill_receipt_inventory_includes_other_records_in_recovery_slot` | `#[test]` | 2964 |
| 562 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:2982` | `in_flight_restore_budget_denial_preserves_claim_and_releases_on_candidate_drop` | `#[test]` | 2981 |
| 563 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3010` | `zero_byte_restore_candidates_are_bounded_by_buffer_count` | `#[test]` | 3009 |
| 564 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3030` | `staging_denial_after_staged_remove_rolls_back_durable_payload` | `#[test]` | 3029 |
| 565 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3060` | `concurrent_writers_at_record_cap_admit_exactly_one_transaction` | `#[test]` | 3059 |
| 566 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3107` | `concurrent_prepare_restore_issues_exactly_one_live_claim` | `#[test]` | 3106 |
| 567 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3148` | `concurrent_recovery_claims_are_generation_fenced_to_latest_owner` | `#[test]` | 3147 |
| 568 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3196` | `tighter_reopen_rejects_existing_inventory_without_erasing_it` | `#[test]` | 3195 |
| 569 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3213` | `per_record_limit_is_checked_on_reopen_even_when_total_still_fits` | `#[test]` | 3212 |
| 570 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3230` | `persisted_payload_usage_mismatch_fails_closed_without_erasing_record` | `#[test]` | 3229 |
| 571 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3264` | `every_invalid_limit_relation_rejects_before_creating_database_file` | `#[test]` | 3263 |
| 572 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3309` | `capped_file_backend_rejects_extension_before_file_mutation` | `#[test]` | 3308 |
| 573 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3331` | `reopening_above_database_file_cap_fails_without_erasing_payload` | `#[test]` | 3330 |
| 574 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3351` | `database_file_cap_denial_preserves_prior_durable_record_across_reopen` | `#[test]` | 3350 |
| 575 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3457` | `partial_storage_full_write_reopens_to_whole_records_only` | `#[test]` | 3456 |
| 576 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3505` | `partial_storage_full_during_restore_commit_preserves_one_whole_copy` | `#[test]` | 3504 |
| 577 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3557` | `partial_storage_full_during_finalize_reopens_present_or_finalized` | `#[test]` | 3556 |
| 578 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3604` | `unsupported_explicit_schema_versions_fail_closed_before_v2_initialization` | `#[test]` | 3603 |
| 579 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3646` | `missing_schema_marker_on_v2_shaped_database_fails_without_restamping` | `#[test]` | 3645 |
| 580 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3692` | `missing_metadata_table_on_v2_shaped_database_fails_without_recreation` | `#[test]` | 3691 |
| 581 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3726` | `incomplete_backend_identity_fails_closed_without_replacing_surviving_half` | `#[test]` | 3725 |
| 582 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3769` | `schema_v2_missing_required_authority_fails_without_recreating_it` | `#[test]` | 3768 |
| 583 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3829` | `schema_v2_payload_disposition_mismatch_fails_without_repair` | `#[test]` | 3828 |
| 584 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3874` | `sep23_fup001_legacy_watermark_file_migrates_payload_and_fails_closed` | `#[test]` | 3873 |
| 585 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:3963` | `sep23_fup002_stale_generation_rejected_after_claim_steal` | `#[test]` | 3962 |
| 586 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:4003` | `sep23_fup002_finalize_proof_is_per_record_not_last_writer` | `#[test]` | 4002 |
| 587 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:4032` | `sep23_fup002_foreign_backend_claim_rejected_on_every_permission_change` | `#[test]` | 4031 |
| 588 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:4077` | `sep23_fup002_backend_identity_stable_across_reopen` | `#[test]` | 4076 |
| 589 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:4094` | `backend_identity_and_pending_claim_survive_cold_rename` | `#[test]` | 4093 |
| 590 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:4113` | `consistent_cold_copy_preserves_logical_lineage_identity` | `#[test]` | 4112 |
| 591 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:4139` | `sep23_audit_finalize_retry_succeeds_while_unrelated_claim_is_live` | `#[test]` | 4138 |
| 592 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:4175` | `ambiguous_finalize_probe_uses_own_record_proof_with_foreign_claim_live` | `#[test]` | 4174 |
| 593 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:4202` | `lost_finalize_and_abort_acks_allow_a_new_claim_before_reconciliation` | `#[test]` | 4201 |
| 594 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:4230` | `sep23_audit_finalize_proof_does_not_mask_reappeared_recovery_payload` | `#[test]` | 4229 |
| 595 | spill-redb | `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:4255` | `accept_at_exhausted_keyspace_fails_closed_without_overwriting` | `#[test]` | 4254 |
| 596 | crash-rails | `crates/quanta-memory-platform-facade/tests/spill_redb_kill_reopen.rs:273` | `sep23_fup006_sigkill_after_commit_reopens_with_stable_identity_and_exact_drain` | `#[test]` | 272 |
| 597 | crash-rails | `crates/quanta-memory-platform-facade/tests/spill_redb_kill_reopen.rs:312` | `sep23_fup006_sigkill_during_write_stream_converges_without_duplicates_or_tears` | `#[test]` | 311 |
| 598 | crash-rails | `crates/quanta-memory-platform-facade/tests/spill_redb_kill_reopen.rs:359` | `sep23_fup006_reserved_hole_stays_absent_across_sigkill` | `#[test]` | 358 |
| 599 | crash-rails | `crates/quanta-memory-platform-facade/tests/spill_redb_kill_reopen.rs:383` | `durable_spill_store_charge_and_record_survive_process_boundary` | `#[test]` | 382 |
| 600 | crash-rails | `crates/quanta-memory-platform-facade/tests/spill_redb_finalize_kill.rs:256` | `durability_l1_store_and_ledger_recover_across_finalize_kill` | `#[test]` | 255 |
| 601 | crash-rails | `crates/quanta-memory-platform-facade/tests/spill_redb_finalize_kill.rs:393` | `durability_l1_finalize_before_commit_kill_recovers_present_and_drains_exactly_once` | `#[test]` | 392 |
| 602 | crash-rails | `crates/quanta-memory-platform-facade/tests/spill_redb_finalize_kill.rs:445` | `durability_l1_finalize_after_commit_kill_resolves_finalized_without_resurrection` | `#[test]` | 444 |
| 603 | crash-rails | `crates/quanta-memory-platform-facade/tests/cross_store_hellgate.rs:152` | `distinct_grouped_stores_compete_for_one_global_slot_and_handoff_after_release` | `#[test]` | 151 |
| 604 | crash-rails | `crates/quanta-memory-platform-facade/tests/cross_store_hellgate.rs:248` | `store_scratch_and_zero_byte_authorities_survive_denied_remap_and_legal_raise` | `#[test]` | 247 |
| 605 | crash-rails | `crates/quanta-memory-platform-facade/tests/cross_store_hellgate.rs:330` | `other_store_reclaim_frees_admission_and_drains_every_ticket` | `#[test]` | 329 |
| 606 | crash-rails | `crates/quanta-memory-platform-facade/tests/cross_store_hellgate.rs:443` | `mixed_store_kind_lifetimes_balance_every_axis` | `#[test]` | 441 |
| 607 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:72` | `charged_store_bytes_flow_through_the_ledger_and_drain_to_zero` | `#[test]` | 71 |
| 608 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:104` | `close_drains_all_charges_once_and_rejects_later_mutation` | `#[test]` | 103 |
| 609 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:127` | `failed_close_during_rebuild_preserves_store_and_retries_after_fence` | `#[test]` | 126 |
| 610 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:149` | `store_drop_during_rebuild_defers_refund_until_finish` | `#[test]` | 148 |
| 611 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:174` | `e2e_spill_tier_round_trips_fifo_and_never_touches_the_ledger` | `#[test]` | 173 |
| 612 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:210` | `class_cap_gates_store_bytes_fail_closed` | `#[test]` | 209 |
| 613 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:247` | `overwrite_recharge_admits_net_fitting_replacement` | `#[test]` | 246 |
| 614 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:266` | `rebuild_fences_charged_store_writes_then_resumes` | `#[test]` | 265 |
| 615 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:295` | `reconfigure_raise_preserves_charged_usage` | `#[test]` | 294 |
| 616 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:332` | `concurrent_charged_inserts_never_exceed_global_and_stay_consistent` | `#[test]` | 331 |
| 617 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:385` | `double_remove_does_not_double_refund_the_ledger` | `#[test]` | 384 |
| 618 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:415` | `broker_recharge_denied_preserves_managed_authority` | `#[test]` | 414 |
| 619 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:463` | `release_many_drains_a_batch_atomically` | `#[test]` | 462 |
| 620 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:476` | `dropping_managed_charges_releases_each_exact_authority` | `#[test]` | 475 |
| 621 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:488` | `release_many_with_an_already_released_ticket_is_fail_closed` | `#[test]` | 487 |
| 622 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:514` | `release_many_during_rebuild_is_fail_closed` | `#[test]` | 513 |
| 623 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:559` | `charged_payload_dedup_pin_delete_flow_through_the_ledger` | `#[test]` | 558 |
| 624 | crash-rails | `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs:591` | `charged_payload_over_cap_put_is_fail_closed` | `#[test]` | 590 |
| 625 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1071` | `unbudgeted_admission_never_exceeds_a_complete_gc_root_set` | `#[test]` | 1070 |
| 626 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1127` | `prepare_capacity_failure_leaves_all_state_unchanged` | `#[test]` | 1126 |
| 627 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1136` | `prepared_payload_is_only_reachable_by_exact_token` | `#[test]` | 1135 |
| 628 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1150` | `identical_prepare_replays_same_token_without_second_charge` | `#[test]` | 1149 |
| 629 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1161` | `identity_payload_drift_is_typed_conflict` | `#[test]` | 1160 |
| 630 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1171` | `abort_releases_exact_payload_and_is_retry_safe` | `#[test]` | 1170 |
| 631 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1195` | `concurrent_identical_prepare_has_one_applied_winner` | `#[test]` | 1194 |
| 632 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1223` | `concurrent_conflicting_prepare_has_one_winner` | `#[test]` | 1222 |
| 633 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1250` | `root_referenced_token_is_never_enumerated_or_collected_as_an_orphan` | `#[test]` | 1249 |
| 634 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1269` | `orphan_gc_with_unknown_root_reference_is_batch_atomic` | `#[test]` | 1268 |
| 635 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1292` | `aborted_root_reference_fails_closed_without_collecting_other_entries` | `#[test]` | 1291 |
| 636 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1315` | `tombstone_limits_bound_n_cycle_churn_and_fail_closed_at_saturation` | `#[test]` | 1314 |
| 637 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1564` | `large_identity_with_zero_payload_is_denied_before_retention` | `#[test]` | 1563 |
| 638 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1578` | `prepared_identity_index_and_tombstone_keep_exact_independent_cost` | `#[test]` | 1577 |
| 639 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1712` | `orphan_roots_bound_and_fail_closed` | `#[test]` | 1711 |
| 640 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1807` | `prepared_component_sum_overflow_is_fail_closed_before_reservation` | `#[test]` | 1806 |
| 641 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1831` | `resolve_clone_reentry_under_read_lock_is_typed_busy_and_preserves_charge` | `#[test]` | 1830 |
| 642 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1872` | `prepare_measurement_reentry_is_typed_busy_and_replay_skips_measurement` | `#[test]` | 1871 |
| 643 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1913` | `prepare_measurement_panic_preserves_readability_and_allows_retry` | `#[test]` | 1912 |
| 644 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1956` | `resolve_clone_panic_preserves_readability_and_charge` | `#[test]` | 1955 |
| 645 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1988` | `replay_charges_once_and_abort_preserves_identity_evidence_once` | `#[test]` | 1987 |
| 646 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2013` | `failed_abort_keeps_payload_index_and_charge_coherent` | `#[test]` | 2012 |
| 647 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2033` | `broker_capacity_denial_leaves_no_local_or_ledger_state` | `#[test]` | 2032 |
| 648 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2046` | `failed_orphan_gc_preserves_every_payload_index_and_charge` | `#[test]` | 2045 |
| 649 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2081` | `close_refunds_all_entries_and_is_terminal_and_idempotent` | `#[test]` | 2080 |
| 650 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2121` | `failed_close_keeps_exact_payloads_tokens_and_charges_for_retry` | `#[test]` | 2120 |
| 651 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2144` | `drop_refunds_live_entries_and_rebuild_drop_defers_exact_release` | `#[test]` | 2143 |
| 652 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2179` | `multiple_panicking_client_drops_during_outer_unwind_drain_exact_charges` | `#[test]` | 2178 |
| 653 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2225` | `mutation_gate_returns_typed_busy_and_exposes_no_partial_plan` | `#[test]` | 2224 |
| 654 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2266` | `poisoned_store_reports_error_but_drop_still_releases_every_charge` | `#[test]` | 2265 |
| 655 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2285` | `checked_overflow_and_underflow_reject_before_broker_or_index_mutation` | `#[test]` | 2284 |
| 656 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:2330` | `orphan_gc_rejects_corrupt_active_lease_cost_without_partial_refund` | `#[test]` | 2329 |
| 657 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:165` | `identity_pipeline_stages_then_commits_published` | `#[test]` | 164 |
| 658 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:180` | `failed_commit_never_publishes_and_never_increments_count` | `#[test]` | 179 |
| 659 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:214` | `default_commit_transition_applies_identity_with_no_missing_step` | `#[test]` | 213 |
| 660 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:229` | `default_pipeline_runs_stage_then_commit_each_exactly_once_no_rollback` | `#[test]` | 228 |
| 661 | pipeline-shared | `crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:250` | `published_count_only_reflects_committed_not_staged` | `#[test]` | 249 |
| 662 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:766` | `builder_first_happy_path_builds_a_usable_payload_store` | `#[test]` | 765 |
| 663 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:784` | `payload_builder_without_binding_fails_with_build_error` | `#[test]` | 783 |
| 664 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:794` | `retained_map_platform_builder_routes_bytes_through_the_ledger` | `#[test]` | 793 |
| 665 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:807` | `publish_pipeline_builder_without_binding_fails_with_build_error` | `#[test]` | 806 |
| 666 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:815` | `transcript_builder_without_binding_fails_with_build_error` | `#[test]` | 814 |
| 667 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:824` | `transcript_platform_builder_routes_bytes_through_the_ledger` | `#[test]` | 823 |
| 668 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:848` | `revocable_builder_without_binding_fails_with_build_error` | `#[test]` | 847 |
| 669 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:857` | `revocable_platform_builder_routes_bytes_through_the_ledger` | `#[test]` | 856 |
| 670 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:873` | `default_build_composes_charged_stores_without_manual_glue` | `#[test]` | 872 |
| 671 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:895` | `default_build_platform_sink_is_the_same_ledger_as_the_broker` | `#[test]` | 894 |
| 672 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/builder.rs:921` | `memory_platform_builder_builds_broker_from_preset` | `#[test]` | 920 |
| 673 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:367` | `hostile_panic_payload_drop_cannot_abort_outer_unwind` | `#[test]` | 366 |
| 674 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:396` | `client_callback_reentry_on_held_container_lock_is_typed_busy` | `#[test]` | 395 |
| 675 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:409` | `reentry_returns_typed_busy_without_blocking` | `#[test]` | 408 |
| 676 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:432` | `close_excludes_mutations_and_other_close_attempts` | `#[test]` | 431 |
| 677 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:453` | `failed_close_reopens_for_mutation_and_retry` | `#[test]` | 452 |
| 678 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:466` | `committed_close_stays_closed_and_double_close_is_idempotent` | `#[test]` | 465 |
| 679 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:484` | `drop_transition_is_terminal_and_reports_cleanup_once` | `#[test]` | 483 |
| 680 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/measure.rs:226` | `retained_cost_v1_sums_each_component_exactly_once` | `#[test]` | 225 |
| 681 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/measure.rs:239` | `retained_cost_v1_rejects_component_and_cross_component_overflow` | `#[test]` | 238 |
| 682 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/measure.rs:257` | `measured_value_charges_caller_supplied_size_not_inner_len` | `#[test]` | 256 |
| 683 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/measure.rs:267` | `measured_value_into_inner_returns_wrapped_value` | `#[test]` | 266 |
| 684 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/measure.rs:273` | `measured_value_zero_size_is_charged_verbatim` | `#[test]` | 272 |
| 685 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/measure.rs:281` | `arc_byte_size_delegates_to_inner` | `#[test]` | 280 |
| 686 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/measure.rs:287` | `cloning_measured_arc_shares_the_allocation_not_a_deep_copy` | `#[test]` | 286 |
| 687 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/measure.rs:305` | `cloning_measured_value_preserves_size_and_value_context` | `#[test]` | 304 |
| 688 | pipeline-shared | `crates/quanta-memory-platform-facade/src/shared/measure.rs:318` | `measured_arc_any_handle_charges_caller_size` | `#[test]` | 317 |
| 689 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:73` | `live_broker_bridge_is_present_only_with_core_live_accounting` | `#[test]` | 72 |
| 690 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:184` | `prepared_publication_builder_exposes_root_bound_resolution_contract` | `#[test]` | 183 |
| 691 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:260` | `measured_value_round_trips_value_and_size` | `#[test]` | 259 |
| 692 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:277` | `portable_presets_cover_every_preset_with_full_class_taxonomy` | `#[test]` | 276 |
| 693 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:299` | `memory_platform_builder_builds_a_broker_from_a_preset` | `#[test]` | 297 |
| 694 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:309` | `memory_platform_builder_without_config_fails_closed` | `#[test]` | 307 |
| 695 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:315` | `report_only_builder_resolves_without_runtime_authority` | `#[test]` | 314 |
| 696 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:324` | `report_only_builder_without_config_fails_closed` | `#[test]` | 323 |
| 697 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:331` | `default_builder_over_cap_store_mutation_is_denied` | `#[test]` | 330 |
| 698 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:374` | `default_builder_and_report_only_share_override_resolution` | `#[test]` | 373 |
| 699 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:414` | `platform_builder_installs_reclaimer_before_live_construction` | `#[test]` | 413 |
| 700 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:439` | `payload_store_builder_yields_a_queryable_store` | `#[test]` | 438 |
| 701 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:459` | `retained_map_builder_exposes_public_charged_path` | `#[test]` | 458 |
| 702 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:474` | `explicit_unbudgeted_store_is_a_distinct_product_and_does_not_charge_the_platform` | `#[test]` | 473 |
| 703 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:502` | `retained_map_atomic_insert_returns_occupied_value_without_overwrite` | `#[test]` | 501 |
| 704 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:539` | `revocable_builder_exposes_public_charged_path` | `#[test]` | 538 |
| 705 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:558` | `transcript_builder_exposes_public_charged_path` | `#[test]` | 557 |
| 706 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:592` | `spill_controller_round_trips_a_handle_via_restore` | `#[test]` | 591 |
| 707 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:612` | `spill_controller_fails_closed_when_target_unavailable` | `#[test]` | 611 |
| 708 | pipeline-shared | `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:624` | `noop_spill_controller_rejects_and_restores_nothing` | `#[test]` | 623 |
| 709 | pipeline-shared | `crates/quanta-memory-platform-facade/src/test_support.rs:46` | `isolated_child_selection_rejects_zero_tests` | `#[test]` | 45 |
| 710 | pipeline-shared | `crates/quanta-memory-platform-facade/src/test_support.rs:57` | `isolated_child_selection_rejects_multiple_tests` | `#[test]` | 56 |
| 711 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1687` | `deterministic_trace_matches_independent_model` | `#[test]` | 1686 |
| 712 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1907` | `property_trace_matches_independent_model` | `#[test]` | 1906 |
| 713 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1935` | `reentrant_sink_is_contained_outside_owner_lock` | `#[test]` | 1934 |
| 714 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1960` | `suite_rejects_black_hole_accounting` | `#[test]` | 1959 |
| 715 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1968` | `suite_rejects_commit_swallowing_target` | `#[test]` | 1967 |
| 716 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1976` | `suite_rejects_release_swallowing_target` | `#[test]` | 1975 |
| 717 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1984` | `suite_rejects_duplicate_commit_accepting_target` | `#[test]` | 1983 |
| 718 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:1992` | `suite_rejects_wrong_context_axis` | `#[test]` | 1991 |
| 719 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2000` | `suite_rejects_wrong_charge_kind_axis` | `#[test]` | 1999 |
| 720 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2014` | `sep23_fup004_suite_rejects_class_confined_kind_swap` | `#[test]` | 2013 |
| 721 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2034` | `sep23_fup004_suite_rejects_context_confined_kind_swap` | `#[test]` | 2033 |
| 722 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2051` | `sep23_fup004_suite_rejects_aggregate_preserving_bucket_swap` | `#[test]` | 2050 |
| 723 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2082` | `sep23_fup004_mixed_kind_trace_agrees_on_every_axis_bucket` | `#[test]` | 2081 |
| 724 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2145` | `suite_rejects_stale_ticket_accepting_target` | `#[test]` | 2144 |
| 725 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2153` | `snapshot_error_is_failed_verdict_not_zero` | `#[test]` | 2152 |
| 726 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2166` | `suite_rejects_transfer_double_owner` | `#[test]` | 2165 |
| 727 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2179` | `prepared_publication_suite_accepts_honest_service` | `#[test]` | 2178 |
| 728 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2247` | `prepared_suite_rejects_correct_replay_with_unbilled_identity` | `#[test]` | 2246 |
| 729 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2333` | `prepared_publication_suite_rejects_duplicate_charging_fake` | `#[test]` | 2332 |
| 730 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2410` | `prepared_publication_suite_rejects_root_blind_gc_fake` | `#[test]` | 2409 |
| 731 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2421` | `transcript_append_suite_accepts_honest_service` | `#[test]` | 2420 |
| 732 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2487` | `transcript_append_suite_rejects_non_idempotent_fake` | `#[test]` | 2486 |
| 733 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2498` | `transcript_suite_rejects_target_that_ignores_declared_horizon` | `#[test]` | 2497 |
| 734 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2555` | `transcript_suite_rejects_correct_replay_with_unbilled_identity` | `#[test]` | 2554 |
| 735 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2581` | `honest_fake_passes_every_accounting_invariant` | `#[test]` | 2580 |
| 736 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2593` | `suite_rejects_fake_that_under_reports_owner_bytes` | `#[test]` | 2592 |
| 737 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2609` | `suite_rejects_fake_whose_class_diverges_from_owner` | `#[test]` | 2608 |
| 738 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2648` | `all_break_variants_are_distinct_non_honest_and_family_mapped` | `#[test]` | 2647 |
| 739 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2675` | `suite_rejects_every_accounting_fidelity_break_variant` | `#[test]` | 2674 |
| 740 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2697` | `owner_partition_break_localizes_to_owner_partition_not_balance` | `#[test]` | 2696 |
| 741 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2714` | `honest_fidelity_is_the_only_variant_the_suite_accepts` | `#[test]` | 2713 |
| 742 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2839` | `suite_rejects_target_that_mutates_accounting_on_a_denied_reserve` | `#[test]` | 2838 |
| 743 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2855` | `honest_fake_passes_the_strengthened_fail_closed_check` | `#[test]` | 2854 |
| 744 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2957` | `suite_rejects_target_that_mutates_on_allocator_observation` | `#[test]` | 2956 |
| 745 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:2973` | `commit_then_release_returns_balance_to_zero_on_honest_fake` | `#[test]` | 2972 |
| 746 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:3000` | `commit_then_release_returns_balance_to_zero_on_real_broker` | `#[test]` | 2999 |
| 747 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:3059` | `cap_deny_agrees_between_honest_fake_and_real_broker` | `#[test]` | 3058 |
| 748 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:3134` | `both_targets_still_pass_the_portable_suite_after_strengthening` | `#[test]` | 3133 |
| 749 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:3183` | `spill_routing_verdict_holds_for_honest_and_violates_for_a_dropping_target` | `#[test]` | 3182 |
| 750 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/invariants.rs:144` | `equal_bytes_hold_unequal_violate_with_drift_evidence` | `#[test]` | 143 |
| 751 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/invariants.rs:157` | `predicate_assertion_carries_supplied_evidence` | `#[test]` | 156 |
| 752 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/invariants.rs:181` | `every_family_has_a_unique_stable_label` | `#[test]` | 180 |
| 753 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/invariants.rs:198` | `drift_evidence_sign_distinguishes_over_from_under_reporting` | `#[test]` | 197 |
| 754 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/invariants.rs:218` | `family_attribution_is_preserved_through_both_assertion_constructors` | `#[test]` | 217 |
| 755 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/mod.rs:120` | `honest_fake_yields_passed_verdict` | `#[test]` | 119 |
| 756 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/mod.rs:128` | `broken_fake_yields_failed_verdict_localized_to_balance_family` | `#[test]` | 127 |
| 757 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/mod.rs:141` | `empty_assertion_list_is_failed_not_silently_passed` | `#[test]` | 140 |
| 758 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/mod.rs:180` | `public_suite_rejects_every_break_variant_and_localizes_it` | `#[test]` | 179 |
| 759 | testkit | `crates/quanta-memory-platform-testkit/src/conformance/mod.rs:224` | `real_core_broker_passes_the_same_portable_suite` | `#[test]` | 223 |
| 760 | testkit | `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:293` | `reclaim_returns_zero_is_contained` | `#[test]` | 292 |
| 761 | testkit | `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:300` | `pinned_delete_failure_is_contained_and_bytes_survive` | `#[test]` | 299 |
| 762 | testkit | `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:308` | `pinned_delete_classifier_rejects_reference_loss_and_accounting_lies` | `#[test]` | 307 |
| 763 | testkit | `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:412` | `rebuild_race_write_is_rejected` | `#[test]` | 411 |
| 764 | testkit | `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:418` | `spill_failure_is_contained_without_counting` | `#[test]` | 417 |
| 765 | testkit | `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:430` | `spill_fault_classification_reports_escaped_for_a_lying_controller` | `#[test]` | 429 |
| 766 | testkit | `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:473` | `spill_fault_classification_reports_escaped_when_count_drifts` | `#[test]` | 472 |
| 767 | testkit | `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:508` | `spill_fault_classification_reports_escaped_when_count_is_unreadable` | `#[test]` | 507 |
| 768 | testkit | `crates/quanta-memory-platform-testkit/src/faults/mod.rs:121` | `harness_covers_reclaim_pin_rebuild_and_spill` | `#[test]` | 120 |
| 769 | testkit | `crates/quanta-memory-platform-testkit/src/faults/mod.rs:139` | `empty_harness_runs_nothing` | `#[test]` | 138 |
| 770 | testkit | `crates/quanta-memory-platform-testkit/src/fake/broker.rs:667` | `honest_reserve_keeps_class_owner_balanced` | `#[test]` | 666 |
| 771 | testkit | `crates/quanta-memory-platform-testkit/src/fake/broker.rs:685` | `owner_hard_cap_denies_before_mutation` | `#[test]` | 684 |
| 772 | testkit | `crates/quanta-memory-platform-testkit/src/fake/broker.rs:711` | `resource_group_cap_denies_aggregate_over_owners` | `#[test]` | 710 |
| 773 | testkit | `crates/quanta-memory-platform-testkit/src/fake/broker.rs:746` | `rebuild_window_rejects_writes_then_resumes` | `#[test]` | 745 |
| 774 | testkit | `crates/quanta-memory-platform-testkit/src/fake/broker.rs:773` | `reclaim_yield_zero_frees_nothing` | `#[test]` | 772 |
| 775 | testkit | `crates/quanta-memory-platform-testkit/src/fake/broker.rs:810` | `reclaim_preserves_ticket_provenance_kind_axes_and_group_refunds` | `#[test]` | 809 |
| 776 | testkit | `crates/quanta-memory-platform-testkit/src/fake/broker.rs:856` | `reclaim_terminates_full_tickets_and_preserves_partial_commit_authority` | `#[test]` | 855 |
| 777 | testkit | `crates/quanta-memory-platform-testkit/src/fake/broker.rs:904` | `reclaim_rebuild_and_incomplete_group_preflight_leave_all_authority_untouched` | `#[test]` | 903 |
| 778 | testkit | `crates/quanta-memory-platform-testkit/src/fake/broker.rs:935` | `broken_under_report_corrupts_only_the_reported_snapshot` | `#[test]` | 934 |
| 779 | testkit | `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs:148` | `testkit_runs_conformance_against_its_fake_and_passes` | `#[test]` | 147 |
| 780 | testkit | `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs:165` | `fake_pressure_fixture_propagates_pressure_into_the_fake_broker` | `#[test]` | 164 |
| 781 | testkit | `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs:175` | `conformance_verdict_localizes_to_contract_family_not_a_host_package` | `#[test]` | 174 |
| 782 | testkit | `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs:187` | `live_broker_materializes_from_fixture_manifest` | `#[test]` | 186 |
| 783 | testkit | `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs:197` | `testkit_runs_facade_store_suite_against_live_broker_and_passes` | `#[test]` | 196 |
| 784 | testkit | `crates/quanta-memory-platform-testkit/src/alloc_tracking/mod.rs:131` | `fresh_probe_reports_zero_signals` | `#[test]` | 130 |
| 785 | testkit | `crates/quanta-memory-platform-testkit/src/alloc_tracking/mod.rs:140` | `recorded_signals_accumulate_and_net_live_is_correct` | `#[test]` | 139 |
| 786 | testkit | `crates/quanta-memory-platform-testkit/src/alloc_tracking/mod.rs:154` | `dealloc_exceeding_alloc_saturates_at_zero_not_nonsense` | `#[test]` | 153 |
| 787 | testkit | `crates/quanta-memory-platform-testkit/src/alloc_tracking/mod.rs:166` | `signals_feed_an_invariant_assertion` | `#[test]` | 165 |
| 788 | testkit | `crates/quanta-memory-platform-testkit/src/alloc_tracking/mod.rs:191` | `tracker_callbacks_route_into_the_same_counters` | `#[test]` | 190 |
| 789 | testkit | `crates/quanta-memory-platform-testkit/src/loom_models.rs:18` | `loom_reserve_vs_reconfigure_preserves_installed_cap` | `#[test]` | 17 |
| 790 | testkit | `crates/quanta-memory-platform-testkit/src/loom_models.rs:64` | `loom_ticket_transition_has_single_terminal_owner` | `#[test]` | 63 |
| 791 | testkit | `crates/quanta-memory-platform-testkit/src/loom_models.rs:93` | `loom_close_vs_mutation_never_mutates_after_close` | `#[test]` | 92 |
| 792 | testkit | `crates/quanta-memory-platform-testkit/src/loom_models.rs:129` | `loom_spill_transfer_never_has_zero_or_double_owner` | `#[test]` | 128 |
| 793 | testkit | `crates/quanta-memory-platform-testkit/src/loom_models.rs:165` | `loom_versioned_callback_compensation_is_exactly_once` | `#[test]` | 164 |
| 794 | testkit | `crates/quanta-memory-platform-testkit/src/lib.rs:70` | `conformance_suite_rejects_a_fake_that_under_reports_usage` | `#[test]` | 69 |
| 795 | testkit | `crates/quanta-memory-platform-testkit/src/lib.rs:85` | `conformance_suite_rejects_a_fake_with_divergent_class_totals` | `#[test]` | 84 |
| 796 | testkit | `crates/quanta-memory-platform-testkit/src/lib.rs:92` | `conformance_suite_accepts_an_honest_fake` | `#[test]` | 91 |
| 797 | testkit | `crates/quanta-memory-platform-testkit/tests/facade_conformance_bite.rs:109` | `honest_fake_and_all_five_real_stores_pass_lifecycle_oracle` | `#[test]` | 108 |
| 798 | testkit | `crates/quanta-memory-platform-testkit/tests/facade_conformance_bite.rs:116` | `lifecycle_oracle_rejects_leaking_close_fake` | `#[test]` | 115 |
| 799 | testkit | `crates/quanta-memory-platform-testkit/tests/facade_conformance_bite.rs:123` | `lifecycle_oracle_rejects_state_losing_close_fake` | `#[test]` | 122 |
| 800 | testkit | `crates/quanta-memory-platform-testkit/tests/facade_conformance_bite.rs:130` | `lifecycle_oracle_rejects_double_refund_fake` | `#[test]` | 129 |
| 801 | testkit | `crates/quanta-memory-platform-testkit/tests/public_api_surface.rs:111` | `fixture_mode_exposes_the_facade_suite_end_to_end` | `#[test]` | 110 |
<!-- TEST_CENSUS_END -->

## Verification

```bash
python3 docs/bugbash/sep-26-fail-open-audit/verify_current_source_census.py
```

The verifier requires each current source SHA-256 and parsed function row to match the JSON snapshot, independently recomputes the anchored `rg` count per file, and compares every Markdown census row with the JSON inventory. A source change fails verification until this new snapshot is explicitly refreshed with `--write`.

Machine-readable snapshot: `current-source-census.json`. The audited base is pinned independently of current HEAD so commits and merges do not redefine the historical count. The newly added test_support.rs is explicitly permitted to be absent from that base even when tracked. No Cargo command is run by this verifier.

## Dirty source paths at capture

```text
M  crates/quanta-memory-platform-core/src/api/control.rs
M  crates/quanta-memory-platform-core/src/application/controller.rs
M  crates/quanta-memory-platform-core/src/application/scheduler.rs
M  crates/quanta-memory-platform-core/src/domain/admission.rs
M  crates/quanta-memory-platform-core/src/domain/config.rs
M  crates/quanta-memory-platform-core/src/domain/ledger.rs
M  crates/quanta-memory-platform-facade/src/payload_store/service.rs
M  crates/quanta-memory-platform-facade/src/retained_map/service.rs
M  crates/quanta-memory-platform-facade/src/revocable_store/service.rs
M  crates/quanta-memory-platform-facade/src/shared/lifecycle.rs
M  crates/quanta-memory-platform-facade/src/spill/service.rs
A  crates/quanta-memory-platform-facade/src/test_support.rs
M  crates/quanta-memory-platform-facade/src/transcript_store/service.rs
M  crates/quanta-memory-platform-testkit/src/fake/broker.rs
M  crates/quanta-memory-platform-testkit/src/faults/scenarios.rs
M  crates/quanta-memory-platform-testkit/src/fixtures/mod.rs
```
