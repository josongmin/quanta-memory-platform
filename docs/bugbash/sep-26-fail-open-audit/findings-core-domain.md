# Fail-Open Test Audit — core/domain — Auditor T1

## Header

Scope (owned files, grep `#[test]` + `#[tokio::test]` counts):

| file | `#[test]` | `#[tokio::test]` | total |
| --- | --- | --- | --- |
| `crates/quanta-memory-platform-core/src/domain/ledger.rs` | 38 | 0 | 38 |
| `crates/quanta-memory-platform-core/src/domain/policy.rs` | 14 | 0 | 14 |
| `crates/quanta-memory-platform-core/src/domain/config.rs` | 27 | 0 | 27 |
| `crates/quanta-memory-platform-core/src/domain/admission.rs` | 3 | 0 | 3 |
| `crates/quanta-memory-platform-core/tests/policy_contracts.rs` | 15 | 0 | 15 |
| **total** | **97** | **0** | **97** |

- Census rows below: **97** (== grep count; no sampling).
- Other `*.rs` under `core/src/domain/` (`context.rs`, `errors.rs`,
  `management_limits.rs`, `mod.rs`, `recharge.rs`, `stats.rs`) contain no
  `#[cfg(test)]` and no test attributes (verified by grep).
- Ledger/contract tests are `#[cfg(feature = "live_control")]`; `live_control`
  is a **default** feature, so plain `cargo test -p quanta-memory-platform-core`
  compiles them (no feature-gate fail-open). Verified green:
  `cargo test -p quanta-memory-platform-core --all-features --lib` (273 passed)
  and `--all-features --test policy_contracts` (15 passed).
- Verdict tally: **89 STRONG / 7 WEAK / 1 FAILOPEN / 0 VACUOUS**.

Track-specific (fail-closed atomicity): every ledger/contract test that names
denial-without-mutation proves it with full `LedgerSnapshot` equality
(derived `PartialEq` over class/owner/context maps + `live_ticket_count` +
`rebuild_in_progress`) — a byte-for-byte equivalent compare, not just an `Err`.
Error paths use exact variants (`matches!(Capacity(_))` /
`matches!(InvalidState(_))`), several with exact message pins
(`ticket_id_exhaustion…`, `unrepresentable_global_aggregate…`). The
`replace_committed` rollback path is snapshot-compared in two tests. Gaps are
the 7 WEAK + 1 FAILOPEN rows below.

## Census

Verdicts: STRONG / WEAK / FAILOPEN / VACUOUS. Mutations are one-line SUT
edits the critic can apply to confirm the oracle bites (for the FAILOPEN row,
the mutation demonstrates the named breakage staying green pre-fix and turning
red post-fix).

### ledger.rs

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `zero_byte_tickets_obey_independent_live_ticket_cap_fail_closed` (ledger.rs:1104) | STRONG | Fills 16 384 live zero-byte tickets, denies the next with `Capacity`, snapshot-eq, rollback frees exactly one slot. | — | delete the `live_ticket_limit_reached()` check in `reserve` |
| `committed_replacement_at_live_ticket_cap_is_a_net_zero_authority_swap` (ledger.rs:1133) | STRONG | Denied swap snapshot-eq; admitted swap `validate_exact` + exact ticket count/owner bytes; old ticket release rejected. | — | skip `restore_released_ticket_v1(old)` on the denied `reserve` in `replace_committed` |
| `owner_key_byte_limit_is_exact_and_denial_is_fail_closed` (ledger.rs:1186) | STRONG | Max-byte key admitted, +1 byte denied `Capacity` with snapshot-eq; UTF-8 half pins bytes-vs-chars (denial lacks snapshot pole; identical code path). | add snapshot-eq to the UTF-8 denial | compare `owner.chars().count()` instead of `owner.len()` in `reserve` |
| `reserve_denied_when_global_hard_limit_would_be_exceeded` (ledger.rs:1211) | STRONG | Typed `Capacity` + full snapshot-eq + total 600 pinned. | — | delete the `global_after > global_hard_limit_bytes` check in `check_ledger_postimage` |
| `reserve_for_unconfigured_class_is_denied_without_mutation` (ledger.rs:1228) | STRONG | Unconfigured future class denied `Capacity`, snapshot-eq on empty ledger. | — | delete the `UnknownClass` block in `check_ledger_postimage` |
| `global_ceiling_reserves_headroom_for_non_headroom_classes` (ledger.rs:1241) | STRONG | 900 into an 800-byte band denied + total 0; Headroom 900 admitted + total 900 (two-sided). | — | compute the band as `global` (ignore `headroom_bytes`) in the protected-band check |
| `headroom_usage_does_not_consume_protected_non_headroom_band_twice` (ledger.rs:1260) | STRONG | Headroom 200 + non-headroom 800 = exact 1000 admitted. | — | count Headroom bytes into `non_headroom_after` |
| `reserved_floor_protects_class_from_peer_consumption` (ledger.rs:1281) | STRONG | Peer 601 denied (floor 400), 600+400 admitted, exact total 1000, denial leaves 0. | — | drop `unused_class_floor` from `protected_postimage` |
| `double_release_of_a_cloned_ticket_is_rejected_and_does_not_corrupt` (ledger.rs:1319) | STRONG | Cloned replay denied `InvalidState` + snapshot-eq + invariants 1/2 re-checked. | — | drop the `ticket_states.contains_key` liveness check in `validate_release_ticket` |
| `commit_after_release_is_rejected` (ledger.rs:1344) | STRONG | Typed `InvalidState` on released-ticket commit (commit moves no byte counters; variant checked, not bare `is_err`). | optional snapshot-eq for ticket state | make the `None` ticket-state arm return `Ok(ticket.bytes)` |
| `second_commit_of_same_reservation_is_rejected` (ledger.rs:1357) | STRONG | Replay commit denied `InvalidState` + snapshot-eq. | — | treat `TicketState::Committed` as pending in `commit` |
| `ticket_id_exhaustion_fails_without_reuse` (ledger.rs:1372) | STRONG | Exact-`assert_eq!` message on exhaustion, exhaustion outranks byte denial, snapshot-eq. | — | wrap `next_ticket_id` back to `Some(1)` when `checked_add(1)` overflows |
| `borrow_over_class_cap_is_bounded_by_shared_burst_pool` (ledger.rs:1394) | STRONG | Cap fill + in-pool borrow admitted, over-pool denied, exact class usage 650. | — | delete the `total_borrow > shared_burst_pool_bytes` check |
| `shared_burst_pool_is_shared_across_borrowing_classes` (ledger.rs:1416) | STRONG | Cross-class borrow 120 > 100 denied; exact-fill 100 admitted. | — | compute `total_borrow` per class instead of summed |
| `borrow_false_still_denies_over_cap_regardless_of_pool` (ledger.rs:1436) | STRONG | Non-borrowing class over cap denied `Capacity` despite 10 000-pool (typed; no post-state pole needed — first op on empty ledger). | optional snapshot-eq | force `borrow_from_shared_pool = true` for all classes in admission |
| `owner_hard_cap_still_enforced` (ledger.rs:1451) | STRONG | Owner 600 > hard 500 denied `Capacity` (typed; ledger state pole covered by sibling denial tests). | optional snapshot-eq | delete the `owner_quotas` block in `check_ledger_postimage` |
| `empty_release_batch_is_a_noop_even_during_rebuild` (ledger.rs:1467) | STRONG | Empty batch `Ok` mid-rebuild + total unchanged; non-empty denied `InvalidState`; resumes to exact 0. | — | reject empty batches during rebuild (move the `is_empty` early-return after the fence) |
| `zero_byte_reserve_commits_and_releases_coherently` (ledger.rs:1523) | STRONG | Commit returns 0, release returns to pre-reserve snapshot-eq, second release denied + snapshot-eq. | — | skip `ticket_states.remove` for zero-byte tickets in `apply_release` |
| `overflow_denial_preserves_exact_live_ticket_accounting` (ledger.rs:1561) | STRONG | u64::MAX-adjacent overflow denied `Capacity` + snapshot-eq. | — | use `wrapping_add` for `global_after` |
| `exact_boundary_ticket_survives_first_release` (ledger.rs:1576) | STRONG | Sum == u64::MAX exactly; after releasing one ticket the survivor is exactly 10, then 0. | — | subtract the axis total instead of `ticket.bytes` in `apply_release` |
| `unrepresentable_sum_is_denied_without_mutating_any_axis` (ledger.rs:1595) | STRONG | Unrepresentable class postimage denied `Capacity` + snapshot-eq + original ticket still releasable to 0. | — | saturate the class postimage instead of denying |
| `batch_release_preflights_every_charge_kind_before_first_ticket_mutates` (ledger.rs:1610) | STRONG | Kind-slot corruption (class total preserved) denies the batch `InvalidState`; snapshot-eq; both tickets still live — the partial-release pole. | — | apply the first release before preflighting the rest |
| `single_release_preflight_is_bounded_independent_of_owner_cardinality` (ledger.rs:1666) | **FAILOPEN** | see below | see below | see below |
| `aggregate_usage_tracks_charge_kinds_and_headroom` (ledger.rs:1672) | STRONG | Exact `total_semantic_bytes` / live-scratch / `non_headroom_bytes` at each of 4 steps across kinds + Headroom. | — | stop updating `total_usage` in `reserve` |
| `snapshot_validation_rejects_unrepresentable_axis_instead_of_saturating_it` (ledger.rs:1716) | STRONG | Forged unrepresentable snapshot: `validate_exact` errs `InvalidState`; `total_owner_bytes` panics (single-call `catch_unwind`, not A5-abuse — the closure contains only that call). | — | make `checked_axis_total` saturate instead of erroring |
| `forged_snapshot_total_owner_bytes_panics_as_documented` (ledger.rs:1735) | STRONG | `#[should_panic(expected = "ledger owner total exceeds u64")]` + checked companion errs first. | — | make `total_owner_bytes` saturate instead of `expect` |
| `reserve_above_i64_max_does_not_wrap_to_negative_delta` (ledger.rs:1754) | STRONG | bytes>i64::MAX charged at full magnitude on owner + class axes; release returns exactly 0; axes balanced. | — | reintroduce `ticket.bytes as i64` signed delta in `apply_release` |
| `ticket_from_one_ledger_cannot_release_on_another` (ledger.rs:1789) | STRONG | Genuine id-space collision established (both id 1); foreign release denied `InvalidState` + B snapshot-eq; B's own ticket then releases. | — | delete the `ticket.ledger != self.instance_id` check in `validate_ledger_identity` |
| `ticket_ids_are_monotonic_and_unique_across_many_reserves` (ledger.rs:1817) | STRONG | 64 reserves: strictly increasing + set-cardinality == count. | — | reset the id counter when a ticket is released |
| `context_axis_balances_with_owner_after_mixed_sequence` (ledger.rs:1838) | STRONG | 4 fixed mixed reserves, per-step context==owner + all three balances, re-checked after a release. | — | stop updating `context_usage` in `reserve` |
| `mixed_kind_release_preserves_charge_kind_partition` (ledger.rs:1897) | STRONG | Exact per-kind totals: metadata release drains only the metadata slot; payload slot pinned at 500; ends at 0. | — | subtract releases from the payload slot regardless of `ticket.kind` |
| `deterministic_fuzz_preserves_invariants_and_returns_to_zero` (ledger.rs:1948) | STRONG | 400 deterministic steps; invariants 1/2/5/6 asserted after every step (admitted or denied); full drain returns all three axes to exactly 0. | — | omit the `context_usage` update in `reserve` (axes unbalance at step 1) |
| `sep23_fup004_honest_mixed_kind_snapshot_validates` (ledger.rs:2069) | STRONG | Honest mixed-kind snapshot validates + exact `[1000, 250, 500]` kind vectors on all three axes. | — | charge every reserve to the payload slot regardless of `kind` |
| `sep23_audit_admitted_recharge_moves_kind_vectors_exactly` (ledger.rs:2133) | STRONG | `replace_committed` moves kind vectors to exact `[0, 500, 500]` on every axis + `validate_exact` + owner total 1000. | — | in `replace_committed`, charge the replacement with the old ticket's kind |
| `sep23_audit_batch_release_moves_kind_vectors_exactly` (ledger.rs:2158) | STRONG | Batch release leaves exact `[0, 0, 500]`, final release exact zero vectors, both validated. | — | subtract batch refunds from the wrong kind slot in `preflight`/`apply` |
| `sep23_fup004_class_kind_swap_preserving_totals_is_rejected` (ledger.rs:2175) | STRONG | Scalar totals proven still balanced, then kind-swap rejected `InvalidState` — real negative pole for `validate_exact`. | — | delete the per-axis kind-vector comparison in `validate_exact` |
| `sep23_fup004_context_kind_swap_preserving_totals_is_rejected` (ledger.rs:2194) | STRONG | Same pole on the context axis (payload<->scratch swap, totals preserved). | — | as above |
| `sep23_fup004_owner_kind_swap_preserving_totals_is_rejected` (ledger.rs:2211) | STRONG | Same pole on the owner axis, invisible to the old owner-side re-sum. | — | as above |

**FAILOPEN detail — `single_release_preflight_is_bounded_independent_of_owner_cardinality` (ledger.rs:1666)**

Oracle (ledger.rs:899-903, 1646-1649):

```rust
self.last_release_preflight_axis_probes =
    class_refunds.len() + owner_refunds.len() + context_refunds.len();
...
assert_eq!(ledger.last_release_preflight_axis_probes, 3, "one release probes exactly one class/kind, owner/kind and context/kind slot");
```

Mechanism: the "probe" counter is computed from the **refund-map
cardinalities**, which are a function of the *batch* (one ticket → exactly 3
entries), not of the algorithmic path. The validation phase that follows
(lines 905-941) is keyed lookups today, but if it were rewritten to scan every
`owner_usage` entry (the O(cardinality) regression the name warns about), the
counter still reports 3 because it is derived from the aggregated refund maps,
which are built from the ticket list regardless of how validation probes. The
remaining asserts (owner cardinality, totals, balance) pin correctness, not
complexity. So the named failure mode — preflight cost growing with owner
cardinality — occurs while the test stays green at both 32 and 128 owners.
This is A1 (assert-on-construction) feeding an A8 invariant claim with no pole
that can fail. Minimal fix: increment the counter inside the actual probe
loops (count real axis lookups), or assert probe work against a
cardinality-scanning implementation via a `#[cfg(test)]` hook that counts
`BTreeMap::get` calls on the axes. Critic mutation: rewrite the owner
validation loop to `for (owner, usage) in &self.owner_usage` comparing against
refunds — current test stays green; after the fix (counter counts real
lookups) it turns red at `owner_count == 128`.

### policy.rs

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `check_helpers_admit_within_cap_and_reject_over_cap` (policy.rs:95) | STRONG | All five helpers: at-cap admitted, over-cap typed `Capacity` (boundary pinned at exactly 1 000 subqueries). | — | flip `check` to `used < limit` (deny at exact cap) |
| `unlimited_budget_admits_everything` (policy.rs:115) | STRONG | `None` axes admit `u64::MAX`/`usize::MAX`. | — | make `None` mean cap 0 in `check` |
| `for_preset_populates_every_mandatory_class` (policy.rs:470) | STRONG | Per-preset `contains_key` for all 14 mandatory classes + exact map cardinality. | — | skip one mandatory class in `ResidencyPolicy::new` |
| `for_preset_is_structurally_valid` (policy.rs:488) | STRONG | Every preset passes the real `validate_residency` authority. | — | make `default_residency` grant a cap above global |
| `for_preset_class_caps_isolate_below_global` (policy.rs:497) | **WEAK** | Asserts `hard_cap <= global`, but the comment/name claim *strict* isolation; a cap equal to global (nominal isolation) passes. | assert `budget.hard_cap_bytes < global` for non-Headroom classes | set a preset weight so one class cap equals the global limit (stays green pre-fix, red post-fix) |
| `for_preset_scales_caps_with_global_limit` (policy.rs:524) | **WEAK** | Single-class sample (ArtifactCache, dev vs prod) of a 13-class scaling claim; a per-class scaling regression elsewhere stays green. | loop over every non-Headroom class | hard-code one other class's cap to a constant |
| `for_preset_weighted_split_never_over_allocates_distributable` (policy.rs:537) | STRONG | Sum of non-Headroom hard caps <= distributable for all 4 presets (real coherence bound from SUT output). | — | round each weighted share up instead of floor |
| `for_preset_headroom_class_owns_exactly_the_reserved_band` (policy.rs:560) | STRONG | Exact equality: headroom cap == headroom_bytes and band reconstruction == global. | — | set the Headroom class cap to `global` |
| `for_preset_no_single_class_cap_plus_headroom_exceeds_global` (policy.rs:581) | STRONG | max(non-Headroom cap) + headroom <= global for all presets. | — | give one class cap = global while headroom > 0 |
| `default_residency_global_one_does_not_underflow_and_validates` (policy.rs:605) | STRONG | global=1 validates, no underflow/div-zero, exact headroom 0, per-class ordering. | — | switch headroom to ceiling division |
| `default_residency_small_globals_all_validate` (policy.rs:626) | STRONG | 9 small globals: validate + global preserved verbatim (panics would also fail the test). | — | clamp/recompute `global_hard_limit_bytes` in `default_residency` |
| `validate_rejects_constructed_headroom_above_global` (policy.rs:643) | STRONG | headroom 2 000 > global 1 000 rejected as typed `Config`. | — | delete the reserved+headroom floor check |
| `validate_accepts_headroom_equal_to_global` (policy.rs:655) | STRONG | Boundary acceptance: headroom == global is the exact floor (pairs with the reject row). | — | make the floor check strict `<` |
| `borrow_flag_with_zero_shared_pool_is_valid_and_preserved` (policy.rs:663) | STRONG | Validation accepts borrow=true with empty pool; builder preserves flag and pool (admission behavior deferred to contracts, as the comment states). | — | make `with_class_budget` force-clear `borrow_from_shared_pool` |

### config.rs

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `higher_precedence_layer_wins_and_is_recorded_in_provenance` (config.rs:464) | STRONG | Out-of-order registration; final value 900 + exact ordered provenance `[Manifest, Env, TestFixture]`. | — | drop the `ordered.sort_by_key` (apply in call order) |
| `removing_top_layer_changes_effective_value` (config.rs:505) | STRONG | Positive/negative pair: dropping TestFixture flips 900 → 700 + `assert_ne!`. | — | make `Env` outrank `TestFixture` in the `OverrideSource` ordering |
| `empty_layer_is_skipped_in_provenance` (config.rs:535) | STRONG | Empty Cli layer produces no provenance record (negative pole via `any`). | — | record provenance before the `is_empty` skip |
| `validate_rejects_reserved_above_soft` (config.rs:548) | STRONG | reserved 500 > soft 300 rejected typed `Config`. | — | delete the class `reserved > soft` check |
| `validate_rejects_global_below_reserved_plus_headroom_floor` (config.rs:563) | STRONG | floor 2 500 > global 1 000 rejected typed `Config`. | — | delete the class-axis floor check |
| `validate_rejects_class_hard_cap_above_global` (config.rs:582) | STRONG | class hard 2 000 > global 1 000 rejected typed `Config`. | — | delete the class hard-cap-vs-global check |
| `validate_rejects_owner_hard_cap_above_global` (config.rs:597) | STRONG | owner hard 5 000 > global 1 000 rejected typed `Config`. | — | delete the owner hard-cap-vs-global check |
| `validate_accepts_well_formed_policy` (config.rs:606) | STRONG | Positive pole for the reject family (headroom + real class budget accepted). | — | reject any policy with headroom > 0 |
| `validate_rejects_policy_map_cardinality_above_the_caps` (config.rs:628) | STRONG | At-cap accepted / one-over rejected typed `Config` for groups (256), mappings (4 096), quotas (4 096) — boundary pairs on all three axes. | — | delete the `resource_groups.len()` cardinality check |
| `runtime_reconfigure_layers_are_rejected_at_build_time` (config.rs:708) | STRONG | Build-time RuntimeReconfigure layer rejected typed `Config`; static stack precedence unchanged. | — | remove the RuntimeReconfigure rejection in `resolve` |
| `validate_rejects_shared_burst_pool_above_global_hard_limit` (config.rs:737) | STRONG | pool 20 000 > global 10 000 rejected; pool == global accepted (boundary pair). | — | delete the burst-pool-vs-global check |
| `validate_rejects_zero_admission_weight_with_burst_demand` (config.rs:751) | STRONG | weight 0 + burst 50 rejected; weight 0 + burst 0 accepted (pair). | — | delete the `admission_weight == 0 && burst_percent > 0` check |
| `validate_rejects_missing_each_mandatory_class` (config.rs:774) | STRONG | Removes each of the 14 mandatory classes in turn; every removal rejected typed `Config`. | — | delete the mandatory-class totality loop |
| `validated_policy_materializes_no_implicit_class_holes` (config.rs:788) | STRONG | `TryFrom` output contains every mandatory class explicitly. | — | make `try_from` strip non-budgeted classes |
| `reserved_floor_checked_add_overflow_is_config_error` (config.rs:801) | STRONG | Two u64::MAX reserved bytes: the `checked_add` overflow is itself the typed error (wrapping add would pass the floor and return Ok → red). | — | use `wrapping_add` for `total_reserved` |
| `validate_accepts_well_formed_resource_group` (config.rs:842) | STRONG | Coherent group passes (baseline for the one-field mutations below). | — | reject every policy containing resource groups |
| `validate_accepts_resource_group_reserved_equal_to_soft_cap` (config.rs:851) | STRONG | Inclusive-boundary acceptance with the result printed. | — | make group `reserved <= soft` strict |
| `validate_rejects_resource_group_reserved_above_soft` (config.rs:863) | STRONG | Single-field mutation of the baseline; typed `Config`. | — | delete the group reserved>soft check |
| `validate_rejects_resource_group_soft_above_hard` (config.rs:874) | STRONG | Single-field mutation; typed `Config`. | — | delete the group soft>hard check |
| `validate_rejects_resource_group_hard_above_global` (config.rs:885) | STRONG | Single-field mutation; typed `Config`. | — | delete the group hard-vs-global check |
| `validate_rejects_resource_group_burst_percent_over_100` (config.rs:897) | STRONG | 101 rejected typed `Config`. | — | delete the burst-percent band check |
| `validate_accepts_resource_group_burst_percent_boundaries` (config.rs:908) | STRONG | Both inclusive boundaries 0 and 100 accepted. | — | reject `burst_percent == 0` |
| `resource_group_reserved_has_an_independent_global_floor` (config.rs:922) | **WEAK** | All class `reserved_bytes` are 0 in the fixture, so a validator that summed class+group reserved into one joint floor also passes; "independent" is not pinned (the group floor's *existence* is pinned by the 939 row). | add class reserved 5 000 (own floor fits) so a joint floor of 11 000 > 10 000 would reject — acceptance then proves independence | change the validator to sum class+group reserved+headroom into one floor (stays green pre-fix, red post-fix) |
| `validate_rejects_group_reserved_plus_headroom_above_global` (config.rs:939) | STRONG | 5 000 + 5 000 + 1 000 > 10 000 rejected typed `Config`. | — | delete the group-axis floor check |
| `validate_rejects_owner_group_mapping_to_unknown_group` (config.rs:969) | STRONG | Dangling owner→group mapping rejected typed `Config`. | — | delete the unknown-group reference check |
| `validate_accepts_owner_group_mapping_to_declared_group` (config.rs:976) | STRONG | Declared-group mapping accepted (pair with 969). | — | reject all `owner_groups` mappings |
| `validate_rejects_overlong_owner_and_resource_group_identifiers` (config.rs:994) | STRONG | MAX+1 keys rejected typed `Config` on all three axes (quota owner, mapping owner, group name). | — | delete the key-length checks |

### admission.rs

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `deficit_saturates_for_unrepresentable_postimages` (admission.rs:810) | STRONG | `deficit == u64::MAX` for `postimage: None` + exact error round-trip through `into_error`. | — | compute deficit as `postimage.unwrap_or(0)` |
| `unrepresentable_global_aggregate_blocks_without_panic` (admission.rs:826) | STRONG | Pure engine driven with a view no live ledger can hold; exact `assert_eq!` on the `InvalidState` error and message — fail-closed, no panic. | — | map the `checked_total` failure to `MemoryError::capacity` instead of `InvalidState` |
| `unknown_group_denies_terminally_with_legacy_message` (admission.rs:872) | **WEAK** | Proves denial + exact legacy message via `check_group_admission`, but never calls `preflight()`: the *terminality* classification (TerminalDenial vs ReclaimCandidate — whether a reclaim callback fires) is not exercised; a preflight misclassification of unknown groups stays green. | build a `PreflightInputs` with `owner_group = Some("missing")` and `assert!(matches!(preflight(..), AdmissionVerdict::TerminalDenial { .. }))` | make the zero-state probe in `preflight` return `Ok(())` for unknown groups (verdict becomes ReclaimCandidate; current test green, fixed test red) |

### tests/policy_contracts.rs

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `execution_budget_default_matches_runtime_budget_truth` (policy_contracts.rs:10) | STRONG | Pins all five public default caps against accidental drift (golden-value contract). | — | change any default in `ExecutionBudget::default` |
| `memory_class_taxonomy_keeps_mandatory_and_future_classes_split` (policy_contracts.rs:21) | STRONG | Exact set sizes, membership, and mandatory/future disjointness. | — | move a class between `mandatory()` and `future()` |
| `every_preset_and_custom_validated_policy_is_total_over_mandatory_classes` (policy_contracts.rs:38) | STRONG | 4 presets + custom policy × all 14 mandatory classes present after `TryFrom`. | — | drop a mandatory class from `for_preset`'s residency |
| `residency_policy_tracks_class_quota_and_resource_group_vocabulary` (policy_contracts.rs:59) | **WEAK** | Shape-not-content: `contains_key` for class/quota/group + scalar echo of builder inputs; a builder that stores a *default/mangled* quota or group under the right key stays green. | assert exact stored `MemoryClassBudget`/`OwnerQuota`/`ResourceGroup` equality (round-trip content) | make `with_resource_group` store a `ResourceGroup::default`-like value under the key (green pre-fix, red post-fix); bite check: drop quotas in `with_owner_quota` |
| `builder_emits_effective_budget_report_with_override_provenance` (policy_contracts.rs:100) | STRONG | Exact preset, execution cap, `override_sources == ["preset"]`, revision 1, source label, and 512 MiB global at the public report surface. | — | emit an empty `override_sources` vector |
| `memory_error_has_typed_config_and_report_variants` (policy_contracts.rs:115) | STRONG | Exact `Display` strings for the typed variants. | — | change a variant's Display prefix |
| `global_hard_limit_denies_reserve_through_public_broker_and_is_fail_closed` (policy_contracts.rs:172) | STRONG | Public broker denial typed `Capacity` + before/after `LedgerSnapshot` equality + totals + invariants 1/2. | — | skip the global check on the broker reserve path |
| `headroom_band_is_reserved_for_headroom_class_at_public_surface` (policy_contracts.rs:208) | STRONG | Non-headroom 900 into 800-byte band denied + total 0; Headroom 900 admitted + total 900 (two-sided). | — | ignore headroom in the broker's band computation |
| `shared_burst_pool_bounds_borrowing_through_public_broker` (policy_contracts.rs:241) | STRONG | Cap + 50 admitted, +60 denied; snapshot-eq; exact total 650. | — | treat the shared pool as unlimited at the broker surface |
| `zero_shared_pool_cannot_extend_borrowing_class_hard_cap` (policy_contracts.rs:285) | STRONG | 1 byte over cap denied; snapshot-eq; exact 800; Drop returns ledger to 0. | — | treat an empty pool as unlimited |
| `non_borrowing_class_cannot_use_burst_pool_at_public_surface` (policy_contracts.rs:327) | STRONG | Non-borrowing 700 > cap 600 denied despite 10 000 pool; total 0. | — | force `borrow_from_shared_pool = true` in the broker admission |
| `managed_charge_double_release_is_rejected_without_counter_drift` (policy_contracts.rs:352) | STRONG | Second `release()` typed `InvalidState` + snapshot-eq with post-first-release + invariants 1/2. | — | make `ManagedCharge::release` idempotent (`Ok` on second call) |
| `dropping_pending_public_authority_rolls_reservation_back` (policy_contracts.rs:388) | **WEAK** | Proves the byte refund (10 000 → 0) but not full rollback: `live_ticket_count`/snapshot equality unasserted, so a Drop that refunds bytes while leaking the ticket slot (later hitting the 16 384 live-ticket cap) stays green. | capture a pre-reserve snapshot and `assert_eq!` after drop (pins `live_ticket_count == 0` and empty maps) | in `apply_release`, skip `ticket_states.remove` while keeping byte refunds (green pre-fix, red post-fix); bite check: make the pending-authority `Drop` a no-op |
| `preset_class_caps_bite_at_public_reserve_surface` (policy_contracts.rs:409) | STRONG | Per preset: cap+1 denied `Capacity` + fail-closed total 0; exactly cap admitted + total == cap (boundary pair on the real report value). | — | make the broker ignore class caps (global-only admission) |
| `preset_broker_global_limit_is_consistent_with_enforcement` (policy_contracts.rs:458) | **WEAK** | One-sided: asserts over-report-limit reserves are denied for all classes, but never that an at-limit reserve is admitted — enforcement *stricter* than the report (diverged limits) stays green. | add an admission probe at the reported limit (permissive custom policy whose class cap == global) proving enforcement is not tighter than the report | enforce `global/2` while reporting the full limit (green pre-fix, red post-fix); bite check: report 1 MiB while enforcing 512 MiB |

## Footer — risk summary

1. The fail-closed core is genuinely well-guarded: every denial-named ledger and
   public-broker test compares full `LedgerSnapshot` equality (byte-for-byte
   plus ticket count), error variants are typed (never bare `is_err()`), and
   the batch-release/replacement rollback paths have real preflight-failure
   poles — the Err-with-corrupted-state bug class has no uncovered instance in
   this scope.
2. Residual risk concentrates in incomplete bounds: the one FAILOPEN row
   (ledger.rs:1666) measures probe *map cardinality*, not probe *work*, so an
   O(owner-cardinality) preflight regression ships green; the WEAK rows leave
   unilateral claims unpinned (strict cap isolation, preset scaling for 12 of
   13 classes, group-floor independence, unknown-group terminality, full
   rollback incl. ticket slots, report-vs-enforcement consistency).
3. No A2/A4/A5/A6/A7 archetype landed in scope (no `let _ =` on ops under
   test, no vacuous loops — all iterables provably non-empty, no fault
   coordinates, catch_unwind is single-call scoped); the residual A1/A8
   exposure is exactly the FAILOPEN row plus the contains_key / golden-echo
   WEAK rows listed above.
