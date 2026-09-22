# T09 deep audit: spill / redb backings / scaling bench / facade public API surface

Scope (all bodies opened and read):
- `crates/quanta-memory-platform-facade/src/spill/mod.rs` (191 lines, 0 tests — trait/marker contracts + 2 `compile_fail` doc guards)
- `crates/quanta-memory-platform-facade/src/spill/service.rs` (857 lines, 20 `#[test]`)
- `crates/quanta-memory-platform-facade/src/spill/backings/mod.rs` (2 lines, re-export only, 0 tests)
- `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs` (1270 lines, 13 `#[test]`)
- `crates/quanta-memory-platform-facade/benches/scaling.rs` (684 lines, 0 `#[test]` — bench harness, 6 emitted operations)
- `crates/quanta-memory-platform-facade/tests/public_api_surface.rs` (685 lines, 22 `#[test]`)

Grep census command:
`grep -rn --include='*.rs' -E '#\[(tokio::)?test\]' <scope>` → **55 hits**
(20 service.rs + 13 redb.rs + 22 public_api_surface.rs + 0 mod.rs + 0 backings/mod.rs + 0 scaling.rs).
Census table below has **55 rows**. No `#[tokio::test]` in scope; all are plain `#[test]`.

Timing source: `docs/bugbash/sep-22-test-optimization-deep/timing/top40.md` (present; 639 tests green).
In-scope entries there: redb rank 1 (10.946s), 3, 4, 7, 9, 10, 11, 12, 13, 14, 15, 17.
No `spill/service.rs` unit test appears in the top-40 (all < 0.225s).

## Census (55 rows)

| # | test (file:line of `#[test]`) | verdict | lens / rationale |
|---|---|---|---|
| 1 | service.rs:497 `poisoned_spill_inventory_returns_error_not_zero` | KEEP | only poisoned-mutex path; cheap |
| 2 | service.rs:512 `unavailable_spill_returns_the_only_handle_without_dropping_it` | KEEP | only Drop-ownership proof (DropProbe); keeper for denial cluster |
| 3 | service.rs:533 `noop_spill_rejects_and_returns_the_handle` | MERGE | exact dup of #8 modulo type param; fold into #8 |
| 4 | service.rs:542 `routed_spill_to_available_target_succeeds_and_counts` | KEEP | canonical happy path + receipt shape |
| 5 | service.rs:572 `routed_spill_rolls_back_the_count_when_accept_fails` | KEEP | only post-preflight accept-failure branch (`AcceptFailingTarget` :555) |
| 6 | service.rs:588 `spill_to_unavailable_target_fails_closed_and_does_not_count` | MERGE | 3rd offline-controller denial; fold into #2 + #4 pair |
| 7 | service.rs:605 `available_controller_succeeds_while_unavailable_controller_rejects` | MERGE | online+offline pair adds nothing over #4 + one offline keeper |
| 8 | service.rs:620 `noop_controller_rejects_without_a_target_and_returns_handle` | KEEP | noop keeper (absorbs #3) |
| 9 | service.rs:629 `routed_spill_after_target_goes_offline_fails_closed_leaving_inventory_unchanged` | DELETE | flawed mechanism: never toggles, builds a fresh offline controller; covered by #10 (service.rs:658) |
| 10 | service.rs:658 `in_memory_target_accept_after_toggle_offline_is_rejected` | KEEP | only real availability-toggle test |
| 11 | service.rs:672 `in_memory_target_write_then_restore_round_trips_the_payload` | MERGE | FIFO dup; keeper #14 (service.rs:739, asserts terminal `None`) |
| 12 | service.rs:689 `in_memory_target_restore_of_missing_handle_is_none_not_an_error` | KEEP | only drained→`None` (vs offline→err) distinction pin |
| 13 | service.rs:705 `in_memory_target_restore_after_offline_fails_closed_leaving_retained_state_unchanged` | KEEP | recovery keeper (offline deny + byte recovery) |
| 14 | service.rs:739 `retaining_target_round_trips_handles_fifo` | KEEP | strongest FIFO oracle (terminal `None` + count 0) |
| 15 | service.rs:753 `retaining_target_offline_rejects_accept_but_state_unchanged` | MERGE | offline-accept dup of #10 across impl; genericize via helper pattern (:785) |
| 16 | service.rs:764 `retaining_target_offline_restore_fails_closed_like_in_memory` | DELETE | subsumed by #17 (service.rs:794, both impls) + #13 (recovery) |
| 17 | service.rs:794 `both_spill_targets_share_the_offline_restore_contract` | KEEP | LSP substitutability proof via generic helper (:785) |
| 18 | service.rs:804 `routed_controller_spill_then_restore_balances_the_spilled_count` | KEEP | only controller-level spill/restore balance |
| 19 | service.rs:821 `concurrent_spills_keep_spilled_count_consistent_then_drain_to_zero` | SHRINK | keep exact-drain oracle; 8×20 threads → 3–4 threads |
| 20 | service.rs:852 `noop_controller_restore_is_none` | MERGE | fold into noop keeper #8 (one noop test: spill-err + restore-None) |
| 21 | redb.rs:952 `create_on_an_invalid_path_is_a_neutral_memory_error` | KEEP | only invalid-path pin; no DB opened, ~ms |
| 22 | redb.rs:959 `durable_spill_target_round_trips_fifo_through_the_trait` | KEEP | canonical durable happy path (top40 2.000s, rank 17) |
| 23 | redb.rs:980 `reopened_backing_resumes_sequence_and_restores_persisted_handles` | KEEP | backing-level reopen keeper (absorbs #31); rank 4, 2.918s |
| 24 | redb.rs:997 `drained_queue_never_reuses_sequence_or_accepts_stale_restore_token` | KEEP | only stale-token / no-resurrection pin; rank 10, 2.300s |
| 25 | redb.rs:1029 `persisted_restore_claim_blocks_duplicate_prepare_and_recovers_after_reopen` | KEEP | Prepared-phase reopen; rank 7, 2.472s |
| 26 | redb.rs:1054 `committed_restore_payload_survives_reopen_until_resident_ack` | KEEP | Committed-phase reopen; pair with #25 covers both phases; rank 3, 2.946s |
| 27 | redb.rs:1086 `injected_write_crash_points_converge_without_duplicate_records` | KEEP | only write-crash injection; rank 9, 2.343s |
| 28 | redb.rs:1115 `injected_restore_and_finalize_ack_loss_preserve_recovery_authority` | KEEP | only restore/finalize-ack-loss injection; rank 15, 2.162s |
| 29 | redb.rs:1149 `generic_restore_is_non_destructive_and_requires_owned_two_phase_path` | KEEP | `Unsupported` contract pin; rank 13, 2.234s |
| 30 | redb.rs:1168 `generic_write_is_non_mutating_and_returns_the_original_payload` | KEEP | `Unsupported` + handle-return pin; rank 12, 2.246s |
| 31 | redb.rs:1183 `reopened_redb_reconciles_controller_count_from_persisted_inventory` | MERGE | reopen dup of #23 at controller layer; add `spilled_count` asserts to #23; rank 11, 2.281s |
| 32 | redb.rs:1213 `concurrent_accepts_lose_no_handle_and_assign_distinct_keys` | SHRINK | suite's slowest (rank 1, 10.946s); 8×25 → 3×10, keep BTreeSet exact-multiset oracle |
| 33 | redb.rs:1253 `accept_at_exhausted_keyspace_fails_closed_without_overwriting` | KEEP | only u64::MAX edge; rank 14, 2.230s |
| 34 | public_api_surface.rs:72 `live_broker_bridge_is_present_only_with_core_live_accounting` | KEEP | only live-bridge presence pin; cheap |
| 35 | public_api_surface.rs:84 `accounting_mode_negative_contracts_remain_in_the_public_source` | SHRINK | keep 2 `compile_fail`-presence checks; drop `!contains` absence greps |
| 36 | public_api_surface.rs:109 `full_and_default_features_do_not_enable_unbudgeted_stores` | SHRINK | brittle TOML/README string-splits; parse TOML or use `cfg!` probes |
| 37 | public_api_surface.rs:235 `prepared_publication_builder_exposes_root_bound_resolution_contract` | KEEP | only root-bound contract in scope file; not spill-dup |
| 38 | public_api_surface.rs:311 `measured_value_round_trips_value_and_size` | MERGE | dup of `shared/measure.rs` unit home; shrink to presence smoke |
| 39 | public_api_surface.rs:328 `portable_presets_cover_every_preset_with_full_class_taxonomy` | SHRINK | count-only oracle; assert one per-preset distinguishing value |
| 40 | public_api_surface.rs:349 `memory_platform_builder_builds_a_broker_from_a_preset` | KEEP | only live build-from-preset pin in scope |
| 41 | public_api_surface.rs:359 `memory_platform_builder_without_config_fails_closed` | KEEP | only live unconfigured-fails-closed pin |
| 42 | public_api_surface.rs:366 `report_only_builder_resolves_without_runtime_authority` | KEEP | report-only keeper (absorbs #43) |
| 43 | public_api_surface.rs:375 `report_only_builder_without_config_fails_closed` | MERGE | fold both directions into #42 |
| 44 | public_api_surface.rs:382 `default_builder_over_cap_store_mutation_is_denied` | KEEP | only over-cap denial + ledger-neutrality in scope |
| 45 | public_api_surface.rs:425 `default_builder_and_report_only_share_override_resolution` | KEEP | only override-parity pin |
| 46 | public_api_surface.rs:465 `platform_builder_installs_reclaimer_before_live_construction` | KEEP | only reclaim-port pin; cheap |
| 47 | public_api_surface.rs:490 `payload_store_builder_yields_a_queryable_store` | KEEP | payload slice wiring pin |
| 48 | public_api_surface.rs:510 `retained_map_builder_exposes_public_charged_path` | KEEP | 73-byte ledger pin for retained_map slice |
| 49 | public_api_surface.rs:525 `explicit_unbudgeted_store_is_a_distinct_product_and_does_not_charge_the_platform` | KEEP | only unbudgeted-product pin |
| 50 | public_api_surface.rs:553 `retained_map_atomic_insert_returns_occupied_value_without_overwrite` | KEEP | only `Arc::ptr_eq` no-overwrite proof |
| 51 | public_api_surface.rs:590 `revocable_builder_exposes_public_charged_path` | KEEP | revocable slice wiring + 57-byte pin |
| 52 | public_api_surface.rs:609 `transcript_builder_exposes_public_charged_path` | KEEP | transcript slice wiring + 236-byte pin |
| 53 | public_api_surface.rs:643 `spill_controller_round_trips_a_handle_via_restore` | MERGE | dup of unit #18 (service.rs:804); keeper is the unit; keep at most a compile pin here |
| 54 | public_api_surface.rs:663 `spill_controller_fails_closed_when_target_unavailable` | DELETE | 5th copy of the offline denial; keeper #2 (service.rs:512) strictly dominates (Drop proof) |
| 55 | public_api_surface.rs:675 `noop_spill_controller_rejects_and_restores_nothing` | MERGE | combo of unit #8 + #20; fold into unit noop keeper |

Row count: 55 = grep count (20 + 13 + 22 + 0 + 0 + 0). Bench `scaling.rs` contributes 0 rows (no `#[test]`).

## Findings

### T09-01 (E) — offline/noop denial asserted 6 times across two files
- Oracle bodies opened: `crates/quanta-memory-platform-facade/src/spill/service.rs:512`, `:533`, `:588`, `:605`, `:620`, and `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:663`.
- Quoted oracle (service.rs:525–528):
  `"denial must return ownership"`
  `let (error, handle) = failure.into_parts();`
  `assert!(matches!(error, MemoryError::Capacity(_)));`
- Mechanism: all six prove "deny with `Capacity` + return handle + count 0". Only :512 adds a branch-distinct fact (DropProbe ownership, count/check-then-drop); only :572–:586 (the `AcceptFailingTarget` test) drives the post-preflight accept-failure branch. The rest vary constants, not branches.
- Action: keep service.rs:512 (ownership) + service.rs:572 (accept-failure) + one offline-controller case (service.rs:588); MERGE :533→:620, :605→(:542+:588), surface :663→DELETE (keeper service.rs:512). Est saving: ~55 lines, 0 branch loss.

### T09-02 (E) — FIFO round-trip proved twice for two in-memory impls
- Bodies: `crates/quanta-memory-platform-facade/src/spill/service.rs:672` vs `:739`.
- Quoted oracle (service.rs:747–749):
  `assert_eq!(target.restore().expect("r1"), Some("first".to_string()));`
  `assert_eq!(target.restore().expect("r2"), Some("second".to_string()));`
  `assert_eq!(target.restore().expect("r3"), None);`
- Mechanism: identical accept×2 → restore×2 → drain shape; only the target type differs, and :739 additionally asserts the terminal `None`.
- Action: MERGE :672 into keeper service.rs:739 (or route both through the generic-helper pattern at service.rs:785). Est saving: ~15 lines.

### T09-03 (E) — offline-restore contract triple-covered; generic helper already wins
- Bodies: `crates/quanta-memory-platform-facade/src/spill/service.rs:705`, `:764`, `:794` (helper `:785`).
- Quoted oracle (service.rs:788–791):
  `matches!(SpillTarget::restore(&target), Err(MemoryError::Capacity(_))),`
  `"offline restore must be a Capacity error for every SpillTarget impl"`
- Mechanism: :794+:785 already prove LSP substitutability for both impls; :705 and :764 both additionally prove post-recovery byte recovery — one recovery variant suffices.
- Action: keep service.rs:794 + service.rs:705; DELETE service.rs:764. Est saving: ~16 lines.

### T09-04 (D) — `routed_spill_after_target_goes_offline…` never goes offline
- Body: `crates/quanta-memory-platform-facade/src/spill/service.rs:629`.
- Quoted oracle (service.rs:639–645):
  `// The controller owns the target by value; take availability offline by`
  `// building a fresh offline controller representing the same transition path.`
  `let offline = RoutedSpillController::new(InMemorySpillTarget::unavailable());`
- Mechanism: the test name promises a transition (spill → toggle offline → deny + inventory unchanged) but constructs a second, always-offline controller; the "inventory unchanged" comparison is against the fresh controller's own count, so the transition path (and any toggle bug) is untested. The comment itself admits the gap.
- Action: DELETE service.rs:629; keeper service.rs:658 (real `set_available(false)` toggle test). Est saving: ~27 lines + removes a misleading green.

### T09-05 (C) — two thread-spawning tests dominate suite wall time
- Bodies: `crates/quanta-memory-platform-facade/src/spill/service.rs:821` (8 threads × 20 spills + full drain) and `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:1213` (8 threads × 25 redb writes + `BTreeSet` drain at redb.rs:1242). Top40: redb.rs:1213 is rank 1 at 10.946s.
- Quoted oracle (redb.rs:1239–1246):
  `assert_eq!(backing.retained_count().expect("len"), expected, "no accept was lost");`
  `assert!(drained.insert(h), "a handle was restored twice (key collision)");`
  `assert_eq!(drained.len() as u64, expected, "every accepted handle restored exactly once");`
- Mechanism: cost is thread fan-out + (for redb) 200 serialized write txns + full drain; the race covered (distinct keys, no loss) already manifests at 3–4 threads.
- Action: SHRINK both to 3–4 threads × 10 (keepers stay at service.rs:821 / redb.rs:1213, same drain oracles); run 8-wide only under nightly stress. Est saving: ~7s wall per full run (rank-1 test alone).

### T09-06 (C) — redb reopen tests pay ~2–3s each; two overlap
- Bodies: `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:980` (rank 4, 2.918s) vs `:1183` (rank 11, 2.281s).
- Quoted oracle (redb.rs:991–993):
  `assert_eq!(receipt, SpillReceipt::Durable { sequence: 1, inventory_after: 2 });`
  `assert_eq!(restore_backing(&backing).expect("r1"), Some(b"persisted".to_vec()));`
- Mechanism: wall time is redb create/close/reopen IO, not assertions; both tests reopen one DB and assert sequence-resume + FIFO restore. :1183 adds only controller-level `spilled_count` reconciliation through `RoutedSpillController`.
- Action: MERGE redb.rs:1183 into keeper redb.rs:980 by adding two `spilled_count` asserts to the reopened backing half. Est saving: ~2.3s wall + ~25 lines; retains both-layer coverage in one DB open.

### T09-07 (A) — redb test helpers duplicated in near-identical pairs
- Bodies: `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:895` (`restore_backing`) vs `:909` (`restore_controller`); `:925` (`spill_backing`) vs `:937` (`spill_controller`).
- Quoted oracle (redb.rs:925–927):
  `let record_id = backing.reserve_record_id()?;`
  `backing.commit_spill(record_id, bytes).map_err(|failure| match failure {`
- Mechanism: pairwise-identical prepare→commit→finalize / reserve→commit bodies differing only in trait bound; the controller-bounded generic already accepts the backing.
- Action: merge each pair into the generic (`restore_controller`, `spill_controller` keepers at redb.rs:909/:937). Est saving: ~30 lines, zero oracle change.

### T09-08 (B) — 12 tests repeat the temp-file prologue/epilogue by hand
- Bodies: e.g. `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs:961`, repeated at `:982, :997, :1029, :1054, :1086, :1115, :1149, :1168, :1183, :1213, :1253`; helper `temp_db_path` at redb.rs:890.
- Quoted oracle (redb.rs:961–962):
  `let path = temp_db_path();`
  `let _ = std::fs::remove_file(&path);`
- Mechanism: no RAII guard — a failing assert between the two `remove_file` calls leaks the file, and uniqueness rests solely on the static `SEQ` counter (redb.rs:888).
- Action: add a test-only `TempDb` guard owning the path with `Drop: remove_file` (keeper site redb.rs:890); each test becomes one line. Est saving: ~30 lines + fixes leak-on-failure.

### T09-09 (D) — all denials collapse to `Capacity(_)`; denial source is untested
- Bodies: `crates/quanta-memory-platform-facade/src/spill/service.rs:527`, `:593`, `:615`, `:625`; `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:668`.
- Quoted oracle (service.rs:593–596):
  `matches!(err.error(), MemoryError::Capacity(_)),`
  `"unavailable spill must be a Capacity error, got {err:?}"`
- Mechanism: offline ("spill target unavailable", service.rs:195), noop ("spill target not configured", service.rs:155), and accept-failure ("accept always fails", service.rs:562) produce distinct messages but every oracle matches only the variant — a test wiring the wrong denial source still passes.
- Action: add message-substring asserts per denial source in keepers service.rs:512/:572/:588 (e.g. `err.to_string().contains("unavailable")`). Est saving: 0 lines (adds ~3 lines); converts 3 weak oracles to source-specific ones.

### T09-10 (E) — surface spill trio duplicates unit keepers
- Bodies: `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:643` (dup of unit service.rs:804), `:663` (dup of unit service.rs:588 shape), `:675` (combo of unit service.rs:620 + :852).
- Quoted oracle (public_api_surface.rs:667–670):
  `let failure = result.expect_err("an unavailable spill target must fail closed");`
  `assert!(matches!(failure.error(), quanta_memory_platform_core::MemoryError::Capacity(_)));`
  `assert_eq!(failure.into_parts().1, 7, "failed spill returns the only handle");`
- Mechanism: the integration file's value is compiling against the public surface only; the behavioral assertions repeat unit bodies nearly line-for-line.
- Action: DELETE surface :663 (keeper unit service.rs:512); MERGE :643→keeper unit service.rs:804 and :675→keepers unit service.rs:620/:852, leaving at most compile-shape pins (the `names_spill_reexports` fn at public_api_surface.rs:283 already pins the types). Est saving: ~40 lines.

### T09-11 (D/B) — surface meta-tests assert on source text, not behavior
- Bodies: `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:84` (`include_str!(…).contains("```compile_fail")`, `!contains("pub fn charged_against")`, per-service line scan :95–:106) and `:109` (`split_once("\nfull = [")`, `starts_with("default = ")`, README substring at :127).
- Quoted oracle (public_api_surface.rs:89–93):
  `assert!(builder.contains("```compile_fail"), "unset builder build must be rejected");`
  `assert!(!builder.contains("pub fn charged_against"), "no public late/rebinding seam");`
- Mechanism: reformatting, comment moves, feature-list reorder, or README rewrap flip these with no contract change; the `compile_fail` doc examples guarded by :84 already fail at build time when broken (as do the spill seals at `src/spill/mod.rs:31,49`).
- Action: SHRINK :84 to the two `compile_fail`-presence checks, replacing `!contains` absence checks with sealed-trait absence proofs; rewrite :109 to parse `Cargo.toml` (or `cfg!(feature=…)` probes). Est saving: ~0 lines; removes 2 brittle failure modes.

### T09-12 (C) — `scaling.rs` bench: large default matrix, setup inside the measured epoch, no spill coverage
- Bodies: `crates/quanta-memory-platform-facade/benches/scaling.rs:157` (sizes [1000,10000] × threads [1,2,8,32] × reps 5; main loop :279 = 2×5×4 single + 2×4×3×5 contention = 160 scenario runs), `:516` (`begin_allocation_epoch` before `BTreeMap` construction at :517, polluting the must-not-allocate window asserted at :546), `emit` 11-arg at :222–:234.
- Quoted oracle (scaling.rs:546–550):
  `"scheduler admission must not allocate after policy construction"`
  `assert_eq!(allocations.deallocated.load(Ordering::Relaxed), deallocated_before_admission,`
- Mechanism: construction allocations land inside the measured epoch so the "must not allocate" assertion covers setup noise; the bench covers payload/revocable/broker/scheduler only — no spill, redb, prepared-publication, or transcript scenario — so its CI cost buys this track zero signal.
- Action: default CI to `--smoke` (already supported at scaling.rs:159); move map construction before `begin_allocation_epoch` (keeper scaling.rs:511); bundle `emit` args into a struct (keeper scaling.rs:222); add one spill round-trip scenario or document the gap. Est saving: CI bench time (matrix 160→small runs); 0 test rows affected (bench has no `#[test]`).

### T09-13 (D) — magic ledger constants in scope surface tests
- Bodies: `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:521` (`73`), `:604`–`:605` (`57`), `:631`–`:636` (`236` ×3).
- Quoted oracle (public_api_surface.rs:520–521):
  `store.insert_checked("k".to_string(), vec![0u8; 32]).expect("insert");`
  `assert_eq!(platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(), 73);`
- Mechanism: expectations are hand-derived literals with no link to the cost constructors production uses; a formula change yields "expected 73, got 75" with no pointer to the fix.
- Action: derive expectations from the same cost constructors (keeper pattern: `shared/measure.rs` `ByteSized` path), keeping one exact-number test per store and relaxing the rest to `> 0` + refund-to-zero. Est saving: 0 lines now; avoids N× breakage later.

### T09-14 (B/D) — `MeasuredValue` trivials asserted in two files
- Bodies: `crates/quanta-memory-platform-facade/tests/public_api_surface.rs:311` vs `shared/measure.rs` unit home (`into_inner`, caller-size, zero-size).
- Quoted oracle (public_api_surface.rs:319–322):
  `assert_eq!(mv.size(), 7, "MeasuredValue::size must return the recorded size");`
  `assert_eq!(mv.byte_size(), 7, "MeasuredValue charges the declared size via ByteSized");`
- Mechanism: same four accessors, no new behavior at the surface layer.
- Action: MERGE — keeper is `shared/measure.rs` unit; shrink public_api_surface.rs:311 to a presence smoke or delete. Est saving: ~12 lines.

## v1 re-verification (04 track; only findings with bodies in this scope)

| v1 | verdict | note |
|---|---|---|
| F01 unavailable/noop denial cluster (service.rs:513,534,589,606,621) | CONFIRM | all six bodies re-opened (:512,:533,:588,:605,:620 + surface :663); T09-01 subsumes. Note: v1 omits surface :663 — added here |
| F02 FIFO dup (:673 vs :740) | CONFIRM | re-opened both; T09-02 |
| F03 offline-restore triple (:706,:765,:795+:785) | CONFIRM | re-opened all three + helper; T09-03 |
| F07 builder/surface duplication (builder.rs:913/887/758 vs surface :351/:511/:591/:610/:491) | CONFIRM (in-scope half) | surface bodies re-opened; unit counterparts out of scope (not re-opened); T09-10/T09-13 |
| F08 MeasuredValue two-file trivials (measure.rs:241/251/257 vs surface :312) | CONFIRM | surface :311 re-opened; measure.rs counterparts out of scope (cited, not re-opened); T09-14 |
| F10 redb temp-file prologue ×13 (helper :890) | CONFIRM | all 12 call sites + helper re-opened; T09-08 |
| F11 helper pairs (:895/:909, :925/:937) | CONFIRM | all four re-opened; T09-07 |
| F12 concurrency cost (service.rs:822, redb.rs:1214) | CONFIRM (in-scope half) | both re-opened; prepared_publication bodies out of scope; T09-05 |
| F16 bench matrix + epoch pollution (scaling.rs:157,:279,:516) | CONFIRM | all three re-opened; 6 emitted operations confirmed; T09-12 |
| F17 variant-only oracles (:593,:606–:617,:621–:626, surface :668) | CONFIRM with correction | core claim holds (T09-09), but v1's parenthetical "`:589` does not check the handle value" is wrong — service.rs:597 asserts `into_parts().1 == "handle"` |
| F18 source-grep test (surface :85) | CONFIRM | re-opened; T09-11 |
| F19 TOML/README string-split (surface :110) | CONFIRM | re-opened; T09-11 |
| F20 preset/measure presence oracles (surface :329) | CONFIRM | re-opened :328–:344 (count-only); T09-14 + census #39 |
| F21 magic ledger constants (surface :521,:605) | CONFIRM | re-opened; plus :631–:636 third instance found; T09-13 |
| F22 bench `emit` 11 args (:222) | CONFIRM | re-opened; T09-12 |
| F04 publish_pipeline counter ×5 | NOT IN SCOPE | bodies in `publish_pipeline/service.rs`, outside this scope |
| F05 prepared-publication replay/abort/close ×2 | NOT IN SCOPE | bodies in `prepared_publication.rs`, outside this scope |
| F06 builder ledger magic numbers | NOT IN SCOPE | bodies in `shared/builder.rs`, outside this scope |
| F09 missing-binding ×4 | NOT IN SCOPE | bodies in `shared/builder.rs`, outside this scope |
| F13 `prepare_after_busy` spin | NOT IN SCOPE | body in `prepared_publication.rs`, outside this scope |
| F14/F15 lifecycle hostile-drop / 3-way unwind | NOT IN SCOPE | bodies in `shared/lifecycle.rs` / `prepared_publication.rs`, outside this scope |

Adjacent out-of-scope v1 findings (spill-adjacent, bodies NOT in scope, not re-verified):
03-F18 (redb IO tail in `revocable_store/service.rs`), 03-F19 (spill fail-closed pair in `revocable_store/service.rs`),
05-F-11/F-12 (testkit spill conformance/fault helpers). Consistent with T09-05/T09-06 direction; no contradiction found.

## Lens summary
- A (cheap code): T09-07 (helper pairs), T09-12 (`emit` bundle).
- B (cheap test): T09-08 (RAII guard), T09-11 (de-brittle meta-tests), T09-14 (trivial dup).
- C (slow, measured): T09-05 (rank-1 10.946s + service concurrency), T09-06 (reopen ×2 @ ~2.9s/2.3s), T09-12 (bench matrix). No service.rs unit test exceeds 0.225s (absent from top40).
- D (weak oracle): T09-04 (vacuous transition), T09-09 (denial source), T09-11, T09-13 (magic constants).
- E (duplicates): T09-01, T09-02, T09-03, T09-10.

## Unresolved
- Timing file present (`timing/top40.md` used); per-test timings for the 20 `spill/service.rs` unit tests and 22 surface tests are below the 0.225s top-40 cutoff, so no measured data distinguishes them — verdicts among cheap tests rest on branch/oracle analysis, not timings.
- `shared/measure.rs`, `shared/builder.rs`, `publish_pipeline/*`, `revocable_store/service.rs`, testkit suites cited as keepers/counterparts were NOT re-opened (out of scope); keeper `file:line`s for those point at v1-cited lines, not fresh inspection.
- No fix code applied (per instructions).

## Est. total saving
- Lines: ~230 (T09-01 ~55, -02 ~15, -03 ~16, -04 ~27, -07 ~30, -08 ~30, -10 ~40, -14 ~12).
- Wall time: ~9s per full run (T09-05 ~7s + T09-06 ~2.3s from measured top40 ranks).
