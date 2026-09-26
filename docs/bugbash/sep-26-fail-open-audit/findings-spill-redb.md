# T6 — Spill / redb fail-open audit (sep-26)

## Header

Scope files (all under `crates/quanta-memory-platform-facade/src/spill/`):

| file | `#[test]` | `#[tokio::test]` | rows |
| --- | --- | --- | --- |
| `service.rs` | 20 | 0 | 20 |
| `backings/redb.rs` | 55 | 0 | 55 |
| `mod.rs` | 0 | 0 | 0 |
| `backings/mod.rs` (2 lines, module decl only) | 0 | 0 | 0 |
| **total** | **75** | **0** | **75** |

- **Row count 75 == grep count 75.** No sampling. Runtime cross-check:
  `cargo test -p quanta-memory-platform-facade --all-features --lib spill` →
  81 passed / 0 failed / 0 ignored; exactly 20 run under `spill::service::tests::`
  and 55 under `spill::backings::redb::tests::` (= 75; the other 6 spill-named
  tests live in `revocable_store::service::tests`, outside this scope).
- `mod.rs` note: the "feature-gated test fns" there are `#[cfg(all(test, …))]`
  constructor *helpers* (`DurableRestoreClaim::for_test`/`issued`,
  `DurableRestoreCandidate::for_test`) consumed by revocable-store tests — not
  test functions. `mod.rs` additionally carries 7 `compile_fail` doc-tests
  (seal/forge/Clone/Copy negative poles) that are real tests but are not
  `#[test]` rows and are excluded from the census per the grep rule.
- Out of scope but boundary-relevant: `facade/tests/spill_redb_kill_reopen.rs`
  and the kill-child binary exercise the `durability-test-hooks`
  `finalize_kill_hooks` / `restore_commit_volume_hook` modules (redb.rs:328/392);
  no in-crate test does. That process-kill crash rail is another auditor's area.
- `tools/pm/sources/rules/determinism.md` contains no backing round-trip
  section (grep for spill/redb/durable/round-trip: no hits); tests.md L15-16
  places backing round-trips in the backing module under its feature — honored.

### A4 verification — the ~18 `let _ =` in `backings/redb.rs`

18 occurrences, **none swallows the op under test**:
- redb.rs:2300 — `let _ = std::fs::remove_dir_all(&self.directory);` in
  `Drop for TestDb`: temp-dir cleanup, correct usage.
- redb.rs:2915, 2924, 2925, 2935, 2950, 2951, 2959, 2968, 2986, 3013, 3033,
  3112, 3159, 3201, 3218, 3235, 3336 (17 sites) — all are
  `let _ = op.expect("…")`: the `.expect` **panics on `Err`**; only the `Ok`
  receipt is discarded. Failure propagation is intact at every site.
- Production-side redb.rs:1863/1964 `let _next = self.prepare_restore()?;`
  inside `#[cfg(test)]` fault hooks — `?` propagates; only the candidate value
  is dropped (intentional: the minted claim must survive).
- The only *unguarded* discard in scope is service.rs:600
  `let _ = controller.spill("h2".to_string());` — see census row service.rs:589.

## Census table

Verdicts: STRONG / WEAK / FAILOPEN / VACUOUS. 73 STRONG, 2 WEAK, 0 FAILOPEN,
0 VACUOUS.

### `service.rs`

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| poisoned_spill_inventory_returns_error_not_zero (service.rs:514) | STRONG | real mutex poison via thread panic under lock; asserts `Err(Poisoned)` not `Ok(0)` | — | make `retained_count` map lock poison to `Ok(0)` → red |
| unavailable_spill_returns_the_only_handle_without_dropping_it (service.rs:529) | STRONG | DropProbe drop-counting: 0 drops on denial, handle returned, 1 drop only after caller drop; Capacity kind asserted | — | drop handle inside `RoutedSpillController::spill` denial → red |
| noop_spill_rejects_and_returns_the_handle (service.rs:550) | STRONG | Noop rejects with Capacity + exact handle; real negative pole (redundant with :637) | — | make Noop return a fake receipt → red |
| routed_spill_to_available_target_succeeds_and_counts (service.rs:559) | STRONG | exact receipt values (`inventory_after` 1,2) + count from real target state | — | off-by-one `inventory_after` → red |
| routed_spill_rolls_back_the_count_when_accept_fails (service.rs:589) | **WEAK** | denial + handle poles are strong (fake passes preflight, fails accept), but the named "rolls back the count" oracle is `AcceptFailingTarget::retained_count` hardcoded `Ok(0)` — a constant; no controller mutation can fail it (A1). service.rs:600 `let _ =` on the 2nd spill is A4-shaped, covered only by the inert count check | give the fake a real counter (e.g. `Cell<u64>`) incremented on success, assert it stays 0; or drop the count claim from the name | mutate `RoutedSpillController::spilled_count` to `Ok(0)` constant → row stays GREEN (proves count assertion inert) |
| spill_to_unavailable_target_fails_closed_and_does_not_count (service.rs:605) | STRONG | Capacity + handle + count via real `VecDeque` state. Note: count pole is on an *empty* target (0→0), inert vs a destructive denial | also assert retained payload survives a denial on a non-empty offline target | — |
| available_controller_succeeds_while_unavailable_controller_rejects (service.rs:622) | STRONG | positive/negative pair differing only in availability | — | — |
| noop_controller_rejects_without_a_target_and_returns_handle (service.rs:637) | STRONG | duplicate of :550, still real | merge with :550 | — |
| routed_spill_after_target_goes_offline_fails_closed_leaving_inventory_unchanged (service.rs:646) | **WEAK** | the named *transition* (spilled ok → target goes offline → denied, inventory byte-identical) never happens: the controller owns the target by value, so the test builds a **fresh empty** offline controller; `count_before == count_after == Ok(0)` trivially; the denial half duplicates :605. The fail-open it names — a denial that disturbs prior retained inventory — is untested anywhere in service.rs for the *accept* path (:722/:781 cover restore only) | test the real transition at target level: accept 2 handles → `set_available(false)` → denied `accept` → assert `retained_count()==Ok(2)` and both handles still restore byte-for-byte after recovery | make `InMemorySpillTarget::accept` drain `store` when unavailable (destructive denial): this row stays GREEN (its target is empty); :675 stays GREEN (`accepted_count` is cumulative); nothing in service.rs goes red — demonstrates the gap |
| in_memory_target_accept_after_toggle_offline_is_rejected (service.rs:675) | STRONG | real availability toggle; `accepted_count` only moves on success | also assert `retained_count` survives the denied accept | — |
| in_memory_target_write_then_restore_round_trips_the_payload (service.rs:689) | STRONG | FIFO order + exact payload equality + cumulative vs retained counts | — | swap restore order to LIFO → red |
| in_memory_target_restore_of_missing_handle_is_none_not_an_error (service.rs:706) | STRONG | drained-but-online restore is `Ok(None)`, restorable-past-empty inert | — | — |
| in_memory_target_restore_after_offline_fails_closed_leaving_retained_state_unchanged (service.rs:722) | STRONG | non-empty offline denial: Capacity error, retained count intact, byte-for-byte restore after recovery — the model the :646 row should follow | — | make offline restore pop the queue → red |
| retaining_target_round_trips_handles_fifo (service.rs:756) | STRONG | FIFO + equality + `None` when drained | — | — |
| retaining_target_offline_rejects_accept_but_state_unchanged (service.rs:770) | STRONG | denial + handle return; note state pole on empty target (0→0) | see :605 fix | — |
| retaining_target_offline_restore_fails_closed_like_in_memory (service.rs:781) | STRONG | Liskov pole: offline restore = Capacity not `None`; retained byte-for-byte intact | — | — |
| both_spill_targets_share_the_offline_restore_contract (service.rs:811) | STRONG | generic helper runs both impls through the same offline-Capacity contract | — | — |
| routed_controller_spill_then_restore_balances_the_spilled_count (service.rs:821) | STRONG | count balances through each restore; drain-past-empty never underflows | — | — |
| concurrent_spills_keep_spilled_count_consistent_then_drain_to_zero (service.rs:838) | STRONG | 8×20 concurrent spills, `.expect` inside threads + join-unwrap, exact total, full drain to 0 | — | — |
| noop_controller_restore_is_none (service.rs:869) | STRONG | default trait method negative pole (trivial but authority-tied) | — | — |

### `backings/redb.rs`

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| create_on_an_invalid_path_is_a_neutral_memory_error (redb.rs:2436) | STRONG | open under missing dir → typed `Build`, no redb leak | — | — |
| live_backing_excludes_second_open_and_drop_releases_persisted_file (redb.rs:2443) | STRONG | redb's real file lock excludes 2nd open; after drop, reopen restores `b"persisted"` byte-for-byte | — | — |
| durable_spill_target_round_trips_fifo_through_the_trait (redb.rs:2461) | STRONG | exact receipts (seq 0/1), FIFO restore byte-for-byte, drained `None`, count 2→0 | — | — |
| reopened_backing_resumes_sequence_and_restores_persisted_handles (redb.rs:2479) | STRONG | sequence resumes at 1 (no collision) + persisted content byte-for-byte after cold reopen | — | reset `NEXT_SEQUENCE` on empty open → red |
| drained_queue_never_reuses_sequence_or_accepts_stale_restore_token (redb.rs:2494) | STRONG | ABA pole: stale claim commit `Retained`, finalized id not resurrectable (same *and* different bytes), replacement gets seq 1 with exact content | — | reuse seq 0 after drain → red |
| persisted_restore_claim_blocks_duplicate_prepare_and_recovers_after_reopen (redb.rs:2523) | STRONG | persisted claim blocks 2nd prepare (`OperationInProgress`), survives reopen, recovery→reconcile→`Present{1}`→claim released | — | — |
| committed_restore_payload_survives_reopen_until_resident_ack (redb.rs:2552) | STRONG | Committed phase + bytes (`b"crash-safe"`) survive reopen; finalize→`Finalized`, count 0 | — | — |
| injected_write_crash_points_converge_without_duplicate_records (redb.rs:2583) | STRONG | fault a: pre-txn injection — `Err(NotCommitted)` then `Absent`, id burned safely; fault b: lost write ack — post-commit probe converts to exact `Ok` receipt; idempotent retry returns identical receipt; count 1. See fault table for firing analysis | optional: match `"injected"` marker in the fault-a error string | remove the `lose_write_commit_ack` probe so lost acks return `Err(NotCommitted)` → red (conversion is load-bearing) |
| injected_restore_and_finalize_ack_loss_preserve_recovery_authority (redb.rs:2609) | STRONG | lost restore-commit ack → `Ambiguous`, own-claim reconcile → `Finalized`, finalize converges to `Ok`, claim cleared, count 0 | — | make ambiguous-commit path drop the claim → red |
| generic_restore_is_non_destructive_and_requires_owned_two_phase_path (redb.rs:2638) | STRONG | generic `SpillTarget::restore` = `Unsupported`, record intact, prepare returns exact bytes | — | — |
| generic_write_is_non_mutating_and_returns_the_original_payload (redb.rs:2654) | STRONG | generic `accept` = `Unsupported`, payload returned byte-equal, count 0 | — | — |
| reopened_redb_reconciles_controller_count_from_persisted_inventory (redb.rs:2666) | STRONG | controller count re-derived from persisted inventory (2), not in-memory state; restores both in order | — | — |
| concurrent_accepts_lose_no_handle_and_assign_distinct_keys (redb.rs:2694) | STRONG | 4×2 barriered writers: distinct sequences (BTreeSet), contiguous 0..8, exact payload **multiset** round-trip, full drain | — | allocate key outside txn → duplicate sequence → red |
| sep23_fup001_reserved_hole_is_not_finalized_by_later_record (redb.rs:2763) | STRONG | hole `Absent` before/after consuming id 1 and across reopen; exact dispositions survive | — | infer disposition from `max(seq)` → red |
| sep23_fup001_reserve_only_reopen_is_not_consumed (redb.rs:2808) | STRONG | bare reserve survives reopen as hole; same id commits late exactly once; content `b"late"` | — | — |
| sep23_fup001_out_of_order_commit_finalized_reinsert_rejected (redb.rs:2830) | STRONG | reverse-order commits; finalized id rejected (same+diff bytes); conflicting bytes on live id rejected *preserving original* (`b"bbb"`) | — | — |
| sep23_fup001_same_bytes_retry_keeps_inventory_stable (redb.rs:2882) | STRONG | same id+bytes = duplicate-insert-free retry, identical receipt, count 1 | — | — |
| limits_reject_oversized_record_before_commit_without_consuming_reserved_id (redb.rs:2906) | STRONG | 5B>4B cap → `NotCommitted(Capacity)`; id stays `Absent` (not consumed); same id then commits `b"1234"` and restores | — | — |
| retained_limits_count_recovery_and_refund_only_after_finalize (redb.rs:2921) | STRONG | recovery-phase record still counts (3rd write denied `Capacity`/`Absent`); refund only after finalize; reopen restores `def`,`xy` in order | — | refund on commit_restore instead of finalize → red |
| retained_payload_cap_denies_next_record_without_changing_durable_authority (redb.rs:2946) | STRONG | byte-cap denial leaves `Absent` + count 2; freed quota reused; three exact restores | — | — |
| spill_receipt_inventory_includes_other_records_in_recovery_slot (redb.rs:2965) | STRONG | `inventory_after` counts queue+recovery (2), stable on idempotent retry, drops to 1 after finalize | — | — |
| in_flight_restore_budget_denial_preserves_claim_and_releases_on_candidate_drop (redb.rs:2982) | STRONG | denial from the **byte** budget with counters asserted exactly (8 bytes / 2 buffers = caps); claim preserved; lease released on drop; final counters 0/0 | — | skip lease release on drop → third recover stays denied → red |
| zero_byte_restore_candidates_are_bounded_by_buffer_count (redb.rs:3010) | STRONG | 0-byte candidates still consume buffer slots (2/2 → `Capacity`); converges after drops; counters zero | — | — |
| staging_denial_after_staged_remove_rolls_back_durable_payload (redb.rs:3030) | STRONG | capacity denial *after* staged `table.remove` → whole txn rolls back (`Present{1}`, claim alive); retry converges after lease release | — | commit txn despite staging denial → `Absent` → red |
| concurrent_writers_at_record_cap_admit_exactly_one_transaction (redb.rs:3060) | STRONG | cap 1, 2 barriered writers: exactly 1 `Ok` (Present) + 1 `NotCommitted(Capacity)` (Absent); unexpected shapes panic; drain byte-equal | — | — |
| concurrent_prepare_restore_issues_exactly_one_live_claim (redb.rs:3107) | STRONG | one `Some(candidate)`, one `OperationInProgress`; two claims escaping panics; lease counters zero at end | — | — |
| concurrent_recovery_claims_are_generation_fenced_to_latest_owner (redb.rs:3148) | STRONG | 2 recoverers get strictly ordered generations; older `Retained`; latest commits+finalizes; budget counters 0/0 | — | — |
| tighter_reopen_rejects_existing_inventory_without_erasing_it (redb.rs:3196) | STRONG | tighter caps → `Capacity`; file untouched; reopen restores `b"four"` byte-for-byte | — | erase-on-deny → red |
| per_record_limit_is_checked_on_reopen_even_when_total_still_fits (redb.rs:3213) | STRONG | per-record cap enforced on reopen (4B record vs 3B cap) although total cap would fit; content survives | — | — |
| persisted_payload_usage_mismatch_fails_closed_without_erasing_record (redb.rs:3230) | STRONG | raw-redb fixture corrupts `RETAINED_PAYLOAD_BYTES`→0: open fails `InvalidState`; after manual repair, record restores byte-for-byte (no erase-on-reject) | — | trust recorded counter over scan → green open → red |
| every_invalid_limit_relation_rejects_before_creating_database_file (redb.rs:3264) | STRONG | 6 distinct invalid relations each → `Config` **and** `!path.exists()` (fs authority); no vacuous iteration — every case asserted | — | — |
| capped_file_backend_rejects_extension_before_file_mutation (redb.rs:3309) | STRONG | `set_len(5)`/`write(5B)` over cap 4 rejected with file length unchanged (fs-verified); exact-cap ops succeed | — | — |
| reopening_above_database_file_cap_fails_without_erasing_payload (redb.rs:3331) | STRONG | cap = filesize−1 → `Capacity`; byte length unchanged (fs); reopen + content intact | — | — |
| database_file_cap_denial_preserves_prior_durable_record_across_reopen (redb.rs:3351) | STRONG | 63×512KiB writes until the *physical* cap fires (message + fs length ≤ cap asserted every iteration); anchor + accepted 0xA5 records survive; capacity-denied restore converges via recover path with byte equality; final drain to 0/`None` | — | — |
| partial_storage_full_write_reopens_to_whole_records_only (redb.rs:3457) | STRONG | strict-half write fault; `observed` + `0<written<attempted` asserted; reopen: anchor byte-equal, uncertain record `Present`(whole, byte-equal) xor `Absent`; other shapes panic | — | — |
| partial_storage_full_during_restore_commit_preserves_one_whole_copy (redb.rs:3505) | STRONG | StorageFull mid-commit typed `Capacity` (`Retained`/`Ambiguous`); fault landing asserted; cold reopen → exactly one whole copy; recovery converges | — | — |
| partial_storage_full_during_finalize_reopens_present_or_finalized (redb.rs:3557) | STRONG | fault landing + strict-partial asserted; `let _failure` discards only the *value* (`expect_err` already forces `Err`) — kind not matched, minor; postimage constrained to `Present`(Committed, `b"four"`) xor `Finalized`, both converge to drained | match the failure kind like :3505 does | — |
| unsupported_explicit_schema_versions_fail_closed_before_v2_initialization (redb.rs:3604) | STRONG | versions 0/1/3/u64::MAX → `Unsupported` containing the version; `assert_no_v2_initialization`; version + sentinel row survive | — | — |
| missing_schema_marker_on_v2_shaped_database_fails_without_restamping (redb.rs:3646) | STRONG | v2 evidence w/o marker → `InvalidState` (not silent legacy migration); marker still absent, both identity halves, payload (`b"whole"`) and disposition survive byte-exact | — | restamp schema on reject → red |
| missing_metadata_table_on_v2_shaped_database_fails_without_recreation (redb.rs:3692) | STRONG | deleted meta table → `InvalidState`; table still absent, payload + disposition survive | — | — |
| incomplete_backend_identity_fails_closed_without_replacing_surviving_half (redb.rs:3726) | STRONG | (lo,−),(−,hi),(−,−) all `InvalidState`; surviving halves + schema marker intact | — | regenerate missing half → red |
| schema_v2_missing_required_authority_fails_without_recreating_it (redb.rs:3769) | STRONG | 5 deleted authorities (queue/recovery/state/proof/frontier) each `InvalidState`; each stays deleted after rejection | — | — |
| schema_v2_payload_disposition_mismatch_fails_without_repair (redb.rs:3829) | STRONG | disposition None / Recovery / u8::MAX against queued bytes → `InvalidState`; corruption + payload survive unrepaired | — | — |
| sep23_fup001_legacy_watermark_file_migrates_payload_and_fails_closed (redb.rs:3874) | STRONG | hand-built v1 file: exact migration of payload evidence; ambiguous 0/1/2/4 → `InvalidState` "unresolved" + not rewritable; fresh ids `Absent`; frontier continues at 6; watermark never consulted | — | guess `Finalized` from watermark → red |
| sep23_fup002_stale_generation_rejected_after_claim_steal (redb.rs:3963) | STRONG | stale gen: commit `Retained`, finalize `InvalidState` (even after record gone); winner idempotent retry `Ok` | — | accept stale generation → red |
| sep23_fup002_finalize_proof_is_per_record_not_last_writer (redb.rs:4003) | STRONG | per-record proofs: both claimants' retries idempotent after the other's finalize; both `Finalized` | — | single last-finalized slot → second retry errs → red |
| sep23_fup002_foreign_backend_claim_rejected_on_every_permission_change (redb.rs:4032) | STRONG | all 4 permission ops reject foreign claim (`Retained`/`InvalidState`); both records `Present`; own claims still work; abort releases own | — | skip backend check on any op → red |
| sep23_fup002_backend_identity_stable_across_reopen (redb.rs:4077) | STRONG | identity equality across reopen; recovered candidate bytes `b"stable"`; commit+finalize succeed | — | — |
| backend_identity_and_pending_claim_survive_cold_rename (redb.rs:4094) | STRONG ⚠ | identity equality + pre-rename claim commits/finalizes on renamed lineage. ⚠ restored content never byte-compared (existence enforced only indirectly by commit requiring the queued record) | assert `candidate.bytes() == b"rename"` and restore equality after rename | corrupt payload bytes at rename (presence kept, content changed): row stays GREEN — content-integrity gap, critic-verifiable |
| consistent_cold_copy_preserves_logical_lineage_identity (redb.rs:4113) | STRONG ⚠ | copied lineage keeps identity; same-lineage claim commits/finalizes. ⚠ same content gap as above | assert bytes `b"snapshot"` before commit | corrupt copied payload: row stays GREEN — same gap |
| sep23_audit_finalize_retry_succeeds_while_unrelated_claim_is_live (redb.rs:4139) | STRONG | landed-finalize retry `Ok` while unrelated claim live; live claim + candidate untouched; post-abort retry still `Ok`; record stays exactly `Finalized` | — | mask retry behind active-claim equality → red |
| ambiguous_finalize_probe_uses_own_record_proof_with_foreign_claim_live (redb.rs:4175) | STRONG | tests the probe oracle directly: false preimage, true postimage, false for foreign generation, false for unfinalized record | — | key proof on live claim instead of record → red |
| lost_finalize_and_abort_acks_allow_a_new_claim_before_reconciliation (redb.rs:4202) | STRONG | `claim_after_lost_ack` hook's side effect asserted directly (active claim becomes seq 1 / new generation); stale abort then `InvalidState` | — | — |
| sep23_audit_finalize_proof_does_not_mask_reappeared_recovery_payload (redb.rs:4230) | STRONG | raw fixture re-inserts recovery bytes after finalize: proof must NOT mask contradicting bytes → `InvalidState` | — | return `Ok` on proof hit → red |
| accept_at_exhausted_keyspace_fails_closed_without_overwriting (redb.rs:4255) | STRONG | `seed_key_for_test(u64::MAX)` seam (justified: unreachable otherwise); reserve → `Capacity`; seeded record survives byte-equal | — | wrap keyspace → overwrite → red |

## Fault-injection table

"Guaranteed to fire" = the faulted closure/hook provably executes on the path
the test drives. "Firing asserted" = the test distinguishes fault-arrival from
an unrelated `Err` (side-effect flag, marker, or uniquely-reachable variant).

| test (file:line) | fault kind | guaranteed to fire? | firing asserted? |
| --- | --- | --- | --- |
| redb.rs:2583 (fault a) | pre-transaction error injection (`fail_write_before_transaction`, redb.rs:1160) | YES — one-shot swap at `commit_spill` head after `check_record`; 5B payload < 16MiB cap, id freshly reserved → no earlier check can produce `Err` on this input | INDIRECT — `Err(NotCommitted)` is uniquely attributable here, but no fault-marker match; an unrelated pre-txn failure would masquerade |
| redb.rs:2583 (fault b) | post-commit ack loss (`lose_write_commit_ack`, redb.rs:1336) | YES — commit succeeds in test env | NO — `Ok` receipt identical with the hook removed (outcome-equivalent); the *conversion behavior* is load-bearing though: a probe that returned `Err` on lost ack turns the test red |
| redb.rs:2609 (restore ack) | post-commit ack loss (`lose_restore_commit_ack`, redb.rs:1709) | YES — commit succeeds | YES (effectively) — `Err(Ambiguous)` is producible only by a real commit failure or the injection; commit succeeds here |
| redb.rs:2609 (finalize ack) | post-commit ack loss (`lose_finalize_commit_ack`, redb.rs:1848) | YES | INDIRECT — `Ok` either way if hook removed; broken `finalize_landed` probe → `Err` → red |
| redb.rs:4202 (finalize) | ack loss + next-claim steal (`inject_lost_finalize_ack_with_next_claim`, redb.rs:1862 hook) | YES — finalize commit succeeds, hook then runs `prepare_restore()` | YES — `active_claim()==Some((1,_))` is observable *only* via the hook's minted claim |
| redb.rs:4202 (abort) | ack loss + next-claim steal (`inject_lost_abort_ack_with_next_claim`, redb.rs:1963 hook) | YES | YES — new active generation asserted `≠` stale, stale abort then rejected |
| redb.rs:3457 | strict-half partial write → StorageFull (`PartialWriteBackend`, redb.rs:2257) | YES — armed right before a fresh-record commit whose pages differ on disk | YES — `observed` flag + `0 < written < attempted` strict-partial asserted |
| redb.rs:3505 | same, during `commit_restore` | YES | YES — flag + strict partial + typed `Capacity` |
| redb.rs:3557 | same, during `finalize_restore` | YES | YES — flag + strict partial (failure-kind value discarded, minor) |
| redb.rs:3030 | limit violation: staging-buffer exhaustion *after* staged remove | YES — 2/2 in-flight buffers held by live candidates; `copy_restore_bytes` needs a 3rd | YES — `Err(Retained(Capacity))` + `Present{1}` rollback postimage |
| redb.rs:2982 / 3010 | limit violation: in-flight byte budget / buffer count | YES — budget arithmetic pinned by asserted counters (8/2) | YES — counters asserted exactly before/after |
| redb.rs:3230, 3604, 3646, 3692, 3726, 3769, 3829, 3874, 4230 | raw-redb fixture corruption (meta/table/disposition/watermark tampering) | YES — direct writes to the real database file before open | YES — `InvalidState`/`Unsupported` + postimage equality on the tampered keys |

No encoder-panic or `catch_unwind` fault injection exists anywhere in scope
(A5 N/A); panics occur only as deliberate mutex-poison seeding (service.rs:514)
and assertion failures. No fault coordinate was found that fails to intersect
the write path it names — the track's signature risk (A6) does not materialize
here.

## Track question — durable round-trip content verification

- **Byte-for-byte equality on restore: yes, pervasively.** `restore_backing`
  returns the restored `Vec<u8>` and tests compare against the spilled literal:
  redb.rs:2456, 2472-2474, 2489-2490, 2512, 2568, 2825, 2888, 2916, 2940-2941,
  2958-2961, 3102, 3209, 3226, 3260, 3347, 3403, 3410 (`0xA5`×512KiB), 3439,
  3488-3497, 3548, 3592, 3681, 3715, 3862, 4267; multiset round-trip at 2751.
- **Content verified after cold reopen (not just key existence): yes.**
  Examples: redb.rs:2453-2457 (drop→reopen→restore `b"persisted"`), 2486-2490
  (reopen + resumed-sequence record), 2552-2579 (Committed-phase recovery slot
  reopened, `candidate.bytes() == b"crash-safe"`), 2938-2942, 3196-3210,
  3213-3227, 3331-3348, 3351-3454 (anchor + accepted records across two
  reopens), 3457-3502 / 3505-3554 / 3557-3601 (post-partial-write cold reopen),
  4077-4091. The rename (4094) and cold-copy (4113) lineage tests are the only
  round-trips that never byte-compare restored content — flagged ⚠ above.

## Footer — risk summary

1. redb.rs (55/55 STRONG): fault tests assert fault landing (`observed` flag,
   strict-partial counters, claim side effects) and every denial pole checks the
   persisted postimage across reopen; the only residual gaps are cosmetic
   (finalize fault kind unmatched at :3577) plus the two ⚠ content-unverified
   lineage round-trips.
2. service.rs (18 STRONG, 2 WEAK): the two weak rows are controller-level
   claims whose *count/rollback* oracle is a constant-0 fake (:589) or a
   never-exercised spilled→offline transition on an empty target (:646) — the
   destructive offline-accept-on-non-empty-target failure mode has no negative
   pole anywhere in the file.
3. A4 is clean: all 18 `let _ =` in redb.rs are fs-cleanup or
   `.expect`-guarded receipt discards; the single unguarded discard
   (service.rs:600) is backstopped by a follow-up count assertion that is
   itself the weak oracle flagged in row :589.
