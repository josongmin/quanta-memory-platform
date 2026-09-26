# T8 — Fail-Open Test Audit: publish_pipeline / presets / shared / public_api_surface

Auditor: T8 (parallel fail-open audit, 2026-09-26 track). Read-only SUT; this file is
the only artifact.

## Header — scope, census, verification

Scope files (all under `crates/quanta-memory-platform-facade/` unless noted):

| file | `#[test]`+`#[tokio::test]` grep count |
| --- | --- |
| `src/publish_pipeline/prepared_publication.rs` | 32 |
| `src/publish_pipeline/service.rs` | 5 |
| `src/publish_pipeline/mod.rs` | 0 (no tests; no `#[cfg(test)]` mod) |
| `src/presets/mod.rs` | 0 (no tests; no `#[cfg(test)]` mod — nothing to audit) |
| `src/shared/builder.rs` | 11 |
| `src/shared/lifecycle.rs` | 7 |
| `src/shared/measure.rs` | 9 |
| `src/shared/accounting.rs`, `src/shared/mod.rs` | 0 each (no test fns) |
| `tests/public_api_surface.rs` | 20 |

**Grep total = 84. Census rows = 84. Equal.** No other test macros
(`test_case`/`proptest`/`rstest`) exist in scope; no `#[cfg(test)]` module hides tests
outside the counted files.

Execution verification (baseline, all green, counts match census):
`cargo test -p quanta-memory-platform-facade --all-features --lib publish_pipeline`
→ 38 passed (my 37 + 1 name-matched `shared::builder` test);
`--lib shared::` → 27 passed (11+7+9);
`--test public_api_surface` → 20 passed.

Feature gates noted: `prepared_publication::tests::accounting` needs
`core-live-accounting`; `shared::builder::tests` needs `full`; surface tests are
per-feature `cfg`'d — all counted under `--all-features`.

Verdict tally: **79 STRONG, 5 WEAK, 0 FAILOPEN, 0 VACUOUS.** All 5 WEAK are in
`tests/public_api_surface.rs`.

## Census table

Verdicts: STRONG = breaking the named behavior turns red; WEAK = asserts strictly less
than the name; FAILOPEN = named failure can occur while green; VACUOUS = named path
unreachable. Archetypes A1–A8 checked per row; only hits are named.

### prepared_publication.rs (32 rows)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| unbudgeted_admission_never_exceeds_a_complete_gc_root_set (prepared_publication.rs:1071) | STRONG | 16 384 live roots; +1 denial is typed `Storage(Capacity)`, whole-stats struct compared byte-identical before/after, GC frees exactly 1, replacement admitted — state, not return value | — | delete the `entries.len() >= MAX_REFERENCED_ROOT_TOKENS` admission check → denial assertions go red |
| prepare_capacity_failure_leaves_all_state_unchanged (prepared_publication.rs:1127) | STRONG | denial typed `Capacity`; post-state: `stats()==default` and `orphan_tokens(&[])==[]` | — | admit past capacity and repair stats afterwards → stats assertion red |
| prepared_payload_is_only_reachable_by_exact_token (prepared_publication.rs:1136) | STRONG | exact token resolves payload; forged `0xA5..` → `UnknownToken`; `prepared_count==1` | — | make `resolve` fall back to first entry for unknown tokens → red |
| identical_prepare_replays_same_token_without_second_charge (prepared_publication.rs:1150) | STRONG | same token, `Applied` vs `Replayed`; `charged_bytes == live_cost` (independent literal oracle, not production formula) | — | re-charge on replay → charged_bytes doubles → red |
| identity_payload_drift_is_typed_conflict (prepared_publication.rs:1161) | STRONG | drift → `IdentityConflict`; original payload intact; charge unchanged | — | overwrite payload on drift → resolve-“old” red |
| abort_releases_exact_payload_and_is_retry_safe (prepared_publication.rs:1171) | STRONG | `Aborted` then `Replayed`; resolve → `AbortedToken`; full stats struct pinned incl. tombstone bytes | — | skip tombstone insert on abort → stats assertion red |
| concurrent_identical_prepare_has_one_applied_winner (prepared_publication.rs:1195) | STRONG | 8 threads/barrier; exactly 1 `Applied` + 7 `Replayed`, one shared token, exact charged bytes | — | remove identity-index replay check → >1 Applied or double charge → red |
| concurrent_conflicting_prepare_has_one_winner (prepared_publication.rs:1223) | STRONG | 2 conflicting payloads: 1 Ok + 1 `IdentityConflict`; `prepared_count==1` | — | last-writer-wins insert → count 2 or wrong error → red |
| root_referenced_token_is_never_enumerated_or_collected_as_an_orphan (prepared_publication.rs:1250) | STRONG | enumeration lists only unreferenced; GC removes it; collected → `AbortedToken`, survivor resolves; count 1 | — | GC ignores reference set and collects survivor → red |
| orphan_gc_with_unknown_root_reference_is_batch_atomic (prepared_publication.rs:1269) | STRONG | forged ref → `UnknownToken`; BOTH payloads + exact stats survive (batch atomicity of state) | — | classify before validating refs (partial collect) → resolve/stats red |
| aborted_root_reference_fails_closed_without_collecting_other_entries (prepared_publication.rs:1292) | STRONG | aborted ref → `AbortedToken`; survivor intact; exact stats incl. tombstone | — | treat aborted ref as absent → survivor collected → red |
| tombstone_limits_bound_n_cycle_churn_and_fail_closed_at_saturation (prepared_publication.rs:1315) | STRONG | 4 cycles then denial with ALL FOUR `TombstoneCapacity` fields pinned; payload intact post-denial; stats exact | — | drop `ensure_tombstone_capacity` count check → field-matching `matches!` red |
| large_identity_with_zero_payload_is_denied_before_retention (prepared_publication.rs:1564) | STRONG | identity 101 > class hard 100 → `Accounting(Capacity)`; stats 0 AND broker ledger 0 (authority, not advisory) | — | pre-charge then refund on denial leaves a window → ledger snapshot red |
| prepared_identity_index_and_tombstone_keep_exact_independent_cost (prepared_publication.rs:1578) | STRONG | literal-pinned buckets 17/129 across ledger kind axes, ticket count, per-axis sole-entry checks; rebuild-fence denial; GC/abort/close drain verified to zero | — | double-charge the persistent lease on replay → kind-axes assertion red |
| orphan_roots_bound_and_fail_closed (prepared_publication.rs:1712) | STRONG | at-cap input OK; +1 raw token → typed `Storage(Capacity)` with stats AND ledger snapshot compared byte-identical; mixed unknown → prior typed error, no partial GC; busy lane; aborted ref; final drain 0 | — | build the normalized set before the raw-length bound → oversized-input denial red |
| prepared_component_sum_overflow_is_fail_closed_before_reservation (prepared_publication.rs:1807) | STRONG | `byte_size()==u64::MAX` → `Accounting(Capacity)`; stats default; ledger 0 — overflow rejected before any broker mutation | — | use `saturating_add` in `active_cost` → silent wrong charge; ledger/stats red |
| resolve_clone_reentry_under_read_lock_is_typed_busy_and_preserves_charge (prepared_publication.rs:1831) | STRONG | payload's `clone()` asserts `stats()` is `OperationInProgress("prepared publication service")` from inside the read lock; `clone_hits==1`; exact charge; close refunds 0 | — | hand the read callback an unlocked clone of state → reentrancy assert inside payload fires red |
| prepare_measurement_reentry_is_typed_busy_and_replay_skips_measurement (prepared_publication.rs:1872) | STRONG | `measure_hits==1` after first, still 1 after replay; exact ledger total; close refunds | — | re-measure on replay → measure_hits 2 → red |
| prepare_measurement_panic_preserves_readability_and_allows_retry (prepared_publication.rs:1913) | STRONG | see catch_unwind table (site 1923): panic transitively pinned — a first-prepare panic outside `byte_size` leaves the one-shot flag armed, so the retry `expect` would panic; post-state: stats default, ledger 0, retry `Applied`, replay skips measurement, exact totals | add `assert!(!panic_measure.load())` right after the catch to pin flag consumption explicitly instead of via the retry | remove the guard-drop before `resume_unwind` at prod line 608 → mutex poisons → `stats()` Ok-default red |
| resolve_clone_panic_preserves_readability_and_charge (prepared_publication.rs:1956) | STRONG | see catch_unwind table (site 1973): retry-resolve succeeding transitively proves the clone flag was consumed; stats==before, ledger==charged, close refunds | — | poison state on resolve panic → stats-readable assertion red |
| replay_charges_once_and_abort_preserves_identity_evidence_once (prepared_publication.rs:1988) | STRONG | ledger pinned at each step: live → evidence after abort → unchanged after abort replay → 0 after close | — | release evidence charge on abort → ledger red |
| failed_abort_keeps_payload_index_and_charge_coherent (prepared_publication.rs:2013) | STRONG | rebuild fence denies release → `Accounting`; resolve/stats/ledger intact; fence lifted → abort succeeds (denial is retryable, state coherent) | — | drop the charge-restore path in abort's release-failure branch → resolve/stats red |
| broker_capacity_denial_leaves_no_local_or_ledger_state (prepared_publication.rs:2033) | STRONG | broker(4) < request → `Accounting(Capacity)`; stats default; ledger 0 | — | mutate local map before charging → stats red |
| failed_orphan_gc_preserves_every_payload_index_and_charge (prepared_publication.rs:2046) | STRONG | fence denies batch release → `Accounting`; both payloads + stats + ledger exact; retry GC returns 2; close drains | — | remove per-token charge restore in GC release-failure loop → ledger red |
| close_refunds_all_entries_and_is_terminal_and_idempotent (prepared_publication.rs:2081) | STRONG | close → ledger 0 + default stats + `UnknownToken`; prepare/abort/gc → `Closed`; double close Ok; ledger stays 0 (terminal STATE, not just Ok) | — | skip `close.commit()` on success → post-close prepare succeeds → red |
| failed_close_keeps_exact_payloads_tokens_and_charges_for_retry (prepared_publication.rs:2121) | STRONG | fence denies close → stats==before, both resolves intact, ledger exact; retry close refunds 0 | — | leave charges taken but state emptied on failed close → resolve red |
| drop_refunds_live_entries_and_rebuild_drop_defers_exact_release (prepared_publication.rs:2144) | STRONG | healthy drop → 0; fenced drop defers exact charge until fence finish (fence outlives service by construction) | — | release through the fence in Drop → “deferred” snapshot red |
| multiple_panicking_client_drops_during_outer_unwind_drain_exact_charges (prepared_publication.rs:2179) | STRONG | 3 ops × 3 panicking payloads: outer panic stays observable, `drops==3`, ledger 0 — charges drained even during hostile unwind | — | skip retirement when already unwinding (`panic::catch_unwind` swallow in retire) → drops/ledger red |
| mutation_gate_returns_typed_busy_and_exposes_no_partial_plan (prepared_publication.rs:2225) | STRONG | all five entry points `OperationInProgress` while lane held; after drop, exact charge; after close, empty stats; ledger 0 | — | let stats read past the mutation lane → busy assertion red |
| poisoned_store_reports_error_but_drop_still_releases_every_charge (prepared_publication.rs:2266) | STRONG | poison → `stats`/`close` typed `Storage(_)` errors; ledger still charged; Drop still refunds to 0 | — | make Drop skip cleanup when mutex poisoned → “drop refunds” red |
| checked_overflow_and_underflow_reject_before_broker_or_index_mutation (prepared_publication.rs:2285) | STRONG | white-box counter corruption: `charged_bytes=u64::MAX` → prepare `Capacity`, ledger 0; underflow → `InvalidState`, resolve/ledger intact; restored → abort succeeds | — | replace checked_sub with wrapping in abort → no InvalidState, wrong ledger → red |
| orphan_gc_rejects_corrupt_active_lease_cost_without_partial_refund (prepared_publication.rs:2330) | STRONG | corrupted `active_bytes=u64::MAX` → `InvalidState`, payloads/stats/ledger intact, zero tombstones; restored → GC 2 | — | skip `require_charge_shape` in GC → mismatched-authority corruption silently refunds → ledger red |

### publish_pipeline/service.rs (5 rows)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| identity_pipeline_stages_then_commits_published (service.rs:165) | STRONG | staged value visible only via `StagedHandle.staged`; `published_count` 0 before commit, 1 after — state machine counter is the assertion target | — | increment count in `stage` → “count before” red |
| failed_commit_never_publishes_and_never_increments_count (service.rs:180) | STRONG | A8 clean: failing transition → typed `Capacity`, count 0; positive correction commits and counts 1 | — | increment before running transition → count-after-failure red |
| default_commit_transition_applies_identity_with_no_missing_step (service.rs:214) | STRONG | drives `DefaultCommitTransition::apply` directly: `Ok(staged-value)` passthrough, no error path | — | make apply return Err → expect red |
| default_pipeline_runs_stage_then_commit_each_exactly_once_no_rollback (service.rs:229) | STRONG | per-iteration count checked pre-stage (`i`) and post-commit (`i+1`); final 3 — each step exactly once | — | double-increment on commit → intermediate counts red |
| published_count_only_reflects_committed_not_staged (service.rs:250) | STRONG | 3 staged + 1 committed → count exactly 1 | — | count staged handles → red |

### shared/builder.rs (11 rows)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| builder_first_happy_path_builds_a_usable_payload_store (builder.rs:766) | STRONG | built store actually `put_canonical`s; stats `item_count==1`; published key `payload-0` pinned | — | builder returns store bound to a detached broker → stats red |
| payload_builder_without_binding_fails_with_build_error (builder.rs:784) | STRONG | A8 pair present: missing binding → typed `MemoryError::Build`, no panic | — | default the binding silently → expect_err red |
| retained_map_platform_builder_routes_bytes_through_the_ledger (builder.rs:794) | STRONG | insert 64B → ledger `total_owner_bytes()==105` exact (independent arithmetic in comment) | — | builder skips broker wiring → snapshot 0 ≠ 105 red |
| publish_pipeline_builder_without_binding_fails_with_build_error (builder.rs:807) | STRONG | typed `Build` error on missing binding | — | default binding → red |
| transcript_builder_without_binding_fails_with_build_error (builder.rs:815) | STRONG | typed `Build` error | — | default binding → red |
| transcript_platform_builder_routes_bytes_through_the_ledger (builder.rs:824) | STRONG | append → ledger `236` exact with component arithmetic in comment | — | skip ledger wiring → red |
| revocable_builder_without_binding_fails_with_build_error (builder.rs:848) | STRONG | typed `Build` error | — | default binding → red |
| revocable_platform_builder_routes_bytes_through_the_ledger (builder.rs:857) | STRONG | insert 16B → ledger `49` exact | — | skip ledger wiring → red |
| default_build_composes_charged_stores_without_manual_glue (builder.rs:873) | STRONG | both poles: insert → 89; remove → 0 (charge and refund through one platform ledger) | — | remove() forgets release → final 0 red |
| default_build_platform_sink_is_the_same_ledger_as_the_broker (builder.rs:895) | STRONG | two stores, one broker: sum `117 == (10+41)+(25+41)` — shared-authority identity, not two ledgers | — | builder constructs a fresh broker per store → total 66-max → red |
| memory_platform_builder_builds_broker_from_preset (builder.rs:921) | STRONG | built broker is usable: `attach_runtime` succeeds, kind pinned | — | build returns inert broker → attach_runtime red |

### shared/lifecycle.rs (7 rows)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| hostile_panic_payload_drop_cannot_abort_outer_unwind (lifecycle.rs:366) | STRONG | subprocess isolation; child asserts outer panic propagates with exact payload (`&"outer unwind"`) after hostile `panic_any` destructor; parent asserts child exit success — both survival and panic identity pinned | — | let the hostile drop abort (unwind-through-unwind) → child status non-zero → red |
| client_callback_reentry_on_held_container_lock_is_typed_busy (lifecycle.rs:394) | STRONG | held lock → `try_lock_state` = `OperationInProgress("callback owner")` (resource name pinned); after drop, reads 7 | — | block instead of try-lock → deadlock/timeout, not typed busy |
| reentry_returns_typed_busy_without_blocking (lifecycle.rs:407) | STRONG | second `begin_mutation`, `ensure_readable`, `begin_close` all typed busy with resource name; permit drop reopens | — | use a plain `lock()` in `begin_mutation` → test hangs (fail-closed detection via timeout) |
| close_excludes_mutations_and_other_close_attempts (lifecycle.rs:430) | STRONG | phase `Closing`; mutation and second close excluded; rollback reopens to `Open` (state asserted) | — | rollback leaves phase Closing → phase assertion red |
| failed_close_reopens_for_mutation_and_retry (lifecycle.rs:451) | STRONG | dropped (uncommitted) close guard → phase `Open`; mutation resumes; close re-acquires | — | Drop on close guard commits instead of rollback → phase red |
| committed_close_stays_closed_and_double_close_is_idempotent (lifecycle.rs:464) | STRONG | commit → `Closed`; terminal image still readable; mutation → `Closed` err; `begin_close` → `AlreadyClosed`; phase stable | — | reopen after commit → phase red |
| drop_transition_is_terminal_and_reports_cleanup_once (lifecycle.rs:482) | STRONG | `CleanupRequired` exactly once, then `AlreadyClosed`; pre-committed lifecycle reports `AlreadyClosed` — no double cleanup | — | allow repeated CleanupRequired → second call red |

### shared/measure.rs (9 rows)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| retained_cost_v1_sums_each_component_exactly_once (measure.rs:226) | STRONG | 1+2+3+4+5 = 15; doubled = 30, literal-pinned | — | drop a component from `total()` → 15 red |
| retained_cost_v1_rejects_component_and_cross_component_overflow (measure.rs:239) | STRONG | component overflow, cross-component aggregate overflow, and direct `total()` all `is_err` (checked math on every path) | — | make `total()` wrapping → third assert red |
| measured_value_charges_caller_supplied_size_not_inner_len (measure.rs:257) | STRONG | 3-byte inner, declared 4096 → `byte_size()==4096` | — | infer from inner len → 4096 red |
| measured_value_into_inner_returns_wrapped_value (measure.rs:267) | STRONG | `into_inner` returns original | — | drop value → compile/expect red |
| measured_value_zero_size_is_charged_verbatim (measure.rs:273) | STRONG | A8 pair: declared 0 with 100-byte inner → charges 0 | — | special-case zero to inner len → red |
| arc_byte_size_delegates_to_inner (measure.rs:281) | STRONG | `Arc<Vec<u8>>` → 12 | — | charge pointer size → red |
| cloning_measured_arc_shares_the_allocation_not_a_deep_copy (measure.rs:287) | STRONG | `Arc::ptr_eq` AND `strong_count==3` — identity, not shape | — | deep-clone in `MeasuredValue::clone` → both red |
| cloning_measured_value_preserves_size_and_value_context (measure.rs:305) | STRONG | size 8192 + value + full-tuple equality across clone | — | recompute size on clone → red |
| measured_arc_any_handle_charges_caller_size (measure.rs:318) | STRONG | `Arc<dyn Any>` has no inherent size; 2048 supplied → 2048 charged | — | panic/0 on dyn Any → red |

### tests/public_api_surface.rs (20 rows)

Surface/extraction-contract context applied: locking paths and signatures is the
legitimate job here; verdicts judge the *behavioral* claims each body makes.

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| live_broker_bridge_is_present_only_with_core_live_accounting (public_api_surface.rs:73) | WEAK | presence-only: `let _broker = platform.broker();` — the “only with” half is legitimately compile-gated, but the broker view is merely *used* with its shape unlocked (return type unnamed, no method called) | `let _broker: &LiveProcessMemoryBroker = platform.broker();` or invoke one observable (`ledger_snapshot().live_ticket_count == 0`) | change `broker()` to return an opaque newtype with no methods → body still compiles and passes |
| prepared_publication_builder_exposes_root_bound_resolution_contract (public_api_surface.rs:184) | STRONG | real behavior through the public builder: capacity/tombstone knobs, `Applied`, resolve content, referenced root excluded from orphan enumeration, token bytes round-trip | — | builder ignores `with_capacity_bytes`/`with_tombstone_limits` → covered knobs silently default; the orphan/resolve assertions still catch core regressions, and capacity is covered by lib tests |
| measured_value_round_trips_value_and_size (public_api_surface.rs:260) | STRONG | value/size/`byte_size`/`into_inner` all content-asserted | — | swap size semantics → red |
| portable_presets_cover_every_preset_with_full_class_taxonomy (public_api_surface.rs:277) | WEAK | cardinality-only: `classes.len() == mandatory().len()` on a `BTreeMap<MemoryClass,_>` — a same-count *wrong* class set passes, and `ResidencyPolicy::new` pre-fills all mandatory classes, so the check is nearly tautological; the 4-preset array is hand-maintained with no compile-time tie to the enum (A2-adjacent: a 5th `MemoryPreset` variant stays green) | assert the class KEY SET equals `BTreeSet::from(MemoryClass::mandatory())` per preset, and enumerate presets via a non-exhaustive `match` so a new variant fails compilation | in one preset swap a mandatory class budget for an opt-in class (same count) → stays green |
| memory_platform_builder_builds_a_broker_from_a_preset (public_api_surface.rs:299) | WEAK | `is_ok()`-only positive pole; the built broker is never observed in this body (doc claims “wired to the core builder”) — that wiring is actually proven by builder.rs:921 `attach_runtime` | call one method on the built platform (e.g. `broker().ledger_snapshot()` is-ok) or rely on builder.rs:921 and rename this to a pure compile pin | make `build()` return `Ok` of a platform whose broker view is inert/unwired → still green here |
| memory_platform_builder_without_config_fails_closed (public_api_surface.rs:309) | STRONG | negative pole: unconfigured builder `is_err` (typed, no panic) | — | default to Development preset silently → red |
| report_only_builder_resolves_without_runtime_authority (public_api_surface.rs:315) | STRONG | report-only resolves; `report.preset == Production` pinned | — | preset field dropped/misrouted → red |
| report_only_builder_without_config_fails_closed (public_api_surface.rs:324) | STRONG | unconfigured report builder `is_err` | — | default preset → red |
| default_builder_over_cap_store_mutation_is_denied (public_api_surface.rs:331) | STRONG | over-cap insert → typed `Capacity`; ledger snapshot byte-identical before/after; denied value not readable — denial proven on STATE (ledger + visibility), not return value | — | mutate ledger then roll back on denial → snapshot equality red |
| default_builder_and_report_only_share_override_resolution (public_api_surface.rs:374) | STRONG | `effective_report() == report_only` structurally; global limit 80 and override provenance source pinned | — | live path drops the override layer → equality red |
| platform_builder_installs_reclaimer_before_live_construction (public_api_surface.rs:414) | WEAK | A6-adjacent (wiring never lands): `with_reclaimer` acceptance + build Ok + `attempts==0`. A silently DISCARDED reclaimer also yields `attempts==0`, so “installs” is unproven; only compile-time ordering (no post-build setter) is locked | install a counting `MemoryReclaimer`, trigger one reclaim via the broker, assert the counter incremented (or assert the reclaimer is live via an observable effect) | facade `with_reclaimer` discards its argument (`self` unchanged) → test stays green |
| payload_store_builder_yields_a_queryable_store (public_api_surface.rs:439) | WEAK | zero-state-only: `charged_bytes()==0`, `item_count==0` — a constant-zero stub store passes; no put→observe flow in this body (positive pole lives in builder.rs:766) | add one `put_canonical` + nonzero `charged_bytes`/`item_count` assertion, or delegate to builder.rs:766 and keep this as a type-shape pin | hardcode `stats()`/`charged_bytes()` to return zeros → stays green |
| retained_map_builder_exposes_public_charged_path (public_api_surface.rs:459) | STRONG | insert 32B → ledger `73` exact | — | skip ledger wiring → red |
| explicit_unbudgeted_store_is_a_distinct_product_and_does_not_charge_the_platform (public_api_surface.rs:474) | STRONG | both poles + type distinctness (`only_budgeted`/`only_unbudgeted` compile pins): unbudgeted write → platform ledger 0 and own stats 54; budgeted write → ledger 61 | — | unbudgeted builder secretly charges the platform → ledger-0 red |
| retained_map_atomic_insert_returns_occupied_value_without_overwrite (public_api_surface.rs:502) | STRONG | `Inserted` then `AlreadyPresent` with `Arc::ptr_eq` on both the returned handle and the stored value (allocation identity, not shape); stats 44 | — | overwrite on conflict → ptr_eq red |
| revocable_builder_exposes_public_charged_path (public_api_surface.rs:539) | STRONG | insert 24B → store stats 57 AND ledger 57 (store stats cross-checked against authority) | — | stats/ledger drift → red |
| transcript_builder_exposes_public_charged_path (public_api_surface.rs:558) | STRONG | `Applied`, load len 1, stats 236, retention stats (1/1/236), ledger 236 | — | retention counts drift → red |
| spill_controller_round_trips_a_handle_via_restore (public_api_surface.rs:592) | STRONG | spill Ok → count 1 → restore `Some(42)` → count 0 (inventory state transitions) | — | restore without decrement → count red |
| spill_controller_fails_closed_when_target_unavailable (public_api_surface.rs:612) | STRONG | typed `Capacity` failure, handle returned via `into_parts().1 == 7` (no silent drop), count stays 0 | — | drop handle on failure → parts red |
| noop_spill_controller_rejects_and_restores_nothing (public_api_surface.rs:624) | STRONG | spill rejected with handle returned; restore `None`; doc claim “must not certify discard” matches assertions | — | noop accepts and discards → expect_err red |

## catch_unwind sites (prepared_publication.rs)

| site | context | panic pinned to intended closure/phase? | post-panic publication state asserted? |
| --- | --- | --- | --- |
| ~608 (production, `prepare`) | catches caller `byte_size()` panic, drops state guard, `resume_unwind` | YES — pinned by test :1913: the one-shot flag is consumed only inside `byte_size`; if the panic originated elsewhere the retry `expect` would re-panic (unconsumed flag), and if the guard were NOT dropped first the mutex would poison | YES — stats `Ok(default)`, ledger 0, retry `Applied`, replay `Replayed` with measurement skipped (flag still armed), exact charge, close refunds 0 |
| ~1923 (test, `prepare` measure-panic) | `catch_unwind` around `service.prepare(one_shot_request(..))` with `panic_measure=true, panic_clone=false` | YES (transitive) — only `byte_size` is armed; any other panic leaves the flag armed → retry prepare panics → red. Note `let _ =` on the Result is unreachable-by-panic, and the panic itself IS asserted (`.is_err()`) | YES — full post-state (see :1913 row above) |
| ~1973 (test, `resolve` clone-panic) | `catch_unwind` around `service.resolve(token)` with `panic_clone=true` | YES (transitive) — retry-resolve succeeding proves the clone flag was consumed; an internal panic elsewhere would leave it armed → red | YES — `stats()==before`, ledger `==charged`, retry resolve returns payload, close refunds 0 |
| ~2202 (test, outer unwind over panicking client drops) | `catch_unwind` around the test's own `panic!("outer unwind")` with `_owned`/`RetireDuringUnwind` dropping the service during unwind | YES — the closure body's only panics are the explicit `panic!`s; location is structural (not any-panic-anywhere) | YES — outer panic remains observable (`.is_err()`), `drops==3` for all three ops, ledger drained to 0 |
| ~2270 (test, mutex poisoning) | `let _ = catch_unwind(..)` — deliberate panic-swallow to poison the state mutex; the swallowed payload is intentionally discarded | YES — the closure locks `service.state` (`.expect("healthy lock")` proves pre-poison state) then panics; the poisoning EFFECT is asserted (this is the “asserted elsewhere” case for the `let _ =`) | YES — `stats`/`close` typed `Storage(_)` errors, ledger still fully charged, Drop refunds every charge to 0 |

Related out-of-file note: `shared/lifecycle.rs:22` (`retire_client_value`) and
lifecycle.rs:106/211 production containment are pinned by lifecycle.rs:366 and
prepared_publication.rs:2179 — not location-insensitive.

`let _ =` census in prepared_publication.rs: exactly 3 (:1924, :1974, :2270).
:1924/:1974 discard Results that can never materialize (the call panics; the panic
itself is asserted via `.is_err()`); :2270 is deliberate panic-swallowing whose effect
(poison) is asserted via typed errors + ledger + drop-refund. No A4 defect found.

## Footer — 3-line risk summary

1. The prepared-publication state machine is the best-defended area audited so far:
independent numeric oracles, byte-identical ledger/stats snapshots on every denial,
and panic sites pinned by construction or by transitive one-shot-flag consumption —
0 FAILOPEN, 0 VACUOUS in 5 023 lines of scoped SUT+tests.
2. All residual risk is concentrated in `tests/public_api_surface.rs`, where 5 of 20
rows are presence/cardinality/acceptance-only (`broker()` shape unlocked, preset
taxonomy by count, is_ok build, discarded reclaimer, zero-state store) — each has a
one-line-or-small fix and a concrete critic mutation above.
3. The single highest-value fix is `platform_builder_installs_reclaimer_…`: it is the
only scoped test whose named wiring can be silently deleted (A6) while staying green;
the reclaimer port is the ledger's eviction authority, so an unwired reclaimer is a
real fail-open in production configuration, not just a test gap.
