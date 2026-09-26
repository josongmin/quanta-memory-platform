# Fail-Open Test Audit — Round Index & Decision Document

> Integration update: the Rust fixes and source-bound proof are being consolidated
> in the local commit containing this notice, together with the gate owner branch.
> Earlier dirty-source and isolated-extraction descriptions below are historical
> receipt boundaries. The census verifier now pins the audited base independently
> of HEAD and accepts the newly tracked helper absent from that base.


## Final filtered disposition — remediation complete

The current decision is **10 retained actual problems resolved**: one reproduced
testkit runtime accounting defect and nine oracle/harness defects. Two separately
identified kit dispatch/composition guards were also strengthened. Production
core/facade failures were not established by this audit.

[Final changes and proof](remediation-verification.md): canonical full repository
gate PASS; 19 baseline GREEN /19 mutant RED /19 restored GREEN; exact one-test
execution in every mutation run; clean isolated-candidate extraction dry-run PASS.
Child selection is guarded at16 launch sites, including the revocable wrapper
found in cross-review. The shared base remains `61e9c7c`; changes are uncommitted.

[Current source census](current-source-census.md): **801 actual rows =801 anchored
rg matches**, all nine scopes. The nine historical findings, critic and this index
exist and are complete. Historical census tables contain795 raw rows, including
one compile pin; they represent794 actual tests before seven new tests were added.

The historical synthesis below is preserved as an audit record. Its broad WEAK,
VACUOUS and critic classifications are not the final fix queue. In particular,
Loom model-only coverage and metrics test-assertion deletion do not establish
broken production behavior; the filtered review explains the exclusions.

---


Synthesis step, 2026-09-26 round. This file (`99-index.md`) is the round's single
decision document: it merges the nine census reports and the mutation-verification
critic report into one confirmed inventory, one fix-verification checklist, and one
set of process notes.

**Inputs (all in `docs/bugbash/sep-26-fail-open-audit/`, cited below by bare
filename):** `findings-core-domain.md` (T1), `findings-core-api.md` (T2),
`findings-app-ports.md` (T3), `findings-revocable-store.md` (T4),
`findings-facade-stores.md` (T5), `findings-spill-redb.md` (T6),
`findings-crash-rails.md` (T7), `findings-pipeline-shared.md` (T8),
`findings-testkit.md` (T9), and the critic report `findings-critic.md`.

**Method constraint honored by this synthesis:** a concurrent FIX PHASE is editing
~15 files under `crates/` plus a new `test_support.rs` in this worktree
(findings-critic.md:150–169). Those files are mid-edit and do not match the audited
state, so this document was produced **only** from the ten report files above; no
`crates/` contents were read or relied on, and nothing outside this one file was
modified. All `file:line` citations below are (report file : line-in-report) unless
the line explicitly names a SUT location (e.g. `ledger.rs:1666`), which is itself
quoted from a report.

---

## 1. Executive summary

**Census.** 795 tests censused across 9 tracks, every track with row-count ==
grep-count and zero sampling (each report's own header; critic re-verified all nine,
9/9 pass — findings-critic.md:62–83).

| track | scope | tests | STRONG | WEAK | FAILOPEN | VACUOUS |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| T1 | core `domain/` + `tests/policy_contracts.rs` | 97 | 89 | 7 | 1 | 0 |
| T2 | core `api/` + `tests/public_api_surface.rs` | 145 | 134 | 9 | 1 | 0 |
| T3 | core `application/` + `adapters/` + `ports/` | 55 | 49 | 4 | 1 | 1 |
| T4 | facade `revocable_store/` | 72 | 69 | 3 | 0 | 0 |
| T5 | facade `transcript_store/ retained_map/ payload_store/` | 151 | 149 | 2 | 1 | 0 |
| T6 | facade `spill/` (service + redb backing) | 75 | 73 | 2 | 0 | 0 |
| T7 | crash/durability rails (4 integration test files) | 29 | 29 | 0 | 0 | 0 |
| T8 | facade `publish_pipeline/ shared/` + facade surface tests | 84 | 79 | 5 | 0 | 0 |
| T9 | `quanta-memory-platform-testkit` crate | 87 | 78 | 1 | 2 | 6 |
| **total** | | **795** | **749** | **33** | **6** | **7** |

Per-track tallies are each auditor's own header count (findings-core-domain.md:25,
findings-core-api.md:31–32, findings-app-ports.md:32, findings-revocable-store.md:24,
findings-facade-stores.md:21, findings-spill-redb.md:51–52, findings-crash-rails.md:77,
findings-pipeline-shared.md:36, findings-testkit.md:28). T2's 145 census rows include
one **non-test compile-pin row** (`names_every_public_reexport`, surface:96, a
`#[allow(dead_code)]` resolution pin; the 9th grep hit in its surface file is a
doc-comment false positive at line 11) which T2's header counts outside its 134
STRONG — disclosed in the report (findings-core-api.md:21–40, :361) and independently
corroborated by the critic and by cargo's executed count of 8 (findings-critic.md:74–81).

**Critic verification outcome** (findings-critic.md:87–100, 109–121):

- **11 mutations measured.** All 6 FAILOPEN mutations run → **6/6 CONFIRMED**
  (broken SUT, accused test scoped-run **GREEN** — rows 1–6).
- **3/3 VACUOUS representatives CONFIRMED** (rows 7–9: metrics zero-assert body,
  dead `violated_families()` loop, loom file with zero production references).
- **2 WEAK samples CONFIRMED green** (rows 10–11: facade reclaimer silently
  discarded; class-cap "strict" isolation asserted as `<=`).
- **1 WEAK sample SKIPPED** (row 12, T6 `spill/service.rs:589`) — the target file
  was dirty with the concurrent fix phase's uncommitted work; mutating it would
  have forced an unsafe `git checkout --` restore (findings-critic.md:100, :174–175).
- **Census audit: 9/9 pass** (findings-critic.md:62–83).

**REJECTED findings: none.** Every executed mutation produced exactly the
auditor-predicted fail-open (findings-critic.md:123–126: "Rows measured but NOT
confirmed: none"). Accordingly, no auditor finding is retracted here, and nothing
that a report flagged appears in this index as "not a problem".

**T2 decision-rule nuance (the one place the brief's rule needed re-judging).**
Critic row 2 ran three configurations (findings-critic.md:90):

- **Config A** — mutated SUT (allocator-resident preferred in `classify_pressure`) +
  current test (never calls `observe_allocator`): **GREEN**. This is the direct
  fail-open proof, matching T2's prediction verbatim (findings-core-api.md:44–70).
- **Config B** — mutated SUT + test augmented with T2's own minimal fix
  (`observe_allocator(resident_bytes: Some(999_999))` then assert `Soft`):
  **RED** (`left: Fatal, right: Soft`). The brief's rule maps "B red → REJECTED
  (it bites via that path)", but B's redness is *precisely what T2's report
  predicted* ("with the minimal fix applied the same mutation turns it red",
  findings-core-api.md:67–70). B is the proof **the FIX bites**, not a refutation.
- **Config C** — clean SUT + augmented test: **GREEN**. The fix is sound on a
  correct SUT.

By the mission's own criterion (SUT broken + current test green) the row stands
**CONFIRMED**, with the auditor's minimal fix validated by configurations B/C. The
only RED observed in the entire critic run is this fix-bites proof
(findings-critic.md:123–126).

**Headline reading.** The fail-closed core contract is genuinely well defended —
T7 crash rails are 29/29 STRONG with zero findings, and T4/T6 have no FAILOPEN or
VACUOUS rows. The confirmed fail-open surface concentrates in exactly two places:
(1) **the ledger/conformance authority seam** — one ledger complexity-oracle that
measures map cardinality instead of probe work, and a testkit whose host-facing
conformance entries are positive-only (Tier 1); (2) **assertion-strength gaps** on
the public API surface and store/spill/pipeline behavior (Tiers 2–3), each with a
small, pattern-copying fix.

---

## 2. Confirmed fail-open inventory, ranked by risk tier

Legend: **FO** = FAILOPEN, **VAC** = VACUOUS, **WK** = WEAK. "Critic row" cites the
mutation table in findings-critic.md:87–100. Rows without a critic row are
auditor-reported with the report's own proposed demonstration; the critic's P2
budget covered only 3 WEAK samples, so **30 of 33 WEAK rows are auditor findings,
not critic-measured** — they are listed for fix planning, not as mutation-proven.

### Tier 1 — ledger fail-closed & conformance authority

| # | verdict | test (SUT file:line) | proving mutation (critic) | one-line fix |
| --- | --- | --- | --- | --- |
| 1.1 | **FO** | `single_release_preflight_is_bounded_independent_of_owner_cardinality` — `ledger.rs:1666`; oracle counter at `ledger.rs:899–903`, assert at `1646–1649` (findings-core-domain.md:70, 87–115) | **Row 1**: owner preflight loop rewritten from keyed `owner_usage.get(owner)` to a full `for (owner, usage) in &self.owner_usage` scan × 3 kinds (`ledger.rs:914`, O(cardinality)) → test **GREEN at both 32 and 128 owners** — the counter derives from refund-map cardinality, not from real lookups | Increment the probe counter inside the actual probe loops (count real axis lookups, or count `BTreeMap::get` via a `#[cfg(test)]` hook); add a cardinality-scaling pole |
| 1.2 | **FO** | `testkit_runs_conformance_against_its_fake_and_passes` — testkit `fixtures/mod.rs:119`; the accused seam is `MemoryTestKit::run_conformance` at `fixtures/mod.rs:59` (FX:59, gap G1; findings-testkit.md:139, :245) | **Row 5**: `run_conformance` body stubbed to `ConformanceVerdict::Passed` (no suite run) → **GREEN**; the kit's advertised host-facing conformance entry can be hollowed out with its only driving test staying green | Add a kit-level negative pole: a `with_broken_fidelity(...)` knob (or test hook) so `run_conformance` can be driven to `Failed` through the kit facade; assert it |
| 1.3 | **FO** | `testkit_runs_facade_store_suite_against_live_broker_and_passes` — testkit `fixtures/mod.rs:158`; slice folding at `suites.rs:683–686` inside `run_facade_store_suite` (`S:681–689`, gaps G2/G5; findings-testkit.md:143, :246, :249) | **Row 6**: the four `verdicts.extend(run_{payload_store,retained_map,transcript,spill}_slice(...))` lines deleted (lifecycle suites kept) → folded verdict still **GREEN**; nothing asserts slice coverage, and the `from_assertions` empty-guard (CM:140) only catches *total* removal | Assert the expected verdict-name set (or count) emitted by `run_facade_store_suite` — e.g. all 8 slice verdict names + the 5×4 lifecycle names (name list in findings-testkit.md:143) |
| 1.4 | gap holes | **(a)** `RebuildWriteFence` family never violated in any self-test — checking site `suites.rs S:497–541`; no fault accepts writes during rebuild (gap-map row 8 / G4; findings-testkit.md:196, :248). **(b)** 6 of 8 facade-slice verdicts positive-only — `payload_dedup_refund` S:1004–1063, `pin_safe_delete` S:1064–1086, `retained_map_refunds` S:1163–1179, `retained_atomic_recharge_overwrite` S:1134–1161, `transcript_truncate_refund` S:1264–1273, `transcript_over_cap_fail_closed` S:1274–1286 (slice table findings-testkit.md:230–239; "6 of 8" per footer :273). **(c)** reclaim/pin/rebuild fault drivers lack rubber-stamp poles — drivers at `S:58–82`, `S:88–124`, `S:130–152`; only spill has poles (G3; findings-testkit.md:247) | Not individually mutation-measured; these are green-by-absence (no pole exists to fire). Adjacent seam measurements: critic rows 5–6 prove the kit-level positive-only pattern. Proposed demonstrations: a `RebuildAcceptingTarget` fake (verdict must fire, gap row 8); a verifier-weakening of any of the 6 slice verdicts (predicate → `true`) flips no test (slice table); an "unconditionally Contained" mutation in the reclaim/pin/rebuild drivers is caught by nothing (G3) | (a) add `RebuildAcceptingTarget` fake asserting the `RebuildWriteFence` family fires; (b) feed one broken store per slice verdict (mirror the BITE fake pattern, findings-testkit.md:219–226) or pin the verdict-name set (1.3 fix); (c) replicate the spill driver's pole pattern (FS:239/282/317) for reclaim/pin/rebuild |
| 1.5 | **VAC** | `conformance_verdict_localizes_to_contract_family_not_a_host_package` — testkit `fixtures/mod.rs:137`; dead loop body at `:141–144` (findings-testkit.md:141) | **Row 8**: loop body replaced with unconditional `panic!("critic: violated_families iterated")` → **GREEN**; the `for family in verdict.violated_families()` loop never iterates on the passing verdict the test builds | Drive a broken kit fixture (delivered by the 1.2 fix) so the loop iterates and every violated family round-trips through `label()`/`Display` — or delete the dead loop |
| 1.6 | **VAC** | all 5 loom models — `loom_models.rs` LM:17, LM:63, LM:92, LM:128, LM:164 (findings-testkit.md:160–164; G9 :253) | **Row 9** (static): `grep -E 'crate::|quanta_memory_platform' loom_models.rs` → **zero matches**; the models are hand-rolled state machines verifying only themselves — no production mutation can turn any of the 5 red | Bind the models to production authorities (e.g. drive `LiveProcessMemoryBroker` under Loom-cfg double-gating) or stop counting them as SUT coverage (mark as protocol-shape documentation) |

### Tier 2 — public API surface & control.rs claims

| # | verdict | test (SUT file:line) | proving mutation | one-line fix |
| --- | --- | --- | --- | --- |
| 2.1 | **FO** | `pressure_classifies_from_semantic_usage` — `control.rs:3434`; oracle at `3437`/`3461`, doc promise at `1887` (findings-core-api.md:44–70, :257) | **Critic row 2, Config A**: `last_allocator_resident: Option<u64>` set by `observe_allocator` and preferred by `classify_pressure` when present → current test **GREEN** (no snapshot ever observed → semantic fallback). Config B (augmented test) RED, Config C GREEN — fix validated | After charging 800k, call `observe_allocator(&AllocatorSnapshot { resident_bytes: Some(999_999), .. })`, assert `classify_pressure() == Soft` (not the allocator's Fatal) and ledger totals still 800_000 — exactly Config B/C |
| 2.2 | **WK** (critic-confirmed) | `for_preset_class_caps_isolate_below_global` — `policy.rs:497` (findings-core-domain.md:125) | **Critic row 11**: caps inflated via `hard_cap_bytes.max(global_hard_limit_bytes)` (mutation at `policy.rs:426`) → **GREEN**; the code's own doc (`policy.rs:388`) promises "strictly below" but the assert is `<=` | Assert `hard_cap_bytes < global` for non-Headroom classes |
| 2.3 | **WK** (auditor, unmeasured) | T1 remainder: `for_preset_scales_caps_with_global_limit` `policy.rs:524`; `resource_group_reserved_has_an_independent_global_floor` `config.rs:922`; `unknown_group_denies_terminally_with_legacy_message` `admission.rs:872`; `residency_policy_tracks_class_quota_and_resource_group_vocabulary` `policy_contracts.rs:59`; `dropping_pending_public_authority_rolls_reservation_back` `policy_contracts.rs:388`; `preset_broker_global_limit_is_consistent_with_enforcement` `policy_contracts.rs:458` (findings-core-domain.md:126, 162, 174, 183, 192, 194) | Proposed mutations in each census row (e.g. joint class+group floor sum at config.rs:922; preflight returning `Ok(())` for unknown groups at admission.rs:872) — all predicted green pre-fix / red post-fix by T1 | Per row: loop every non-Headroom class (524); add class reserved 5 000 so a joint floor rejects (922); add a `preflight()` leg asserting `TerminalDenial` (872); assert exact stored quota/group content (contracts:59); snapshot-eq after Drop incl. `live_ticket_count == 0` (contracts:388); add an at-limit admission probe (contracts:458) |
| 2.4 | **WK** (auditor, unmeasured) | T2 F2–F10: `owner_and_group_soft_caps_each_trigger_the_same_reclaim_lane` control.rs:6282; `reserve_denied_class_hard_cap_no_borrow` control.rs:3025; `reserve_borrow_beyond_shared_pool_denied` control.rs:3068; `owner_without_group_mapping_is_not_gated_by_admission` control.rs:4158; `live_reconfig_class_hard_cap_below_class_usage_denied` control.rs:3180; `live_reconfig_owner_quota_below_owner_usage_is_denied` control.rs:3241; `rebuild_fence_rejects_foreign_clone_finish_and_second_live_begin` control.rs:3803; `sep23_fup003_ticket_exhaustion_blocks_without_callback` control.rs:7313; `report_only_builder_returns_data_without_runtime_authority` core surface:331 (findings-core-api.md:72–178, census rows :245–246, :250, :252, :265, :271, :314, :335, :368) | Proposed mutations per F-row (e.g. strand a phantom zero-byte ticket on denial for F3/F4; non-soft lane for soft crossings for F2; stop setting `ticket_id_space_exhausted` in the broker's `PreflightInputs` for F9) — all predicted green/red by T2 | Per row: assert `trigger == SoftThreshold` both sides (6282); post-denial totals+tickets 0 (3025, 3068); assert ledger 5M then release→0 (4158); copy the 3206 before/after pattern (3180, 3241); rename / compile-fail pin for `fence.clone()` (3803); broker leg poking ticket-id to `u64::MAX` (7313); richer report content or rename (surface:331) |
| 2.5 | **WK** (auditor, unmeasured) | T8, four of five surface rows: `live_broker_bridge_is_present_only_with_core_live_accounting` surface:73; `portable_presets_cover_every_preset_with_full_class_taxonomy` surface:277; `memory_platform_builder_builds_a_broker_from_a_preset` surface:299; `payload_store_builder_yields_a_queryable_store` surface:439 (facade `tests/public_api_surface.rs`; findings-pipeline-shared.md:141, :144, :145, :152). The fifth (surface:414, reclaimer) is ranked into Tier 3 below for its production impact | Proposed mutations per row (opaque-newtype `broker()`; same-count class swap in one preset; inert/unwired platform from `build()`; constant-zero stub stats) — all predicted green by T8 | Per row: type-pin `&LiveProcessMemoryBroker` or one observable (73); assert the class KEY SET equals `mandatory()` + exhaustive-match preset enumeration (277); call one method on the built platform (299); one `put_canonical` + nonzero counters (439) |

### Tier 3 — store/spill/pipeline behavior

| # | verdict | test (SUT file:line) | proving mutation | one-line fix |
| --- | --- | --- | --- | --- |
| 3.1 | **FO** | `canonical_digests_are_randomized_per_store_seed` — facade `payload_store/service.rs:879` (PS:879; findings-facade-stores.md:208, :277–279) | **Critic row 4**: `digest_seed: RandomState` → `BuildHasherDefault<DefaultHasher>` fixed seed in `with_accounting` (every store shares one deterministic, grindable digest) → **GREEN**; the body never constructs a store, it compares two std `RandomState::new()` instances | Build two real stores and compare `digest_seed.hash_one(k)` across them (white-box), or port the RM:1702 probe-count pattern across two stores |
| 3.2 | **WK** (critic-confirmed; T8's highest-value row) | `platform_builder_installs_reclaimer_before_live_construction` — facade surface:414; mutation site `shared/builder.rs:112` (findings-pipeline-shared.md:151, :191–193) | **Critic row 10**: facade `with_reclaimer` discards its argument (`let _ = reclaimer; self`) → **GREEN**; the platform ships with the eviction authority silently unwired, and `attempts == 0` is equally true of a discarded reclaimer | Install a counting `MemoryReclaimer`, trigger one reclaim via the broker, assert the counter incremented (an observable effect, not acceptance) |
| 3.3 | **FO** | `tiny_non_zero_limit_saturates_percent_without_wrapping` — `controller.rs:421`; SUT pipeline at `controller.rs:159–161` (findings-app-ports.md:52, 57–77) | **Critic row 3**: u128 percent pipeline replaced with `used_bytes.wrapping_mul(100)/limit` (u64); `.saturating_mul(100)` variant too → **both GREEN** at limit=1 (both classify u64::MAX ≥98% → Fatal); the wrap under-report (wrapping 0% → Normal at limit=u64::MAX) has no red test — the cheatsheet `qmp-saturating-mul-then-divide` regression is unpinned | Add the large-limit case: `LadderPressureController::new(u64::MAX, ..).classify_semantic(u64::MAX) == Fatal` (wrapping and saturating impls both return Normal there → red) |
| 3.4 | hazard (11 tests) | Subprocess-harness rename-desync — parent re-launches the test binary with a name filter and asserts only `status.success()`; renaming any test fn desynchronizes the filter → child runs 0 tests → exits 0 → parent green (findings-facade-stores.md:23–30, footer :281–284) | Auditor-specified hazard (latent A2/vacuous-iteration), not critic-measured; the ⚠-marked subproc rows are enumerated across the three census tables (TS:1383–3655, RM:1203–3130, PS:1522/1588) | Adopt the model harness's check everywhere: `stdout.contains("1 passed")` (only `sweep_multiple_panicking_destructors_does_not_abort_process` RM:2609, check at :2690, has it) — mechanical, 11 sites |
| 3.5 | gap | Offline destructive-accept uncovered — `routed_spill_after_target_goes_offline_fails_closed_leaving_inventory_unchanged` `spill/service.rs:646` builds a fresh *empty* offline controller, so the named transition never happens; a denial that destroys prior retained inventory on a non-empty target has no negative pole anywhere in the file (findings-spill-redb.md:66, footer :189–193; model row `service.rs:722`) | Auditor-specified demonstration: make `InMemorySpillTarget::accept` drain `store` when unavailable (destructive denial) → :646 stays green (empty target), :675 stays green, nothing in service.rs goes red | Test the real transition: accept 2 handles → `set_available(false)` → denied accept → assert `retained_count()==Ok(2)` and byte-for-byte restore after recovery (copy the :722 model) |
| 3.6 | **VAC** | `emitter_holds_no_accounting_state` — `adapters/metrics.rs:69`; zero asserts in body (findings-app-ports.md:113, 115–140) | **Critic row 7**: emitter gains `events: Arc<Mutex<u64>>` incremented in `emit()` (holds AND mutates accounting-ish state) → **GREEN**; subsumes the sibling compile-marker row metrics.rs:47 | Under `feature = "telemetry"`, install a `metrics_util` debugging recorder, emit a known `MemoryEvent`, assert exactly one counter ("quanta_memory_operation_total" + operation/outcome labels) and one histogram — converts the advisory-lane claim into a behavioral oracle |

### Tier 4 — crash rails: zero findings

T7 audited all 29 crash/durability-rail tests **STRONG — WEAK 0, FAILOPEN 0, VACUOUS 0**
(findings-crash-rails.md:77, :150–154). Coordinate proofs are airtight (post-durable-write
`READY`/`ARMED` handshakes, verified `SIGKILL` signal 9, fail-closed missed-hook exits,
full partial-state detectors). Two cosmetic hardening opportunities only, no verdict
impact: the trivially-true guard at CS:155 and widening the total-only detectors at
HE:590/HE:173 to full `LedgerSnapshot` equality (findings-crash-rails.md:160–163).
**No fix-phase work required for T7.**

### Below tier threshold (remaining WEAK rows, auditor-reported)

For completeness — the remaining 11 of 33 WEAK rows, none critic-measured:

- T3: `ladder_gets_more_aggressive_with_pressure` controller.rs:300 (exact Hard/Fatal ladder vec); `admission_reuses_policy_static_fair_share_table_at_high_group_cardinality` scheduler.rs:520; `emitter_surface_is_emit_only_and_takes_shared_ref` metrics.rs:47 (subsumed by 3.6 fix); `operational_labels_are_closed_bounded_sets` telemetry.rs:203 (label dedup) — findings-app-ports.md:45, :95, :112, :169. Plus the cosmetic `assert_eq!(x,x)` placeholder at controller.rs:417 (:51).
- T4: `spill_and_close_emit_bounded_facade_events` service.rs:1845 (exact event counts, not membership); `constructed_budgeted_store_exposes_its_single_broker_authority` service.rs:4488 (identity, not `is_some()`); `unbudgeted_store_is_a_distinct_explicit_product_type` service.rs:4509 (`broker().is_none()` + mode assert) — findings-revocable-store.md:52, :110, :112.
- T5: `uncharged_retained_map_bytes_are_not_reflected_in_a_broker_ledger_snapshot`-sibling `uncharged_store_is_explicit_non_budgeted_mode` RM:3119; `explicit_unbudgeted_store_is_a_distinct_non_ledger_mode` PS:1305 (both need the ledger-0 pole; RM has the RM:2027 pattern, PS has no sibling) — findings-facade-stores.md:195, :223.
- T6: `routed_spill_rolls_back_the_count_when_accept_fails` service.rs:589 — **the critic-skipped row 12**; give the fake a real counter (findings-spill-redb.md:62; findings-critic.md:100).
- T9: `fixture_mode_exposes_the_facade_suite_end_to_end` PAS:110 (verdict-set assert; closed by the 1.3 fix) — findings-testkit.md:179.

Count check: Tier 2 lists 20 WEAK rows (2.2, 2.3×6, 2.4×9, 2.5×4) + Tier 3 has 1 (3.2) +
below-threshold 11 + the Tier-3-ranked 3.5 is one of T6's two WEAK rows = **33 WEAK total**;
FAILOPEN 6 = 1.1, 1.2, 1.3, 2.1, 3.1, 3.3; VACUOUS 7 = 1.5, 1.6×5, 3.6. ✓

---

## 3. Fix-phase plan — the checklist the in-progress fix effort should be verified against

A fix effort is **already running in this worktree** (~15 modified files across
core/facade/testkit plus a new `test_support.rs`; findings-critic.md:150–169). This
section is therefore **not a from-scratch plan** — it is the acceptance checklist,
grouped by the nine original track ownerships, in suggested verification order
(Tier 1 first). The critic already observed that the concurrent diff contains "the
control.rs diff [that] is precisely the T2/F1 fix my Config B/C validated, and the
testkit fixtures diff is the G1 faulted-broker negative pole" (findings-critic.md:146–149)
— so items **3.2 (T2 F1)** and **3.9-a (G1)** are likely already implemented and should
be verified first, then extended to the rows the diff does not cover.

| order | track (ownership boundary) | effort | checklist (verify the fix does…) | tier |
| --- | --- | --- | --- | --- |
| 1 | **T9 testkit** | **L** | (a) G1: kit-level broken-fidelity knob drives `run_conformance` to `Failed` (closes 1.2; likely in-flight per critic); (b) G2/G5: expected verdict-name set asserted at `run_facade_store_suite` and `run_conformance` entries (closes 1.3 + PAS:110); (c) G4: `RebuildAcceptingTarget` fake proves the `RebuildWriteFence` family fires; (d) G3: reclaim/pin/rebuild driver poles mirroring the spill pattern (FS:239/282/317); (e) FX:137 dead loop driven by the (a) fixture or deleted (closes 1.5); (f) loom: bind to production or de-count from coverage (closes 1.6); (g) lower: G6 multi-class `reclaim_once`, G7 `fake/views.rs`, G8 doctor, broker-suite row 1 over-deny pole | 1 |
| 2 | **T1 core-domain** | **M** | (a) ledger probe counter counts real lookups + cardinality pole (closes 1.1 — note `ledger.rs` is dirty in the fix tree; verify against the audited baseline semantics, findings-core-domain.md:87–115); (b) 7 WEAK rows per 2.2/2.3, starting with the critic-confirmed `policy.rs:497` `<` assert | 1–2 |
| 3 | **T2 core-api** | **M** | (a) F1 allocator-observation leg (closes 2.1; Config B/C is the acceptance test: mutated-SUT+fixed-test RED, clean-SUT+fixed-test GREEN — findings-critic.md:90); (b) F3/F4/F6/F7 post-denial ledger asserts; F5 charge-landing; F2 lane kind; F8 rename/compile-pin; F9 broker leg; F10 report content | 2 |
| 4 | **T3 app-ports** | **M** | (a) large-limit Fatal case (closes 3.3, one line); (b) metrics recorder-backed oracle (closes 3.6 + metrics.rs:47); (c) 4 WEAK rows + the controller.rs:417 placeholder (below-threshold list) | 3 |
| 5 | **T5 facade-stores** | **M** | (a) two-store digest comparison (closes 3.1); (b) `stdout.contains("1 passed")` on the 11 subproc tests (closes 3.4, mechanical); (c) RM:3119 / PS:1305 ledger-0 poles | 3 |
| 6 | **T8 pipeline-shared** | **S** | (a) counting-reclaimer leg for surface:414 (closes 3.2 — "the only scoped test whose named wiring can be silently deleted while staying green", findings-pipeline-shared.md:191–193); (b) four surface one-liners per 2.5 | 2–3 |
| 7 | **T6 spill-redb** | **S** | (a) real counter in the accept-failing fake, service.rs:589 (the critic-skipped row 12 — verify by review or measure post-fix); (b) destructive offline-accept pole on a non-empty target (closes 3.5) | 3 |
| 8 | **T4 revocable-store** | **S** | 3 assertion strengthenings (service.rs:1845 exact counts; :4488 identity; :4509 mode assert) | below |
| 9 | **T7 crash-rails** | **S (none required)** | Zero findings. Optional cosmetics only: CS:155 guard, HE:590/HE:173 full-snapshot detectors (findings-crash-rails.md:160–163) | 4 |

**Cross-cutting acceptance rules for the fix effort:**

1. Every fix must flip its critic mutation red (rows 1–11 of findings-critic.md give
   the exact mutations; row 12 must be measured once `spill/service.rs` is committed).
2. Fixes must strengthen oracles, never weaken neighboring assertions (AGENTS.md
   closeout rule: "fix behavior rather than weaken assertions").
3. The house style to copy already exists in each file: before==after full
   `LedgerSnapshot` equality (T1/T2), exact typed variants, the RM:2690 stdout check,
   the RM:2027 ledger-0 pole, the FS:239/282/317 driver poles.
4. Canonical rail on the fix's own branch:
   `python3 tools/pm/pm.py sync && scripts/check-workspace.sh`.
5. `test_support.rs` (untracked, findings-critic.md:166) should land as part of the
   fix effort with its ownership attributed in the fix-phase summary.

---

## 4. Conformance gap map (from findings-testkit.md:181–253) — summary & pole priority

The full gap map lives at findings-testkit.md:181–253. Status in one paragraph:
the **broker-accounting, prepared-publication, transcript-append, and
store-lifecycle suites have genuine, localized negative poles (23+ distinct broken
fakes) with SUT-independent oracles** — the "conformance must bite" contract is real
for those four families (findings-testkit.md:272). The residual fail-open surface is
concentrated at the **seams**: kit entries positive-only (G1/G2), 6 of 8 facade-slice
verdicts never proven able to fire (G4/G5), 3 of 4 fault drivers lack rubber-stamp
poles (G3), and the 5 loom "concurrency" tests verify hand-rolled models, not
production code (G9) (findings-testkit.md:273).

**Cross-cutting G-list** (findings-testkit.md:245–253):

| # | gap | consequence if unfixed |
| --- | --- | --- |
| G1 | Kit conformance entry positive-only (`run_conformance`, FX:59) | Stub to `Passed` → all 87 green (**FAILOPEN 1.2**, critic row 5) |
| G2 | Kit facade-store entry positive-only + slice coverage unasserted (FX:67, S:683–686) | Deleting slice sub-suites stays green (**FAILOPEN 1.3**, critic row 6) |
| G3 | Fault-driver Escaped branches: only spill self-verified (reclaim S:58–82, pin S:88–124, rebuild S:130–152) | "Unconditionally Contained" drivers caught by nothing |
| G4 | `RebuildWriteFence` family never violated in any self-test (S:497–541) | Fence-verdict weakening undetected |
| G5 | Partial suite-emptying passes the fold (`from_assertions` fails only on total emptiness, CM:38–44) | Any conformance runner can silently drop checks (multiplies G2) |
| G6 | Fake `reclaim_once` multi-class cross-axis consistency untested (FB:708 single class/owner) | Class/owner desync after reclaim with ≥2 classes slips through every test |
| G7 | `fake/views.rs` zero tests anywhere in the crate | `RejectingScratchAllocator`'s honesty contract (rejects scratch) asserted nowhere — a granting regression is invisible |
| G8 | `doctor/mod.rs` zero tests | `MemoryDoctorReport::is_clean` untested (low risk) |
| G9 | Loom models exercise no production code (LM:11–6 doc admits model-only; `loom` not in `full`) | 5 "concurrency tests" green-light broken production code (**VACUOUS 1.6**, critic row 9) |

Also from the per-suite tables: broker-suite row 1 (`in_cap_reserves_succeed`,
S:411–421) has no over-denying pole — the verdict's ability to fire is unproven
(findings-testkit.md:189); row 8 is G4 (findings-testkit.md:196). The
prepared-publication, transcript-append, and store-lifecycle tables are fully pole'd
(findings-testkit.md:201–226).

**Priority ordering of missing negative poles** (cheapest-per-risk-closed first):

1. **G2+G5 — verdict-name contracts** at `run_facade_store_suite` / `run_conformance`
   entries. One assertion each; closes FAILOPEN 1.3, guards every current and future
   runner against silent check-dropping.
2. **G1 — kit-level broken-fidelity knob.** Closes FAILOPEN 1.2 and, for free,
   un-deadens FX:137 (VACUOUS 1.5: the localization loop finally iterates).
3. **G4 — `RebuildAcceptingTarget` fake.** The only broker-suite family with no
   firing proof; a host broker accepting reserves during rebuild must be catchable.
4. **G3 — reclaim/pin/rebuild driver poles.** Mirror the spill driver's three poles
   (FS:239 lying controller / FS:282 count drift / FS:317 unreadable inventory).
5. **G9 — loom binding or de-counting.** Either bind models to production
   authorities or stop counting 5 VACUOUS rows as SUT coverage.
6. **Broker-suite row 1 — over-denying pole** (a target that denies the 1_750-byte
   workload must fail `in_cap_reserves_succeed`).
7. **G6, G7, G8 — residual:** multi-class `reclaim_once` case, `fake/views.rs`
   scratch-rejection test, doctor smoke.

---

## 5. Process notes

1. **Census quality: 9/9 count match.** The critic re-grepped every scope file and
   matched every report's claimed row count exactly (findings-critic.md:62–83),
   including T2's doc-comment grep false positive, which T2 disclosed in its own
   header rather than hiding (findings-core-api.md:21–40; findings-critic.md:74–81),
   corroborated by cargo's executed count (8, not 9). No report inflated or deflated
   its scope.
2. **Auditor overclaim rate: 0 REJECTED.** All 11 executed mutations reproduced the
   auditors' predictions (findings-critic.md:123–126). The single RED observed
   (critic row 2, Config B) was itself auditor-predicted and served as the fix-bites
   proof — see the decision-rule nuance in §1. Nothing the auditors reported was
   withdrawn; the flagging precision (auditors pre-computed predicted outcomes,
   e.g. T3's numeric wrapping/saturating verification, findings-app-ports.md:63–71)
   held up under measurement.
3. **The two critic-stall incidents.** A previous critic attempt stalled twice and
   left no report; the successful run was a clean restart (findings-critic.md:3–4).
   The lesson was institutionalized as the **incremental report rule**: append each
   mutation row immediately after its run so a stall can never lose more than one
   row (findings-critic.md:20–21, progress log :42–54). This rule should be
   mandatory for any future mutation-verification pass.
4. **The accidental mutation leak into user commit `4a952c6`.** A critic mutation of
   the exact T2/F1 shape (the `last_allocator_resident` fallback in `control.rs`,
   10 lines) was still in the tree when the user committed `4a952c6` ("done"), which
   also committed the nine auditor reports; it was reverted in `61e9c7c` ("Revert
   accidental critic mutation in control.rs (last_allocator_resident fallback leaked
   into 4a952c6)") — verified: `git show --stat 4a952c6` includes
   `core/src/api/control.rs | 10 +-`; `61e9c7c` reverts exactly those 10 lines
   (2 insertions, 8 deletions); also recorded at findings-critic.md:37–38. Lessons:
   (a) mutation hygiene must fence against *user* commit activity, not just critic
   self-discipline — the per-mutation restore (`git checkout --` + empty-diff
   verification, findings-critic.md:131–133) happened, but a mutation existed in the
   tree across a commit boundary; (b) the closeout marker-grep
   (`CRITIC MUTATION|CRITIC AUGMENT` over `crates/`, findings-critic.md:139–141)
   caught nothing because the leak had already been *committed and reverted* by the
   user, not left in the tree.
5. **Concurrent-fix-phase coordination.** The fix phase began editing this worktree
   mid-critic-run (first observed as `reviewed-actual-findings.md`, then ~15 dirty
   `crates/` files + `test_support.rs`; findings-critic.md:144–169). Consequences
   handled correctly and to keep: the critic **skipped** its row-12 mutation rather
   than `git checkout --` against someone else's uncommitted work
   (findings-critic.md:100, :174–175), and this synthesis worked **only** from the
   ten committed report files, treating `crates/` as untrustworthy and
   `reviewed-actual-findings.md` as out-of-scope (it belongs to the fix effort;
   noted here for existence only). Row 12 (`spill/service.rs:589`) therefore remains
   the one unmeasured WEAK sample and should be measured or review-verified after
   the fix phase commits. Going forward: mutation-verification passes and fix phases
   must not share a working tree, or must coordinate on a lock file.

---

*Synthesis step, 2026-09-26. Inputs: the nine census reports + findings-critic.md
(all in this directory). No code changes; `99-index.md` is the only file created by
this step; nothing was committed.*
