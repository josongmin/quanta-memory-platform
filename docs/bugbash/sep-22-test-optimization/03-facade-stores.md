# Track 3 — Facade Stores Test Audit

Scope: `crates/quanta-memory-platform-facade/src/payload_store/service.rs` (1344 lines),
`retained_map/service.rs` (2808 lines), `revocable_store/service.rs` (2506 lines),
`transcript_store/service.rs` (3041 lines). Every `#[test]` body opened and read.
No fix code — audit only.

Lenses: **A** cheap code · **B** cheap tests · **C** slow/inefficient compute ·
**D** weak-oracle quality · **E** duplicates (with keeper pick). Severity **S**mall / **M**edium / **L**arge.

---

## 1. payload_store/service.rs (~30 tests, lines 480–1344)

### F1 (D/M) — `include_str!` source-grep oracle is brittle, not behavioral
`source_has_no_public_raw_constructor_or_late_attach` (payload_store/service.rs:1221–1232)
asserts on source text (`pub fn new(`, `charged_against`, `pub(crate) fn new_budgeted`).
It passes/fails on formatting and comments, and proves nothing about the actual
public API surface (a `pub fn new (` with different spacing, a macro-generated
constructor, or a re-export would all evade it). Same anti-pattern recurs in
retained_map (F7) and revocable (F12).

### F2 (D/S) — `retained_cost` oracle hardcodes magic constants, will drift silently
`retained_cost` (payload_store/service.rs:533–536) re-derives the charge as
`payload + ref.len + 64 + 16`. The `64 + 16` duplicates production constants
(`DIGEST_BYTES*2`, `ENTRY_STATE_BYTES`, payload_store/service.rs:320–329) without
naming them. If production changes a constant, every test using this oracle fails
with a bare numeric mismatch and no pointer to which component moved. Same shape in
retained_map (`retained_cost`, retained_map/service.rs:729–732) and revocable
(`expected_entry_cost`, revocable_store/service.rs:1076–1078).

### F3 (C/M) — 2048-insert probe test is the slowest cheap assertion in the file
`payload_lookup_probe_count_is_n_independent` (payload_store/service.rs:703–753)
inserts 2,048 unique payloads (each paying full broker charge + two index inserts)
to assert a probe counter equals 2 (payload_store/service.rs:747–751). The
N-independence claim needs two N values (e.g. 64 vs 256), not 2048 rows; cost
scales linearly for a constant-bound assertion.

### F4 (C/M) — child-process spawn test re-executes the whole test binary
`delete_contains_two_panicking_client_destructors_after_exact_refund`
(payload_store/service.rs:1235–1343) spawns `current_exe` as a subprocess
(payload_store/service.rs:1331–1336). Correct isolation for a double-panic drop
path, but it is the single slowest test in the file (process spawn + full harness
startup) and its pattern is copied in retained_map (F9) and transcript (F18).

### F5 (E/S) — three dedup tests overlap; keeper covers the context dimension
`canonical_put_deduplicates_and_charges_once` (payload_store/service.rs:689–700),
`duplicate_canonical_callback_reentry_under_lock_is_typed_busy_and_charges_once`
(payload_store/service.rs:873–909), and
`dedup_hit_preserves_first_context_and_stores_no_second_entry`
(payload_store/service.rs:912–945) all assert dedup-hit identity + charge-once.
**Keeper: `dedup_hit_preserves_first_context…` (912)** — it is the only one that
also pins the first-write-wins context rule (Session charged / Request zero,
payload_store/service.rs:931–944). The other two add only the reentry-callback and
bare-identity angles, which are one `assert` each inside the keeper's setup.

### F6 (E/S) — close/drop refund quintet overlaps; keeper is the concurrency test
`close_refunds_every_live_entry_exactly_once` (payload_store/service.rs:1061–1072),
`close_refund_failure_preserves_entries_and_charge_ownership`
(payload_store/service.rs:1075–1089), `drop_during_rebuild_defers_then_finish_drains_capacity`
(payload_store/service.rs:1092–1103), `double_close_is_idempotent_without_double_refund`
(payload_store/service.rs:1153–1159), and
`concurrent_mutation_and_close_has_one_typed_linearized_outcome`
(payload_store/service.rs:1162–1192) all end at "ledger is 0, stats are 0".
**Keeper: `concurrent_mutation_and_close…` (1162)** — it subsumes the terminal
postimage and adds the linearization guarantee; the rest are single-threaded
special cases of the same postimage (keep the rebuild-failure one, 1075, as the
second keeper since it covers the rollback path).

---

## 2. retained_map/service.rs (~55 tests, lines 713–2808)

### F7 (D/M) — source-grep ownership test, same anti-pattern as F1
`retained_map_owner_has_one_keyed_container_and_no_linear_point_path`
(retained_map/service.rs:756–773) greps `include_str!("service.rs")` for six
concatenated forbidden fragments (retained_map/service.rs:758–765) plus two
positive `contains` checks. A lawful `LinkedHashMap` refactor that happens to
contain e.g. `.iter().find(` in a comment fails the suite; a real linear path
spelled differently passes it.

### F8 (C/L) — `concurrent_checked_read…` is the most expensive test in all four files
`concurrent_checked_read_observes_only_complete_old_or_new_payloads_v1`
(retained_map/service.rs:875–973): 4 readers × 2,000 iterations × two-phase barrier
rendezvous, 4 KB payload clones per epoch (retained_map/service.rs:882–884,
936–947), each read spinning up to 10,000 retries
(retained_map/service.rs:906–916). ~8,000 barrier round-trips of 5 threads plus
megabytes of cloning to assert a binary old/new classification. Halving
`ITERATIONS` to 200 and `PAYLOAD_BYTES` to 64 preserves the torn-read oracle at a
fraction of the cost.

### F9 (C/M) — second child-process spawn test (sweep multi-panic)
`sweep_multiple_panicking_destructors_does_not_abort_process`
(retained_map/service.rs:2186–2272) spawns the test binary and greps stdout for
`"1 passed"` (retained_map/service.rs:2264–2271) — couples the test to harness
output formatting. Justification (multi-panic-during-unwind aborts the parent
runner) is real; keep, but it is the file's slowest test alongside F8.

### F10 (C/M) — endpoint probe helper seeds 4096 entries twice per test
`exercise_endpoint_point_operations_without_linear_probe_growth`
(retained_map/service.rs:1357–1423) seeds 4,096 entries, then runs get/overwrite/
remove probe-bound assertions; the single caller
`first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1`
(retained_map/service.rs:1426–1429) invokes it for id 0 AND id 4095 — i.e. 8,192
inserts for a bound (`≤ 64 probes`, retained_map/service.rs:1396–1400) that a
256-entry map would exercise equally well. Cut `LIVE_ENTRIES` to 256.

### F11 (B/S) — trivial getter/empty-map tests add files, not signal
`get_missing_key_is_none` (retained_map/service.rs:1557–1560, 3 lines),
`keys_on_empty_store_is_empty` (retained_map/service.rs:1605–1609),
`keys_after_remove_drops_the_removed_key` (retained_map/service.rs:1594–1602),
`put_get_remove_get_none_round_trip_corrects_bytes`
(retained_map/service.rs:1237–1256). Each is a 1–2 assertion round-trip on an
unbudgeted fixture. Fold into one `basic_crud_round_trip` table test.

### F12 (E/M) — sweep test sprawl: six tests, two keepers
`sweep_reclaims_at_least_target_bytes_and_releases_them`
(retained_map/service.rs:1462–1476), `byte_targeted_sweep_reclaims_full_cost_of_newest_entry`
(retained_map/service.rs:1516–1536), `byte_targeted_sweep_of_empty_values_reclaims_key_and_index_cost`
(retained_map/service.rs:1539–1554), `charged_sweep_refunds_reclaimed_bytes_to_the_ledger`
(retained_map/service.rs:2516–2536), `charged_full_eviction_leaves_no_entry_key_ticket_or_accounting_residue_v1`
(retained_map/service.rs:2539–2559), plus `sweep_hash_callback_reentry_is_typed_busy_before_refund`
(retained_map/service.rs:1092–1127).
**Keepers: `charged_full_eviction…` (2539)** — only test asserting key, ticket,
ledger, AND internal `entries.is_empty()` residue (retained_map/service.rs:2551–2558);
**`byte_targeted_sweep_reclaims_full_cost_of_newest_entry` (1516)** — only test
pinning newest-first partial-target semantics. The rest re-assert
reclaimed_items/bytes arithmetic on the same path.

### F13 (E/M) — over-cap/overflow denial tested three times in three costumes
`committed_byte_overflow_rejects_before_payload_or_accounting_mutation_v1`
(retained_map/service.rs:789–824), `empty_value_with_huge_retained_key_is_denied_without_ledger_or_map_mutation`
(retained_map/service.rs:1767–1778, charged mod), and
`aggregate_cost_overflow_is_denied_before_new_entry_or_ledger_charge`
(retained_map/service.rs:1852–1867, charged mod) all assert Capacity + unchanged
stats + zero ledger delta. **Keeper: `aggregate_cost_overflow…` (1852)** — the
`u64::MAX` declaration is the strictest overflow probe; fold the huge-key variant
into it as a second case.

### F14 (E/S) — denied-overwrite trilogy; last test is the only end-to-end proof
`charged_overwrite_denial_leaves_existing_entry_unchanged`
(retained_map/service.rs:2424–2449), `entry_remains_refundable_after_a_denied_overwrite`
(retained_map/service.rs:2452–2476), and
`charged_overwrite_during_rebuild_is_fail_closed_and_leaves_entry_intact`
(retained_map/service.rs:2479–2513). **Keeper: `entry_remains_refundable…` (2452)**
— it alone proves the survivor ticket is still live by draining the ledger to
zero (retained_map/service.rs:2469–2475); the first test stops at "charge
unchanged", which a stranded non-live ticket would also satisfy.

---

## 3. revocable_store/service.rs (~46 tests, lines 1143–2506)

### F15 (D/S) — third source-grep oracle
`revocable_full_drain_uses_fifo_deque_without_front_shifts`
(revocable_store/service.rs:1176–1187) asserts `source.contains("entries:
VecDeque<RevocableEntry")` and absence of `"entries" + ".remove(0)"`. Same
brittleness as F1/F7; a `pop_front` performance regression (e.g. `remove(1)`,
drain-then-reinsert) passes the grep while reintroducing quadratic drain.

### F16 (B/M) — `reclaim_until` family is four tests for one function + an exact dup pair
`reclaim_until_frees_at_least_target_and_reports_reclaimed_bytes`
(revocable_store/service.rs:2118–2129), `reclaim_until_on_empty_store_reports_zero`
(revocable_store/service.rs:2132–2137), `reclaim_until_charges_and_evicts_zero_payload_keys`
(revocable_store/service.rs:2140–2155), `reclaim_until_of_zero_payload_entries_reclaims_their_key_and_index_cost`
(revocable_store/service.rs:2158–2168) — four tests differing only in seed sizes.
**Keeper: `reclaim_until_charges_and_evicts_zero_payload_keys` (2140)** — covers
multi-evict + zero-payload key cost + FIFO survivor in one body. Separately,
`reclaim_one_on_empty_returns_none` (revocable_store/service.rs:2111–2115) and
`empty_reclaim_leaves_state_byte_for_byte_unchanged`
(revocable_store/service.rs:2181–2186) assert the identical empty-reclaim None;
**keeper: 2181** (adds the stats-unchanged postimage).

### F17 (E/S) — rebuild-denial trio shares one body shape
`reclaim_batch_failure_preserves_every_entry_and_charge`
(revocable_store/service.rs:2287–2301), `reclaim_one_during_rebuild_is_fail_closed`
(revocable_store/service.rs:2304–2316), `reclaim_until_during_rebuild_is_fail_closed`
(revocable_store/service.rs:2319–2332) — seed N entries, `begin_rebuild`, assert
`InvalidState`, assert item_count + bytes + ledger unchanged, `finish_rebuild`.
Parametrize over (reclaim_one / reclaim_until) in one test; the batch variant is
already covered by the `reclaim_until` leg.

### F18 (C/L) — redb IO tests + 128-spill identity test are the file's slow tail
`durable_redb_round_trip_transfers_charge_exactly_once`
(revocable_store/service.rs:1621–1667), `durable_redb_committed_restore_recovers_after_process_restart`
(revocable_store/service.rs:1671–1720), `durable_redb_prepared_restore_recovers_after_process_restart`
(revocable_store/service.rs:1724–1769), and `denied_durable_restore_leaves_redb_record_authoritative`
(revocable_store/service.rs:1773–1798) each create real temp files
(`qmp-revocable-*-PID-SEQ.redb`), open a redb backing twice (crash-recovery
simulation), and clean up — real filesystem + embedded-DB IO inside unit tests.
`many_spills_share_large_binding_identity_without_unbilled_clones`
(revocable_store/service.rs:1226–1293) builds 64 KB owner/runtime strings
(revocable_store/service.rs:1229–1230) × 128 spill/restore cycles with per-cycle
ledger snapshots. Gate the redb four behind `spill-redb` (already done) AND mark
`#[ignore]`-by-default or shrink to one round-trip + one recovery test; shrink
the 64 KB identity strings to 1 KB (Arc-sharing is proven by `ptr_eq`, not by
string length).

### F19 (E/S) — spill fail-closed pair; mock-target boilerplate (~60 lines each)
`failed_destination_write_preserves_source_entry_and_charge`
(revocable_store/service.rs:1322–1352) vs `noop_spill_does_not_discard_source`
(revocable_store/service.rs:1356–1368): same arrange (two inserts, snapshot total),
same postimage (count + bytes + ledger unchanged + FIFO head intact).
**Keeper: `failed_destination_write…` (1322)** — asserts `spilled_count == 0`
and exercises a real `SpillTarget` impl rather than the canned noop controller.
Adjacent: the durable-mock tests (1409–1617, 1802–2108) each define 40–70 lines
of throwaway `SpillTarget`/`DurableSpillTarget` impls; a shared
`RejectingDurableTarget` fixture would delete ~150 lines.

### F20 (B/S) — single-assert tests that restate the constructor
`constructed_budgeted_store_exposes_its_single_broker_authority`
(revocable_store/service.rs:2476–2480, one `is_some` assert),
`unbudgeted_store_is_a_distinct_explicit_product_type`
(revocable_store/service.rs:2497–2505), `reclaim_until_on_empty_store_reports_zero`
(2132, folded in F16). Merge into a single `construction_modes` test.

---

## 4. transcript_store/service.rs (~50 tests, lines 1229–3041)

### F21 (E/M) — three single-axis conflict tests subsumed by one table test
`same_identity_with_body_drift_is_payload_conflict`
(transcript_store/service.rs:1496–1502), `reused_sequence_with_different_message_id_is_typed_conflict`
(transcript_store/service.rs:1505–1510), `reused_message_id_at_different_sequence_is_typed_conflict`
(transcript_store/service.rs:1513–1518) vs `every_conflict_axis_preserves_rows_indexes_and_accounting`
(transcript_store/service.rs:2742–2765), which fires all three conflicts AND
asserts rows/indexes/ledger preservation that the singles never check.
**Keeper: `every_conflict_axis…` (2742).** Delete the three singles (or keep one
as a doc example); net −3 tests, stronger oracle.

### F22 (E/S) — zero-body large-identity test exists twice (charged vs uncharged)
`zero_body_with_large_ids_and_session_retains_semantic_evidence_cost`
(transcript_store/service.rs:1653–1671, TestUnbudgeted) and
`zero_body_large_identity_and_session_are_charged_until_evidence_retirement`
(transcript_store/service.rs:2414–2438, Budgeted/charged). Same 512/1024-char
seeds, same truncate-to-evidence flow, same `cost(…)` oracle.
**Keeper: the charged version (2414)** — additionally asserts stats and ledger
at every step (transcript_store/service.rs:2422–2431); the uncharged version adds
only the ledger-invisibility angle already covered by the retained_map F#3 pair.

### F23 (C/M) — `racing_close_and_append…` runs 16 serialized race trials
`racing_close_and_append_have_one_linearizable_terminal_image`
(transcript_store/service.rs:3008–3039) loops `for _ in 0..16`, spawning a closer
thread per iteration (transcript_store/service.rs:3013–3018) and retry-closing
afterwards (transcript_store/service.rs:3031). Each iteration is a full broker +
charged store build. 3–4 iterations give the same flake-hunting value; 16 is a
soak test wearing a unit-test name.

### F24 (C/S) — 150 lines of callback-trap scaffolding for four tests
`ComparisonTrap`/`OrderedSessionId` (transcript_store/service.rs:1789–1850),
`CallbackTrap`/`CallbackMessageId`/`callback_id`/`callback_store`
(transcript_store/service.rs:1853–1956) — ~170 lines of fixture before the first
test at transcript_store/service.rs:2015. The traps are precise (panic-at-Nth-
comparison proves no post-detach client `Ord`), but `assert_seed_only_after_append_panic`
(transcript_store/service.rs:2056–2091) reaches into `store.state` internals to
re-assert version/row/evidence counts the public `load` + `retained_bytes` +
ledger triple already covers. Trim the helper to the ledger assertion.

### F25 (D/S) — weak oracles: loose inequalities and 2-row sort proofs
`retention_stats_separate_active_evidence_and_session_costs`
(transcript_store/service.rs:1472–1493) asserts `active_bytes > 0`,
`evidence_bytes > 0`, `session_key_bytes > 0` (transcript_store/service.rs:1478–1480)
instead of exact `cost(…)` values like every neighboring test — the one test
that could catch a miscategorized byte asserts the least.
`ordered_load_sorts_by_caller_sequence_and_allows_declared_gaps`
(transcript_store/service.rs:1770–1776) sorts exactly two rows (7, 2); a reversed
comparator on >2 rows or an insertion-order leak with 3+ rows passes. Add a third
row (e.g. sequence 5) to make the sort proof non-trivial.

### F26 (A/S) — `cost` oracle duplicates the production formula by hand
`cost` (transcript_store/service.rs:1213–1227) re-encodes session/row/evidence
constants (`24 + 16`, `2 * 24 + 2 * 32 + 2 * 16`). It is genuinely independent
(good), but its magic numbers are unlabeled — a future reader cannot tell which
summand is key-identity vs index vs digest without diffing production. Name the
summands (`KEY_IDENTITY`, `INDEX_LINK`, `DIGEST_PAIR`) as `const`s in the test
module. Same note as F2.

---

## Top-5 quick wins

1. **Delete the subsumed transcript conflict trio (F21).** Remove
   `same_identity_with_body_drift…` (1496), `reused_sequence…` (1505),
   `reused_message_id…` (1513); `every_conflict_axis…` (2742) already proves all
   three axes plus index/ledger preservation. −3 slow-neutral tests, +oracle strength.
2. **Collapse the revocable `reclaim_until` 4-pack + empty pair (F16).** One table
   test seeded over (normal / empty / zero-payload / all-zero) keeping body 2140;
   fold 2111 into 2181. −4 tests, same coverage.
3. **Halve the two slowest concurrency tests (F8, F23).** `ITERATIONS` 2000→200 and
   `PAYLOAD_BYTES` 4096→64 in retained_map 875; race loop `0..16`→`0..4` in
   transcript 3009. Cuts wall-clock of the suite's long tail without weakening either
   oracle (binary classification / linearizable terminal image are iteration-count
   independent).
4. **Shrink the N-independence fixtures (F3, F10).** 2048→256 inserts in payload
   732–737; `LIVE_ENTRIES` 4096→256 in retained_map 1358. Both assert constant
   probe bounds; proving them at N=256 is equally valid and ~8–16× cheaper.
5. **Merge the denial/overwrite duplicates, keeping the end-to-end keepers (F13,
   F14, F17).** One overflow test (keeper 1852 + huge-key case), one overwrite test
   (keeper 2452, proving ticket liveness via drain-to-zero), one rebuild-denial
   test parametrized over reclaim_one/until. −5 tests, and the kept assertions are
   the strictly stronger ones.
