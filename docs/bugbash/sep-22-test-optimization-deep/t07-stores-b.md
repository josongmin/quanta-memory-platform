# t07-stores-b — revocable_store + transcript_store deep audit

Scope: `crates/quanta-memory-platform-facade/src/revocable_store/service.rs`
(2506 lines, test module lines 1040–2506) and
`crates/quanta-memory-platform-facade/src/transcript_store/service.rs`
(3041 lines, test modules lines 1201–3041).
Every `#[test]` body opened and read. No fix code — audit only.

Grep census: `grep -c '#\[test\]\|#\[tokio::test\]'` →
revocable_store/service.rs: **46**, transcript_store/service.rs: **47**,
scope total **93**. Census rows below: 46 + 47 = 93. Verdicts:
KEEP = unique oracle, SHRINK = keep test, cut body/cost,
MERGE = fold into keeper, DELETE = remove outright (none — every test
pins at least one live path; cheapest singles merge instead).

Lenses: **A** cheap code · **B** cheap tests · **C** slow/measured ·
**D** weak-oracle quality · **E** duplicates (keeper pick).
Timing: `timing/top40.md` present and used; per-test times from
`timing/full.txt` cited as measured.

## 1. Census — revocable_store/service.rs (46 rows)

| file:line | test fn | verdict | rationale / keeper |
|---|---|---|---|
| revocable_store/service.rs:1143 | insert_tracks_reclaimable_bytes | KEEP | only exact `committed_bytes` + `StoreStats` pin on insert path |
| revocable_store/service.rs:1156 | reclaim_one_is_fifo_and_releases_its_bytes | KEEP | FIFO-order keeper (order + store/ledger byte release); absorbs 2171, 2482 |
| revocable_store/service.rs:1176 | revocable_full_drain_uses_fifo_deque_without_front_shifts | SHRINK | source-grep oracle (see F15); replace with behavioral drain-bound test |
| revocable_store/service.rs:1191 | reclaim_to_in_memory_tier_keeps_bytes_charged | KEEP | unique spill→restore→FIFO-order round trip with retained charge |
| revocable_store/service.rs:1226 | many_spills_share_large_binding_identity_without_unbilled_clones | SHRINK | 128 cycles × 64 KB strings; `ptr_eq` proves sharing, not string length |
| revocable_store/service.rs:1296 | spill_and_close_emit_bounded_facade_events | KEEP | only event-emission oracle in file |
| revocable_store/service.rs:1321 | failed_destination_write_preserves_source_entry_and_charge | KEEP | F19 keeper: real `SpillTarget` impl + `spilled_count == 0` |
| revocable_store/service.rs:1355 | noop_spill_does_not_discard_source | MERGE | same postimage as 1321; fold as canned-controller leg → keeper 1321 |
| revocable_store/service.rs:1371 | foreign_store_cannot_adopt_spill_charge | KEEP | unique cross-broker charge authority proof |
| revocable_store/service.rs:1392 | source_close_does_not_refund_live_in_process_tier | KEEP | unique tier-authority proof (close source, charge survives, drop retires) |
| revocable_store/service.rs:1408 | durable_spill_releases_source_only_after_commit | KEEP | only test asserting charge inside the target pre-commit (1426–1430) |
| revocable_store/service.rs:1496 | failed_durable_write_and_encoder_error_restore_source_fifo_and_charge | KEEP | two error legs (write-fail + encoder-error) already tabled in one body |
| revocable_store/service.rs:1563 | panicking_durable_encoder_restores_source_before_unwind | KEEP | panic (unwind) leg ≠ 1496 error legs; distinct restoration mechanism |
| revocable_store/service.rs:1620 | durable_redb_round_trip_transfers_charge_exactly_once | SHRINK | measured 2.133 s (top40 #16); keep as the single round-trip keeper, trim encode ceremony |
| revocable_store/service.rs:1670 | durable_redb_committed_restore_recovers_after_process_restart | MERGE | differs from 1723 only in pre-crash phase; parametrize → keeper 1723 |
| revocable_store/service.rs:1723 | durable_redb_prepared_restore_recovers_after_process_restart | KEEP | merged recovery-test keeper (covers prepared+committed crash points) |
| revocable_store/service.rs:1772 | denied_durable_restore_leaves_redb_record_authoritative | KEEP | unique quota-denied restore with record retained (measured 2.383 s, top40 #8) |
| revocable_store/service.rs:1801 | failed_durable_delete_rolls_back_provisional_source_charge | KEEP | unique commit-failure rollback (charge → 0, count stays 1) |
| revocable_store/service.rs:1871 | ambiguous_durable_delete_fences_source_until_absence_is_reconciled | KEEP | unique ambiguity→fence→reconcile→publish flow |
| revocable_store/service.rs:1985 | panicking_finalize_fences_published_source_until_reconciliation | KEEP | unique finalize-panic fencing + double-panic retry; measured <0.025 s |
| revocable_store/service.rs:2110 | reclaim_one_on_empty_returns_none | MERGE | identical None as 2180 minus postimage → keeper 2180 |
| revocable_store/service.rs:2117 | reclaim_until_frees_at_least_target_and_reports_reclaimed_bytes | MERGE | seed-size variant of 2139 → keeper 2139 |
| revocable_store/service.rs:2131 | reclaim_until_on_empty_store_reports_zero | MERGE | empty leg of reclaim_until table → keeper 2139 (with 2180 postimage) |
| revocable_store/service.rs:2139 | reclaim_until_charges_and_evicts_zero_payload_keys | KEEP | F16 keeper: multi-evict + zero-payload key cost + FIFO survivor |
| revocable_store/service.rs:2157 | reclaim_until_of_zero_payload_entries_reclaims_their_key_and_index_cost | MERGE | all-zero seed variant → keeper 2139 |
| revocable_store/service.rs:2170 | revoke_makes_entry_unretrievable_and_decrements_accounting | MERGE | single-entry insert/reclaim/None shape → keeper 1157 |
| revocable_store/service.rs:2180 | empty_reclaim_leaves_state_byte_for_byte_unchanged | KEEP | F16 empty-keeper: None + stats-unchanged postimage |
| revocable_store/service.rs:2188 | insert_over_cap_is_denied_and_fails_closed | KEEP | admission-path denial (tiny broker cap); distinct from cost-arithmetic denials |
| revocable_store/service.rs:2198 | large_key_with_zero_value_is_charged_and_denied_without_state_change | MERGE | same postimage as 2209; fold as huge-key case → keeper 2209 |
| revocable_store/service.rs:2208 | cost_component_sum_overflow_is_fail_closed_before_charge | KEEP | strictest overflow probe (`u64::MAX`); absorbs 2198 |
| revocable_store/service.rs:2229 | insert_charges_and_reclaim_refunds_the_ledger | KEEP | ledger charge/refund keeper (reclaim_one + reclaim_until drain to 0) |
| revocable_store/service.rs:2248 | close_refunds_every_live_entry_exactly_once_and_stays_closed | KEEP | close + idempotence + terminal Closed denials in one body |
| revocable_store/service.rs:2266 | close_release_failure_preserves_entries_fifo_and_ledger_authority | KEEP | unique close-rollback path with FIFO proof after retry |
| revocable_store/service.rs:2286 | reclaim_batch_failure_preserves_every_entry_and_charge | MERGE | F17: covered by reclaim_until rebuild leg → keeper 2318 |
| revocable_store/service.rs:2303 | reclaim_one_during_rebuild_is_fail_closed | MERGE | F17: parametrize (one/until) → keeper 2318 |
| revocable_store/service.rs:2318 | reclaim_until_during_rebuild_is_fail_closed | KEEP | F17 keeper: rebuild-denial parametrized over reclaim_one/until |
| revocable_store/service.rs:2334 | drop_without_explicit_close_releases_capacity | KEEP | unique implicit-drop refund proof |
| revocable_store/service.rs:2348 | drop_during_rebuild_defers_exact_release_until_finish | KEEP | unique deferred-release (conservative → 0 across fence) |
| revocable_store/service.rs:2363 | drop_during_outer_unwind_contains_two_client_panics_and_refunds_all_charges | KEEP | unique in-process double-panic unwind containment via catch_unwind |
| revocable_store/service.rs:2395 | concurrent_insert_and_close_have_one_linearized_outcome | KEEP | only concurrency test in file; terminal drain-to-0 postimage |
| revocable_store/service.rs:2428 | counter_overflow_is_rejected_before_broker_charge_or_mutation | KEEP | unique `committed_bytes = u64::MAX` probe; but see G6 (state poke) |
| revocable_store/service.rs:2440 | poisoned_stats_is_error_not_empty_and_drop_still_releases | KEEP | unique poison + drop-refund combination |
| revocable_store/service.rs:2458 | reentrant_read_and_close_under_state_lock_return_typed_busy | KEEP | unique Busy reentry pair with open postimage + retry close |
| revocable_store/service.rs:2475 | constructed_budgeted_store_exposes_its_single_broker_authority | MERGE | single `is_some` assert → new `construction_modes` table with 2496 |
| revocable_store/service.rs:2482 | reclaim_eviction_order_is_deterministic_fifo_at_capacity_boundary | MERGE | 3-key FIFO order leg → keeper 1157 |
| revocable_store/service.rs:2496 | unbudgeted_store_is_a_distinct_explicit_product_type | MERGE | trivial constructor round-trip → new `construction_modes` table with 2475 |

Revocable totals: KEEP 30, SHRINK 3, MERGE 13, DELETE 0 = 46.

## 2. Census — transcript_store/service.rs (47 rows)

| file:line | test fn | verdict | rationale / keeper |
|---|---|---|---|
| transcript_store/service.rs:1229 | reentrant_load_stats_and_close_under_read_lock_are_typed_busy | KEEP | only read-lock Busy triple (unbudgeted); retry-close postimage |
| transcript_store/service.rs:1242 | client_message_clone_during_load_reenters_as_typed_busy_without_ledger_drift | KEEP | unique Clone-reentry-during-load with ledger-drift pin |
| transcript_store/service.rs:1357 | exact_request_retried_n_times_applies_once | KEEP | 8-retry Applied-once/Replayed-7 count + byte oracle |
| transcript_store/service.rs:1374 | append_truncate_cycles_cannot_grow_past_fail_closed_evidence_bound | KEEP | unique append/truncate-cycle saturation with stats-unchanged pin |
| transcript_store/service.rs:1397 | exact_retry_and_conflicts_remain_deterministic_at_saturation | KEEP | saturation determinism triple (replay ok + 2 conflict axes) |
| transcript_store/service.rs:1418 | tombstone_byte_limit_counts_large_message_ids | KEEP | unique byte-cap saturation with exact evidence-bytes match |
| transcript_store/service.rs:1440 | retention_reconfigure_below_live_evidence_is_rejected | KEEP | unique reconfigure path (reject + exact-live accept + saturation) |
| transcript_store/service.rs:1471 | retention_stats_separate_active_evidence_and_session_costs | KEEP | keep, strengthen oracle (see F25 re-verify: `> 0` → exact `cost()`) |
| transcript_store/service.rs:1495 | same_identity_with_body_drift_is_payload_conflict | MERGE | F21: axis covered with index/ledger postimage → keeper 2741 |
| transcript_store/service.rs:1504 | reused_sequence_with_different_message_id_is_typed_conflict | MERGE | F21 → keeper 2741 |
| transcript_store/service.rs:1512 | reused_message_id_at_different_sequence_is_typed_conflict | MERGE | F21 → keeper 2741 |
| transcript_store/service.rs:1520 | append_identity_is_partitioned_by_session | KEEP | only cross-session partitioning proof |
| transcript_store/service.rs:1535 | concurrent_identical_appends_have_one_applied_winner | KEEP | 8-thread barrier race; measured <0.001 s (full.txt:65) |
| transcript_store/service.rs:1567 | concurrent_conflicting_appends_have_one_applied_winner | KEEP | race conflict-winner oracle (Applied × 1 + PayloadConflict × 1) ≠ 1535 |
| transcript_store/service.rs:1611 | truncate_retains_tombstone_so_late_retry_cannot_reappend | KEEP | tombstone replay + drift-conflict in one body |
| transcript_store/service.rs:1629 | truncate_to_zero_retains_all_identity_tombstones | KEEP | full-truncate evidence-cost pin + double replay |
| transcript_store/service.rs:1652 | zero_body_with_large_ids_and_session_retains_semantic_evidence_cost | MERGE | F22: uncharged variant; charged keeper 2413 adds stats+ledger per step |
| transcript_store/service.rs:1673 | selective_retain_preserves_sparse_rows_and_evicted_identity_tombstones | KEEP | unique sparse-retain + evicted-replay + exact cost triple |
| transcript_store/service.rs:1695 | selective_retain_rejects_invalid_selector_without_mutation | KEEP | two invalid selectors (dup + out-of-range) already tabled, bytes-unchanged |
| transcript_store/service.rs:1714 | versioned_snapshot_retain_rejects_concurrent_append_without_dropping_rows | KEEP | unique stale-snapshot → retain → applied flow with row-sequence pins |
| transcript_store/service.rs:1756 | checked_load_reports_poisoned_storage_instead_of_empty_success | KEEP | only poison test in unbudgeted mod (load + stats) |
| transcript_store/service.rs:1769 | ordered_load_sorts_by_caller_sequence_and_allows_declared_gaps | KEEP | keep, add third row (see F25 re-verify) |
| transcript_store/service.rs:2015 | append_message_id_clone_and_order_reentry_is_typed_busy | KEEP | unique Clone+Ord reentry counter (`>= 4`) with ledger postimage |
| transcript_store/service.rs:2093 | panicking_message_id_clone_at_each_publication_point_leaves_append_atomic | KEEP | already parametrized over 2 panic points + shared state-postimage helper |
| transcript_store/service.rs:2133 | panicking_message_id_clone_cannot_publish_an_empty_new_session | KEEP | unique empty-session atomicity (sessions map empty) + fresh retry |
| transcript_store/service.rs:2180 | panicking_new_session_order_after_charge_preserves_old_session_and_ledger | KEEP | unique Ord-panic after charge with old-session readability pin |
| transcript_store/service.rs:2230 | panicking_message_id_order_at_publication_leaves_append_atomic | KEEP | unique Ord-panic + retry-Applied + comparison-count oracle (2280–2284) |
| transcript_store/service.rs:2379 | semantic_identity_component_overflow_denies_before_ledger_or_index_mutation | KEEP | `u64::MAX` × 2 axes (session, message) already tabled in one body |
| transcript_store/service.rs:2413 | zero_body_large_identity_and_session_are_charged_until_evidence_retirement | KEEP | F22 keeper: session/stats/ledger pins at every step |
| transcript_store/service.rs:2440 | retain_rollback_and_commit_do_not_compare_client_session_id_after_detach | KEEP | unique post-detach no-compare proof (trap counts `before + 1` both legs) |
| transcript_store/service.rs:2498 | panicking_session_order_before_detach_preserves_rows_and_ledger | KEEP | pre-detach panic atomicity; phase-distinct from 2440 (see G5) |
| transcript_store/service.rs:2551 | drop_contains_multiple_client_destructor_panics_during_outer_unwind | KEEP | child-process isolation justified (multi-panic aborts parent runner); measured 0.052 s (full.txt:38), file's slowest non-IO test |
| transcript_store/service.rs:2608 | retain_retires_multiple_panicking_client_bodies_after_ledger_postimage | KEEP | unique retain-with-panicking-bodies + close-survivor drain |
| transcript_store/service.rs:2667 | quota_denial_leaves_row_indexes_and_charge_unchanged | KEEP | quota denial + no-residue retry-Applied proof |
| transcript_store/service.rs:2685 | retention_saturation_denies_before_any_new_ledger_charge | KEEP | saturation with stats+ledger snapshot-equality postimage |
| transcript_store/service.rs:2703 | retention_saturation_emits_one_bounded_tombstone_event | KEEP | only event-emission oracle in transcript (exact single-tombstone vec) |
| transcript_store/service.rs:2741 | every_conflict_axis_preserves_rows_indexes_and_accounting | KEEP | F21 keeper: all 3 axes + rows/indexes/ledger preservation |
| transcript_store/service.rs:2767 | exact_retry_charges_once_and_truncate_refunds_only_active_body | KEEP | charge-once + partial-refund + replay-ledger triple |
| transcript_store/service.rs:2788 | truncate_refund_failure_preserves_rows_indexes_and_accounting | KEEP | unique truncate-fence rollback (InvalidState + full postimage) |
| transcript_store/service.rs:2804 | close_refunds_all_sessions_exactly_once_and_exposes_terminal_empty_reads | KEEP | close keeper: multi-session + double-close + 3 terminal Closed denials |
| transcript_store/service.rs:2839 | failed_close_preserves_rows_versions_tombstones_and_all_lease_authority | KEEP | unique close-fence rollback with version pin + replay readability |
| transcript_store/service.rs:2868 | drop_releases_all_live_rows_even_after_storage_poison | KEEP | unique poisoned-close + drop-refund combination (charged mod) |
| transcript_store/service.rs:2890 | drop_during_rebuild_defers_exact_release_until_finish | KEEP | deferred-release; different store than revocable 2348, keep |
| transcript_store/service.rs:2906 | mutation_lane_rejects_reentry_and_reads_without_exposing_partial_images | KEEP | unique lane-permit reentry (append/close/load/stats all Busy, image stable) |
| transcript_store/service.rs:2930 | checked_byte_and_version_overflow_denials_leave_ledger_and_indexes_intact | KEEP | two overflow axes (bytes, version) already tabled in one body |
| transcript_store/service.rs:2960 | failed_multirow_retain_preserves_positions_tombstones_and_version | KEEP | unique conditional-retain fence + retry-Applied + evicted-replay cost |
| transcript_store/service.rs:3007 | racing_close_and_append_have_one_linearizable_terminal_image | SHRINK | 16 iterations → 4; oracle is iteration-count independent (see F23) |

Transcript totals: KEEP 42, SHRINK 1, MERGE 4, DELETE 0 = 47.

## 3. New findings (all bodies opened; line = `#[test]` attribute line)

### G1 (E/M) — revocable FIFO-order coverage exists four times; two keepers suffice
`revocable_store/service.rs:2170` (`revoke_makes_entry…`): oracle
`assert_eq!((key.as_str(), value.len()), ("only", 9));`
is a single-entry insert/reclaim/None shape already proven by 1157
(`assert_eq!((key.as_str(), value.len()), ("first", 4));` at 1167) and the
empty-None leg already proven by 2180. `…:2482`
(`reclaim_eviction_order…`) asserts oldest/middle/newest order via
`reclaim_until(1)` — a third spelling of the 1157 FIFO pin.
Mechanism: same FIFO queue + same ledger release, differing only in seed width.
Action: fold 2170 and 2482 as seed legs into keeper 1157 (order pin) and keep
ledger-drain coverage in keeper 2229. Est: −2 tests, ~30 lines, no wall-clock
(all <0.001 s class) — maintenance win.

### G2 (E/S) — revocable denial trio shares one postimage; huge-key is a case, not a test
`revocable_store/service.rs:2198` (`large_key_with_zero_value…`): oracle
`assert_eq!(store.stats().expect("unchanged state"), StoreStats::default());`
is byte-identical in shape to 2208 (`cost_component_sum_overflow…`, same two
asserts at 2225–2226) and 2188 (same at 2194–2195). Mechanism: all three deny
before mutation with zero ledger delta; 2198 (101-char key vs 100-cap) vs 2208
(`u64::MAX` byte_size) differ only in which cost summand overflows.
Action: keep 2188 (admission path: broker cap) and 2209-strictest keeper 2208,
fold 2198 as a second seed case inside 2208. Est: −1 test, ~10 lines.

### G3 (C/L, measured) — redb four are ~9.5 s of suite wall-clock
`revocable_store/service.rs:1670`: body opens two redb backings on temp files
(`RedbSpillBacking::create(&path)` at 1682 and 1698, crash-recovery
simulation). Measured (timing/top40.md): #5 `…committed_restore…` 2.747 s,
#6 `…prepared_restore…` 2.654 s, #8 `…denied_durable_restore…` 2.383 s,
#16 `…round_trip…` 2.133 s — ≈9.9 s combined, the entire file's slow tail
(everything else in scope is <0.06 s per timing/full.txt).
Mechanism: real filesystem + embedded-DB open ×2 per test inside unit tests.
Action: keep round-trip keeper 1620, merge 1670 into keeper 1723 parametrized
over pre-crash phase (prepared vs committed), keep denial keeper 1772; run the
three redb survivors in the nightly/ignored IO lane. Est: −1 test and ≈2.7 s
suite wall-clock (the merged recovery leg), plus lane-gating for the rest.

### G4 (B/S) — constructor single-assert pair merges to one table test
`revocable_store/service.rs:2475`: whole body is
`assert!(store.broker().is_some());`
(2479). `…:2496` (`unbudgeted_store…`) is insert/assert-stats/close with no
ledger or denial angle. Mechanism: both restate constructor shape, zero
behavioral branches. Action: merge both into one `construction_modes` test
(budgeted authority + unbudgeted estimate + empty-close). Est: −1 test, ~10 lines.

### G5 (E/S, considered-not-duplicate) — transcript trap pair is phase-distinct, keep both
`transcript_store/service.rs:2440` (`retain_rollback_and_commit…`) vs `…:2498`
(`panicking_session_order_before_detach…`): both use `ComparisonTrap` +
`OrderedSessionId` on the retain path, but 2440 proves post-detach no-compare
(`trap.calls` exactly `before + 1` on fenced-rollback AND committed legs,
2475/2487) while 2498 proves pre-detach panic atomicity (rows intact, charges
active, 2531–2546). Mechanism differs (fence-rollback vs unwind); trap-count
oracle in 2440 has no counterpart in 2498. Action: keep both; no saving.
Recorded so a future dedup pass does not merge them.

### G6 (D/S) — revocable overflow probe reaches into lock internals
`revocable_store/service.rs:2428` (`counter_overflow…`): oracle setup is
`store.state.lock().expect("state").committed_bytes = u64::MAX;`
(2432) — direct mutation of production `state` behind the lock, same
white-box shape as v1 F24's `assert_seed_only_after_append_panic` reaching
into `store.state` (transcript 2060–2086). Mechanism: test depends on the
`state` field name and `committed_bytes` layout; a lawful refactor (e.g.
checked-update constructor) breaks the test without breaking behavior.
Action: keep the test (unique `u64::MAX` committed-bytes probe), add a
`#[cfg(test)]`-visible constructor or `set_committed_bytes_for_tests` helper
at the production `state` declaration and call it from keeper 2428.
Est: 0 tests removed; −1 white-box coupling.

### G7 (A/S) — shared durable-mock boilerplate across six tests (~300 lines)
`revocable_store/service.rs:1408` (`durable_spill_releases…`): mock occupies
1410–1469 (~60 lines of `SpillTarget`/`ReconciledDurableSpillTarget` impls)
before the 20-line behavior at 1471–1492. Same shape recurs at 1498–1535,
1565–1600, 1803–1854, 1873–1953 (~80 lines), 1987–2055 (~69 lines).
Mechanism: each test hand-rolls an identical-trait mock differing in one
method (accept/commit/finalize behavior). Action: extract a shared
`RejectingDurableTarget`/`ScriptedDurableTarget` fixture in the test module;
per-test keeps only its behavioral asserts (keepers: each test's own lines).
Est: 0 tests removed, ≈−150 lines of mock boilerplate (confirms v1 F19 adjacent note).

## 4. v1 re-verification (scope findings F15–F26, 03-facade-stores.md)

| v1 | verdict | evidence (opened) |
|---|---|---|
| F15 source-grep revocable 1176–1187 | CONFIRM | `source.contains("entries: VecDeque<RevocableEntry")` (1179), `["entries", ".remove(0)"].concat()` (1182); a `remove(1)`/drain-reinsert regression passes the grep. Action: SHRINK to behavioral drain-bound test. |
| F16 reclaim_until 4-pack + empty pair | CONFIRM | 2110–2115, 2117–2129, 2131–2137, 2139–2155, 2157–2168, 2180–2186 all as described; keepers 2139 + 2180. Action: MERGE 4 tests. |
| F17 rebuild-denial trio 2286–2332 | CONFIRM | 2286–2301 (until+finish), 2303–2316 (one), 2318–2332 (until): identical seed→`begin_rebuild`→`InvalidState`→unchanged→`finish_rebuild` shape. Action: MERGE into keeper 2318 parametrized over one/until. |
| F18 redb + 128-spill slow tail | CONFIRM + measured | redb: `qmp-revocable-*-PID-SEQ.redb` temp files, double `RedbSpillBacking::create`, all `#[cfg(feature = "spill-redb")]` (gate already done, as v1 noted); top40 #5/#6/#8/#16 = 2.747/2.654/2.383/2.133 s. 128-spill 1226–1293: 64 KB strings at 1229–1230, per-cycle ledger snapshots at 1255–1258. Action: SHRINK strings to 1 KB + cycles to 8; MERGE 1670→1723. |
| F19 spill fail-closed pair 1321 vs 1355 | CONFIRM | same arrange (2 inserts, snapshot total), same postimage (count + bytes + ledger + FIFO head at 1347–1351 vs 1365–1367); keeper 1321 asserts `spilled_count == 0` + real impl. Action: MERGE 1355→1321. |
| F20 single-assert constructor tests | CONFIRM (nuance) | 2475–2480 is one `is_some` assert; 2496–2505 has insert/stats/close but no ledger/denial angle; 2131 folded per F16. Action: MERGE 2475+2496 into `construction_modes`. |
| F21 transcript conflict trio vs 2741 | CONFIRM | 1495–1502 (PayloadConflict + body pin), 1504–1510, 1512–1518 are single-axis, no index/ledger postimage; keeper 2741 fires all three (2747–2751) + rows/bytes/ledger pins (2761–2764). Action: MERGE trio → 2741 (−3 tests). |
| F22 zero-body large-identity pair | CONFIRM | 1652–1671 (TestUnbudgeted, session/retained pins) vs 2413–2438 (Budgeted, adds stats+ledger per step at 2421–2431). Same 512/1024 seeds. Action: MERGE 1652→2413. |
| F23 16-iteration race 3007–3039 | CONFIRM (tempered) | `for _ in 0..16` at 3009, per-iteration broker+store build at 3010–3011, thread spawn at 3015–3018. Measured only 0.003 s (full.txt:52) — cheap today, but 16× full-broker builds is CI-flake surface. Action: SHRINK to 4 iterations. |
| F24 callback-trap scaffolding ~150 lines | CONFIRM | `ComparisonTrap`/`OrderedSessionId` 1789–1851, `CallbackTrap`/`CallbackMessageId`/`callback_id`/`callback_store` 1853–1956 (~170 lines before first test at 2015); `assert_seed_only_after_append_panic` 2056–2091 reaches into `store.state` (2060) re-asserting version/rows/evidence the public triple covers. Action: trim helper to ledger assertion. |
| F25 weak oracles (inequalities, 2-row sort) | CONFIRM | 1471–1493: `active_bytes > 0`, `evidence_bytes > 0`, `session_key_bytes > 0` (1478–1480) instead of exact `cost()`; 1769–1776: two rows (7, 2) at 1772–1775. Action: exact `cost()` asserts in 1471; third row (e.g. seq 5) in 1769. |
| F26 `cost` magic numbers 1213–1227 | CONFIRM | `(session_id.len() + 24 + 16)` (1215), `(2 * 24 + 2 * 32 + 2 * 16)` (1219) unlabeled; same shape as revocable `expected_entry_cost` (`key.len + payload + 16 + 16`, revocable 1076–1078). Action: name summands as test-module consts. |

No in-scope findings in 01/02/04/05 (only passing mentions of revocable/
transcript in 04:83 and 05:47–50,205 — no test-shape claims to re-verify).

## 5. Summary of actions

- MERGE 17 tests (revocable 13 → keepers 1157/1321/1723/2139/2180/2208/2318/new
  `construction_modes`; transcript 4 → keepers 2413/2741): −17 tests, same oracles.
- SHRINK 4 tests (1176 grep→behavioral; 1226 64 KB×128 → 1 KB×8; 1620 trim
  ceremony; 3007 16→4 iterations): largest measured win ≈2.7 s (G3 merge leg).
- Oracle hardening, no test-count change: F25 (exact `cost()`, 3rd sort row),
  G6 (test-only setter for 2428), F26 (named cost consts), F24 (trim state-poking
  helper), G7 (−150 lines shared durable-mock fixture).
- KEEP everything else (68 tests): each pins a unique path, most <0.003 s measured.

Unresolved: none for timing (top40.md + full.txt both present and used).
Savings are estimates — no code was changed and shrinks were not re-timed.
