# Fail-Open Test Audit — T4: `facade/src/revocable_store/`

Auditor: T4 · Date: 2025-09-26 · Repo: `quanta-memory-platform` @ working tree (read-only)

## Header — Scope, Grep Counts, Row Count

Scope files (all `*.rs` under `crates/quanta-memory-platform-facade/src/revocable_store/`):

| File | Lines | `#[test]`+`#[tokio::test]` grep count |
| --- | ---: | ---: |
| `service.rs` | 4518 | **68** |
| `durable_codec.rs` | 161 | **3** |
| `cost.rs` | 30 | **1** |
| `mod.rs` | 22 | **0** (trait decl only, no test fns) |
| **Total** | 4731 | **72** |

- Census command: `grep -c -E '#\[test\]|#\[tokio::test\]' <file>` (all are plain `#[test]`; zero `#[tokio::test]`, zero `#[ignore]`).
- **Grep count = 72. Census table rows = 72. Sampling: none — every test fn has a row.**
- Behavior check: `cargo test -p quanta-memory-platform-facade --all-features --lib revocable_store` → **72 passed; 0 failed; 0 ignored** (290 filtered out). Every test in scope compiles and runs under `--all-features` (`spill-redb`, `unbudgeted-stores` included).
- SUT read in full: production `service.rs:1–1422`, test module `service.rs:1423–4518`, plus `cost.rs`, `durable_codec.rs`, `mod.rs`.

## Census Table

Verdicts: STRONG / WEAK / FAILOPEN / VACUOUS. Tally: **69 STRONG, 3 WEAK, 0 FAILOPEN, 0 VACUOUS.**
("—" in the last two columns = not applicable for STRONG rows; every WEAK row carries a minimal fix and a critic mutation.)

### cost.rs

| # | test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
|---|---|---|---|---|---|
| 1 | `revocable_entry_cost_has_one_checked_formula` (cost.rs:26) | STRONG | Exact positive oracle `entry_cost(1,1)==Ok(34)` (1+1+16+16) plus negative pole `u64::MAX` key → `Err(Capacity)`. Anti-A8: both poles. | — | — |

### durable_codec.rs

| # | test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
|---|---|---|---|---|---|
| 2 | `literal_v1_record_decodes_without_using_the_current_encoder` (durable_codec.rs:114) | STRONG | Hand-written 22-byte fixture decodes to `("k","v")` with exact charge 34 — the oracle does not route through the current encoder (anti-A7 shape-only round-trip). Negative pole: `inspect_bounded(&V1,21)` → Capacity. | — | — |
| 3 | `versioned_codec_round_trip_and_exact_cost` (durable_codec.rs:127) | STRONG | Exact wire len 30, exact charge 42 (3-byte UTF-8 key + 7 payload + 32 overhead), cap 29 → Capacity (anti-A8). Pairs with row 2 so the round-trip is not self-referential. | — | — |
| 4 | `malformed_or_forged_lengths_never_reach_allocation` (durable_codec.rs:138) | STRONG | Four concrete mutants (short header, bad magic, `key_len=u64::MAX`, non-UTF-8 key) each asserted `inspect(..).is_err()` — non-vacuous iteration (anti-A2), and the fault is asserted to land pre-allocation (anti-A6). | — | — |

### service.rs

| # | test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
|---|---|---|---|---|---|
| 5 | `sparse_reclaim_partition_and_rollback_preserve_large_fifo` (service.rs:1445) | STRONG | Drives the real rollback helpers (`extract_entries`/`restore_entries`, the exact code `reclaim_until`/`close` use on failure) over 16 384 entries × 2 selection shapes; asserts complement order and exact `0..count` restoration. | — | — |
| 6 | `compound_measurement_and_drop_panic_does_not_abort_insert` (service.rs:1555) | STRONG | Subprocess isolation; child asserts unwind is caught, stats/ledger untouched (0 tickets), close ok; parent asserts exit success — the abort pole propagates through process status (anti-A6: fault provably lands via trap flags). | — | — |
| 7 | `insert_tracks_reclaimable_bytes` (service.rs:1608) | STRONG | Exact arithmetic oracle (`expected_entry_cost` = key+payload+16+16) on `committed_bytes` and full `StoreStats` equality. | — | — |
| 8 | `reclaim_one_is_fifo_and_releases_its_bytes` (service.rs:1621) | STRONG | FIFO head returned; committed bytes AND broker ledger (`ledger_snapshot().total_owner_bytes()`) both asserted — oracle is the ledger authority, not the advisory counter (anti-A3). | — | — |
| 9 | `same_store_reclaimer_returns_busy_without_losing_fifo` (service.rs:1640) | STRONG | Fault landing proven by `inner_results.len()==1` (anti-A6); typed Busy; byte-identical FIFO/charge preservation asserted on stats+committed+ledger; retry pole included. | — | — |
| 10 | `reclaim_to_in_memory_tier_keeps_bytes_charged` (service.rs:1736) | STRONG | In-process transfer keeps the exact ledger total; payload/key/charge equality on the handle; FIFO order after re-attach; drained to 0. | — | — |
| 11 | `many_spills_share_large_binding_identity_without_unbilled_clones` (service.rs:1771) | STRONG | Identity oracle is `Arc::ptr_eq(&handle.binding, &store.binding)` + `Arc::strong_count == 129` — pointer-level authority, not shape; exact ledger totals throughout; closed → 0. | — | — |
| 12 | `spill_and_close_emit_bounded_facade_events` (service.rs:1845) | **WEAK** | Asserts only `recorded.contains(event)` membership. The name's "bounded" claim (no event storm / no duplicate emission) is not asserted: emitting each event 1 000× keeps the test green (A8 no negative pole on multiplicity). | Assert exact filtered counts (e.g. exactly one `SpillInProcess` Succeeded and one `Close` Succeeded, and nothing else) or `recorded.len()`. | Make `accounting.emit_result` push the event twice per op → test stays green while "bounded" is violated. |
| 13 | `failed_destination_write_preserves_source_entry_and_charge` (service.rs:1870) | STRONG | Injected destination failure; committed bytes, item count, and ledger all preserved; FIFO head still readable. | — | — |
| 14 | `noop_spill_does_not_discard_source` (service.rs:1904) | STRONG | Noop controller Err; stats + ledger preserved; payload still reclaimable. | — | — |
| 15 | `foreign_store_cannot_adopt_spill_charge` (service.rs:1920) | STRONG | Foreign broker rejection is `Err(InvalidState)`; original broker ledger unchanged; original owner can still re-attach. | — | — |
| 16 | `source_close_does_not_refund_live_in_process_tier` (service.rs:1941) | STRONG | Charge lives in the tier after source close; dropping the tier retires it — ledger 0 at the end. | — | — |
| 17 | `closed_source_rejects_restored_handle_without_losing_its_charge` (service.rs:1957) | STRONG | `Err(Closed)`; handle keeps payload AND charge through rejection and tier drop; final drop → ledger 0. | — | — |
| 18 | `durable_spill_releases_source_only_after_commit` (service.rs:1995) | STRONG | Ledger probed *inside* `commit_spill` (mid-operation authority oracle): source charge still live at commit time; released exactly after receipt. | — | — |
| 19 | `failed_durable_write_and_encoder_error_restore_source_fifo_and_charge` (service.rs:2083) | STRONG | Three distinct failure poles (bounded Unsupported with `reservations==0`, unbounded write failure with `reservations==1`, encoder `Err`) each with full state+ledger restoration asserts. | — | — |
| 20 | `bounded_codec_spill_preflights_backend_and_broker_before_allocating` (service.rs:2168) | STRONG | Dual-pole exact arithmetic: redb cap 40 then cap 41 with 93-byte broker (53 source + 41 wire = 94 > 93); no reservation/pending/ledger drift; source reclaimable. | — | — |
| 21 | `bounded_codec_cold_reopen_restore_denial_and_retry_keep_one_authority` (service.rs:2214) | STRONG | Cold reopen; tight broker denial leaves durable authoritative (spilled_count still 1, no active claim, ledger 0); retry publishes exactly once. | — | — |
| 22 | `bounded_codec_rejects_malformed_durable_record_without_publication` (service.rs:2263) | STRONG | Seeded `QMR2` magic; `Err(InvalidState)`; record retained, nothing published/charged (stats default, ledger 0). | — | — |
| 23 | `bounded_codec_keeps_output_charged_until_durable_commit_returns` (service.rs:2294) | STRONG | In-`commit_spill` ledger probe == expected peak 94 (source 53 + codec output 41 coexist); released only after commit returns; `reservations==1`. | — | — |
| 24 | `bounded_codec_committed_recovery_retries_after_capacity_denial` (service.rs:2364) | STRONG | Tight broker (52) denial → `OperationInProgress` with the claim still active; honest retry publishes exactly once; drained. | — | — |
| 25 | `panicking_durable_encoder_restores_source_before_unwind` (service.rs:2409) | STRONG | Panic polarity asserted (`panic.is_err()`) plus compensating authority asserts: committed bytes, ledger, and FIFO payload `"first"` reclaimed intact (see `let _ =` table row for 2454). Panic payload is not downcast (residual A5 sensitivity), but the only panic source in the closure is the injected encoder. | Optional: downcast payload to `"encoder panic"`. | — |
| 26 | `panicking_durable_commit_fences_source_until_exact_record_resolution` (service.rs:2467) | STRONG | Model dual-pole test: `persist_before_panic ∈ {false,true}`; both times the source is fenced (`pending==[id]`), unreadable, charged; reconcile `Present` → retire vs `Absent` → restore-with-payload; ledger drained both ways. Non-vacuous iteration (anti-A2). | — | — |
| 27 | `durable_redb_round_trip_transfers_charge_exactly_once` (service.rs:2567) | STRONG | Real redb; exact receipt, ledger 0 after spill, exact restored charge, payload equality, drained. | — | — |
| 28 | `oversized_encoded_record_preserves_charged_source_and_no_durable_payload` (service.rs:2599) | STRONG | Cap-3 denial; no pending; cold reopen `retained_count()==0` proves nothing durable was written; source payload intact; ledger drained. | — | — |
| 29 | `redb_file_cap_denial_keeps_store_source_charged_until_exact_resolution` (service.rs:2643) | STRONG (one WEAK sub-assert) | Loop until the real file-cap denial with `assert!(denied)` termination pole; charge preserved per iteration; post-resolution state (pending drained, survivor readable, ledger 0) asserted unconditionally. WEAK spot: the `matches!(resolution, Present|Finalized|Absent)` at 2703–2708 accepts every enum variant, so it pins nothing by itself (A8); the surrounding state asserts carry the behavior. | Branch on the resolution like row 26 does (Present → ledger 0 path; Absent → survivor path). | Replace the 3-way `matches!` body with `unreachable!()` → the assert still passes, proving it is vacuous today. |
| 30 | `durable_redb_committed_restore_recovers_after_process_restart` (service.rs:2723) | STRONG | Simulated crash (no finalize) leaves an active committed claim; recovery `Restored{0}`, claim cleared, payload exact, ledger drained. | — | — |
| 31 | `durable_redb_prepared_restore_recovers_after_process_restart` (service.rs:2776) | STRONG | Prepared-only claim survives; recovery publishes exactly once; active claim cleared; ledger drained. | — | — |
| 32 | `sep23_fup001_uncertain_commit_keeps_source_fenced_and_blocks_wrong_retire` (service.rs:2832) | STRONG | **Prompt lead disproven:** the body does NOT delegate assertions away — the local `UncertainOnce` controller is a thin wrapper over the *real* `RedbSpillBacking`, and the test ends in 12 top-level asserts (2957–2995): `Err(OperationInProgress)` fencing, `pending==[id 0]`, charge+ledger intact, later record seq 1 commits, reconcile id 0 == `Absent` (the hole — the exact FUP-001 regression), both payloads readable, ledger drained. The `let _ = bytes;` at 2871 is scaffolding, not an assertion (see table). | — | — |
| 33 | `sep23_fup002_concurrent_recovery_has_exactly_one_owner` (service.rs:3036) | STRONG | Barrier race; `Restored{0}` count == 1; loser covered in *both* observable outcomes (`RecoveryRequired` → reconcile `Superseded` → charge 0; `Empty` → charge 0); exactly one readable copy; ledger 0. Race outcomes are exhaustively partitioned — no vacuous branch. | — | — |
| 34 | `sep23_audit_panicking_decode_aborts_prepared_claim_and_retries_clean` (service.rs:3142) | STRONG | Decode panic → Prepared claim aborted (`active_restore()==None` — this is the test that verifies the *success pole* of the production `let _ = abort_restore` at 1052), no fence, 0 bytes, ledger 0, durable record retained (`spilled_count==1`), retry publishes exactly once. | — | — |
| 35 | `panicking_restore_measurement_aborts_prepared_claim_and_retries_clean` (service.rs:3212) | STRONG | Caller `ByteSized` panic (second-phase client callback); claim aborted, durable retained, no resident state; retry restores with exact payload `b"payload"`. | — | — |
| 36 | `panicking_recovery_measurement_preserves_committed_claim_for_retry` (service.rs:3268) | STRONG | Committed-phase measurement panic preserves the persisted claim (`active_restore()==Some(0)` — the opposite disposition vs row 35, deliberately); retry publishes exactly once. | — | — |
| 37 | `panicking_restore_commit_fences_resident_until_redb_reconciliation` (service.rs:3324) | STRONG | Dual `commit_before_panic` poles; provisional copy fenced `[id]`, unreadable, charged>0 with ledger equality; reconcile `Finalized` → publish vs `Present` → retire + ordinary retry; `reclaim_one` proves no duplicate. | — | — |
| 38 | `panicking_prepare_after_persisted_claim_has_explicit_recovery_path` (service.rs:3430) | STRONG | Prepare-then-panic leaves the persisted claim (`active_restore()==Some(id)`), no provisional resident charge; `recover_active_durable_restore_unbounded` publishes exactly once; no duplicate. | — | — |
| 39 | `sep23_audit_panicking_decode_preserves_committed_recovery_until_retry` (service.rs:3517) | STRONG | Committed-phase decode panic preserves claim untouched; honest retry publishes exactly once; ledger drained. | — | — |
| 40 | `sep23_fup002_stolen_claim_reconcile_retires_provisional_copy` (service.rs:3563) | STRONG | Deterministic steal via mpsc channel orchestration (no sleeps): stale claimant gets `RecoveryRequired`, fenced copy never readable, reconcile → `Superseded`, charge retired; winner readable once; ledger 0. | — | — |
| 41 | `sep23_fup002_decode_failure_and_denied_admission_preserve_durable` (service.rs:3644) | STRONG | Two poles (decode `Err(Report)`, capacity denial) both abort the claim and keep the record (`spilled_count==1`, ledger 0); fresh claim then restores exactly once. | — | — |
| 42 | `sep23_fup002_commit_ack_loss_same_claimant_publishes_on_retry` (service.rs:3701) | STRONG | Injected lost-ack (`inject_lost_restore_commit_ack` on the real backing) → fenced with claim; same-claimant reconcile → `Finalized` publishes once with exact charge. | — | — |
| 43 | `denied_durable_restore_leaves_redb_record_authoritative` (service.rs:3762) | STRONG | Admission denial (decode Ok but 100-byte value vs 5-byte broker): record retained, nothing published/charged. | — | — |
| 44 | `failed_durable_delete_rolls_back_provisional_source_charge` (service.rs:3793) | STRONG | `DurableCommitFailure::Retained` → abort-Ok → `Err(Report)`; provisional charge rolled back (ledger 0), not published. Covers the abort-Ok pole of `finish_failed_restore_prepare`. | — | — |
| 45 | `ambiguous_durable_delete_fences_source_until_absence_is_reconciled` (service.rs:3870) | STRONG | `Ambiguous` → fenced `[11]`, unreadable, charged>0; flipping `present=false` then reconcile → `Finalized` publishes; ledger released. Two-state fault oracle, both states asserted. | — | — |
| 46 | `panicking_finalize_fences_published_source_until_reconciliation` (service.rs:3993) | STRONG | Finalize panic #1 fences the *published* copy; reconcile → finalize panic #2 re-fences (`pending==[13]` again, still unreadable); third reconcile publishes. The re-fence pole (published → fenced) is exercised, not just the first fence. | — | — |
| 47 | `reclaim_one_on_empty_returns_none` (service.rs:4123) | STRONG | Exact `None` + committed bytes 0. Small but complete for the named behavior. | — | — |
| 48 | `reclaim_until_frees_at_least_target_and_reports_reclaimed_bytes` (service.rs:4130) | STRONG | Exact `SweepStats{2, 2×cost}` (over-shoot to satisfy target is pinned) + remaining bytes exact. | — | — |
| 49 | `reclaim_until_on_empty_store_reports_zero` (service.rs:4144) | STRONG | Exact zero sweep stats. | — | — |
| 50 | `reclaim_until_charges_and_evicts_zero_payload_keys` (service.rs:4152) | STRONG | Zero-payload entry participates in the sweep (regression for "skip zero-byte entries" over-eviction bug class); exact stats + FIFO survivor. | — | — |
| 51 | `reclaim_until_of_zero_payload_entries_reclaims_their_key_and_index_cost` (service.rs:4170) | STRONG | All-zero-payload FIFO; target 5 reclaims exactly 1 entry at its key+index cost. | — | — |
| 52 | `revoke_makes_entry_unretrievable_and_decrements_accounting` (service.rs:4183) | STRONG | Payload returned once, bytes → 0, second reclaim `None`. | — | — |
| 53 | `empty_reclaim_leaves_state_byte_for_byte_unchanged` (service.rs:4193) | STRONG | Full `StoreStats` equality before/after a no-op reclaim; on the empty store stats is the complete public observable (item_count incl. pending maps + estimated_bytes). | — | — |
| 54 | `insert_over_cap_is_denied_and_fails_closed` (service.rs:4201) | STRONG | `Err(Capacity)` + default stats + ledger 0 (charge never landed). | — | — |
| 55 | `large_key_with_zero_value_is_charged_and_denied_without_state_change` (service.rs:4211) | STRONG | 101-byte key vs 100 cap; state and ledger byte-identical after denial. | — | — |
| 56 | `cost_component_sum_overflow_is_fail_closed_before_charge` (service.rs:4221) | STRONG | `u64::MAX` key → `Err(Capacity)` *before* broker charge or mutation (stats default, ledger 0). | — | — |
| 57 | `insert_charges_and_reclaim_refunds_the_ledger` (service.rs:4242) | STRONG | Ledger mirrors insert and both reclaim forms; ends at exactly 0. | — | — |
| 58 | `close_refunds_every_live_entry_exactly_once_and_stays_closed` (service.rs:4261) | STRONG | Ledger 0; idempotent double close; post-close insert/reclaim are `Err(Closed)`; terminal default stats. | — | — |
| 59 | `close_release_failure_preserves_entries_fifo_and_ledger_authority` (service.rs:4279) | STRONG | Rebuild fence forces `release_many` failure → `Err(InvalidState)` with entries/FIFO/charge fully restored; after fence finish FIFO head is still `"a"`; retry close drains. | — | — |
| 60 | `reclaim_batch_failure_preserves_every_entry_and_charge` (service.rs:4299) | STRONG | Batch sweep failure mid-plan restores all 3 entries + charges; FIFO preserved. | — | — |
| 61 | `reclaim_one_during_rebuild_is_fail_closed` (service.rs:4316) | STRONG | `Err(InvalidState)` + full preservation asserts. | — | — |
| 62 | `reclaim_until_during_rebuild_is_fail_closed` (service.rs:4331) | STRONG | Same for the sweep path. | — | — |
| 63 | `drop_without_explicit_close_releases_capacity` (service.rs:4347) | STRONG | Lexical drop → ledger 0. | — | — |
| 64 | `drop_during_rebuild_defers_exact_release_until_finish` (service.rs:4361) | STRONG | Conservative ledger held during rebuild; exact release on finish. | — | — |
| 65 | `drop_during_outer_unwind_contains_two_client_panics_and_refunds_all_charges` (service.rs:4376) | STRONG | **Location-sensitive A5:** the panic payload IS downcast (`outer.downcast_ref::<&str>() == Some(&"outer unwind")`); both client destructor panics contained (`drops == 2`); ledger fully refunded. This is the only catch_unwind site in scope that proves WHERE the panic came from. | — | — |
| 66 | `concurrent_insert_and_close_have_one_linearized_outcome` (service.rs:4408) | STRONG | The insert-outcome disjunction (Ok / Closed / Busy) is the correct linearization set, and the invariant is the authority: ledger 0 + terminal stats regardless of interleaving; close tolerates only Ok/Busy (any other error panics). | — | — |
| 67 | `counter_overflow_is_rejected_before_broker_charge_or_mutation` (service.rs:4441) | STRONG | White-box setup pins `committed_bytes = u64::MAX`; insert → `Err(Capacity)` with 0 entries and 0 ledger bytes (charge rejected before mutation). | — | — |
| 68 | `poisoned_stats_is_error_not_empty_and_drop_still_releases` (service.rs:4453) | STRONG | Poison → `Err(Poisoned)` (explicitly NOT projected as an empty store — anti-A3); drop still releases the charge. | — | — |
| 69 | `reentrant_read_and_close_under_state_lock_return_typed_busy` (service.rs:4471) | STRONG | Held state lock → typed `Err(OperationInProgress)` for stats and close; released → readable, close succeeds, ledger 0. | — | — |
| 70 | `constructed_budgeted_store_exposes_its_single_broker_authority` (service.rs:4488) | **WEAK** | Asserts only `store.broker().is_some()` — shape, not authority (A8 no negative pole). If the store were wired to the wrong/different broker (or two), the test stays green; "single" and "its" are unverified. | Assert identity: charge one byte and observe it on *this* broker's `ledger_snapshot()`, or compare broker identity (e.g. `Arc` ptr equality on the broker's inner handle) rather than `is_some()`. | Make `new_budgeted`/`with_accounting` install a freshly-built foreign broker (or a second one) → `broker()` still `Some` → test green while "single broker authority" is false. |
| 71 | `reclaim_eviction_order_is_deterministic_fifo_at_capacity_boundary` (service.rs:4495) | STRONG | FIFO order pinned at the capacity boundary (oldest evicted for target 1; middle next). | — | — |
| 72 | `unbudgeted_store_is_a_distinct_explicit_product_type` (service.rs:4509) | **WEAK** | The named claim ("distinct explicit product type") is a type-level property this runtime body cannot falsify (A7-adjacent); what the body actually proves is that an unbudgeted store inserts/stats/closes with correct semantic bytes. If the unbudgeted product silently became budgeted, this test stays green — nothing inspects the accounting mode. | Add `assert!(store.broker().is_none())` (the `#[cfg(test)]` accessor exists on all modes) and cross-reference the compile-fail doc tests for raw construction. | Make `new_unbudgeted` delegate to `with_accounting(binding, StoreAccounting::Budgeted(..))` → stats asserts still pass → green while the product is no longer distinct/unbudgeted. |

## `let _ =` Classification Table

All `let _ =` occurrences in scope live in `service.rs` (49 total). Arithmetic: **49 = 2 DOC-EXAMPLE + 2 PRODUCTION + 45 TEST-BODY**; the 45 test-body occurrences = **34 CLEANUP (fs) + 10 PANIC-PATH (compensated) + 1 SCAFFOLDING**. **SWALLOWED-VERIFICATION (uncompensated fail-open): 0 found.**

### Out-of-scope-by-definition (noted, excluded from the 45)

| Line | Context | Class | Note |
|---|---|---|---|
| service.rs:141 | `/// let _ = SimpleRevocableStore::<..>::new(binding);` | DOC-EXAMPLE | Inside a `compile_fail` doc example proving raw construction is private. |
| service.rs:147 | `/// let _ = SimpleRevocableStore::<..>::charged_against;` | DOC-EXAMPLE | Same doc block; field privacy. |
| service.rs:1052 | `let _ = controller.abort_restore(&candidate.claim);` | PRODUCTION (reasoned) | **Prompt lead corrected:** this is NOT test code — it is `abort_prepared_restore_on_panic` (production, 1036–1054), guarded by `#[expect(clippy::let_underscore_must_use, reason = "panic unwind must preserve the original payload; abort failure cannot replace it")]`, which satisfies the cheatsheet's reasoned-exception rule. Outcome verification: the **success pole is tested** — rows 34 (`sep23_audit_panicking_decode…`, asserts `active_restore()==None` at 3176) and 35 (`panicking_restore_measurement…`, 3241). The **failure pole** (abort errors while unwinding → claim stays active and the backing's recovery path owns it) is untested in scope; residual gap, not a test defect. |
| service.rs:1340 | `let _ = self.lifecycle.transition_to_closed_for_drop();` | PRODUCTION | `Drop` impl; transition failure has nothing to return to. Covered behaviorally by rows 63/64/65 (drop releases charges). |

### Test-body occurrences — CLEANUP (34, all `std::fs::remove_file`)

Lines: 2575, 2594, 2607, 2638, 2651, 2718, 2731, 2771, 2784, 2820, 2943, 2998, 3045, 3133, 3150, 3191, 3221, 3263, 3277, 3319, 3384, 3424, 3483, 3512, 3525, 3558, 3572, 3639, 3652, 3696, 3709, 3757, 3770, 3788.
Every one is `let _ = std::fs::remove_file(path-ish);` — pre-test stale-file clearing (with `assert!(!path.exists())` fresh-backing guards where it matters) or post-test teardown of the temp `.redb` backing. Removing a temp file is not the operation any test claims to verify. **Class: CLEANUP — no fail-open.**

### Test-body occurrences — SCAFFOLDING (1)

| Line | Test | Quote | Class / why not fail-open |
|---|---|---|---|
| 2871 | row 32 (`sep23_fup001…`) | `if self.fail_next_commit.swap(false, SeqCst) { let _ = bytes; return Err(DurableWriteFailure::RecoveryRequired{..}) }` | Discards the *input parameter* `bytes` in the scripted lost-ACK branch so the failure is injected before the real backing commits. Not a `Result` of the op under test; the fault's landing is proven downstream by `Err(OperationInProgress)` + `pending==[id 0]` asserts. **SCAFFOLDING (deliberate input discard).** |

### Test-body occurrences — PANIC-PATH (10, each compensated)

Pattern: the call is wrapped in `catch_unwind(AssertUnwindSafe(..))`; the `let _ =` only exists because the closure must return `()` — in the scenario under test the call never returns a value, it panics. Each is paired with (a) a panic-polarity assert (`assert!(panic.is_err(), ..)`) that goes red if the injected panic stops propagating, and (b) authority asserts on ledger/store/controller state that go red if the named preservation/fencing behavior breaks. None is a swallowed verification; none leaves the named failure mode green.

| Line | Test (row) | Quote | Compensation that closes the fail-open |
|---|---|---|---|
| 2454 | row 25 | `let _ = store.spill_one_durable_unbounded(&controller, \|_,_\| -> MemoryResult<Vec<u8>> { panic!("encoder panic") });` | `panic.is_err()` (3rd-party panic would also pass — noted residual A5) + committed bytes, ledger total, and FIFO reclaim of `"first"` with original payload. |
| 2532 | row 26 | `let _ = store.spill_one_durable_unbounded(&controller, \|_,_\| Ok(b"source\0data".to_vec()));` | `panic.is_err()` + `pending_durable_spills()==[id 0]`, fenced read `None`, charge+ledger retained, then BOTH reconcile branches asserted (`Present`→retire / `Absent`→restore payload). |
| 3168 | row 34 | `let _ = store.restore_one_durable_unbounded(&controller, \|_: &[u8]\| -> .. { panic!("restore decode panic") });` | `panic.is_err()` + claim aborted (`active_restore()==None`), no fences, 0 bytes, ledger 0, record retained, retry restores once. |
| 3238 | row 35 | `let _ = store.restore_one_durable_unbounded(&controller, decode);` (measurement panics inside `ByteSized`) | `panic.is_err()` + aborted claim, durable retained, no resident state, retry exact payload. |
| 3289 | row 36 | `let _ = store.recover_active_durable_restore_unbounded(&controller, \|bytes\| { .. PanicOnceMeasuredValue .. });` | `panic.is_err()` + claim PRESERVED (`Some(id)`) — opposite disposition vs 3238, deliberately — no fences, 0 charge, retry publishes once. |
| 3394 | row 37 | `let _ = store.restore_one_durable_unbounded(&injected, sep23_fup002_decode);` (injected commit panic) | `panic.is_err()` + fenced `[id]`, unreadable, charged>0 with ledger equality, dual reconcile branches + no-duplicate read. |
| 3491 | row 38 | `let _ = store.restore_one_durable_unbounded(&PanicAfterPrepare(&controller), sep23_fup002_decode);` | `panic.is_err()` + claim persisted `Some(id)`, no provisional copy/charge, explicit recovery path publishes once. |
| 3533 | row 39 | `let _ = store.recover_active_durable_restore_unbounded(&controller, \|_: &[u8]\| -> .. { panic!("recovery decode panic") });` | `panic.is_err()` + committed claim untouched, no fences, 0 charge, honest retry publishes once. |
| 4076 | row 46 | `let _ = store.restore_one_durable_unbounded(&controller, {decode}).expect("finalize panic occurs after resident preparation");` | The `.expect` inside the closure asserts Ok-if-return; panic polarity; fenced published copy `[13]` + unreadable + charged. |
| 4100 | row 46 | `let _ = store.reconcile_durable_restore(&controller, DurableRecordId::new(13)).expect("second finalize panic occurs after provisional publication");` | Panic polarity + RE-fence assert (`pending==[13]` again, still unreadable) before the final publish assert. |

**A5 residual (all 10):** none of these downcasts the panic payload, so strictly they prove "some panic unwound", not "the injected panic unwound". In every case the injected target/fault is the only scripted panic source and the compensating authority asserts pin the named behavior, so no exploitable green-path was found; row 65 shows the house style for payload-sensitive asserts when it matters. Optional hardening: `panic.downcast_ref::<&str>() == Some(&"…")`.

### Prompt leads — disposition summary

1. **`service.rs:1052`** — misattributed in the brief: production code with a reasoned `#[expect]`, not a test. Abort success pole verified by rows 34/35; abort failure pole untested (noted above).
2. **`sep23_fup001_uncertain_commit…` helper chain** — ends in 12 real top-level asserts against ledger + store + real redb backing state (row 32). Not fail-open.
3. **catch_unwind ~2452 / ~2530 (`spill_one_durable_unbounded` panic preservation)** — both assert state preservation and payload survival (exact charge, FIFO key, ledger totals), and 2532 asserts both reconcile dispositions; only the payload downcast is missing (residual A5, compensated).

## Footer — 3-line risk summary

1. This area is in strong shape: 69/72 STRONG with oracles tied to the ledger authority (`ledger_snapshot().total_owner_bytes()`), in-callback mid-operation probes, real redb backings, and dual-pole fault matrices; zero FAILOPEN/VACUOUS and zero genuinely swallowed verifications — the 45 test-body `let _ =` are 34 fs-cleanup, 10 compensated panic-path discards, 1 input scaffolding.
2. The three real defects are assertion-strength WEAK spots, all cheap to fix: membership-only event asserts (row 12), `broker().is_some()` shape-only authority (row 70), and an unbudgeted-product test that cannot see accounting mode (row 72); plus one vacuous 3-way `matches!` sub-assert inside an otherwise STRONG test (row 29).
3. Residual risk concentrates in panic-path diagnostics: 10 catch_unwind sites do not downcast panic payloads (location-insensitive in principle, compensated in practice), and the production `abort_restore` failure-during-unwind pole (line 1052) has no pinning test — a regression there would surface only via the backing recovery path, not this suite.
