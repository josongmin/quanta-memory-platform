# Track 1 — core-domain audit (`core/src/domain/` + `core/tests/policy_contracts.rs`)

Scope: `ledger.rs` (1865 lines, ~29 unit tests), `policy.rs` (~14 tests),
`config.rs` (~22 tests), `policy_contracts.rs` (441 lines, 6 unit + 8
`live_control` integration tests). `context.rs`, `errors.rs`, `recharge.rs`,
`stats.rs` contain **zero** `#[test]` bodies (verified by grep — no
`#[test]`/`mod tests` hits in any of the four).

Summary counts: findings 14 (S:3, M:7, L:4). Lenses — A:3, B:3, C:3, D:3, E:2
(some findings carry two lenses; primary lens is listed first).

## Findings

| # | Sev | Lens | Location | Problem | Action | Est. saving |
|---|-----|------|----------|---------|--------|-------------|
| 1 | S | C | `ledger.rs:1022-1030` (snapshot clones 3 maps) × 50 `ledger.snapshot()` call sites in tests | Every fail-closed assertion (`assert_eq!(ledger.snapshot(), before)`) deep-clones `class_usage` + `owner_usage` + `context_usage`. At 10k owners (F3) each snapshot is ~10k entries × 3 maps. `zero_byte_reserve…` (`ledger.rs:1389-1423`) snapshots 6+ times for a 1-ticket ledger. | Add a cheap `assert_unchanged(before: &LedgerSnapshot)`-style helper is not the fix; the fix is a `ledger.assert_snapshot_eq(&before)` that compares without a second clone, or compare `total_semantic_bytes()` + `live_ticket_count` where full-map equality adds nothing. | ~50 full-map clones → ~50% of domain test allocs |
| 2 | S | C | `ledger.rs:1498-1527` (`single_release_preflight_…_at_10k_owners`) | 10 000 `reserve()` calls each doing `format!("owner-{owner}")` + BTree insertions, then a 9 999-ticket `release_batch`, all to assert one `u32 == 3`. The bounded-preflight property could be proven with ~10 owners. | Shrink to 32 owners (still multi-digit cardinality, still proves O(affected) not O(all)); keep one 10k case behind `#[ignore]` as a capacity smoke test. | ~99% of this test's wall time (dominant test in the file) |
| 3 | S | C | `ledger.rs:1784-1864` (`deterministic_fuzz_…`, 400 steps) | Each of the 400 steps calls `ledger.snapshot()` (3-map clone) plus 3 balance predicates; the per-step snapshot exists only to feed the assertions. With ≤3 owners / 3 classes the maps stay tiny — the cost here is moderate today but scales with any taxonomy growth. | Snapshot once per 50 steps + at end, or assert on scalar getters (`total_semantic_bytes()`, `non_headroom_bytes()`) per step and full-balance every 50. | ~8× fewer full snapshots in fuzz |
| 4 | M | A | `ledger.rs:1039-1047` vs `policy_contracts.rs:137-145` (`class_budget` helper defined twice) | Identical 4-arg `class_budget` constructor copy-pasted in unit tests and integration tests; signatures already drifted (`soft`/`hard`/`borrow` vs `reserved`/`soft`/`hard`/`borrow`). | Export one `#[cfg(test)]`/`testkit` helper or accept the 6-line dup with a comment pointing at the canonical copy. | 6 lines dup, prevents drift |
| 5 | M | A | `ledger.rs:1052-1063` (`ledger_from_policy`) vs `policy_contracts.rs:147-162` (`permissive_policy` + `broker_from_residency`) | Two "permissive policy" builders with subtly different semantics (zero-fill-to-global vs `with_class_budget(0,global,global)` per mandatory class). A reader cannot tell whether the difference is load-bearing. | Unify on one constructor; document why `ledger_from_policy`'s mutate-in-place form exists (it preserves `ResidencyPolicy::new` fail-closed zero defaults for non-mandatory paths). | Removes a semantic trap |
| 6 | M | B,D | `policy.rs:663-689` (`borrow_flag_with_zero_shared_pool_is_valid_but_grants_no_burst`) | Tautological oracle: `let burstable = r.shared_burst_pool_bytes; assert_eq!(burstable, 0)` (`policy.rs:687-688`) reads back the field the test itself set to 0. It proves nothing about ledger behavior — the real claim ("class can never oversubscribe") is never exercised with a reserve. | Either drive a reserve over the hard cap and assert denial, or delete the last 6 lines. | Converts a vacuous test into a real one |
| 7 | M | B | `config.rs:508-523`, `593-600`, `648-658`, `716-732` (`validate_accepts_*`, `empty_layer_is_skipped…`) | Five accept-path tests assert only `.is_ok()` / absence without checking any value (e.g. `validate_accepts_well_formed_policy` never checks the floor math; `empty_layer_is_skipped_in_provenance` never checks the resolved value is the manifest default). A validator returning `Ok(())` unconditionally would pass all of them. | Assert at least one resolved value per accept test (global, provenance len, floor). | Closes "unconditionally-Ok passes" hole |
| 8 | M | D | `policy_contracts.rs:114-118` (`memory_error_has_typed_config_and_report_variants`) | Covers `Display` for 2 of 9 `MemoryError` variants (`errors.rs:12-26`); `Capacity`, `Closed`, `OperationInProgress`, `Poisoned`, `InvalidState`, `Unsupported`, `Build` have no Display assertion anywhere in scope. | Table-driven test over all 9 variants asserting prefix strings. | 7 untested Display arms |
| 9 | M | D | ledger enforcement of `resource_groups`/`owner_groups`: zero references in `ledger.rs` (grep clean) | `config.rs:571-732` thoroughly validates group *structure* (13 tests), but no test in scope reserves through a group-capped ledger — because the ledger never reads `resource_groups`. Either enforcement lives elsewhere (then scope has zero coverage of it) or it is missing. | Track-2 owner must confirm where group admission is enforced and add a reserve-level test; if nowhere, file a correctness gap, not a test gap. | Potential missing enforcement |
| 10 | M | D | `stats.rs:33-71` (`from_manifest` / `from_resolved`) | `from_resolved`'s O(n²) `contains`-dedup (`stats.rs:54-59`) and the `"preset"` vs `"manifest"` default-source asymmetry (`stats.rs:37` vs `:61`) have no unit test in scope; only indirect coverage via `builder_emits_effective_budget_report…` (`policy_contracts.rs:100-112`), which never touches `from_resolved`. | Add 2 unit tests: dedup ordering with multi-field provenance, and empty-provenance default source. | Covers dedup + default asymmetry |
| 11 | M | A | Magic numbers across ledger tests: `1_000`, `1_000_000`, `10_000_000`, `u64::MAX-10` repeated in ~20 tests | Same globals re-declared per test; changing one budget invalidates no single source of truth. `ticket_ids_are_monotonic…` (`ledger.rs:1653-1670`) reserves 64 tickets with byte sizes `0..64` conflating id order with zero-byte handling. | Named consts (`SMALL_GLOBAL`, `LARGE_GLOBAL`); use nonzero sizes in the id-order test. | Readability; kills a confound |
| 12 | L | B | `policy.rs:94-122` (`execution_budget_tests`: 2 tests for 5 axes) | `check_helpers_admit_within_cap_and_reject_over_cap` covers all 5 axes in one body — a failure names no axis, and `unlimited_budget_admits_everything` passes `usize::MAX`/`u64::MAX` which on 32-bit targets would truncate via `as u64` casts (`policy.rs:67`). | Split per-axis; use `u64::MAX` uniformly or gate width-dependent cases. | Precision + portability |
| 13 | L | E | `for_preset_scales_caps_with_global_limit` (`policy.rs:523-532`) checks only `ArtifactCache` | Claims "the split is a deterministic function of the global limit" but samples 1 of 13 non-headroom classes. | Loop over all classes like its sibling tests do. | 1-line change, full claim |
| 14 | L | E | `context.rs:28-32` (`MemoryContextHandle::new`), `recharge.rs:5-15` (private types) | Trivial constructor + private error types with no tests — correctly so; listed only to record the deliberate zero. | No action. Keep as-is. | — |

## Duplicate clusters (lens E, keeper pick)

**Cluster 1 — global/headroom/burst/non-borrow/double-release, unit vs boundary (5 pairs).**
Unit: `ledger.rs:1082-1097`, `:1112-1129`, `:1260-1280`, `:1302-1314`,
`:1190-1213`. Boundary: `policy_contracts.rs:171-205`, `:207-238`, `:240-282`,
`:284-307`, `:309-343`. Near-identical byte values (600/600, 900/200,
600+50/60, 700, 100_000) driven through `reserve` vs `reserve_managed_charge`.
**Keeper: the `policy_contracts.rs` versions** — they prove the public wiring
(fail-closed through the broker lock) which units cannot. **Do not delete the
ledger versions**: their `assert_eq!(snapshot, before)` whole-map equality is
strictly stronger than the boundary `total_owner_bytes()` checks. Verdict:
keep both layers, extract the shared (global, bytes-a, bytes-b) triples into
one table both files read, so the pairs cannot drift.

**Cluster 2 — preset isolation, three depths.** `policy.rs:496-521`
(structural: caps ≤ global), `policy.rs:536-600` (band arithmetic:
distributable/headroom reconstruction), `policy_contracts.rs:366-440`
(behavioral: over-cap reserve denied per preset, exact-cap admitted).
**Keeper: all three** — structure, arithmetic, and behavior test different
properties. The real dup risk is `for_preset_is_structurally_valid`
(`policy.rs:487-494`) vs `every_preset_…_is_total…`
(`policy_contracts.rs:37-56`): both validate every preset. **Candidate:
fold the contracts totality loop into the unit test** (it already iterates
`presets()`), saving one 4-preset construction.

## Top-5 quick wins

1. Shrink the 10k-owner preflight test to 32 owners + `#[ignore]`d 10k smoke (F2) — biggest wall-time saving.
2. Stop cloning full snapshots per assertion; compare scalars per step, full maps at boundaries (F1, F3).
3. Replace the tautological `burstable == 0` tail with a real over-cap reserve denial (F6).
4. Table-driven `MemoryError` Display test over all 9 variants (F8).
5. Two `from_resolved` unit tests for provenance dedup + default source (F10).

## Out of scope / handed to other tracks

- Resource-group *enforcement* location (F9): needs track-2 (broker/scheduler) evidence.
- `snapshot_validation_rejects_unrepresentable…` (`ledger.rs:1573-1586`) uses
  `catch_unwind` on a debug overflow panic — intentional, keep.
