# Track 5 — Testkit Integration Audit (`testkit` + `hellgate_e2e`)

Scope: every test body in `crates/quanta-memory-platform-testkit/src/`
(`conformance/`, `faults/`, `fixtures/`, `alloc_tracking/`, `fake/`,
`loom_models.rs`, `doctor/`) plus
`crates/quanta-memory-platform-testkit/tests/` (2 files) and
`crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs`.
Total: ~115 `#[test]`s (35 in `suites.rs`, 18 in `hellgate_e2e.rs`,
5 loom models, rest spread across unit modules). No fix code — audit only.

Lenses: **A** cheap code · **B** cheap tests · **C** slow/inefficient compute ·
**D** weak-oracle quality · **E** duplicates (with keeper pick).
Severity: **S** small · **M** medium · **L** large.

## Sleep/time scrutiny (required lens)

**V-0 — `faults/` is clean: zero sleeps, zero clocks. No finding.**
`scenarios.rs:1-7` documents "NO timing heuristics: races are modeled
structurally (open a rebuild window, attempt the racing write) rather than
with sleeps", and the Extraction-verified read of the full
`scenarios.rs:1-249` and `mod.rs:1-146` shows no `sleep`, `Duration`,
`Instant`, `park_timeout`, `elapsed`, or `timeout` token anywhere (repo-wide
`sleep|Duration|Instant|…` search over the whole track returns zero hits).
The only thread/clock-adjacent sites in the track are outside `faults/`:
the busy-retry `yield_now` spin in `hellgate_e2e.rs:346-354` (finding F-3)
and the exhaustive `loom::model` schedulers in `loom_models.rs` (finding
F-4). Nothing to deflake in `faults/`; it is the model citizen of this track.

## Findings

### F-1 [C/M] Proptest trace is the most expensive test in the track
`suites.rs:1829-1843` configures
`ProptestConfig { cases: 128, max_shrink_iters: 8192 }` over
`vec(model_operation(), 1..96)`, and each case runs
`run_generated_trace` **twice** — once against `grouped_fake()`, once
against `grouped_live_broker()` (`suites.rs:1840-1841`). Every generated
op ends with a full `target.view() == Ok(model.view())` equality
(`suites.rs:1749-1753`), i.e. up to ~96 full `AccountingView`/snapshot
comparisons per case per backend (~24k view comparisons on a full pass),
each cloning BTreeMaps through `snapshot()`. On failure, shrinking replays
up to 8k iters. This single test likely dominates
`--all-features` testkit time.
*Quick win:* cut to `cases: 32..64`, or run the live broker only on a
sub-sample of cases (fake every case, live every Nth).

### F-2 [C/M] `run_facade_store_suite` fans out full platform builds per caller
`suites.rs:664-672` extends payload + retained-map + transcript + spill +
5× lifecycle suites; each slice calls `make_platform(kit)`
(`suites.rs:595-600`, a full `MemoryPlatformBuilder::build()`), and the
retained-map and transcript slices build **extra** tight-cap platforms
(`suites.rs:1116`, `suites.rs:1209`). The suite is re-invoked from at least
three entries: `fixtures/mod.rs:159-162`,
`tests/public_api_surface.rs:104-107`, and
`tests/facade_conformance_bite.rs:107-111` — each paying the full fan-out.
*Quick win:* share one built platform per test process (or `OnceLock`
fixture) instead of rebuilding per slice/call.

### F-3 [C/M] Hellgate concurrency test is the wall-clock + flake hotspot
`hellgate_e2e.rs:332-380` spawns 8 threads × 50 `insert_checked` of
`vec![0u8; 1_000]` with an unbounded-until-100k `yield_now` busy-retry loop
(`hellgate_e2e.rs:346-354`,
`assert!(busy_retries < 100_000, "store never left busy phase")`).
Under load the spin count is scheduler-dependent; the 100k tripwire is a
busy-loop cliff, not a typed wait. Same file's `reconfigure` test moves
600 KB + 3 MB payloads (`hellgate_e2e.rs:298-322`) — the largest
allocations in the track.
*Quick win:* bound the retry loop with an iteration cap + counter assertion
already present, but replace `yield_now` spinning with a small capped
backoff, and shrink the reconfigure payloads (e.g. 60 KB / 300 KB still
cross cap boundaries at 1M→4M caps with room to spare).

### F-4 [C/S] Loom models are expensive but correctly quarantined
`loom_models.rs:17-202` runs 5 `loom::model` exhaustive interleaving
enumerations (reserve-vs-reconfigure, ticket single-terminal-owner,
close-vs-mutation, spill transfer, versioned callback). Loom enumeration is
orders of magnitude slower than the enclosed logic, but
`lib.rs:33-34` gates the module behind
`#[cfg(all(test, feature = "loom"))]`, so default `cargo test` pays zero.
No action needed beyond keeping it opt-in; do not move these into the
default suite.

### F-5 [C/S] Per-step full-view equality in the deterministic trace
`suites.rs:1604-1608` asserts `target.view() == Ok(model.view())` after
each of 256 steps (×2 backends in
`deterministic_trace_matches_independent_model`, `suites.rs:1622-1626`),
each `view()` cloning the full ledger maps (`target.rs:353-414`). Cheap
per step, 512 full-snapshot comparisons total — fine, but it is the same
shape as F-1 at small scale.
*Quick win:* compare only affected axes per step, keep the full-view assert
every 16th step + final drain.

### F-6 [E/M] Honest-fake-passes asserted in four places — keep one
`lib.rs:92-98` (`conformance_suite_accepts_an_honest_fake`),
`conformance/mod.rs:120-125` (`honest_fake_yields_passed_verdict`),
`suites.rs:2374-2383` (`honest_fake_passes_every_accounting_invariant`),
`fixtures/mod.rs:120-125`
(`testkit_runs_conformance_against_its_fake_and_passes`) all build an
honest fake/kit and assert pass. **Keeper:** `suites.rs:2374-2383` (asserts
every invariant held with violator evidence). Fold the other three to a
single smoke or delete.

### F-7 [E/M] Under-report rejection asserted in three places — keep one
`lib.rs:70-82`, `conformance/mod.rs:128-138`, and `suites.rs:2386-2399`
all feed `UnderReportOwnerBytes` and assert `ClassOwnerBalance` failure.
**Keeper:** `conformance/mod.rs:128-138` (public folded-verdict surface +
localization). Delete the other two or parametrize over the fidelity enum.

### F-8 [E/S] `DivergeClassFromOwner` rejection duplicated
`lib.rs:85-89` and `suites.rs:2401-2409` assert the same variant bites.
**Keeper:** `suites.rs:2401-2409` (lives next to the exhaustive
`all_break_variants` list, `suites.rs:2418-2436`). Delete the lib.rs copy;
the meta-coverage already subsumes it via
`suite_rejects_every_accounting_fidelity_break_variant`.

### F-9 [E/M] Lifecycle close/rebuild/drop oracle overlaps hellgate e2e
`run_store_lifecycle_suite` (`suites.rs:693-765`) drives close-exactly-once,
rebuild-close-fail-closed, drop-release, rebuild-drop-defer across 5 stores
(`suites.rs:967-973`), while `hellgate_e2e.rs:104-167` re-proves the same
four transitions (`close_drains_all_charges_once`,
`failed_close_during_rebuild_preserves_store`,
`store_drop_during_rebuild_defers_refund`) against a live
retained-map store. Both are valuable (generic oracle vs emergent ledger
proof), but the per-store ×5 repetition plus hellgate is 9× coverage of one
state machine.
**Keeper:** the generic oracle (it also rejects the broken fakes in
`facade_conformance_bite.rs:114-132`). Trim hellgate to one representative
(rebuild-drop-defer) and cite the oracle for the rest.

### F-10 [E/S] Pin discipline proven three times — keep the oracle pair
`faults/scenarios.rs:88-124` (structural pin-delete-reject driver),
`suites.rs:1041-1063` (`pin_safe_delete` verdict: reject + bytes intact +
unpin-then-refund), and `hellgate_e2e.rs:559-588` (dedup→pin→refused
delete→unpin→refund with hardcoded `121` ledger bytes) all prove pin
discipline. **Keeper:** faults driver + suite verdict (structural, no magic
numbers). Demote the hellgate pin section to a ledger-spot-check or delete
(the `121 = 32+9+64+16` constant duplicates `suites.rs:980`).

### F-11 [E/S] Spill fail-closed proven three times — keep two
`faults/scenarios.rs:157-173` (unavailable-target driver),
`suites.rs:1294-1315` (`spill_unavailable_fail_closed` + round-trip
`1 -> 0`), and `hellgate_e2e.rs:174-205` (FIFO round-trip +
ledger-neutrality). The hellgate spill test is the only one proving
**ledger-neutrality** (`hellgate_e2e.rs:193,200-204`), so it earns its keep;
the first two overlap on fail-closed.
**Keeper:** suite verdict (injectable, bites the dropping target per
`suites.rs:513-…` self-test) + hellgate neutrality. Fold the faults spill
driver into the suite's offline check or keep it as the chaos-harness entry
only.

### F-12 [D/M] `spill_fault_classification` tests its own local helper
`faults/scenarios.rs:213-248` defines a local `classify()` closure and a
local `LyingSpillController`, then asserts the helper classifies correctly
— but never routes the lying controller through the real
`drive_spill_failure_after_reservation` driver, so a regression that breaks
the driver's `spilled_count` plumbing would not flip this test.
*Quick win:* drive the lying target through the production driver path (or
assert the driver's `(before, result, after)` triple directly).

### F-13 [D/S] `assert_balanced` reports booleans, not drift
`hellgate_e2e.rs:63-67` asserts `class_owner_balanced()`,
`owner_partition_balanced()`, `context_owner_balanced()` with only the
debug snapshot on failure — no left/right/drift magnitudes, unlike the
testkit's `assert_eq_bytes` evidence (`invariants.rs:116-125`,
`{label}: left={left} right={right} drift={drift}`). Every hellgate balance
failure requires a rerun to learn the direction of the lie.
*Quick win:* mirror the drift format in `assert_balanced`.

### F-14 [D/S] `public_api_surface` fixture test asserts pass-only
`tests/public_api_surface.rs:104-107` runs the whole facade suite and
asserts `passed()` with no family/evidence spot-check — a weakened copy of
`fixtures/mod.rs:159-162` that would still pass if the suite silently ran
zero checks (contrast `conformance/mod.rs:141-145`, which pins
empty-is-failed). The rest of the file is compile-time surface pinning
(dead-code fns), i.e. zero runtime oracle.
*Quick win:* assert `violated_families().is_empty()` + non-empty verdict
count, or delete the test and keep the surface pins.

### F-15 [B/S] `doctor/` has no tests and a trivial oracle
`doctor/mod.rs:1-12` is a 3-vec struct whose `is_clean()` checks only
`errors.is_empty()` — warnings/notes never affect cleanliness by design,
but no test pins even that (`grep #[test]` over `doctor/` = 0). Cheapest
module in the track; one 5-line test would pin it.
*Quick win:* add `is_clean` true/false unit test.

### F-16 [A/S] `FakeBrokerFactory` is a pass-through wrapper
`fake/broker.rs:582-596` re-exports `honest()`/`broken()` one-liners
already on `FakeProcessBroker` (`broker.rs:214-236`), and
`fake/mod.rs:1-15` + `fake/views.rs:54-73` are thin re-export/factory
shims. Harmless but doubles the "how do I make a fake?" surface.
*Quick win:* keep one constructor path in docs; deprecate the other.

### F-17 [B/S] Alloc/invariant/fixture unit tests are trivially cheap (keep)
`alloc_tracking/mod.rs:131-203` (record → snapshot → assert),
`invariants.rs:144-232` (constructor/attribution checks), and
`fixtures/mod.rs:128-135` (pressure getter round-trip) run in microseconds
with zero I/O. No consolidation needed — they pin real contracts
(saturation at zero, drift sign, attribution). Listed here so the track
can show cheap-but-valuable tests were reviewed, not skipped.

## Top-5 quick wins

1. **Shrink the proptest** (F-1): `cases: 128 → 32..64`, live broker on a
   sub-sample; biggest single time saving in the track.
2. **Share one built platform** across `run_facade_store_suite` callers
   (F-2): eliminates 3× full fan-out (payload/map/transcript/spill +
   5× lifecycle + 2 tight-cap platforms per call).
3. **De-duplicate the honest/broken-fake meta-tests** (F-6, F-7, F-8):
   keep the suite-level keepers, delete 4 redundant tests.
4. **Fix the two self-testing-the-test-helper oracles** (F-12, F-14):
   route the lying spill controller through the real driver; assert
   non-empty verdicts in the surface test.
5. **Put drift magnitudes in `assert_balanced`** (F-13): 3-line change,
   every future hellgate accounting failure localizes on first print.
