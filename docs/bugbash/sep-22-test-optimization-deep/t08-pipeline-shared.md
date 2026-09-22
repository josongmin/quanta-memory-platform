# T08 deep audit: publish_pipeline + shared + presets test scope

Scope: `crates/quanta-memory-platform-facade/src/publish_pipeline/` (`mod.rs`, `prepared_publication.rs`, `service.rs`),
`src/shared/` (`accounting.rs`, `builder.rs`, `lifecycle.rs`, `measure.rs`, `mod.rs`), `src/presets/mod.rs`.
Method: opened and read every test body listed below; search hits alone were not used as evidence.
Grep contract: `grep -rn -E '#\[(test|tokio::test)'` over the three scope dirs = **59** hits
(27 prepared_publication + 5 pipeline service + 9 measure + 7 lifecycle + 11 builder + 0 elsewhere, 0 `tokio::test`).
Census rows below = 59. Verdicts: KEEP / SHRINK / MERGE / DELETE.
The census links function declarations; findings may cite the immediately
preceding `#[test]` attribute when discussing harness-level structure.
Timing source: `docs/bugbash/sep-22-test-optimization-deep/timing/top40.md` (present; suite 639 green).
No in-scope `#[test]` fn appears in the top-40 slow list — the only in-scope timing entries are
`compile_fail` doc-tests (publish_pipeline mod.rs lines 25/35, prepared_publication.rs lines 304/311,
shared/builder.rs line 251/354, shared/accounting.rs line 82), each 0.2–0.5 s of doc-build time.

## Census (59 rows)

### publish_pipeline/service.rs — 5

| # | test fn (file:line) | verdict | rationale |
|---|---|---|---|
| 1 | `identity_pipeline_stages_then_commits_published` ([service.rs:162](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:162)) | MERGE | Same happy-path counter as #4 with weaker (single-step) oracle; fold into #4 |
| 2 | `failed_commit_never_publishes_and_never_increments_count` ([service.rs:177](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:177)) | KEEP | Only negative path (failing transition, count stays 0, then positive correction) |
| 3 | `default_commit_transition_applies_identity_with_no_missing_step` ([service.rs:211](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:211)) | MERGE | Unit-drives `DefaultCommitTransition` in isolation; #4 already covers it end-to-end |
| 4 | `default_pipeline_runs_stage_then_commit_each_exactly_once_no_rollback` ([service.rs:226](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:226)) | KEEP | Strongest happy path: per-step counts 0→1→2→3 over x/y/z; keeper for merge of #1/#3/#5 |
| 5 | `published_count_only_reflects_committed_not_staged` ([service.rs:247](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:247)) | MERGE | Same counter property as #1/#4 with staged-3/commit-1 constants; parametrize into #4 |

### publish_pipeline/prepared_publication.rs (unbudgeted) — 11

| # | test fn (file:line) | verdict | rationale |
|---|---|---|---|
| 6 | `prepare_capacity_failure_leaves_all_state_unchanged` ([prepared_publication.rs:1032](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1032)) | KEEP | Only pre-retention capacity denial with full state-unchanged oracle (stats + orphans) |
| 7 | `prepared_payload_is_only_reachable_by_exact_token` ([prepared_publication.rs:1041](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1041)) | KEEP | Only forged-token (`0xA5`) oracle; no budgeted mirror |
| 8 | `identical_prepare_replays_same_token_without_second_charge` ([prepared_publication.rs:1055](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1055)) | SHRINK | Budgeted #20 dominates on ledger; keep as disposition-only (Applied/Replayed + token eq) smoke for no-default-features builds |
| 9 | `identity_payload_drift_is_typed_conflict` ([prepared_publication.rs:1066](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1066)) | KEEP | Only conflict oracle unbudgeted; original-intact + charge asserts are load-bearing |
| 10 | `abort_releases_exact_payload_and_is_retry_safe` ([prepared_publication.rs:1076](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1076)) | SHRINK | Budgeted #20 covers abort+ledger; keep Aborted/Replayed disposition + AbortedToken smoke |
| 11 | `concurrent_identical_prepare_has_one_applied_winner` ([prepared_publication.rs:1100](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1100)) | KEEP | Only N=8 same-payload race (exactly-1-Applied); tune N, keep oracle |
| 12 | `concurrent_conflicting_prepare_has_one_winner` ([prepared_publication.rs:1128](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1128)) | KEEP | Only conflicting-payload race (1 ok + 1 IdentityConflict); distinct from #11 |
| 13 | `root_referenced_token_is_never_enumerated_or_collected_as_an_orphan` ([prepared_publication.rs:1155](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1155)) | KEEP | Only orphan-enumeration/GC-happy-path oracle unbudgeted |
| 14 | `orphan_gc_with_unknown_root_reference_is_batch_atomic` ([prepared_publication.rs:1174](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1174)) | KEEP | Only batch-atomicity oracle (both payloads intact + exact stats after UnknownToken) |
| 15 | `aborted_root_reference_fails_closed_without_collecting_other_entries` ([prepared_publication.rs:1197](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1197)) | KEEP | Only aborted-root-reference denial; mechanism differs from budgeted #23 (rebuild-denial) |
| 16 | `tombstone_limits_bound_n_cycle_churn_and_fail_closed_at_saturation` ([prepared_publication.rs:1220](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1220)) | KEEP | Only tombstone-saturation oracle; exact `TombstoneCapacity{requested_count, available_count, …}` fields — strongest negative oracle in scope |

### publish_pipeline/prepared_publication.rs (budgeted, `core-live-accounting`) — 16

| # | test fn (file:line) | verdict | rationale |
|---|---|---|---|
| 17 | `large_identity_with_zero_payload_is_denied_before_retention` ([prepared_publication.rs:1391](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1391)) | KEEP | Only pre-retention denial (101-byte identity vs 100-byte class cap); pairs with #22 (distinct denial layer) |
| 18 | `prepared_identity_index_and_tombstone_keep_exact_independent_cost` ([prepared_publication.rs:1405](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1405)) | KEEP | Only live→abort→close full lifecycle against the independent (`evidence_cost`/`live_cost`) oracle |
| 19 | `prepared_component_sum_overflow_is_fail_closed_before_reservation` ([prepared_publication.rs:1445](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1445)) | KEEP | Only u64::MAX component-sum overflow; distinct fake (`OverflowPayload`) |
| 20 | `resolve_clone_reentry_under_read_lock_is_typed_busy_and_preserves_charge` ([prepared_publication.rs:1469](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1469)) | KEEP | Only reentrancy-via-`Clone` probe (Weak-service + clone_hits==1); no duplicate |
| 21 | `replay_charges_once_and_abort_preserves_identity_evidence_once` ([prepared_publication.rs:1510](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1510)) | KEEP | Authoritative replay/abort ledger oracle (live→evidence→0 across close); keeper vs unbudgeted #8/#10 |
| 22 | `failed_abort_keeps_payload_index_and_charge_coherent` ([prepared_publication.rs:1535](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1535)) | KEEP | Only abort-denied-under-rebuild oracle with retry-to-success tail |
| 23 | `broker_capacity_denial_leaves_no_local_or_ledger_state` ([prepared_publication.rs:1555](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1555)) | KEEP | Broker-cap (4-byte) denial layer; pairs with #17 (accounting-layer denial) — keep both, one line apart in cost |
| 24 | `failed_orphan_gc_preserves_every_payload_index_and_charge` ([prepared_publication.rs:1568](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1568)) | KEEP | Only GC-denied-under-rebuild oracle with retry-to-2 + evidence-ledger tail |
| 25 | `close_refunds_all_entries_and_is_terminal_and_idempotent` ([prepared_publication.rs:1603](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1603)) | KEEP | Only terminal-close oracle (refund + UnknownToken + 3×Closed denials + double close) |
| 26 | `failed_close_keeps_exact_payloads_tokens_and_charges_for_retry` ([prepared_publication.rs:1643](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1643)) | KEEP | Only close-denied-under-rebuild oracle (stats-equality + retry) |
| 27 | `drop_refunds_live_entries_and_rebuild_drop_defers_exact_release` ([prepared_publication.rs:1666](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1666)) | KEEP | Only Drop-refund oracle (healthy refund + rebuild-deferred release) |
| 28 | `multiple_panicking_client_drops_during_outer_unwind_drain_exact_charges` ([prepared_publication.rs:1697](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1697)) | SHRINK | Real coverage (3 ops × full broker setup) but serial loop hides attribution; split into 3 fns sharing one helper |
| 29 | `mutation_gate_returns_typed_busy_and_exposes_no_partial_plan` ([prepared_publication.rs:1743](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1743)) | KEEP | Only full-gate oracle (5 ops Busy + reopen tail); distinct from lifecycle #35–#36 |
| 30 | `poisoned_store_reports_error_but_drop_still_releases_every_charge` ([prepared_publication.rs:1776](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1776)) | KEEP | Only poison-lock oracle with drop-refund tail |
| 31 | `checked_overflow_and_underflow_reject_before_broker_or_index_mutation` ([prepared_publication.rs:1795](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1795)) | KEEP | Only direct-state-corruption oracle (charged_bytes=MAX then 0); pairs with #32 on different field/op |
| 32 | `orphan_gc_rejects_corrupt_active_lease_cost_without_partial_refund` ([prepared_publication.rs:1840](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1840)) | KEEP | Only active_bytes-corruption oracle on the GC path; pairs with #31 |

### shared/measure.rs — 9

| # | test fn (file:line) | verdict | rationale |
|---|---|---|---|
| 33 | `retained_cost_v1_sums_each_component_exactly_once` ([measure.rs:209](crates/quanta-memory-platform-facade/src/shared/measure.rs:209)) | KEEP | Only exact-sum oracle (15, then 30 via checked_add); 2 asserts, cheapest in scope |
| 34 | `retained_cost_v1_rejects_component_and_cross_component_overflow` ([measure.rs:222](crates/quanta-memory-platform-facade/src/shared/measure.rs:222)) | KEEP | Only overflow oracle (component + cross-component + struct-literal bypass) |
| 35 | `measured_value_charges_caller_supplied_size_not_inner_len` ([measure.rs:240](crates/quanta-memory-platform-facade/src/shared/measure.rs:240)) | KEEP | Core caller-size contract (4096 vs 3-byte inner); keeper for #36 merge |
| 36 | `measured_value_into_inner_returns_wrapped_value` ([measure.rs:250](crates/quanta-memory-platform-facade/src/shared/measure.rs:250)) | MERGE | Single-assert accessor; fold into #35 as a trailing assert |
| 37 | `measured_value_zero_size_is_charged_verbatim` ([measure.rs:256](crates/quanta-memory-platform-facade/src/shared/measure.rs:256)) | KEEP | NEGATIVE-PAIR of #35 (zero stays zero, never infers 100); keep the pair |
| 38 | `arc_byte_size_delegates_to_inner` ([measure.rs:264](crates/quanta-memory-platform-facade/src/shared/measure.rs:264)) | MERGE | Single-assert `Arc<T>` delegation; fold into #41 as the typed-substrate case |
| 39 | `cloning_measured_arc_shares_the_allocation_not_a_deep_copy` ([measure.rs:270](crates/quanta-memory-platform-facade/src/shared/measure.rs:270)) | KEEP | Only `Arc::ptr_eq` + strong_count==3 no-deep-copy proof (scenario F dedup-hit) |
| 40 | `cloning_measured_value_preserves_size_and_value_context` ([measure.rs:288](crates/quanta-memory-platform-facade/src/shared/measure.rs:288)) | KEEP | Only clone-preserves-(value,size)-tuple proof; distinct from #39 (sharing vs context) |
| 41 | `measured_arc_any_handle_charges_caller_size` ([measure.rs:301](crates/quanta-memory-platform-facade/src/shared/measure.rs:301)) | KEEP | Only opaque-handle (`Arc<dyn Any>`) substrate; keeper for #38 merge |

### shared/lifecycle.rs — 7

| # | test fn (file:line) | verdict | rationale |
|---|---|---|---|
| 42 | `hostile_panic_payload_drop_cannot_abort_outer_unwind` ([lifecycle.rs:247](crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:247)) | SHRINK | Correct isolation but respawns the whole test binary in the default run; gate behind `#[ignore]` + CI job, keep child branch as-is |
| 43 | `client_callback_reentry_on_held_container_lock_is_typed_busy` ([lifecycle.rs:275](crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:275)) | KEEP | Only `try_lock_state` on an externally-held mutex; mechanism differs from #44 |
| 44 | `reentry_returns_typed_busy_without_blocking` ([lifecycle.rs:288](crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:288)) | KEEP | Only lane-level reentry oracle (mutation + readable + close all Busy, drop reopens) |
| 45 | `close_excludes_mutations_and_other_close_attempts` ([lifecycle.rs:311](crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:311)) | KEEP | Only Closing-phase exclusion oracle with rollback tail |
| 46 | `failed_close_reopens_for_mutation_and_retry` ([lifecycle.rs:332](crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:332)) | KEEP | Only failed-close (Drop-permit) reopen oracle with retry tail |
| 47 | `committed_close_stays_closed_and_double_close_is_idempotent` ([lifecycle.rs:345](crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:345)) | KEEP | Only committed-close terminal oracle (Closed denials + AlreadyClosed) |
| 48 | `drop_transition_is_terminal_and_reports_cleanup_once` ([lifecycle.rs:363](crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:363)) | KEEP | Only drop-transition oracle (CleanupRequired once, AlreadyClosed after) |

### shared/builder.rs (`#[cfg(all(test, feature = "full"))]`) — 11

| # | test fn (file:line) | verdict | rationale |
|---|---|---|---|
| 49 | `builder_first_happy_path_builds_a_usable_payload_store` ([builder.rs:757](crates/quanta-memory-platform-facade/src/shared/builder.rs:757)) | KEEP | Only payload happy path incl. `payload-0` ref pin |
| 50 | `payload_builder_without_binding_fails_with_build_error` ([builder.rs:775](crates/quanta-memory-platform-facade/src/shared/builder.rs:775)) | MERGE | 1 of 4 copy-pasted missing-binding shapes; collapse #50/#52/#53/#55 into one looping test |
| 51 | `retained_map_platform_builder_routes_bytes_through_the_ledger` ([builder.rs:785](crates/quanta-memory-platform-facade/src/shared/builder.rs:785)) | MERGE | 1 of 3 magic-ledger constants; table-drive #51/#54/#56 (+#57/#58 keepers stay exact) |
| 52 | `publish_pipeline_builder_without_binding_fails_with_build_error` ([builder.rs:798](crates/quanta-memory-platform-facade/src/shared/builder.rs:798)) | MERGE | Same as #50 (PublishPipelineBuilder variant) |
| 53 | `transcript_builder_without_binding_fails_with_build_error` ([builder.rs:806](crates/quanta-memory-platform-facade/src/shared/builder.rs:806)) | MERGE | Same as #50 (transcript variant) |
| 54 | `transcript_platform_builder_routes_bytes_through_the_ledger` ([builder.rs:815](crates/quanta-memory-platform-facade/src/shared/builder.rs:815)) | MERGE | Magic 236; table-drive with #51/#56 |
| 55 | `revocable_builder_without_binding_fails_with_build_error` ([builder.rs:839](crates/quanta-memory-platform-facade/src/shared/builder.rs:839)) | MERGE | Same as #50 (revocable variant) |
| 56 | `revocable_platform_builder_routes_bytes_through_the_ledger` ([builder.rs:848](crates/quanta-memory-platform-facade/src/shared/builder.rs:848)) | MERGE | Magic 49; table-drive with #51/#54 |
| 57 | `default_build_composes_charged_stores_without_manual_glue` ([builder.rs:864](crates/quanta-memory-platform-facade/src/shared/builder.rs:864)) | KEEP | Only charged-path + refund-to-zero (89 → 0) composition proof; keeper |
| 58 | `default_build_platform_sink_is_the_same_ledger_as_the_broker` ([builder.rs:886](crates/quanta-memory-platform-facade/src/shared/builder.rs:886)) | KEEP | Only shared-ledger proof (two stores sum to 117); keeper |
| 59 | `memory_platform_builder_builds_broker_from_preset` ([builder.rs:912](crates/quanta-memory-platform-facade/src/shared/builder.rs:912)) | SHRINK | Preset→broker build pin is worth one smoke; drop the `attach_runtime` tail (runtime attach is covered by core api tests) |

Zero-test files (inspected, no rows owed): `publish_pipeline/mod.rs` (trait + docs only),
`shared/accounting.rs` (no `#[cfg(test)]`; `Budgeted`/`Unbudgeted` exercised via store integration),
`shared/mod.rs` (re-exports), `presets/mod.rs` (four one-line delegations, covered via `builder.rs:912` + surface tests).

## Findings

### D01 (E/S) — pipeline service: 3 happy-path counter tests + 1 isolated transition unit
Bodies: [service.rs:161](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:161),
[service.rs:210](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:210),
[service.rs:225](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:225),
[service.rs:246](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:246).
Quoted oracle (weakest of the set):
`"only committed values count as published"`
Mechanism: #1/#4/#5 all assert `published_count()` after stage/commit with different constants
(0→1 once; 0→1→2→3 in a loop; stage-3/commit-1→1); #3 drives `DefaultCommitTransition::apply` directly,
which #4 already exercises end-to-end through `commit`.
Action: keep [service.rs:225](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:225)
(strongest per-step oracle) + [service.rs:176](crates/quanta-memory-platform-facade/src/publish_pipeline/service.rs:176)
(only negative path); merge #1/#3/#5 into #4 as parameters. Est saving: ~45 lines, zero oracle loss.

### D02 (E/M) — unbudgeted replay/abort/GC matrix duplicated by strictly-stronger budgeted mirrors
Bodies: [prepared_publication.rs:1054](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1054)
vs [prepared_publication.rs:1510](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1510);
[prepared_publication.rs:1075](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1075)
vs the same keeper.
Quoted oracle (budgeted keeper):
`"snapshot").total_owner_bytes(), evidence_cost("id")`
Mechanism: the budgeted mirrors assert everything the unbudgeted tests do (token eq, dispositions)
plus ledger transitions (live→evidence→0) the unbudgeted layer cannot see.
Action: keep budgeted [prepared_publication.rs:1510](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1510)
as authoritative; shrink unbudgeted #8/#10 to disposition-only smokes (needed for
no-`core-live-accounting` slice builds). Est saving: ~20 lines of stats asserts; keeps feature-gate coverage.

### D03 (B/S) — four copy-pasted missing-binding tests
Bodies: [builder.rs:775](crates/quanta-memory-platform-facade/src/shared/builder.rs:775),
[builder.rs:798](crates/quanta-memory-platform-facade/src/shared/builder.rs:798),
[builder.rs:806](crates/quanta-memory-platform-facade/src/shared/builder.rs:806),
[builder.rs:839](crates/quanta-memory-platform-facade/src/shared/builder.rs:839).
Quoted oracle (identical ×4):
`assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");`
Mechanism: same 6–8-line shape (`build().expect_err` + variant match), differing only in builder type.
Action: one test looping over four closures (or a macro) next to
[builder.rs:775](crates/quanta-memory-platform-facade/src/shared/builder.rs:775).
Est saving: ~25 lines, zero oracle loss.

### D04 (E/M) — five magic ledger constants break in five places on any cost-formula change
Bodies: [builder.rs:785](crates/quanta-memory-platform-facade/src/shared/builder.rs:785) (`105`),
[builder.rs:815](crates/quanta-memory-platform-facade/src/shared/builder.rs:815) (`236`),
[builder.rs:848](crates/quanta-memory-platform-facade/src/shared/builder.rs:848) (`49`),
[builder.rs:864](crates/quanta-memory-platform-facade/src/shared/builder.rs:864) (`89`),
[builder.rs:886](crates/quanta-memory-platform-facade/src/shared/builder.rs:886) (`117`).
Quoted oracle:
`assert_eq!(platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(), 105);`
Mechanism: each hand-derives one total in a comment (`value 64 + key 1 + slot 24 + state 16`)
with nothing tying it to `semantic_cost_v1`/`RetainedCostV1::total`.
Action: keep [builder.rs:864](crates/quanta-memory-platform-facade/src/shared/builder.rs:864) +
[builder.rs:886](crates/quanta-memory-platform-facade/src/shared/builder.rs:886) exact;
table-drive the per-store constants deriving expectations from the cost constructors.
Est saving: maintenance (one formula change touches one helper, not five tests).

### D05 (D/S) — single-assert `MeasuredValue` trivials deserve folding, not files
Bodies: [measure.rs:250](crates/quanta-memory-platform-facade/src/shared/measure.rs:250) (`into_inner`),
[measure.rs:264](crates/quanta-memory-platform-facade/src/shared/measure.rs:264) (`Arc` delegates).
Quoted oracle:
`assert_eq!(measured.into_inner(), "payload".to_string());`
Mechanism: one accessor assert each; adjacent tests (#35 caller-size, #41 opaque-handle) already
construct the same values and can carry the extra assert.
Action: fold #36 into [measure.rs:240](crates/quanta-memory-platform-facade/src/shared/measure.rs:240),
#38 into [measure.rs:301](crates/quanta-memory-platform-facade/src/shared/measure.rs:301)
(typed- vs opaque-substrate pair). Keep #37 (zero-size negative pair) separate.
Est saving: ~12 lines, 2 fewer test fns.

### D06 (C/M) — N=8 + Barrier concurrency tests dominate module wall time
Bodies: [prepared_publication.rs:1099](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1099)
(N=8 + `Barrier`), [prepared_publication.rs:1127](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1127)
(2 threads + 10k-yield spin).
Quoted oracle (the strong part to keep):
`receipts.iter().filter(|r| r.disposition() == PrepareDispositionV1::Applied).count(), 1`
Mechanism: each thread busy-spins `prepare_after_busy`
([prepared_publication.rs:1016](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1016),
up to 10 000 `yield_now` per call → 80k lock attempts worst-case) and N=8 is flake-prone;
neither fn appears in top40 (fast when uncontended) but they are the slowest in-module on contention.
Action: default N=8→3–4 (still proves single-winner); keep exact-count drain asserts;
run 8-wide under `--ignored`/nightly stress. Est saving: contention wall time, fewer flakes.

### D07 (C/M) — `prepare_after_busy` busy-spins up to 10 000 yields per call
Body: [prepared_publication.rs:1016](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1016).
Quoted oracle:
`for _ in 0..10_000 {`
Mechanism: `Busy => yield_now` retry loop used by both concurrency tests on every spawned thread;
under contention this is 80k lock attempts for one one-winner assertion, with a bare
`panic!("prepared publication mutation lane failed to reopen")` giving no attempt-count triage.
Action: bounded exponential backoff (or lane-reopen condvar in test builds); at minimum lower the
cap and panic with the attempt count. Est saving: spin time under contention; better triage.

### D08 (C/L) — hostile-drop test respawns the whole test binary in the default run
Body: [lifecycle.rs:247](crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:247).
Quoted oracle:
`.arg("shared::lifecycle::tests::hostile_panic_payload_drop_cannot_abort_outer_unwind")`
Mechanism: `current_exe --exact … --nocapture` + `output()` re-runs the binary as a child;
correct isolation (destructor panic during unwind would abort the runner) but pays a full
process spawn, and the oracle is only "child exited 0" with no state check.
Action: `#[ignore]` for default runs with a CI/nightly job; assert on the child stderr marker too.
Est saving: one process spawn per default `cargo test`.

### D09 (C/M) — 3-op unwind loop triples the heaviest accounting setup and hides attribution
Body: [prepared_publication.rs:1697](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1697).
Quoted oracle:
`for operation in ["drop", "close", "gc"] {`
Mechanism: each iteration builds `broker(1_000_000)` + 3 prepares + `catch_unwind` + ledger drains;
failure messages include `{operation}` but the three ops serialize inside one test (no parallelism).
Action: split into three `#[test]` fns sharing one helper taking `operation`, called from
[prepared_publication.rs:1697](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1697)'s body.
Est saving: parallelizable; clearer failure attribution; same coverage.

### D10 (D/M) — corrupt-state pair (#31/#32) is justified duplication — document, don't merge
Bodies: [prepared_publication.rs:1795](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1795)
(`charged_bytes` corruption → prepare/abort path),
[prepared_publication.rs:1840](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1840)
(`active_bytes` corruption → GC path).
Quoted oracle:
`Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(_)))`
Mechanism: same error variant but different corrupted field and different rejecting op;
merging would lose one (field × op) cell.
Action: keep both; add a one-line comment on each naming its cell
(`charged_bytes×abort` vs `active_bytes×gc`). Est saving: none (clarity only).

### D11 (A/S) — `compile_fail` doc examples are untracked slow-ish build cost in scope
Bodies: `publish_pipeline/mod.rs` lines 25/35, `prepared_publication.rs` lines 304/311,
`shared/builder.rs` lines 251/354, `shared/accounting.rs` line 82 (all ` ```compile_fail `).
Per top40.md: each costs 0.2–0.5 s of doc-build time (ranks 20–29, 32, 37–38).
Mechanism: doc-tests are outside the `#[test]` census but each `compile_fail` sample pays a
full negative rustc invocation on every workspace doc run.
Action: no deletion (they pin the negative API contract); just note the cost so a future
doc-cull knows these 8 samples cost ~2.5 s aggregate. Est saving: none now; ~2.5 s if ever culled.

## V1 re-verification (04-facade-pipeline-spill-shared.md; 01/02/03/05 cross-check)

- F04 (pipeline service 5-tests-one-counter): CONFIRMED — #1/#3/#4/#5 all assert `published_count()`
  with different constants; #3 isolated-transition unit covered end-to-end by #4 (D01 above).
- F05 (budgeted mirrors dominate unbudgeted): CONFIRMED with nuance — #21 dominates #8/#10 on ledger,
  but unbudgeted smokes must stay for no-`core-live-accounting` slice builds (D02).
- F06 (builder magic ledger numbers): CONFIRMED — 105/236/49/89/117 hand-derived, five break sites (D04).
- F07 (surface-vs-builder charged-path dup): CONFIRMED in the unit direction (#51/#54/#56 vs
  `public_api_surface.rs:521,605,636`); integration file is out of scope for this track, keeper
  split stands (keep surface wiring pins + one charged-path per slice there; table-drive here).
- F08 (MeasuredValue trivials in two files): CONFIRMED for the in-scope half — #36/#38 fold into
  #35/#41 (D05); surface-file half is out of scope, noted for its track.
- F09 (4× missing-binding): CONFIRMED verbatim — identical `matches!(err, MemoryError::Build(_))` ×4 (D03).
- F12 (thread-spawning concurrency cost): CONFIRMED — #11 (N=8+Barrier) + #12 over `prepare_after_busy`
  10k-spin; tune N, keep exact-count oracles (D06).
- F13 (10k busy-spin): CONFIRMED at
  [prepared_publication.rs:1016](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1016) (D07).
- F14 (subprocess hostile-drop): CONFIRMED at
  [lifecycle.rs:247](crates/quanta-memory-platform-facade/src/shared/lifecycle.rs:247) (D08).
- F15 (3-op unwind loop): CONFIRMED at
  [prepared_publication.rs:1697](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1697) (D09).
- F20 (preset/trivial-measure presence checks): CONFIRMED for the in-scope half (D05 + zero-test
  `presets/mod.rs` verified: four one-line delegations, nothing to shrink).
- F21 (magic ledger constants sans derivation): CONFIRMED — same evidence as F06 (D04).
- F01/F02/F03/F10/F11/F17 (spill), F16/F22 (bench), F18/F19 (surface-text greps): OUT OF SCOPE
  for this track (no spill/bench/surface-test files in scope); no verdict offered.
- Non-findings re-checked: independent `evidence_cost`/`live_cost` oracles
  ([prepared_publication.rs:1001](crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs:1001)–`:1014`)
  intentionally duplicate the formula — CONFIRMED do-not-merge; `shared/accounting.rs` zero unit tests
  with branch coverage via integration — CONFIRMED no action.
- 01/02/03/05 files: no in-scope findings (01:54 preset loop is core-policy presets, not facade
  `presets/mod.rs`; 02 builder findings are core `api/builder.rs`; 05 lifecycle suites are testkit
  `suites.rs`). Nothing to confirm/refute.

## Lens summary

- A cheap-code: helpers (`binding()`, `service()`, `broker()`, `charged()`, `platform()`) healthy;
  only D11 (doc-sample build cost) noted. No production-code change proposed.
- B cheap-test: D03 (4× missing-binding → 1 loop, ~25 lines), D05 (~12 lines, 2 fns).
- C slow-measured: top40 shows zero in-scope `#[test]` in the slow list; D06/D07/D08/D09 are
  contention/process costs invisible to report-time medians — tune, don't delete.
- D weak-oracle: strongest oracles are #16 (exact TombstoneCapacity fields) and #18/#21 (independent
  formula + ledger); weakest are #36/#38 (single-assert trivials, D05) and #42's exit-0-only child
  check (D08). #59's `attach_runtime` tail exceeds its pin (SHRINK).
- E duplicates: D01 (counter ×3+1), D02 (replay/abort ×2 layers), D03 (binding ×4), D04 (ledger ×5),
  justified pairs documented (#35/#37 negative-pair, #17/#23 denial layers, #31/#32 corrupt cells).
