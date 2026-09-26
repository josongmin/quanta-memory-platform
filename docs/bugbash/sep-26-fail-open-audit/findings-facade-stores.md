# T5 — Fail-Open Test Audit: facade stores (transcript_store, retained_map, payload_store)

Auditor: T5 · Date: 2025-09-26 · Repo: quanta-memory-platform @ working tree (read-only audit)

## Header — scope and census

Scope files (all `*.rs` under the three modules; each module contains only `mod.rs` + `service.rs`, and all three `mod.rs` files contain **0** test fns):

| File | `#[test]`+`#[tokio::test]` grep count |
| --- | --- |
| `crates/quanta-memory-platform-facade/src/transcript_store/service.rs` | **60** (27 outer `tests` + 33 `accounting`) |
| `crates/quanta-memory-platform-facade/src/retained_map/service.rs` | **58** (30 outer `tests` + 28 `charged`) |
| `crates/quanta-memory-platform-facade/src/payload_store/service.rs` | **33** |
| `…/{transcript_store,retained_map,payload_store}/mod.rs` | 0 / 0 / 0 |

**Grep count = 151. Census row count = 151.** Verified green baseline via
`cargo test -p quanta-memory-platform-facade --all-features --lib {transcript_store::,retained_map::,payload_store::}`
→ 60 + 58 + 33 passed, 0 failed, matching the census exactly. No sampling.

File aliases in tables: **TS** = transcript_store/service.rs, **RM** = retained_map/service.rs, **PS** = payload_store/service.rs.
Verdicts: STRONG / WEAK / FAILOPEN / VACUOUS. Tally: **149 STRONG, 2 WEAK, 1 FAILOPEN (PS:879), 0 VACUOUS.**

Cross-cutting harness note (applies to all 12 subprocess-isolated tests, marked ⚠ below): the parent
re-launches the test binary with a filter string and asserts only `status.success()` (one test also
requires `"1 passed"` in stdout). Today every `--exact`/substring filter matches its own test name, so the
child branch genuinely runs. But a **rename of the test fn desynchronizes the filter → child runs 0 tests →
exits 0 → parent stays green** (latent A2/vacuous-iteration hazard). Only
RM `charged::sweep_multiple_panicking_destructors_does_not_abort_process` (line 2690) also checks
`stdout.contains("1 passed")`, which closes this hole. Minimal fix for the rest: add the same
`&& stdout contains "1 passed"` (or `--exact --list` count) assertion.

## Known leads — verification

**A4 `let _ =` sites (all verified individually):**
- TS (11): lines 2368, 3979 are `let _ = thread::spawn(…).join()` poison-seeders whose effect is asserted
  afterwards (`Poisoned` error matches) — benign. Lines 2721, 2751, 2820, 2869, 2902, 2962, 2994, 3069
  (`store.append`) and 3347 (`store.retain_sequences`) all sit **inside `catch_unwind` closures where the
  panic is the oracle** and `panicked.is_err()` is asserted immediately after; if the op returned instead of
  panicking the test goes red. No fail-open.
- RM (5): 1166 `let _ = key;` (drop-suppression only), 1349/1409/1436/2196 inside asserted-panic
  `catch_unwind` closures — benign, same pattern as TS.
- PS (6): 936, 1256 (`put_canonical`) and 1193/1196/1199/1202 (`pin`/`unpin`/`pin_count`/`delete`) — all
  inside asserted-panic `catch_unwind` closures — benign.
Verdict: **zero A4 fail-opens**; the idiom is consistently `catch_unwind(|| { let _ = op() })` + `assert!(panicked.is_err())`.

**PS catch_unwind sites (~935, ~1191, ~1255, ~1383) — panic-location sensitivity (A5):**
- 935 `measurement_panic_precedes_accounting_and_does_not_poison_state`: trap is `PanicMeasuredPayload::byte_size`
  (unconditional), a type used only by this op; post-panic asserts `stats()==default` and `ledger==0`.
  Location pinning: structural — the charge *amount* data-depends on `byte_size()`, so accounting cannot
  precede measurement without a fabricated amount. Any panic elsewhere in the closure is impossible (no other
  panicking trait in scope). Pinned.
- 1191 `reference_eq_panic_before_mutation…`: one-shot `PANIC_NEXT_EQ` static armed immediately before the
  closure; `Ref::eq` is only invoked by `find_reference_slot` inside the four ops; post-panic asserts ledger
  snapshot equality, `item_count`, `pin_count == pins_before`, and a working delete that drains the ledger to 0.
  Any-panic-would-pass? No: only `Ref::eq` can consume the one-shot flag inside the closure. Pinned + post-state asserted.
- 1255 `reference_clone_panic_before_publication…`: one-shot `PANIC_NEXT_CLONE`; `Ref::clone` is reached only
  in `put_canonical`'s stored-ref clone (line 361); post-panic asserts ledger 0, `item_count` 0, then a clean
  retry publish + delete-drain. Pinned.
- 1383 `drop_during_outer_unwind…`: asserts the *outer* panic payload via `downcast_ref::<&str>() == Some(&"outer unwind")`
  (the propagation is the one under test, not any panic), plus `drops == 2` (both client destructors retired)
  and ledger 0. Strongest location pinning of the four.

**RM `first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1` (no top-level assert):**
the helper `exercise_endpoint_point_operations_without_linear_probe_growth(endpoint_id)` (RM:1634-1700) runs
unconditionally for both arguments (0 and 4_095); every path through it reaches the end asserts
(`get_checked == Some(expected_byte)`, `equality_probes <= 64`, `hash_probes == 1`, three times). The only
early exits are `.expect()` failures, which fail the test. Asserts are reachable; counting-key oracle is tied
to the real Eq/Hash work performed by the store. **Not fail-open.**

## Census table — transcript_store/service.rs (60)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| TS:1371 reentrant_load_stats_and_close_under_read_lock_are_typed_busy | STRONG | held mutex → load/stats/close typed `OperationInProgress`; recovery after drop asserted (len 1, close ok) | — | make `try_lock_state` block/fallback-to-empty on contention → red |
| TS:1383 partial_snapshot_clone_and_drop_panics_cannot_abort ⚠subproc | STRONG | child: clone#2 panics inside `load_versioned_snapshot`, Drop traps armed during unwind; asserts `caught.is_err()`, `item_count==2`, close, `live_ticket_count==0` | add `"1 passed"` stdout check | remove `InFlight` guards in `clone_records` → double-panic abort → child red |
| TS:1476 partial_load_clone_and_drop_panics_cannot_abort ⚠subproc | STRONG | same harness for `load`; post-unwind stats/close/ledger asserted | add `"1 passed"` check | same as above |
| TS:1571 new_session_clone_and_drop_panics_cannot_abort ⚠subproc | STRONG | session-key Clone panics on first append; `item_count==0`, close, ledger drained | add `"1 passed"` check | drop the `InFlight` guard around `new_session_key` clone → abort → red |
| TS:1646 owned_session_key_order_and_drop_panics_cannot_abort ⚠subproc | STRONG | Ord trap at owned-key descent; ledger snapshot `before` equality after panic; drain to 0 | add `"1 passed"` check | remove `ClientKey` containment → Drop-during-unwind abort → red |
| TS:1737 partial_append_identity_clone_and_drop_panics_cannot_abort ⚠subproc | STRONG | identity Clone#2 trap; `item_count==0`, close, ledger 0 | add `"1 passed"` check | unguard the identity clones → abort → red |
| TS:1836 client_message_clone_during_load_reenters_as_typed_busy_without_ledger_drift | STRONG | reentrant `stats`/`close` busy asserted inside Clone; `busy_callbacks==1`; stats+ledger == independent `cost()` oracle; close drains | — | let reentrant read observe detached state → inner assert fails → red |
| TS:1951 exact_request_retried_n_times_applies_once | STRONG | 1 Applied + 7 Replayed counted; load len 1; `retained_bytes == cost()` | — | treat replay as new row → counts/bytes red |
| TS:1968 append_truncate_cycles_cannot_grow_past_fail_closed_evidence_bound | STRONG | 3rd append denied `RetentionSaturated{evidence_count:2,max:2}`; stats unchanged vs `before`; load empty | — | allow evidence beyond cap → `expect_err` red |
| TS:1991 exact_retry_and_conflicts_remain_deterministic_at_saturation | STRONG | Replayed + SequenceConflict{7} + MessageIdConflict at saturation; stats equal before/after | — | drop conflict check → matches! red |
| TS:2012 tombstone_byte_limit_counts_large_message_ids | STRONG | byte-horizon denial with matched fields; stats byte-equal before/after (cap seeded from prod fn — setup only, oracle is stats equality) | seed cap from hand-written constant | ignore byte horizon → `expect_err` red |
| TS:2034 retention_reconfigure_below_live_evidence_is_rejected | STRONG | `BelowLiveEvidence{1,0}` + policy unchanged; exact-live bound accepted; subsequent append saturates (both poles) | — | accept below-live reconfigure → red |
| TS:2065 retention_stats_separate_active_evidence_and_session_costs | STRONG | partitions == hand-oracle cross-pinned to literals (46/148/41/235/189); truncate pole re-derived | — | mislabel a component bucket → literal red |
| TS:2103 same_identity_with_body_drift_is_payload_conflict | STRONG | `PayloadConflict{0}` + stored body content `== "old"` | — | accept drift → body assert red |
| TS:2112 reused_sequence_with_different_message_id_is_typed_conflict | STRONG | `SequenceConflict{4}` matched with sequence field | — | return wrong variant → red |
| TS:2120 reused_message_id_at_different_sequence_is_typed_conflict | STRONG | `MessageIdConflict` matched | — | wrong variant → red |
| TS:2128 append_identity_is_partitioned_by_session | STRONG | same (session,seq,msgid) applies twice; bodies `left-body`/`right-body` compared per session | — | cross-session index leak → body red |
| TS:2143 concurrent_identical_appends_have_one_applied_winner | STRONG | 8 threads barrier; exactly 1 Applied; len 1; bytes == cost oracle | — | double-apply → count red |
| TS:2175 concurrent_conflicting_appends_have_one_applied_winner | STRONG | 1 Applied + 1 `PayloadConflict{0}`; len 1 (no byte check — sibling 2143 covers) | add `retained_bytes == cost` | publish both → conflict count red |
| TS:2219 truncate_retains_tombstone_so_late_retry_cannot_reappend | STRONG | late retry `Replayed`; drifted retry `PayloadConflict{1}`; len stays 1 | — | forget tombstone → Replayed red |
| TS:2237 truncate_to_zero_retains_all_identity_tombstones | STRONG | load empty; bytes == evidence-only cost; both replays `Replayed`; still empty | — | drop evidence → replay red |
| TS:2260 zero_body_with_large_ids_and_session_retains_semantic_evidence_cost | STRONG | 512B session/1024B id; session/total bytes == oracle pre+post truncate; replay; close→0 | — | undercharge evidence → oracle red |
| TS:2281 selective_retain_preserves_sparse_rows_and_evicted_identity_tombstones | STRONG | sequences `[0,2]`; bytes == mixed-active cost; evicted replay `Replayed`; len 2 | — | evict evidence too → replay red |
| TS:2303 selective_retain_rejects_invalid_selector_without_mutation | STRONG | duplicate-selector and missing-sequence `InvalidState`; len 2; bytes unchanged (negative pole) | — | ignore selector validity → matches! red |
| TS:2322 versioned_snapshot_retain_rejects_concurrent_append_without_dropping_rows | STRONG | `StaleSnapshot{2,3}` with rows `[0,1,2]`; then `Applied{4}` with `[0,2]` | — | apply stale → disposition red |
| TS:2364 checked_load_reports_poisoned_storage_instead_of_empty_success | STRONG | deterministic poison; `load`/`stats` → `Poisoned(_)` (no empty-success mask) | — | map poison to default → matches! red |
| TS:2377 ordered_load_sorts_by_caller_sequence_and_allows_declared_gaps | STRONG | out-of-order inserts load as `[2,7]` | — | echo insertion order → red |
| TS:2623 append_message_id_clone_and_order_reentry_is_typed_busy | STRONG | reentrant busy asserted in Clone/Ord; `busy_callbacks>=4`; `item_count==2`; ledger == cost; drain | — | reentrant read sees plan → inner assert red |
| TS:2702 panicking_message_id_clone_at_each_publication_point_leaves_append_atomic | STRONG | trap at clone 1/2/3; `assert_seed_only_after_append_panic` (version, rows, both indexes, active charge, bytes, ledger) + close drain | — | publish partial row → white-box assert red |
| TS:2742 panicking_message_id_clone_cannot_publish_an_empty_new_session | STRONG | trap 1/2/3 on fresh store; `sessions.is_empty()` (white-box), stats 0, ledger 0, retry `Applied` | — | publish keyless session → red |
| TS:2789 panicking_new_session_order_after_charge_preserves_old_session_and_ledger | STRONG | Ord trap after charge; old session row/bytes/ledger exact; new session absent; drain. Note: arming offset (+3) is shape-coupled — a different panic point still passes, but the postimage oracle is exact | arm by symbol not offset | leak the charge on unwind → ledger red |
| TS:2839 append_preflight_order_panic_keeps_public_store_available | STRONG | preflight Ord panic; ledger snapshot equality; old row present; close; drain | — | poison state in preflight → stats red |
| TS:2883 append_preflight_message_order_panic_keeps_public_store_available | STRONG | MessageId Ord panic in preflight; ledger equality; load len 1; close | — | same class |
| TS:2919 append_replay_canonical_key_panic_keeps_public_store_available | STRONG | CanonicalKey trap on retained row during replay decode; ledger equality; len 1; close | — | charge before decode → ledger red |
| TS:2975 panicking_message_id_order_at_publication_leaves_append_atomic | STRONG | publication Ord panic; seed-only postimage; **comparison-count pinned** (`before+3`) on retry; committed bytes+ledger == cost | — | extra Ord after detach → count red |
| TS:3051 message_id_keyed_insert_panic_rolls_back_publication_completely | STRONG | P0 oracle: keyed-insert panic → seed-only image, clean `Applied` retry (not swallowed as replay), counters reconcile, close works | — | leave phantom evidence → retry `Applied` red |
| TS:3198 semantic_identity_component_overflow_denies_before_ledger_or_index_mutation | STRONG | `SizedId` byte_size u64::MAX both axes; `Accounting(Capacity)`; sessions empty; bytes 0; ledger 0 | — | charge then fail → ledger red |
| TS:3232 zero_body_large_identity_and_session_are_charged_until_evidence_retirement | STRONG | charged variant of 2260; stats/session/ledger == oracle at each phase; replay; close→0 | — | skip evidence charge → ledger red |
| TS:3259 retain_rollback_and_commit_do_not_compare_client_session_id_after_detach | STRONG | rebuild-fence deny + success each with **exact Ord count** (before+1 only); rows/bytes/ledger exact both directions | — | compare client Ord after detach → count red |
| TS:3317 panicking_session_order_before_detach_preserves_rows_and_ledger | STRONG | trap on first comparison; white-box rows 2 + all charges `is_active`; bytes/ledger == cost; Drop cleanup drains | — | detach before Ord → trap fires later, charge-loss red |
| TS:3370 active_row_and_session_evidence_own_separate_kind_buckets | STRONG | ledger snapshot bucket totals == retention component stats; owner-usage per-bucket; live_scratch 0 | — | wrong ChargeKind → bucket red |
| TS:3420 compound_hostile_session_ord_panic_cannot_abort_append ⚠subproc | STRONG | armed Ord+Drop compound; caught panic; stats 1; load 1; close; ledger 0 in child | add `"1 passed"` check | run Ord with owned key in flight → abort → child red |
| TS:3543 session_order_panic_before_mutation_keeps_public_transcript_available | STRONG | loops retain/truncate/versioned-retain; ledger equality; load 2; stats 2; close; drain | — | mutate before Ord → ledger red |
| TS:3598 session_bytes_ord_panic_preserves_store_and_ledger | STRONG | read-side Ord panic; ledger equality; `checked_stats`/`load`/`session_bytes`/`append` all still work; drain incl. `live_ticket_count==0` | — | poison on read panic → stats red |
| TS:3655 drop_contains_multiple_client_destructor_panics_during_outer_unwind ⚠subproc | STRONG | `PanicOnDropBody` unconditional; outer unwind asserted; ledger `cost()` before, 0 after | add `"1 passed"` check | batch destructor stops at first panic → charge leak → ledger red |
| TS:3712 retain_retires_multiple_panicking_client_bodies_after_ledger_postimage | STRONG | retain Ok with 2 panicking drops contained; postimage stats/bytes/ledger == mixed-active cost; close drains | — | retire before postimage → ledger red |
| TS:3771 quota_denial_leaves_row_indexes_and_charge_unchanged | STRONG | broker cap == exact cost of `ok` body; denial `Accounting(Capacity)`; empty load; bytes 0; retry `Applied` (no residue) | — | index before charge → retry red |
| TS:3789 retention_saturation_denies_before_any_new_ledger_charge | STRONG | full stats **and** ledger-snapshot struct equality across denial | — | emit charge first → snapshot red |
| TS:3807 retention_saturation_emits_one_bounded_tombstone_event | STRONG | event sink: exactly one `TombstoneSaturation/CapacityDenied` event, full-payload vec equality (A3: advisory event tied to deny path) | — | emit per-probe → vec red |
| TS:3845 every_conflict_axis_preserves_rows_indexes_and_accounting | STRONG | all three conflict variants matched; len 1; bytes+ledger == cost | — | mutate on conflict → cost red |
| TS:3871 exact_retry_charges_once_and_truncate_refunds_only_active_body | STRONG | replay charges once; truncate refunds only active; post-truncate replay leaves ledger byte-equal | — | refund evidence → ledger red |
| TS:3892 truncate_refund_failure_preserves_rows_indexes_and_accounting | STRONG | rebuild fence → `InvalidState`; len 2; bytes/ledger == preimage; fence finish clean (A6: fault lands) | — | drop rows on refund failure → len red |
| TS:3908 close_refunds_all_sessions_exactly_once_and_exposes_terminal_empty_reads | STRONG | pre-close ledger == per-session cost sum; terminal reads empty/version 0; idempotent double close; append/truncate/retain `Closed` | — | reopen after close → Closed red |
| TS:3943 failed_close_preserves_rows_versions_tombstones_and_all_lease_authority | STRONG | fenced close `InvalidState`; ledger == cost; stats 2; body `=="four"`; version preserved; replay `Replayed`; retry close drains | — | discard tombstones on failed close → replay red |
| TS:3972 drop_releases_all_live_rows_even_after_storage_poison | STRONG | poison after commits; close/stats `Poisoned`; ledger still charged; Drop refunds to 0 | — | leak on poison → ledger red |
| TS:3994 drop_during_rebuild_defers_exact_release_until_finish | STRONG | fenced drop keeps ledger == cost; `finish()` drains to 0 (deferral proven, A6) | — | force-release under fence → snapshot red |
| TS:4010 mutation_lane_rejects_reentry_and_reads_without_exposing_partial_images | STRONG | held permit → append/close/load/stats all `OperationInProgress`; drop → stable image; ledger inert | — | reads see empty plan → matches! red |
| TS:4034 checked_byte_and_version_overflow_denials_leave_ledger_and_indexes_intact | STRONG | white-box counter/session version injection; typed Capacity errors; state inert; ledger 0 | — | saturate instead of deny → matches! red |
| TS:4065 failed_multirow_retain_preserves_positions_tombstones_and_version | STRONG | fenced multi-row deny: full `TranscriptSnapshotV1 ==` (PartialEq over records incl. bodies), bytes/ledger equal; retry `Applied{v+1}`; evicted replay | — | reorder rows on rollback → snapshot red |
| TS:4112 racing_close_and_append_have_one_linearizable_terminal_image | STRONG | 16 races; outcomes restricted to the legal triad; terminal close forced; rows empty/bytes 0/ledger 0 | — | post-close append lands → terminal red |

## Census table — retained_map/service.rs (58)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| RM:822 insert_commits_envelope_bytes | STRONG | `get == Some(vec)` full value equality; committed == `retained_cost` oracle; stats struct equality | — | charge payload only → oracle red |
| RM:835 committed_byte_overflow_rejects_before_payload_or_accounting_mutation_v1 | STRONG | overwrite + vacant-insert overflow both `Capacity`; value identity via `marker` (1, not 9); bytes/count unchanged; MAX representable pole | — | replace-then-fail → marker red |
| RM:873 concurrent_insert_if_absent_has_one_creator_and_never_overwrites_v1 | STRONG | 12 writers × 10k: exactly 1 `Inserted`, 11 `AlreadyPresent`; item 1; bytes exact | — | overwrite on race → outcome counts red |
| RM:921 concurrent_checked_read_observes_only_complete_old_or_new_payloads_v1 | STRONG | two-phase barrier epochs; every read classified complete old/new (4KiB payloads, torn = invalid); both generations observed; `invalid==0` | — | publish torn value → invalid red |
| RM:1203 compound_hostile_remove_panic_cannot_abort_process ⚠subproc | STRONG | armed Hash+Drop in `remove`; entry intact `Some(vec![1,2,3])`; stats 1; close ok | add `"1 passed"` check | hash with owned value in flight → abort → child red |
| RM:1224 compound_hostile_insert_panic_cannot_abort_process ⚠subproc | STRONG | armed Hash+Drop in `insert`; item 0; close ok | add `"1 passed"` check | same class |
| RM:1245 read_hash_callback_reentry_is_typed_busy_and_preserves_exact_postimage | STRONG | reentrant `stats_checked`/`close` busy (lifecycle gate, resource string matched); callbacks==1; stats/ledger exact; close drains | — | reentrant read succeeds → inner assert red |
| RM:1291 sweep_hash_callback_reentry_is_typed_busy_before_refund | STRONG | planning-hash reentry busy; sweep completes; reclaimed == full cost; callbacks==1; item 0; ledger 0 | — | refund before hash → inner ledger assert red |
| RM:1329 hash_unwind_during_detached_mutation_restores_owned_state_and_charge | STRONG | Hash panic mid-plan; restored read `Some(vec![1,2,3])`; stats/ledger == preimage; close drains (restore content equality ✓) | — | Drop-restore skips entries → get red |
| RM:1392 checked_reads_propagate_client_clone_panics_without_poisoning_owner_mutex_v1 | STRONG | clone panic visible to caller; all checked reads still Ok afterwards; second panic in `entries_checked` contained | — | poison on clone panic → is_ok red |
| RM:1446 partial_entry_read_clone_and_drop_panics_cannot_abort ⚠subproc | STRONG | clone#2 trap in `entries_checked` with Drop armed; stats 2; close ok | add `"1 passed"` check | unguarded enumeration clones → abort → red |
| RM:1513 put_get_remove_get_none_round_trip_corrects_bytes | STRONG | full `Vec<u8>` equality on get **and** on removed; bytes corrected each step; final 0/empty (A7: byte-for-byte) | — | return default value → vec red |
| RM:1569 equal_hash_key_duplicate_overwrites_and_does_not_double_count | STRONG | Eq-equal non-identical keys collapse; bytes == single charge; value readable under third equal form | — | insert duplicate node → item_count red |
| RM:1702 first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1 | STRONG | helper asserts reachable for ids 0 & 4095 (see leads); equality-probe bound ≤64 with 4096 live entries; `hash_probes==1` exact for get/insert/remove | — | degrade to linear probe → probe bound red |
| RM:1708 reinsert_same_key_recharges_to_new_size_not_doubled | STRONG | 10→4 bytes: committed == new cost only | — | additive recharge → oracle red |
| RM:1769 overwrite_drop_reentry_is_bounded_and_other_store_sees_exact_ledger ⚠subproc | STRONG | 30s deadline + kill; Drop reentry: home busy typed, peer insert ok; hits==1; per-store stats + ledger exact; drain | add `"1 passed"` check (filter is substring, no `--exact`) | drop old value under lock → deadlock → deadline red |
| RM:1843 remove_releases_committed_bytes | STRONG | removed value equality; committed→0; item 0; re-remove `None` + still 0 (negative pole) | — | skip refund → committed red |
| RM:1857 sweep_reclaims_at_least_target_bytes_and_releases_them | STRONG | target=cost+1 → exactly 2 items/bytes; remaining bytes/item exact | — | stop below target → stats red |
| RM:1874 overwrite_preserves_original_insertion_position_for_newest_first_sweep | STRONG | sweep stats struct equality; b resident `Some(vec![9,9])`, c evicted; keys `[a,b]` | — | refresh position on overwrite → swept identity red |
| RM:1894 arbitrary_remove_preserves_remaining_newest_first_sweep_order | STRONG | after removing b: d still newest, c survives then goes; keys exact | — | order reset on remove → keys red |
| RM:1911 byte_targeted_sweep_reclaims_full_cost_of_newest_entry | STRONG | target 6 < entry cost → only newest; older entries retained (over-eviction pole) | — | evict to empty → item_count red |
| RM:1934 byte_targeted_sweep_of_empty_values_reclaims_key_and_index_cost | STRONG | payload-empty entries still carry key+index cost; 1 item/41 bytes reclaimed, 2 survive. Note: the `charged_bytes == 0` skip branch is unreachable via public API (min cost 40) — see footer | inject a zero-cost entry white-box to cover the skip branch | skip-zero rule broken → over-eviction red |
| RM:1952 get_missing_key_is_none | STRONG | exact `None` on empty store (minimal by name) | — | Some → red |
| RM:1958 keys_lists_inserted_keys_in_deterministic_sorted_order | STRONG | inserted c,a,b → returned a,b,c | — | echo insertion order → red |
| RM:1973 entries_lists_pairs_in_same_sorted_order_as_keys | STRONG | full `(k,v)` vec equality + key order == `keys()` | — | drop values → vec red |
| RM:1989 keys_after_remove_drops_the_removed_key | STRONG | removed key absent from enumeration (negative pole) | — | stale key → vec red |
| RM:2000 keys_on_empty_store_is_empty | STRONG | keys+entries empty on fresh store | — | non-empty → red |
| RM:2027 uncharged_retained_map_bytes_are_not_reflected_in_a_broker_ledger_snapshot | STRONG | store self-account == oracle **and** live broker ledger == 0 (A3 pole present) | — | wire unbudgeted into ledger → 0 assert red |
| RM:2062 typed_value_round_trips_and_accounts_its_own_byte_size | STRONG | `get == Some(record)` derived struct equality; bytes == declared size + key/index | — | flatten to bytes → struct red |
| RM:2083 measured_any_handle_round_trips_and_accounts_caller_size | STRONG | `Arc::ptr_eq` identity + downcast content equality + caller-size accounting; removal releases charge (negative pole) | — | clone-erase the handle → ptr_eq red |
| RM:2162 empty_value_with_huge_retained_key_is_denied_without_ledger_or_map_mutation | STRONG | 1KiB key vs cap 100 → `Capacity`; stats default; absent; ledger 0 | — | charge key after insert → ledger red |
| RM:2185 key_measurement_panic_precedes_accounting_and_preserves_state | STRONG | unconditional `byte_size` panic; stats default; ledger 0. Ordering structural (charge amount depends on measurement) | optional: event-sink zero-charge-events assert | charge-then-panic with clean drop → stays green (limit, not fail-open: no residue possible) |
| RM:2203 counter_reads_show_open_busy_and_closed_images | STRONG | open: counters == ledger charge; busy: typed contention under permit; closed: default stats + 0 ledger | — | busy returns empty → matches! red |
| RM:2254 overwrite_preserves_retained_original_key_cost_and_exact_ledger | STRONG | hand-computed literal `4+2+24+16`; overwrite by alias-declared-1024B key keeps original key cost; ledger exact; lookup `Some(vec![2;6])` | — | recharge at declared size → ledger red |
| RM:2275 aggregate_cost_overflow_is_denied_before_new_entry_or_ledger_charge | STRONG | u64::MAX declared value → `Capacity`; stats default; ledger 0 | — | insert then fail → stats red |
| RM:2313 panic_in_old_value_drop_cannot_split_committed_bytes_from_ledger | STRONG | overwrite with panicking old-value drop: stats/committed/ledger all == new cost; close drains | — | drop old before postimage → ledger red |
| RM:2351 close_contains_client_destructor_panic_and_keeps_terminal_image | STRONG | `close()` returns `Ok` despite client Drop panic (contained); terminal stats/bytes/ledger 0; idempotent; late insert `Closed` | — | panic escapes close → caught.expect red |
| RM:2382 drop_during_unwind_contains_two_client_destructor_panics_and_releases_all | STRONG | outer unwind + 2 panicking destructors; ledger 0 after | — | abort on double panic → process red |
| RM:2479 sweep_never_hashes_client_key_after_refund | STRONG | Hash asserts ledger == charge during callback (fires if hashed post-refund → panic → sweep fails); callbacks==1; drained | — | hash after refund → inner ledger assert red |
| RM:2544 sweep_hash_panic_before_refund_preserves_charged_preimage | STRONG | planning-hash panic: preimage stats/ledger exact; retry reclaims exactly; postimage 0/0 | — | refund before hash → ledger red |
| RM:2581 panic_in_sweep_value_drop_is_contained_after_exact_refund | STRONG | sweep returns exact stats despite value-Drop panic; postimage 0; ledger 0 | — | retire before refund → stats red |
| RM:2609 sweep_multiple_panicking_destructors_does_not_abort_process ⚠subproc | STRONG | sweep re-entered *during an outer unwind* with 2 key + 2 value Drop panics; exact reclaimed/postimage/ledger; **only test also checking `"1 passed"`** — model harness | — | batch retain stops at first panic → postimage red |
| RM:2698 rebuild_denied_recharge_preserves_charged_preimage_v1 | STRONG | fenced overwrite `InvalidState`; `get == Some(vec![1;30])` content equality; stats/ledger == preimage; fence finish clean | — | remove entry on recovery → get red |
| RM:2725 charged_insert_routes_bytes_through_the_ledger | STRONG | ledger == cost after insert; overwrite re-charges to 40 not sum; remove refunds to 0 | — | bypass ledger → snapshot red |
| RM:2758 atomic_insert_if_absent_never_recharges_or_overwrites_an_existing_entry_v1 | STRONG | oversized loser → `AlreadyPresent`, canonical payload `Some(vec![1;30])`, stats/ledger byte-identical | — | recharge on occupied → ledger red |
| RM:2789 denied_atomic_insert_if_absent_publishes_neither_payload_nor_charge_v1 | STRONG | denial `Capacity`; stats equal; absent read; ledger equal | — | publish then charge → absent red |
| RM:2815 charged_insert_over_cap_is_denied_and_fails_closed | STRONG | denial typed; ledger/committed/stats all equal before; survivor value intact | — | partial publish → survivor red |
| RM:2847 charged_overwrite_denial_leaves_existing_entry_unchanged | STRONG | over-cap overwrite denied; old value `Some(vec![1;30])`; committed/ledger unchanged | — | refund-then-deny → ledger red |
| RM:2875 entry_remains_refundable_after_a_denied_overwrite | STRONG | end-to-end: post-denial remove returns value **and drains ledger to 0** (no stranded ticket) | — | strand ticket → drain red |
| RM:2902 charged_overwrite_during_rebuild_is_fail_closed_and_leaves_entry_intact | STRONG | fenced overwrite denied; value `Some(vec![1;40])`; ledger == preimage; resumes + removes cleanly after fence | — | evict on recovery → value red |
| RM:2939 charged_sweep_refunds_reclaimed_bytes_to_the_ledger | STRONG | 2-entry sweep: ledger refunded by exactly 2×cost | — | skip ledger refund → snapshot red |
| RM:2962 charged_full_eviction_leaves_no_entry_key_ticket_or_accounting_residue_v1 | STRONG | full sweep stats; keys/entries empty; committed 0; ledger 0; **white-box `state.entries` empty** | — | retain dead nodes → entries red |
| RM:2985 charged_remove_during_rebuild_is_fail_closed | STRONG | fenced remove denied; value present; committed/ledger == cost (A6: fault lands) | — | remove anyway → value red |
| RM:3009 charged_remove_rejects_a_ticketless_entry_without_mutating_state | STRONG | white-box ticket extraction → `InvalidState`; value/bytes/ledger intact; restored; close drains | — | remove ticketless → ledger red |
| RM:3038 charged_sweep_during_rebuild_is_fail_closed | STRONG | fenced sweep denied; keys/bytes/ledger intact; **linked order preserved** (post-fence sweep still takes c first) | — | reorder on denial → keys red |
| RM:3078 charged_sweep_rejects_a_ticketless_candidate_without_partial_refund_or_eviction | STRONG | mid-plan ticketless candidate → deny; canonical linked order `[a,b,c]`; no partial refund/eviction; restored; close drains | — | partial refund → ledger red |
| RM:3119 uncharged_store_is_explicit_non_budgeted_mode | WEAK | asserts only positive self-accounting (`committed == cost`); the "non-ledger" half of the name is unobservable in this body (no broker) — sibling RM:2027 carries that pole | assert a live broker ledger stays 0, as RM:2027 does | n/a (WEAK) |
| RM:3130 remove_contains_panicking_key_and_returns_value_after_exact_refund ⚠subproc | STRONG | key+value Drop traps; remove returns value; stats 0; ledger 0; `PANIC_COUNT==2` (both retired); `retire_client_value(returned)` | add `"1 passed"` check (substring filter) | leak returned value → PANIC_COUNT red |

## Census table — payload_store/service.rs (33)

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| PS:708 canonical_put_deduplicates_and_charges_once | STRONG | `first.payload_ref() == second.payload_ref()`; item 1; charged == oracle; ledger == oracle | — | second entry on dedup → item red |
| PS:722 payload_lookup_probe_count_is_n_independent | STRONG | counting CanonicalKey: duplicate lookup == exactly 2 key evaluations with 2048 live entries; ref `"payload-1024"` ties to original | — | linear rescan → probe count red |
| PS:775 payload_delete_reuses_vacant_slots_without_stale_index_alias | STRONG | white-box: 128 slots freed, indices empty; 2nd generation reuses (len 128); drop drains ledger | — | grow instead of reuse → len red |
| PS:812 transient_canonical_key_is_not_charged_but_retained_identity_is | STRONG | 1MB canonical key not charged (cap 100 admits); dedup ref equality; bytes == retained-identity cost | — | charge canonical key → Capacity red |
| PS:835 cost_component_sum_overflow_denies_without_publishing_or_charging | STRONG | u64::MAX payload → `Capacity`; item 0; ledger 0 | — | publish then fail → item red |
| PS:851 composite_entry_charges_one_payload_bucket | STRONG | ledger buckets: payload==charged, metadata 0, scratch 0, `live_ticket_count==1`; owner-usage triple; close drains | — | split charge kinds → bucket red |
| PS:879 canonical_digests_are_randomized_per_store_seed | **FAILOPEN** | **A1/assert-on-input: no `SimplePayloadStore` is ever constructed.** Body asserts `RandomState::new() != RandomState::new()` — a property of std, not of the store's `digest_seed`. The named failure mode (store-level fixed/grindable digest seed) can occur while this stays green | build two real stores and compare `store_a.digest_seed.hash_one(k) != store_b.digest_seed.hash_one(k)` (white-box) or port the RM:1702 probe-count pattern across two stores | in `with_accounting`, replace `digest_seed: RandomState::new()` with any fixed-seed `BuildHasher` (store digests become grindable) → **this test stays green** while PS:722-style bucket-grinding degrades |
| PS:895 counter_reads_show_open_busy_and_closed_images | STRONG | open: stats/charged == ledger; busy: typed contention under permit; closed: default + 0 | — | busy empty-success → matches! red |
| PS:927 measurement_panic_precedes_accounting_and_does_not_poison_state | STRONG | unconditional `byte_size` panic; `stats()==default`; ledger 0. "Precedes accounting" is structural (charge amount depends on the measurement); residue oracle present | optional event-sink zero-charge assert | charge-then-panic cleanly → green (limit; no residue possible) |
| PS:943 metadata_equality_reentry_under_read_lock_is_typed_busy_and_preserves_charge | STRONG | `PayloadRef::eq` reentry asserts busy (resource matched); hits==1; stats; ledger == cost; close drains | — | reentrant read succeeds → inner assert red |
| PS:968 duplicate_canonical_callback_reentry_under_lock_is_typed_busy_and_charges_once | STRONG | CanonicalKey reentry busy; dedup ref equality; hits==1; item 1; ledger == cost once | — | charge on dedup hit → ledger red |
| PS:1007 dedup_hit_preserves_first_context_and_stores_no_second_entry | STRONG | second context does not double-store; ledger unchanged; **context buckets**: Session==bytes, Request==0 | — | recharge under 2nd context → bucket red |
| PS:1043 distinct_payloads_each_charge_once | STRONG | two payloads: charged/ledger == sum of per-ref oracle costs | — | shared charge → sum red |
| PS:1074 non_injective_public_reference_conversion_is_rejected_before_charge | STRONG | constant `From<String>` collision → `InvalidState`; stats/ledger snapshots equal; first still addressable; delete drains | — | publish colliding ref → stats red |
| PS:1094 delete_unpinned_releases_bytes_exactly_once | STRONG | delete → charged 0/item 0/ledger 0; second delete `InvalidState` (negative pole) | — | double refund → ledger red |
| PS:1108 over_cap_put_is_fail_closed_and_doesnt_consume_ref (over_cap_put_is_fail_closed_and_does_not_consume_ref) | STRONG | denial `Capacity`; item 0; **next successful put gets `payload-0`** (ref counter not consumed); ledger == cost | — | burn ref on denial → payload-0 red |
| PS:1122 pin_is_metadata_only_and_pinned_delete_is_rejected | STRONG | pin×2 → count 2 (no charge change); pinned delete `InvalidState`; ledger == cost; unpin×2 → delete → 0 | — | charge per pin → ledger red |
| PS:1143 reference_eq_panic_before_mutation_keeps_public_store_available | STRONG | 4 ops × one-shot Eq trap; ledger snapshot equality; item 1; pin_count == before; delete works and drains | — | mutate before Eq → ledger red |
| PS:1221 reference_clone_panic_before_publication_keeps_public_store_available | STRONG | one-shot Clone trap before publication; ledger 0; item 0; retry publishes + deletes cleanly | — | publish partial entry → item red |
| PS:1268 missing_delete_and_unpin_without_pin_are_invalid_state | STRONG | both error paths typed `InvalidState` | — | Ok on missing → matches! red |
| PS:1279 put_and_delete_during_rebuild_are_fail_closed_then_resume | STRONG | fenced put **and** delete denied `InvalidState`; stats/charged/ledger == preimage; fence finish; delete resumes → 0 (A6) | — | bypass fence → preimage red |
| PS:1305 explicit_unbudgeted_store_is_a_distinct_non_ledger_mode | WEAK | positive self-accounting pole only (`charged == cost`, close ok); "non_ledger" half unobservable — no broker exists in the body, and unlike retained_map (RM:2027) there is **no sibling in this file** asserting unbudgeted ledger invisibility | add a live broker + `ledger_snapshot().total_owner_bytes() == 0` pole | n/a (WEAK) |
| PS:1316 close_refunds_every_live_entry_exactly_once | STRONG | pre-close ledger == per-entry sum; post-close 0; item 0 | — | leak a ticket → 0 red |
| PS:1330 close_refund_failure_preserves_entries_and_charge_ownership | STRONG | fenced close `InvalidState`; item 1; charged/ledger == cost; retry close → 0 | — | clear entries on failure → item red |
| PS:1347 drop_during_rebuild_defers_then_finish_drains_capacity | STRONG | fenced drop: ledger == cost (deferred); finish drains to 0 | — | release under fence → deferred red |
| PS:1361 drop_during_outer_unwind_contains_two_client_panics_and_refunds_all_charges | STRONG | outer payload **downcast-checked** `&str`; `drops==2`; ledger 0 (both client drops contained during unwind) | — | abort on 2nd panic → process red |
| PS:1394 mutation_after_close_is_typed_closed | STRONG | post-close put **and** pin → `Closed("payload store")` resource matched | — | accept post-close → matches! red |
| PS:1408 double_close_is_idempotent_without_double_refund | STRONG | two closes Ok; ledger 0 (no double movement possible) | — | second close errs → expect red |
| PS:1417 concurrent_mutation_and_close_has_one_typed_linearized_outcome | STRONG | close/put race outcomes restricted; forced terminal close; ledger 0; item 0 | — | zombie entry after close → item red |
| PS:1450 counter_overflow_is_rejected_without_accounting_or_entry_mutation | STRONG | white-box `charged_total=u64::MAX`; denial `Capacity`; counter intact; entries empty; ledger 0 | — | wrap counter → matches! red |
| PS:1462 poisoned_stats_is_error_not_empty | STRONG | deterministic poison → `stats()` is `Poisoned(_)`, never a default success | — | map poison to default → matches! red |
| PS:1522 delete_drop_reentry_is_bounded_and_peer_publication_retains_exact_charge ⚠subproc | STRONG | 30s deadline; retired-body Drop reentry: home busy typed, peer publishes; hits==1; home/peer postimages + ledger exact; drain | add `"1 passed"` check (substring filter) | drop under lock → deadlock → deadline red |
| PS:1588 delete_contains_two_panicking_client_destructors_after_exact_refund ⚠subproc | STRONG | Ref+Body Drop traps during delete; delete Ok; `PANIC_COUNT==2`; item 0; ledger 0 | add `"1 passed"` check (substring filter) | abort on field drop → child red |

## Round-trip comparison table (A7 signature risk)

Every test whose name contains round / restore / recover / replay / rebuild / durable / retr(y/ied) / reappend,
plus restore-semantics adjacents. "Equality strength" = what is actually compared on the restored/replayed path.

| Test | Restored-path comparison | Strength |
| --- | --- | --- |
| TS:1951 exact_request_retried_n_times_applies_once | disposition counts (1 Applied/7 Replayed) + `load().len()==1` + `retained_bytes == cost()` | identity+cost, **no body compare** (name claims apply-once, not content) — acceptable |
| TS:1991 exact_retry_and_conflicts_remain_deterministic_at_saturation | disposition equality + full `retention_stats()` equality before/after | strong (stats struct), no body compare |
| TS:2220 truncate_retains_tombstone_so_late_retry_cannot_reappend | `== Replayed` + `load().len()==1` (twice) + `PayloadConflict` on drift | identity-only; drift pole covers content tampering |
| TS:2920 append_replay_canonical_key_panic_keeps_public_store_available | panic containment + ledger snapshot `==` + `load().len()==1` | state equality; no content compare (panic test) |
| TS:3872 exact_retry_charges_once_and_truncate_refunds_only_active_body | ledger `total_owner_bytes == cost(...)` at each phase | byte-exact ledger; no body compare |
| TS:3994 drop_during_rebuild_defers_exact_release_until_finish | ledger `== cost()` fenced, `== 0` after finish | byte-exact ledger (nothing restored by design — deferral) |
| TS:3943 failed_close_preserves_rows_versions_tombstones_and_all_lease_authority (adjacent: restore) | `load()[0].body() == "four"` + version `==` + `== Replayed` | **content equality** ✓ |
| TS:4065 failed_multirow_retain_preserves_positions_tombstones_and_version (adjacent: restore) | `load_versioned_snapshot() == before` — full `TranscriptSnapshotV1` `PartialEq` over records incl. bodies | **strongest in TS** (whole-snapshot equality) ✓ |
| TS:2322 versioned_snapshot_retain_rejects_concurrent_append… (adjacent) | sequence-vector equality `[0,1,2]`→`[0,2]` + disposition fields | row-identity, not body bytes |
| RM:1329 hash_unwind_during_detached_mutation_restores_owned_state_and_charge | restored `get == Some(vec![1,2,3])` + stats/ledger `==` preimage | **content equality** ✓ |
| RM:1513 put_get_remove_get_none_round_trip_corrects_bytes | `get == Some(vec![1,2,3,4])` and `removed == Some(vec![1,2,3,4])` (full `Vec<u8>` `==`) + byte oracle | **byte-for-byte** ✓ |
| RM:2062 typed_value_round_trips_and_accounts_its_own_byte_size | `get == Some(record)` (derived struct `==`, tag+payload) | **full value equality** ✓ |
| RM:2083 measured_any_handle_round_trips_and_accounts_caller_size | `Arc::ptr_eq` + downcast + `*downcast == vec![10,20,30]` | **identity + content** ✓✓ |
| RM:2698 rebuild_denied_recharge_preserves_charged_preimage_v1 | `get == Some(vec![1u8;30])` + stats/ledger `==` preimage | **content equality** ✓ |
| RM:2902 charged_overwrite_during_rebuild_is_fail_closed_and_leaves_entry_intact | `get == Some(vec![1u8;40])` + ledger `==` preimage | **content equality** ✓ |
| RM:2985 charged_remove_during_rebuild_is_fail_closed | `get == Some(vec![0u8;7])` + committed/ledger `==` | **content equality** ✓ |
| RM:3039 charged_sweep_during_rebuild_is_fail_closed | keys-vector equality + post-fence `get("c") == None` | key-set equality ✓ |
| PS:1280 put_and_delete_during_rebuild_are_fail_closed_then_resume | stats/charged/ledger equality; no payload content compare — **`PayloadStore` exposes no bytes read-back API**, so ref-identity/ledger is the strongest available oracle | shape+ledger only (API-limited, not lazy) |
| PS:1348 drop_during_rebuild_defers_then_finish_drains_capacity | ledger `== cost()` then `== 0` | byte-exact ledger (deferral) |
| PS:708 canonical_put_deduplicates_and_charges_once (adjacent: dedup replay) | `first.payload_ref() == second.payload_ref()` + item 1 | identity equality ✓ |

**A7 conclusion:** no shape-only round-trip was found in retained_map or transcript_store — restored content
is compared with full `==` (value, struct, or whole-snapshot equality) wherever the API returns content.
The only name-matching tests that never compare restored *content* are payload-store tests, because
`PayloadStore` has no read-back API; their oracles are ledger/stat equality plus payload-ref identity, which
is the maximum the public surface permits.

## Footer — 3-line risk summary

1. This area is in unusually good shape: 149/151 rows hold exact oracles (independent cost formulas,
   ledger-snapshot equality, full-content `==`, white-box postimage checks) tied to the real authorities
   (ledger SSOT + canonical state), with negative poles on nearly every denial path — the fail-closed ledger
   contract is genuinely pinned, and the four payload-store `catch_unwind` sites each pin their panic to a
   one-shot/only-possible trait trap plus post-panic state.
2. The one real fail-open is PS:879 `canonical_digests_are_randomized_per_store_seed`: it never constructs a
   store, so pinning `digest_seed` to a fixed hasher (re-enabling bucket-grinding that PS:723's N-independence
   then silently degrades on) keeps it green; two WEAK rows (RM:3119, PS:1305) assert only the positive
   self-accounting pole of "non-ledger" modes.
3. Systemic latent hazard: 11 of 12 subprocess-isolated panic-containment tests assert only child
   `status.success()` — a test-fn rename desynchronizes the `--exact`/substring filter and the child runs
   zero tests yet exits 0 (vacuous green); adopt RM:2609's `stdout.contains("1 passed")` pattern everywhere,
   and note RM's `charged_bytes == 0` sweep-skip branch is unreachable from the public API and unexercised.
