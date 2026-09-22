# T01 — ledger.rs deep audit (28 tests)

Scope: `crates/quanta-memory-platform-core/src/domain/ledger.rs` test module
(`ledger.rs:1033-1865`). Grep count: 28 `#[test]` attributes, 0 `#[tokio::test]`
(all synchronous unit tests). All 28 bodies opened and read in full for this report.

Timing source: `docs/bugbash/sep-22-test-optimization-deep/timing/top40.md`
(present). Only one in-scope test appears in the top-40: rank 19,
`domain::ledger::tests::single_release_preflight_is_bounded_by_affected_slots_at_10k_owners`
at 0.580s. All other in-scope tests are below the 0.225s top-40 floor, i.e.
individually fast; the file's cost is allocation volume (full-map snapshots),
not wall time.

Helpers (not tests, no verdict): `class_budget` (`ledger.rs:1039`),
`ledger_from_policy` (`ledger.rs:1052`), `reserve` (`ledger.rs:1065`),
`assert_axes_balanced` (`ledger.rs:1364`), `reserve_kind` (`ledger.rs:1376`).

## Census (28 rows = 28 grep hits)

| # | Test fn (`ledger.rs:line`) | Verdict | Reason |
|---|---------------------------|---------|--------|
| 1 | `reserve_denied_when_global_hard_limit_would_be_exceeded` (:1083) | KEEP | Canonical global-ceiling enforcement + fail-closed snapshot equality; keeper for the denial cluster |
| 2 | `reserve_for_unconfigured_class_is_denied_without_mutation` (:1100) | KEEP | Only test of the fail-closed zero-budget path (bypasses `ledger_from_policy`); unique setup |
| 3 | `global_ceiling_reserves_headroom_for_non_headroom_classes` (:1113) | KEEP | Merge target for the headroom pair; denial + Headroom-admission in one body |
| 4 | `headroom_usage_does_not_consume_protected_non_headroom_band_twice` (:1132) | MERGE | Same headroom band arithmetic as #3; fold as a third phase into #3 (`ledger.rs:1113`), saving one ledger build |
| 5 | `reserved_floor_protects_class_from_peer_consumption` (:1153) | KEEP | Only coverage of `reserved_bytes` floor; distinct gate from headroom/burst |
| 6 | `double_release_of_a_cloned_ticket_is_rejected_and_does_not_corrupt` (:1191) | KEEP | Canonical invariant-7 test; strongest oracle in the ticket-lifecycle cluster (balance predicates + snapshot eq) |
| 7 | `commit_after_release_is_rejected` (:1216) | MERGE | Same ticket-state machine as #6/#8 with the weakest oracle (no snapshot-eq assertion); fold into a single lifecycle test kept at `ledger.rs:1191` |
| 8 | `second_commit_of_same_reservation_is_rejected` (:1229) | MERGE | Same as #7; duplicate-commit phase of the lifecycle test kept at `ledger.rs:1191` |
| 9 | `ticket_id_exhaustion_fails_without_reuse` (:1244) | KEEP | Only `next_ticket_id` exhaustion path; unique `pub(crate)` state poke |
| 10 | `borrow_over_class_cap_is_bounded_by_shared_burst_pool` (:1261) | KEEP | Merge target for the burst cluster; three-phase bound proof (admit/borrow/deny) |
| 11 | `shared_burst_pool_is_shared_across_borrowing_classes` (:1283) | KEEP | Distinct property from #10 (cross-class pool sharing, exact-fill admission); keep both |
| 12 | `borrow_false_still_denies_over_cap_regardless_of_pool` (:1303) | MERGE | 12-line single-denial body; fold as a fourth phase into #10 (`ledger.rs:1261`) with `borrow: false` budget |
| 13 | `owner_hard_cap_still_enforced` (:1318) | KEEP | Only owner-quota test in the file; independent ceiling |
| 14 | `empty_release_batch_is_a_noop_even_during_rebuild` (:1334) | KEEP | Only rebuild-gate test (empty-noop vs non-empty-fenced); unique |
| 15 | `zero_byte_reserve_commits_and_releases_coherently` (:1390) | SHRINK | Real coverage (0-byte ticket lifecycle) but 6+ full-map `snapshot()` clones for a 1-ticket ledger; assert scalars per step, full equality once (see F1) |
| 16 | `overflow_denial_preserves_exact_live_ticket_accounting` (:1428) | MERGE | Same `u64::MAX - 10` + 100-byte denial setup as #18 (:1462); fold into #18 which additionally proves the survivor stays live |
| 17 | `exact_boundary_ticket_survives_first_release` (:1443) | KEEP | Only exact-sum (`u64::MAX`) admission + partial-release survivor; complements the denial pair |
| 18 | `unrepresentable_sum_is_denied_without_mutating_any_axis` (:1462) | KEEP | Merge target for the overflow-denial pair; stronger oracle (releases original to zero) |
| 19 | `batch_release_preflights_every_charge_kind_before_first_ticket_mutates` (:1477) | KEEP | Only kind-slot (not totals) preflight test; deliberately corrupted ledger state is load-bearing |
| 20 | `single_release_preflight_is_bounded_by_affected_slots_at_10k_owners` (:1499) | SHRINK | Measured rank-19 slow test (0.580s); 10k reserves + 9999-ticket drain to assert one `u32 == 3`; shrink to 32 owners, keep 10k behind `#[ignore]` (see F2) |
| 21 | `aggregate_usage_tracks_charge_kinds_and_headroom` (:1530) | KEEP | Only test wiring all three charge kinds + headroom through scalar getters; cheap, no snapshots in the hot path |
| 22 | `snapshot_validation_rejects_unrepresentable_axis_instead_of_saturating_it` (:1574) | KEEP | Only direct `validate_exact` unit; `catch_unwind` on debug-overflow panic is intentional per v1 handoff |
| 23 | `reserve_above_i64_max_does_not_wrap_to_negative_delta` (:1591) | KEEP | Regression guard for the `bytes as i64` cast bug; unique magnitude path |
| 24 | `ticket_from_one_ledger_cannot_release_on_another` (:1626) | KEEP | Only cross-ledger id-collision test; unique two-ledger setup |
| 25 | `ticket_ids_are_monotonic_and_unique_across_many_reserves` (:1654) | SHRINK | 64 reserves with sizes `0..64` conflate id order with zero-byte handling (v1 F11 confound); use nonzero sizes and 16 tickets |
| 26 | `context_axis_balances_with_owner_after_mixed_sequence` (:1675) | KEEP | Only per-step context==owner third-axis test across 4 contexts; distinct from #21/#27 |
| 27 | `mixed_kind_release_preserves_charge_kind_partition` (:1734) | KEEP | Only kind-slot isolation-on-release test (metadata drain leaves payload intact); distinct from #19 (which tests preflight rejection) |
| 28 | `deterministic_fuzz_preserves_invariants_and_returns_to_zero` (:1785) | SHRINK | Valuable property test (400 steps, drain-to-zero) but full 3-map snapshot + 3 predicates per step; full-balance every 50 steps, scalars per step (see F3) |

Row count check: 28 census rows = 28 `#[test]` grep hits. Verdict totals:
KEEP 19 / SHRINK 4 (#15, #20, #25, #28) / MERGE 5 (#4, #7, #8, #12, #16) / DELETE 0.

## Findings

### F1 (C/S) — full-map `snapshot()` per assertion is the file's main alloc cost

Location: `ledger.rs:1087,1095-1096` (representative; ~50 `ledger.snapshot()` call sites in tests).

Oracle:

```rust
assert_eq!(ledger.snapshot(), before, "denied global reserve must not mutate");
```

Mechanism: `snapshot()` (`ledger.rs:1022-1030`) deep-clones `class_usage` +
`owner_usage` + `context_usage`. Every fail-closed denial test clones twice per
assertion (once for `before`, once for comparison). `zero_byte_reserve…`
(`ledger.rs:1390-1423`) snapshots 6+ times for a 1-ticket ledger where scalar
getters would suffice. At 10k owners (F2) each clone is ~10k entries × 3 maps.
Lenses: C (cheap-code, expensive test hammer) primary, B secondary.

Action: in `zero_byte_reserve…` keep full `assert_eq!(ledger.snapshot(), before)`
only at `ledger.rs:1412` and `ledger.rs:1422`; replace the mid-body
`ledger.snapshot().live_ticket_count` / `total_owner_bytes()` calls
(`ledger.rs:1400-1407`) — which already use scalars — and drop the redundant
`assert_axes_balanced(&ledger.snapshot())` at `ledger.rs:1403` (covered by the
final balance state). Same treatment for the denial one-liners: compare
`total_semantic_bytes()` + `live_ticket_count` where full-map equality adds
nothing. Est. saving: ~50 full-map clones → ~50% of domain test allocs (carries
v1 estimate; wall-time saving is small outside F2 since no other in-scope test
is in the top-40).

### F2 (C/S) — 10k-owner preflight test burns 0.580s to assert one `u32 == 3`

Location: `ledger.rs:1499-1527`
(`single_release_preflight_is_bounded_by_affected_slots_at_10k_owners`).

Oracle:

```rust
assert_eq!(ledger.last_release_preflight_axis_probes, 3,
```

Mechanism: 10 000 `reserve()` calls each doing `format!("owner-{owner}")` +
BTree insertions, then a 9 999-ticket `release_batch` drain
(`ledger.rs:1523`), all to prove preflight is O(affected) not O(all). Measured
rank 19 in `timing/top40.md` at 0.580s — the only in-scope test in the top-40
and the dominant test in this file. The drain phase proves nothing about the
probe count. Lenses: C primary.

Action: shrink the default body to 32 owners (still multi-digit cardinality,
still proves bounded preflight) keeping the keeper assertion at
`ledger.rs:1514-1517`; move the 10k construction behind `#[ignore]` as a
capacity smoke test; delete the `release_batch(&tickets)` drain at
`ledger.rs:1523-1526` (drain-to-zero is already covered by the fuzz test at
`ledger.rs:1855-1863`). Est. saving: ~99% of this test's 0.580s wall time.

### F3 (C/M) — fuzz snapshots 3 maps × 400 steps; maps stay tiny so cost is latent

Location: `ledger.rs:1833-1834` (`deterministic_fuzz_…`, 400-step loop).

Oracle:

```rust
let snap = ledger.snapshot();
assert!(snap.class_owner_balanced(), "inv 1 at step {i}: {snap:?}");
```

Mechanism: each of the 400 steps clones all three usage maps plus 3 balance
predicates, only to feed per-step assertions. With ≤3 owners / 3 classes the
maps stay tiny — moderate today, scales with taxonomy growth. Lenses: C
primary, A secondary (the loop body re-derives class/owner/ctx/kind indexes per
step; hoist the arrays — negligible).

Action: assert on scalar getters (`total_semantic_bytes()` at
`ledger.rs:406`, `non_headroom_bytes()` at `ledger.rs:411`) per step; keep the
full `snapshot()` + `assert_axes_balanced` (keeper, `ledger.rs:1364`) every 50
steps and at the end (`ledger.rs:1859-1863`). Est. saving: ~8× fewer full
snapshots in fuzz (carries v1 estimate; not directly measurable — test is below
the top-40 floor).

### F4 (E/M) — ticket-lifecycle trio (#6/#7/#8) tests one state machine in three bodies

Locations: `ledger.rs:1191`, `ledger.rs:1216`, `ledger.rs:1229`.

Oracle (weakest member, `ledger.rs:1222-1225`):

```rust
matches!(ledger.commit(&ticket), Err(MemoryError::InvalidState(_))),
"committing a released ticket must be rejected"
```

Mechanism: `commit_after_release` and `second_commit_of_same_reservation` each
build a full ledger + reserve + commit/release preamble to assert a single
`InvalidState` with no snapshot-equality or balance oracle, while
`double_release…` (`ledger.rs:1191-1213`) covers the same guard with snapshot-eq
plus `class_owner_balanced` + `owner_partition_balanced`. Lenses: E primary, D
secondary (the two weaker bodies never check postimage).

Action: MERGE #7 + #8 as phases into the keeper `ledger.rs:1191` body
(one reserve → commit → release → assert commit-rejected → assert
second-release-rejected, each with `assert_eq!(ledger.snapshot(), before)`).
Est. saving: −2 test bodies/preambles; oracle strength strictly increases.

### F5 (E/M) — overflow-denial pair shares one `u64::MAX - 10` setup

Locations: `ledger.rs:1428`, `ledger.rs:1462`.

Oracle (keeper, `ledger.rs:1467-1471`):

```rust
reserve(&mut ledger, MemoryClass::EvidenceCache, "other", 100),
Err(MemoryError::Capacity(_))
```

Mechanism: both tests reserve `u64::MAX - 10`, attempt a second reserve, assert
`Capacity` + `snapshot() == before`. `unrepresentable_sum…` additionally proves
the survivor stays live (`ledger.release(&first)` → zero at `ledger.rs:1472`),
strictly dominating `overflow_denial…` whose only delta is committing the first
ticket (committed-vs-uncommitted denial path — one line of difference). Lenses:
E primary.

Action: MERGE #16 into keeper `ledger.rs:1462`; add the committed variant as a
second phase (commit first, deny, snapshot-eq) inside the same body. Est.
saving: −1 ledger build + preamble; keeps both committed/uncommitted paths.

### F6 (E/S) — headroom/burst denials each re-declare the same globals; burst trio folds

Locations: `ledger.rs:1113` vs `ledger.rs:1132` (headroom); `ledger.rs:1261` /
`ledger.rs:1283` / `ledger.rs:1303` (burst).

Oracle (foldable member, `ledger.rs:1309-1313`):

```rust
let denied = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", 700);
"a non-borrowing class cannot use the pool"
```

Mechanism: `borrow_false…` is a 12-line single-denial body whose setup differs
from `borrow_over_class_cap…` only in `borrow: false` + pool size; it adds no
new assertion shape. Same for the headroom pair (#3/#4): one band-arithmetic
property in two ledger builds. The shared-pool test (#11) stays separate (exact-fill
admission is a distinct property). Lenses: E primary, A secondary (magic globals
`1_000`/`10_000_000` re-declared per test — v1 F11).

Action: MERGE #12 into keeper `ledger.rs:1261` as a closing phase; MERGE #4
into keeper `ledger.rs:1113` as a third phase. Introduce `SMALL_GLOBAL = 1_000`
/ `LARGE_GLOBAL = 10_000_000` consts at the top of `mod tests`
(`ledger.rs:1034`) and use them in the touched bodies. Est. saving: −2 test
preambles; readability (kills magic-number drift).

### F7 (B/M) — monotonic-id test conflates id order with zero-byte handling

Location: `ledger.rs:1653-1670`
(`ticket_ids_are_monotonic_and_unique_across_many_reserves`).

Oracle:

```rust
let t = reserve(&mut ledger, MemoryClass::ArtifactCache, "o", i).expect("reserve");
```

with `for i in 0..64u64` — first ticket is a 0-byte reserve.

Mechanism: byte sizes `0..64` mean ticket #1 exercises the zero-byte path
(`ledger.rs:1390` scope) while asserting id monotonicity; a zero-byte regression
would surface here misattributed to ordering. 64 tickets is also more than
needed for a strict-increase + uniqueness claim. Lenses: B primary (weak
isolation), A secondary.

Action: SHRINK keeper body at `ledger.rs:1660-1669` to 16 tickets with nonzero
sizes (`1..=16`), keeping the `BTreeSet` uniqueness assertion at
`ledger.rs:1668-1669`. Est. saving: −48 reserves; kills the zero-byte confound
(v1 F11 sub-item).

### F8 (D/M) — boundary-duplicate cluster: keep both layers, table the shared triples

Locations: unit `ledger.rs:1083`, `:1113`, `:1261`, `:1303`, `:1191` vs
`core/tests/policy_contracts.rs:171-343` (per v1 Cluster 1; contracts file not
re-opened — out of scope for T01, pointer only).

Mechanism (from v1, confirmed by unit-side reads here): near-identical byte
values (600/600, 900/200, 600+50/60, 700, 100_000) driven through `reserve`
vs `reserve_managed_charge`. The contracts versions prove public wiring through
the broker lock; the ledger versions carry strictly stronger
`assert_eq!(snapshot, before)` whole-map oracles. Neither layer dominates.
Lenses: E primary, D secondary.

Action: keep both layers; extract the shared (global, bytes-a, bytes-b) triples
into one table both files read so the pairs cannot drift (v1 Cluster 1 action,
no new evidence against it). Est. saving: prevents drift, not wall time. The
contracts-side keeper lines are pointers from v1, not re-verified here.

## V1 re-verification (scope: findings naming `ledger.rs`)

| v1 finding | Status | Basis |
|------------|--------|-------|
| F1 snapshot-clone alloc cost (`ledger.rs:1022-1030`, ~50 call sites) | CONFIRM | Re-counted: `snapshot()` still clones 3 maps at `ledger.rs:1022-1030`; `zero_byte…` still snapshots 6+ times (`ledger.rs:1395-1422`); only F2's 10k case makes clones large, rest is alloc volume |
| F2 shrink 10k-owner preflight to 32 + `#[ignore]`d smoke | CONFIRM | Measured: rank 19 / 0.580s in `timing/top40.md` — the only in-scope top-40 entry; body still 10k reserves + 9999 drain (`ledger.rs:1499-1527`) |
| F3 fuzz snapshot every 50 steps | CONFIRM | Loop still snapshots per step (`ledger.rs:1834`); maps still ≤3 owners/classes so cost is latent, not measured |
| F4 `class_budget` dup vs `policy_contracts.rs` | CONFIRM | `ledger.rs:1039-1047` still a local 4-arg copy; contracts side not re-opened (out of scope), no evidence of unification |
| F5 `ledger_from_policy` vs contracts permissive builders | CONFIRM | `ledger.rs:1052-1063` mutate-in-place form unchanged; semantic-difference question still open |
| F9 group-enforcement gap (zero `resource_groups` refs in ledger) | CONFIRM (no new evidence) | Out-of-scope for test optimization; still no group test in this file by inspection of all 28 bodies |
| F11 magic numbers + id-order/zero-byte confound | CONFIRM | Globals still re-declared per test; `0..64` sizes still at `ledger.rs:1660`; fixed by F6/F7 actions above |
| v1 Cluster 1 (unit vs boundary dup pairs, keep both + table) | CONFIRM | Unit-side oracles re-read (snapshot-eq strictly stronger); contracts side pointer-only, not re-verified |
| v1 quick-win #3 (`policy.rs` tautological `burstable == 0` tail) | SUPERSEDE (out of scope) | Belongs to T02 scope (`policy.rs:663-689`), not this file |
| v1 quick-wins #4/#5 (Display table, `from_resolved` tests) | SUPERSEDE (out of scope) | Belong to T02 scope (`errors.rs`/`stats.rs`), not this file |

V1 files `02-core-api-application.md` … `05-testkit-integration.md` contain no
findings naming `ledger.rs` unit tests (grep verified); nothing further to
re-verify.

## Lens summary

- A (cheap-code): F3 (hoist loop indexes — negligible), F6/F7 (named consts, nonzero sizes).
- B (cheap-test): F1 (scalar asserts replace full clones), F7 (de-confounded id test).
- C (slow-measured): F2 (0.580s → ~0.01s est.), F1/F3 (alloc volume, below top-40 floor otherwise).
- D (weak-oracle): F4 (lifecycle merge strengthens oracles), F8 (document keeper asymmetry).
- E (duplicates): F4/F5/F6 merges (−5 bodies total), F8 (keep-both with shared table).

## Top quick wins

1. F2: shrink 10k-owner preflight (only measured slow test in scope) — ~0.57s back.
2. F1: scalar asserts replace per-step full snapshots in `zero_byte…` and denial one-liners.
3. F4+F5+F6 merges: −5 near-dup bodies, strictly stronger surviving oracles.

No fix code applied (audit only).
