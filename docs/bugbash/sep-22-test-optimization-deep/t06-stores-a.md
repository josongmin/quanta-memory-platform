# t06-stores-a — Deep audit: payload_store/service.rs + retained_map/service.rs

Scope: `crates/quanta-memory-platform-facade/src/payload_store/service.rs`
(1344 lines) and `crates/quanta-memory-platform-facade/src/retained_map/service.rs`
(2808 lines). Every `#[test]` body opened and read. No fix code — audit only.

Grep census: `grep -c -E '#\[(tokio::)?test'` →
payload_store/service.rs **27**, retained_map/service.rs **54**, total **81**.
Census table below has **81 rows**. No `#[tokio::test]` in scope (all plain `#[test]`).

Timing source: `docs/bugbash/sep-22-test-optimization-deep/timing/top40.md`
(present) + `timing/full.txt` (last 300 lines only — per-test times for the
other 79 in-scope tests are not visible; see Unresolved).
In-scope tests visible in timing: rank 18
`retained_map::service::tests::concurrent_checked_read_observes_only_complete_old_or_new_payloads_v1`
(1.036s) and rank 31
`payload_store::service::tests::payload_lookup_probe_count_is_n_independent`
(0.285s). Top40 rows 21/22/35 are doc-test harness entries for these files,
not `#[test]` fns — out of census, no action.

Lenses: **A** cheap-code · **B** cheap-test · **C** slow-measured ·
**D** weak-oracle · **E** duplicates. v1 = `docs/bugbash/sep-22-test-optimization/03-facade-stores.md`
(findings F1–F14 in scope); files 01/02/04/05 contain zero in-scope findings
(grepped for payload_store/retained_map/SimplePayload/SimpleRetained — no hits).

## 1. Census (81 rows)

### 1a. payload_store/service.rs (27 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| P01 | canonical_put_deduplicates_and_charges_once | 689 | MERGE | dedup identity + charge-once subsumed by P09; see G01 |
| P02 | payload_lookup_probe_count_is_n_independent | 703 | SHRINK | 2048 inserts for a constant bound; see G02 |
| P03 | payload_delete_reuses_vacant_slots_without_stale_index_alias | 756 | KEEP | only test pinning slot-reuse + index-emptiness invariant |
| P04 | transient_canonical_key_is_not_charged_but_retained_identity_is | 793 | KEEP | only zero-measured/huge-key semantic-cost proof |
| P05 | cost_component_sum_overflow_denies_without_publishing_or_charging | 816 | KEEP | only u64::MAX measurement overflow probe here |
| P06 | measurement_panic_precedes_accounting_and_does_not_poison_state | 832 | KEEP | only ByteSized-panic-before-accounting proof |
| P07 | metadata_equality_reentry_under_read_lock_is_typed_busy_and_preserves_charge | 848 | KEEP | only PartialEq-reentry path |
| P08 | duplicate_canonical_callback_reentry_under_lock_is_typed_busy_and_charges_once | 873 | MERGE | reentry angle folds into P09; see G01 |
| P09 | dedup_hit_preserves_first_context_and_stores_no_second_entry | 912 | KEEP | keeper: first-write-wins context rule |
| P10 | distinct_payloads_each_charge_once | 948 | KEEP | complement of dedup (distinct keys charge independently) |
| P11 | delete_unpinned_releases_bytes_exactly_once | 964 | KEEP | only delete→refund→second-delete-InvalidState chain |
| P12 | over_cap_put_is_fail_closed_and_does_not_consume_ref | 978 | KEEP | only sequence-continuity proof after denial (payload-0) |
| P13 | pin_is_metadata_only_and_pinned_delete_is_rejected | 992 | KEEP | only pin/unpin counting + pinned-delete rejection |
| P14 | missing_delete_and_unpin_without_pin_are_invalid_state | 1013 | KEEP | cheap negative pair, distinct error kind |
| P15 | put_and_delete_during_rebuild_are_fail_closed_then_resume | 1024 | KEEP | only rebuild gate on put+delete with resume |
| P16 | explicit_unbudgeted_store_is_a_distinct_non_ledger_mode | 1051 | KEEP | only unbudgeted-mode proof here (feature-gated) |
| P17 | close_refunds_every_live_entry_exactly_once | 1061 | MERGE | terminal postimage subsumed by P23; see G03 |
| P18 | close_refund_failure_preserves_entries_and_charge_ownership | 1075 | KEEP | second keeper: rollback path |
| P19 | drop_during_rebuild_defers_then_finish_drains_capacity | 1092 | MERGE | drop-during-rebuild folds into P18's gate coverage; see G03 |
| P20 | drop_during_outer_unwind_contains_two_client_panics_and_refunds_all_charges | 1106 | KEEP | only double-panic-during-unwind drop proof |
| P21 | mutation_after_close_is_typed_closed | 1139 | KEEP | only Closed-typed post-close oracle |
| P22 | double_close_is_idempotent_without_double_refund | 1153 | MERGE | idempotence leg folds into P23; see G03 |
| P23 | concurrent_mutation_and_close_has_one_typed_linearized_outcome | 1162 | KEEP | keeper: linearization + terminal postimage |
| P24 | counter_overflow_is_rejected_without_accounting_or_entry_mutation | 1195 | KEEP | only charged_total=u64::MAX fault injection |
| P25 | poisoned_stats_is_error_not_empty | 1207 | KEEP | only mutex-poison → typed-Poisoned proof |
| P26 | source_has_no_public_raw_constructor_or_late_attach | 1221 | KEEP | keep with oracle hardening; see G04 |
| P27 | delete_contains_two_panicking_client_destructors_after_exact_refund | 1235 | KEEP | justified child-process isolation; see G05 |

### 1b. retained_map/service.rs (54 rows)

| # | test fn | line | verdict | rationale |
| --- | --- | --- | --- | --- |
| R01 | retained_map_owner_has_one_keyed_container_and_no_linear_point_path | 756 | KEEP | keep with oracle hardening; see G04 |
| R02 | insert_commits_envelope_bytes | 776 | KEEP | base insert oracle all else builds on |
| R03 | committed_byte_overflow_rejects_before_payload_or_accounting_mutation_v1 | 789 | MERGE | overflow trilogy folds into R31 keeper; see G06 |
| R04 | concurrent_insert_if_absent_has_one_creator_and_never_overwrites_v1 | 827 | KEEP | only 12-writer atomicity proof (12×10k yield loop, cheap) |
| R05 | concurrent_checked_read_observes_only_complete_old_or_new_payloads_v1 | 875 | SHRINK | 1.036s, top40 #18; see G07 |
| R06 | read_hash_callback_reentry_is_typed_busy_and_preserves_exact_postimage | 1046 | KEEP | only read-path Hash reentry proof |
| R07 | sweep_hash_callback_reentry_is_typed_busy_before_refund | 1092 | KEEP | only sweep-plan Hash reentry proof |
| R08 | hash_unwind_during_detached_mutation_restores_owned_state_and_charge | 1130 | KEEP | only hash-unwind state-restore proof |
| R09 | checked_reads_report_typed_poison_after_owner_mutex_poison_v1 | 1193 | KEEP | only 4-endpoint poison fan-out + no-unwrap_or_default guard |
| R10 | put_get_remove_get_none_round_trip_corrects_bytes | 1237 | MERGE | folds into basic_crud table; see G08 |
| R11 | equal_hash_key_duplicate_overwrites_and_does_not_double_count | 1293 | KEEP | only Eq-without-identity law proof |
| R12 | first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1 | 1426 | SHRINK | 2×4096 seeding via helper; see G02 |
| R13 | reinsert_same_key_recharges_to_new_size_not_doubled | 1432 | KEEP | unbudgeted overwrite-recharge (ledger twin is R40) |
| R14 | remove_releases_committed_bytes | 1448 | MERGE | folds into basic_crud table; see G08 |
| R15 | sweep_reclaims_at_least_target_bytes_and_releases_them | 1462 | MERGE | arithmetic re-assert; keepers R18/R48; see G09 |
| R16 | overwrite_preserves_original_insertion_position_for_newest_first_sweep | 1479 | KEEP | only insertion-position-preserved-on-overwrite proof |
| R17 | arbitrary_remove_preserves_remaining_newest_first_sweep_order | 1499 | KEEP | only arbitrary-remove order proof |
| R18 | byte_targeted_sweep_reclaims_full_cost_of_newest_entry | 1516 | KEEP | keeper: newest-first partial-target semantics |
| R19 | byte_targeted_sweep_of_empty_values_reclaims_key_and_index_cost | 1539 | MERGE | zero-value case of R18; see G09 |
| R20 | get_missing_key_is_none | 1557 | MERGE | folds into basic_crud table; see G08 |
| R21 | keys_lists_inserted_keys_in_deterministic_sorted_order | 1563 | KEEP | keeper for sorted-enumeration |
| R22 | entries_lists_pairs_in_same_sorted_order_as_keys | 1578 | MERGE | folds into R21; see G08 |
| R23 | keys_after_remove_drops_the_removed_key | 1594 | MERGE | folds into basic_crud table; see G08 |
| R24 | keys_on_empty_store_is_empty | 1605 | MERGE | folds into basic_crud table; see G08 |
| R25 | uncharged_retained_map_bytes_are_not_reflected_in_a_broker_ledger_snapshot | 1632 | KEEP | keeper: ledger-invisibility mode proof (feature-gated) |
| R26 | typed_value_round_trips_and_accounts_its_own_byte_size | 1667 | KEEP | only generic Clone+ByteSized value proof |
| R27 | measured_any_handle_round_trips_and_accounts_caller_size | 1688 | KEEP | only Arc<dyn Any> + ptr_eq identity proof |
| R28 | charged::empty_value_with_huge_retained_key_is_denied_without_ledger_or_map_mutation | 1767 | MERGE | overflow trilogy folds into R31; see G06 |
| R29 | charged::key_measurement_panic_precedes_accounting_and_preserves_state | 1790 | KEEP | only key-measurement-panic proof |
| R30 | charged::overwrite_preserves_retained_original_key_cost_and_exact_ledger | 1831 | KEEP | only retained-original-key-cost (alias-key) proof |
| R31 | charged::aggregate_cost_overflow_is_denied_before_new_entry_or_ledger_charge | 1852 | KEEP | keeper: strictest u64::MAX overflow probe |
| R32 | charged::panic_in_old_value_drop_cannot_split_committed_bytes_from_ledger | 1890 | KEEP | only overwrite-time Drop-panic ledger-split proof |
| R33 | charged::close_contains_client_destructor_panic_and_keeps_terminal_image | 1928 | KEEP | only close-time Drop-panic containment + Closed gate |
| R34 | charged::drop_during_unwind_contains_two_client_destructor_panics_and_releases_all | 1959 | KEEP | only unwind-time double Drop-panic release proof |
| R35 | charged::sweep_never_hashes_client_key_after_refund | 2056 | KEEP | only post-refund Hash-ordering invariant |
| R36 | charged::sweep_hash_panic_before_refund_preserves_charged_preimage | 2121 | KEEP | only sweep-plan Hash-panic pre-refund proof |
| R37 | charged::panic_in_sweep_value_drop_is_contained_after_exact_refund | 2158 | KEEP | only sweep-time value-Drop containment proof |
| R38 | charged::sweep_multiple_panicking_destructors_does_not_abort_process | 2186 | KEEP | justified child-process isolation; see G05 |
| R39 | charged::rebuild_denied_recharge_preserves_charged_preimage_v1 | 2275 | KEEP | rebuild quartet member (insert leg); see G10 |
| R40 | charged::charged_insert_routes_bytes_through_the_ledger | 2302 | KEEP | keeper: ledger insert/overwrite/remove cycle |
| R41 | charged::atomic_insert_if_absent_never_recharges_or_overwrites_an_existing_entry_v1 | 2335 | KEEP | only occupied-decision-before-charge proof |
| R42 | charged::denied_atomic_insert_if_absent_publishes_neither_payload_nor_charge_v1 | 2366 | MERGE | vacant-denial dup folds into R43; see G11 |
| R43 | charged::charged_insert_over_cap_is_denied_and_fails_closed | 2392 | KEEP | keeper: vacant denial with survivor triple |
| R44 | charged::charged_overwrite_denial_leaves_existing_entry_unchanged | 2424 | MERGE | folds into R45 keeper; see G12 |
| R45 | charged::entry_remains_refundable_after_a_denied_overwrite | 2452 | KEEP | keeper: ticket-liveness via drain-to-zero |
| R46 | charged::charged_overwrite_during_rebuild_is_fail_closed_and_leaves_entry_intact | 2479 | KEEP | rebuild quartet member (overwrite leg); see G10 |
| R47 | charged::charged_sweep_refunds_reclaimed_bytes_to_the_ledger | 2516 | MERGE | arithmetic re-assert; keeper R48; see G09 |
| R48 | charged::charged_full_eviction_leaves_no_entry_key_ticket_or_accounting_residue_v1 | 2539 | KEEP | keeper: key+ticket+ledger+entries.is_empty residue |
| R49 | charged::charged_remove_during_rebuild_is_fail_closed | 2562 | KEEP | rebuild quartet member (remove leg); see G10 |
| R50 | charged::charged_remove_rejects_a_ticketless_entry_without_mutating_state | 2586 | KEEP | only ticketless-remove corruption invariant |
| R51 | charged::charged_sweep_during_rebuild_is_fail_closed | 2615 | KEEP | rebuild quartet member (sweep leg); see G10 |
| R52 | charged::charged_sweep_rejects_a_ticketless_candidate_without_partial_refund_or_eviction | 2655 | KEEP | only ticketless-sweep ordering invariant |
| R53 | charged::uncharged_store_is_explicit_non_budgeted_mode | 2696 | MERGE | folds into R25 mode proof; see G11 |
| R54 | charged::remove_contains_panicking_key_and_returns_value_after_exact_refund | 2707 | KEEP | justified child-process isolation; third spawn in scope |

Verdict totals: KEEP 55 · SHRINK 3 (P02, R05, R12) · MERGE 23 · DELETE 0.
Net proposed: −23 tests folded into keepers, 3 shrunk in place.

## 2. Findings (lenses A–E)

### G01 (E/M) — payload dedup trio overlaps; keeper pins the context rule
File: `crates/quanta-memory-platform-facade/src/payload_store/service.rs:912`
Oracle (912–944):
```
assert_eq!(second.payload_ref(), first.payload_ref());
assert_eq!(store.stats().expect("stats").item_count, 1);
assert_eq!(store.charged_bytes().expect("bytes after"), bytes_before);
```
Mechanism: P01 (689, bare identity + charge-once), P08 (873, callback-reentry
Busy + charge-once) and P09 (912) all end at dedup-hit identity + charge-once.
Only P09 pins first-write-wins context (Session charged, Request zero, 931–944).
Action: MERGE P01 + P08 into P09
(keeper `payload_store/service.rs:912`); carry the reentry-hit assert
(`hits == 1`, 901) as one extra assert in the keeper. Est: −2 tests, ~−60 lines.

### G02 (C/M) — N-independence fixtures oversized (measured: 0.285s)
File: `crates/quanta-memory-platform-facade/src/payload_store/service.rs:703`
Oracle (746–751):
```
assert_eq!(duplicate.payload_ref(), "payload-1024");
assert_eq!(probes.load(Ordering::SeqCst), 2,
    "lookup evaluates only the query and its digest-bucket candidate, independent of N");
```
Mechanism: 2,048 unique inserts (730–737, each a full broker charge + two index
inserts) to prove a constant probe bound; top40 rank 31 (0.285s) confirms it is
the file's slowest non-spawn test. Two N values (e.g. 64 vs 256) prove
N-independence; 2048 adds linear cost to a constant assertion. Same shape in
`retained_map/service.rs:1357` helper (LIVE_ENTRIES 4096, invoked twice by R12
at 1427–1428 = 8,192 inserts for a ≤64-probe bound, 1396–1400).
Action: SHRINK P02 (`payload_store/service.rs:703`) 2048→256 inserts; SHRINK R12
helper (`retained_map/service.rs:1357`) LIVE_ENTRIES 4096→256. Est: ~8–16×
cheaper per test, ≈ −0.25s + helper-dominated R12 time.

### G03 (E/S) — payload close/drop quintet; keeper is the concurrency test
File: `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1162`
Oracle (1190–1191):
```
assert_eq!(broker.ledger_snapshot().expect("linearized ledger").total_owner_bytes(), 0);
assert_eq!(store.stats().expect("linearized stats").item_count, 0);
```
Mechanism: P17 (1061), P19 (1092), P22 (1153), P23 (1162) all end at ledger-0 /
stats-0. P23 subsumes the postimage and adds the linearization guarantee;
P18 (1075, rebuild-rollback) is the second keeper.
Action: MERGE P17 + P19 + P22 into P23
(keeper `payload_store/service.rs:1162`); keep P18 + P20 (distinct rollback and
double-panic-drop paths). Est: −3 tests, ~−50 lines.

### G04 (D/M) — source-grep oracles assert on text, not behavior
File: `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1221`
Oracle (1222–1231):
```
let source = include_str!("service.rs");
assert!(!source.lines().any(|line| line.trim_start().starts_with("pub fn new(")));
```
Mechanism: P26 greps source text for constructor spellings; a macro-generated
constructor, re-export, or reformatting evades or trips it without changing the
public API. Twin: R01 (`retained_map/service.rs:756`, six concatenated
forbidden fragments 758–765 + two positive contains 771–772) — a lawful
LinkedHashMap refactor mentioning `.iter().find(` in a comment fails the suite.
Action: KEEP both (they are the only construction/ownership guards) but harden:
P26 keeper `payload_store/service.rs:1221` → assert on `cargo public-api` /
rustdoc surface or a `compile_fail` probe instead of string prefixes; R01 keeper
`retained_map/service.rs:756` → replace forbidden-substring grep with a
behavioral probe-bound test (the R12 pattern already proves O(1) point ops).
Est: 0 tests removed; removes brittle-failure class.

### G05 (C/M) — child-process spawn tests are the slowest in scope, but justified
File: `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1331`
Oracle (1337–1342):
```
let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
    .arg("delete_contains_two_panicking_client_destructors_after_exact_refund")
```
Mechanism: P27 (1235–1343), R38 (2186–2272, greps stdout for `"1 passed"`,
2264–2271 — couples to harness formatting), R54 (2707–2806) each re-execute the
whole test binary (process spawn + harness startup). Justification is real:
double-panic-during-drop/unwind aborts the parent runner, so isolation is
required — KEEP all three.
Action: KEEP with micro-trim: R38 keeper `retained_map/service.rs:2186` → assert
on `output.status.success()` only, drop the `"1 passed"` stdout grep. Est: no
test-count saving; decouples from harness output format.

### G06 (E/M) — overflow-denial trilogy; keeper is the strictest probe
File: `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1852`
Oracle (1858–1866):
```
store.insert_checked("k".into(), DeclaredBytesV1 { marker: 1, retained_bytes: u64::MAX })
...
Err(MemoryError::Capacity(_))
```
Mechanism: R03 (789, overwrite-overflow + vacant-overflow two-leg),
R28 (1767, huge-key), R31 (1852, u64::MAX) all assert Capacity + unchanged
stats + zero ledger delta.
Action: MERGE R03 + R28 into R31
(keeper `retained_map/service.rs:1852`); carry the huge-key string
(`"k".repeat(1_024)`, 1770) as a second case in the keeper. Est: −2 tests,
~−70 lines.

### G07 (C/L) — slowest test in scope: 4×2000 barrier rendezvous + 4KB clones
File: `crates/quanta-memory-platform-facade/src/retained_map/service.rs:875`
Oracle (957–967):
```
assert_eq!(old_observations.load(...) + new_observations.load(...), READERS * ITERATIONS,
    "every checked read must classify as one complete published payload");
```
Mechanism: 4 readers × 2,000 iterations × two-phase barrier (904/928/935/951),
4 KB clones per epoch (882–884, 936–947), up-to-10k retry spins (906–916/938–947):
~8,000 five-thread barrier round-trips + megabytes of cloning for a binary
old/new classification. Top40 rank 18 (1.036s) — most expensive test in all four
v1 files.
Action: SHRINK R05 (`retained_map/service.rs:875`): ITERATIONS 2000→200,
PAYLOAD_BYTES 4096→64. Torn-read oracle is iteration-count independent
(binary classification + both-generations-observed). Est: ≈ −0.9s wall-clock.

### G08 (B/S) — trivial getter/empty-map tests + sorted-enumeration pair
File: `crates/quanta-memory-platform-facade/src/retained_map/service.rs:1557`
Oracle (1557–1560):
```
let store = store();
assert_eq!(store.get(&"absent".to_string()), None);
```
Mechanism: R20 (1557, 3 lines), R24 (1605), R23 (1594), R10 (1237), R14 (1448)
are 1–2 assert round-trips on an unbudgeted fixture; R22 (1578) re-proves R21's
(1563) sorted order with values attached.
Action: MERGE R10 + R14 + R20 + R23 + R24 into one `basic_crud_round_trip`
table test (keeper pattern at `retained_map/service.rs:1237`); MERGE R22 into
R21 (keeper `retained_map/service.rs:1563`). Est: −6 tests, ~−80 lines.

### G09 (E/M) — sweep sprawl: six tests, two keepers
File: `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2539`
Oracle (2548–2558):
```
let swept = store.sweep(SweepHint { target_bytes: full_cost }).expect("full sweep");
assert_eq!(swept, SweepStats { reclaimed_items: 3, reclaimed_bytes: full_cost });
...
assert!(store.state.lock().expect("state").entries.is_empty(), ...);
```
Mechanism: R15 (1462), R19 (1539), R47 (2516) re-assert reclaimed_items/bytes
arithmetic already pinned by R18 (1516, only newest-first partial-target proof)
and R48 (2539, only key+ticket+ledger+`entries.is_empty()` residue proof).
Action: MERGE R15 + R19 into R18 (keeper `retained_map/service.rs:1516`);
MERGE R47 into R48 (keeper `retained_map/service.rs:2539`). Est: −3 tests,
~−70 lines.

### G10 (B/M, new) — rebuild-denial quartet shares one body shape
File: `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2275`
Oracle (2280–2283):
```
let error = store.insert_checked("key".to_string(), vec![2u8; 20])
    .expect_err("fenced recharge must fail closed");
assert!(matches!(error, MemoryError::InvalidState(_)));
```
Mechanism: R39 (2275 insert) + R46 (2479 overwrite) + R49 (2562 remove) + R51
(2615 sweep) + payload P15 (1024) all run seed → `begin_rebuild` →
assert-InvalidState → postimage-unchanged → `finish_rebuild` (+ resume).
Distinct op gates, identical shape — kept (each pins a different gate) but they
are the file's largest parametrize candidate.
Action: KEEP all four (keepers `retained_map/service.rs:2275/2479/2562/2615`);
optional follow-up: single `rebuild_denies_all_mutations` table test over the
four ops. Est if parametrized: −3 tests.

### G11 (E/S, new) — vacant-denial and mode-proof pairs
File: `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2392`
Oracle (2413–2420):
```
assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), ledger_before, ...);
assert_eq!(store.committed_bytes().expect("committed after"), committed_before);
assert_eq!(store.stats(), stats_before);
```
Mechanism: R42 (2366, vacant atomic-denial) vs R43 (2392, vacant over-cap denial
with survivor): same fail-closed triple; R43 additionally proves survivor
intactness. R53 (2696, uncharged self-accounting) vs R25 (1632, ledger
invisibility): two halves of the same non-budgeted-mode statement.
Action: MERGE R42 into R43 (keeper `retained_map/service.rs:2392`), carrying
the absent-read assert (2380–2382); MERGE R53 into R25
(keeper `retained_map/service.rs:1632`). Est: −2 tests, ~−40 lines.

### G12 (E/S) — denied-overwrite duology; drain-to-zero is the only live-ticket proof
File: `crates/quanta-memory-platform-facade/src/retained_map/service.rs:2452`
Oracle (2469–2475):
```
let removed = store.remove(&"k".to_string()).expect("remove must succeed");
assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 0,
    "the post-denial ticket was live and refundable (no strand)");
```
Mechanism: R44 (2424) stops at "charge unchanged", which a stranded non-live
ticket would also satisfy; R45 (2452) proves the survivor ticket is live by
draining the ledger to zero.
Action: MERGE R44 into R45 (keeper `retained_map/service.rs:2452`).
Est: −1 test, ~−25 lines.

### Lens A (cheap-code) notes — no action
- `retained_cost` oracles (`payload_store/service.rs:533–536`,
  `retained_map/service.rs:729–732`) hardcode `64 + 16` / `24 + 16` without
  naming production constants — confirmed as v1 F2; name summands as test-mod
  `const`s (0 tests removed, drift-diagnosis only).
- White-box fault injection (`payload_store/service.rs:1197`
  `charged_total = u64::MAX`; `retained_map/service.rs:2590–2599`,
  2661–2670 `.charge.take()`) reaches into `store.state` — legitimate for
  corruption invariants (R50/R52), no cheaper production path exists.

## 3. v1 re-verification (scope = F1–F14; F15+ out of scope)

| v1 | verdict | evidence |
| --- | --- | --- |
| F1 source-grep payload P26 | CONFIRM | `payload_store/service.rs:1221–1232` matches v1 description exactly |
| F2 magic-constant oracle | CONFIRM | `payload_store/service.rs:533–536` (`64 + 16`), `retained_map/service.rs:729–732` (`24 + 16`) unlabeled |
| F3 2048-insert probe test | CONFIRM | `payload_store/service.rs:703–753`; top40 rank 31 (0.285s) corroborates "slowest cheap assertion" |
| F4 child-process spawn P27 | CONFIRM | `payload_store/service.rs:1331–1336` current_exe spawn as described |
| F5 dedup trio, keeper 912 | CONFIRM | P09 `payload_store/service.rs:912–945` alone pins Session/Request context split (931–944) |
| F6 close/drop quintet, keeper 1162 + 1075 | CONFIRM | P23 `payload_store/service.rs:1162–1192` subsumes postimage + linearization; P18 covers rollback |
| F7 source-grep retained R01 | CONFIRM | `retained_map/service.rs:756–773`, six concatenated forbiddens + two positive contains as described |
| F8 concurrent read most expensive | CONFIRM | `retained_map/service.rs:875–973` (4×2000 barriers, 4KB clones, 10k spins); top40 rank 18 (1.036s) |
| F9 spawn test R38 | CONFIRM | `retained_map/service.rs:2186–2272` + `"1 passed"` stdout grep (2264–2271) as described |
| F10 4096-entry probe helper | CONFIRM | `retained_map/service.rs:1357–1423` (LIVE_ENTRIES 4096), R12 `1426–1429` invokes twice (0 and 4_095) = 8,192 inserts |
| F11 trivial getter tests | CONFIRM | R20 `1557–1560`, R24 `1605–1609`, R23 `1594–1602`, R10 `1237–1256` as described |
| F12 sweep sprawl, keepers 2539 + 1516 | CONFIRM | R48 `2539–2559` alone asserts key+ticket+ledger+`entries.is_empty()` (2551–2558); R18 `1516–1536` alone pins newest-first partial semantics |
| F13 overflow trilogy, keeper 1852 | CONFIRM | R31 `1852–1867` u64::MAX probe is strictest; R03 `789–824`, R28 `1767–1778` same postimage |
| F14 denied-overwrite trilogy, keeper 2452 | CONFIRM | R45 `2452–2476` alone drains ledger to zero (2469–2475); R44 stops at charge-unchanged |
| 01/02/04/05 in-scope findings | NONE | grepped all four files for payload_store/retained_map symbols — zero hits |

No REFUTE or SUPERSEDE: every in-scope v1 claim reproduced against the opened bodies.

## 4. Unresolved
- `timing/full.txt` holds only the last 300 lines of the suite log, so measured
  per-test times for 79 of the 81 in-scope tests are unavailable; only the two
  tests above appear. All other "slow" labels are by fixture-size reasoning (lens C),
  not measurement.
- Actual wall-clock savings for G02/G07 SHRINKs are estimates; re-run the nightly
  `--report-time` suite after applying to confirm.
