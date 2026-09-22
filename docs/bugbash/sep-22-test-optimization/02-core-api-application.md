# Audit track 2 — core API + application tests

Scope: `crates/quanta-memory-platform-core/src/api/` (`broker.rs`, `builder.rs`,
`control.rs`), `src/application/` (`controller.rs`, `scheduler.rs`),
`src/adapters/` (all), `src/ports/` (all),
`crates/quanta-memory-platform-core/tests/public_api_surface.rs`.
Audit only — no fix code.

Test census (every `#[test]` body opened and read):

| file | tests |
|---|---|
| `api/broker.rs` | 0 (trait defs only) |
| `api/builder.rs` | 6 (`builder.rs:160,172,192,214,238,254`) |
| `api/control.rs` | 76 (`control.rs:1605`–`control.rs:4429`) |
| `application/controller.rs` | 13 (`controller.rs:250`–`controller.rs:433`) |
| `application/scheduler.rs` | 13 (`scheduler.rs:298`–`scheduler.rs:485`) |
| `adapters/hdrhistogram.rs` | 4 (`hdrhistogram.rs:89,117,128,139`) |
| `adapters/metrics.rs` | 2 (`metrics.rs:46,68`) |
| `adapters/pin.rs` | 5 (`pin.rs:86,116,142,165,181`) |
| `adapters/scratch.rs` | 0 (covered indirectly via `control.rs:2988`–`control.rs:3144`) |
| `ports/scratch.rs` | 8 (`scratch.rs:197,213,225,242,263,279,304,322`) |
| `ports/telemetry.rs` | 9 (`telemetry.rs:199,235,245,254,262,272,314,334,357`) |
| `ports/pin.rs`, `ports/reclaim.rs`, `ports/store.rs` | 0 (pure types/traits) |
| `tests/public_api_surface.rs` | 7 (`public_api_surface.rs:178,227,252,270,293,318,327`) |
| total | **143** |

Lenses: **A** cheap code · **B** cheap tests (gaps) · **C** slow/inefficient compute ·
**D** weak-oracle quality · **E** duplicates (keeper pick). Severity **S/M/L**.

---

## A — cheap code

### F-01 (S) — `ResourceGroup` 6-field literal pasted ~10× in `control.rs` tests
Every group test re-spells the full struct
(`control.rs:1745`, `control.rs:2545`, `control.rs:3108`, `control.rs:3612`,
`control.rs:3668`, `control.rs:3725`, `control.rs:4048`, `control.rs:4284`,
`control.rs:3270`, `control.rs:3321`, `control.rs:4205`).
The file already has the right pattern — `class_budget()` at
`control.rs:1580` and `group()` / `two_group_scheduler()` in
`scheduler.rs:279,290`. Add `fn group(hard, burst) -> ResourceGroup` next to
`class_budget()`; saves ~60 lines and removes drift risk between copies.

### F-02 (S) — scratch-overflow twins duplicate 30-line `u64::MAX` setup
`control.rs:1606` (`scratch_reserve_cap_overflow_is_denied_without_mutation`)
and `control.rs:1637` (`scratch_recharge_cap_overflow_preserves_original_ticket`)
both build a `ResidencyPolicy::new(u64::MAX)` broker with
`max_execution_scratch_bytes = Some(usize::MAX)` and seed a `u64::MAX-10/-11`
charge. The two entry points (reserve vs recharge) both deserve coverage —
extract the broker+seed into a helper, keep both tests.

### F-03 (M) — `format!("o{i}")` inside the 1600-iteration hot loop
`control.rs:3191` (`concurrent_reserves_never_exceed_global_limit_no_toctou`)
formats 16×100 owner strings inside the contended reserve loop. Hoist to one
`Vec<String>` per thread before the loop so the critical section measures the
ledger, not the allocator.

### F-04 (S) — stale "eight" comment vs nine variants
`public_api_surface.rs:174`–`public_api_surface.rs:176` says "All eight
`MemoryError` variants" then lists nine and the test at
`public_api_surface.rs:178` is named `..._exactly_the_nine_documented_variants`.
One-word comment fix.

## B — cheap tests (coverage gaps)

### F-05 (M) — `adapters/scratch.rs` has zero direct unit tests
`BrokerBoundScratchAllocator` (`adapters/scratch.rs:40`–`adapters/scratch.rs:66`)
is exercised only indirectly through the broker-level runtime-scratch tests
(`control.rs:2988`–`control.rs:3144`). The foreign-runtime guard
(`adapters/scratch.rs:46`–`adapters/scratch.rs:51`) is pinned only by
`control.rs:3086`. A 3-case direct test (mismatched runtime, happy-path
delegation, drop-releases) against a stub authority would pin the adapter seam
without a full broker.

### F-06 (S) — builder surface partially untested
`builder.rs:40` (`Debug` impl — the `has_reclaimer` / bounded-sink masking),
`builder.rs:78` (`with_reclaimer`), `builder.rs:87` (`with_event_sink`), and
`builder.rs:111` (`build_live` alias) have no direct assertions; the alias is
never called anywhere in tests. One test building via `build_live()` with a
recording sink would close it.

### F-07 (S) — threshold-validate precedence + controller initial state untested
`controller.rs:67` (`PressureThresholds::validate`) checks `>100` before
monotonicity, but `controller.rs:341` only feeds one violation class per case —
never both at once (e.g. `(200, 50, 50)`), so precedence is unpinned. Likewise
the `initial` param of `LadderPressureController::new` (`controller.rs:103`)
is never read back via `current_pressure()` at construction in any of the 13
tests (all use `Normal`).

### F-08 (S) — `broker.rs` trait surface has no direct tests (accepted gap)
`api/broker.rs:11`–`api/broker.rs:31` is covered only via the trait-exercising
tests `control.rs:2812` and `control.rs:3865`. That is proportionate for a
2-method trait; recorded here for completeness, no action.

## C — slow / inefficient compute

### F-09 (M) — 16 threads × 100 reserve+commit is the heaviest unit test
`control.rs:3191` performs 1600 contended reserve+commit cycles plus 16 thread
spawns to prove a mutex-held invariant that a 4-thread × 25-iteration version
proves identically (TOCTOU is binary: either the lock serializes or it does
not). Halve both axes.

### F-10 (M) — 128 full-policy live reconfigures in one test
`control.rs:4043` (`concurrent_reserve_and_reconfigure_preserve_group_cap_truth`)
runs 128 `try_live_reconfigure` (each re-validates a full residency and rebuilds
scheduler+controller) against 128 concurrent reserves plus a 3-party barrier.
32 iterations exercise the same interlock at ~1/4 the cost.

### F-11 (M) — 7 full broker builds for pure threshold classification
`control.rs:2719` (`pressure_thresholds_are_inclusive_at_exact_boundary_fractions`)
constructs a fresh validated broker per boundary case (7× `from_report`), yet
the classification logic it pins already has a cheap unit home in
`controller.rs:251`. Reuse one roomy broker across all 7 cases (also a nicer
idempotence proof) or move the boundary table to the controller unit test and
keep a single broker-level spot check.

### F-12 (S) — 10k-group scheduler test
`scheduler.rs:463`
(`admission_reuses_policy_static_fair_share_table_at_high_group_cardinality`)
builds 10_000 groups, clones the 10k-entry share map, and runs 2000 admits.
The Hamilton-remainder property it guards is already pinned by the 3-group
`scheduler.rs:397`. 100 groups prove "no per-admission rebuild" identically.

### F-13 (S) — per-step full snapshot in the 100-step reference model
`control.rs:1940` (`owned_charge_sequence_matches_exact_reference_model`)
takes 3 balance snapshots per step (300 snapshots). Sampling every step is
defensible for a model test; if runtime matters, assert totals every step and
the 3 balance predicates every 10th.

## D — weak-oracle quality

### F-14 (S) — compile-only `Send + Sync` test
`control.rs:3258` (`broker_is_send_and_sync`) has no runtime oracle by design —
keep, but it means `LiveProcessMemoryBroker`'s thread-safety rests on
`control.rs:3191`/`control.rs:4043`, which is where it should stay.

### F-15 (M) — metrics emitter tests assert nothing observable
`metrics.rs:46` (`emitter_surface_is_emit_only_and_takes_shared_ref`) and
`metrics.rs:68` (`emitter_holds_no_accounting_state`) call `emit` and assert on
`()` / cloneability — the weakest oracles in scope. This is structural (no
metrics facade is wired in tests), so the honest fix is a comment admitting the
tests pin the signature, not the emission — or a test-only recording sink like
the `RecordingEventSink` at `control.rs:3920`.

### F-16 (S) — loose HDR percentile ranges hide off-by-one
`hdrhistogram.rs:102`–`hdrhistogram.rs:112` assert p50 ∈ 49..=51 and
p99 ∈ 98..=100 for a deterministic 1..=100 set through a 3-sigfig histogram.
The determinism claim in the comment supports exact asserts (`== 50`, `== 99`);
the ±1 windows would pass a fencepost regression.

### F-17 (S) — dead `assert_ne!` after exact equality
`control.rs:2089` (`assert_ne!(after.total_owner_bytes(), 600_000)`) adds zero
information after `control.rs:2088` asserts `== 400_000`. Delete.

### F-18 (S) — thin boundary asserts in scheduler happy path
`scheduler.rs:298` (`within_hard_cap_is_always_admitted`) asserts two interior
points; the cap+1 boundary lives only in `scheduler.rs:315`. Extend with the
exact-cap case (`admits("fg", 1_000)` is present; add `!admits` at share+1) or
merge into F-27.

## E — duplicates (keeper pick)

### F-19 (M) — missing-class denial twins → keep `control.rs:2686`
`control.rs:2672` (`reconfigure_rejects_policy_missing_drained_mandatory_class`)
is subsumed by `control.rs:2686`
(`reconfigure_missing_class_denial_preserves_policy_scheduler_and_usage`),
which adds live usage + scheduler-admission preservation. Delete 2672.

### F-20 (M) — recharge-denial triple → keep `control.rs:3482`, keep `control.rs:1911`
`control.rs:1876` (managed), `control.rs:1911` (owned `recharge_owned`), and
`control.rs:3482` (committed) deny the same over-cap recharge. `control.rs:3482`
has the strongest oracle (partition-axis + class-balance asserts,
`control.rs:3507`–`control.rs:3518`); `control.rs:1911` alone covers the
`recharge_owned` entry point — keep those two, fold 1876. (The further
denial variants `control.rs:3150` scratch-budget, `control.rs:3527` wrong-axis,
`control.rs:3665` grouped-owner each pin a distinct axis and all stay.)

### F-21 (S) — telemetry mismatch single-case subsumed by table → keep `telemetry.rs:272`
`telemetry.rs:235` (`advisory_flags_mismatch_and_reports_semantic_as_authority`,
2000-vs-1000) is row 1 of the table in `telemetry.rs:272`
(`compare_reports_signed_drift_and_mismatch_boundary`). Delete 235.

### F-22 (M) — pending-drop rollback ×4 → keep 3, merge 1
`control.rs:1681` (base), `control.rs:1743` (group axis), `control.rs:3065`
(scratch axis) each pin a distinct dimension — keep all three.
`control.rs:1702` (drop-during-rebuild) overlaps the rebuild-fence theme of
`control.rs:1802`; merge into a single fence-drop test.

### F-23 (S) — controller ladder overlap (keep both, noted)
`controller.rs:251` (exact values incl. 740/899 neighbors) vs
`controller.rs:393` (up-then-down through every rung). 393 subsumes the
determinism re-run at `controller.rs:260` but not the just-below-boundary
neighbors — keep both, no action.

### F-24 (S) — scratch-port `charge_kind` asserted 3×
`scratch.rs:197`, `scratch.rs:213`, `scratch.rs:279` all assert
`LiveScratch`. Harmless one-liners inside otherwise distinct lifecycle tests —
fold opportunistically, no dedicated churn.

### F-25 (S) — cross-file builder missing-manifest dup (keep both, noted)
`builder.rs:160` (unit, both products) vs `public_api_surface.rs:318`
(integration, report-only). Unit + boundary double-coverage is intentional —
no action.

### F-26 (M) — live-reconfig denied sextet shares one shape
`control.rs:2218`, `control.rs:2249`, `control.rs:2275`, `control.rs:2310`,
`control.rs:2339`, `control.rs:2384` are all reserve→commit→denied-reconfig→untouched
with only the denial axis differing. Table-drive the policy construction (or a
`deny_case` helper) to cut ~150 lines of setup while keeping all six axes.

### F-27 (S) — scheduler burst round-trip overlap → keep `scheduler.rs:428`
`scheduler.rs:336` (`release_returns_burst_capacity`) is subsumed by
`scheduler.rs:428` (`usage_release_round_trip_readmits_burst`), which adds
exact-usage restoration + over-release fail-closed (`scheduler.rs:440`–`scheduler.rs:442`).
Fold 336's scenario into 428's prologue or delete.

### F-28 (S) — pin-adapter 2-vs-3 dependents (keep both, noted)
`pin.rs:86` (2 dependents) vs `pin.rs:165` (3 dependents) overlap in shape but
the latter is 12 lines and pins count>2 fan-out — keep both, no action.

---

## Top-5 quick wins

1. **Extract a `group()` test helper in `control.rs`** (F-01, S) — kills ~60 lines
   of pasted 6-field literals at 10+ sites; mirrors the existing `class_budget()`
   pattern. Zero oracle risk.
2. **Delete two subsumed tests** (F-19, F-21, S) — `control.rs:2672` (covered by
   2686) and `telemetry.rs:235` (row 1 of the 272 table). Pure deletion.
3. **Halve the two heavy concurrency tests** (F-09, F-10, M) — 16×100→8×50 at
   `control.rs:3191`, 128→32 reconfigs at `control.rs:4043`. Same invariants,
   ~1/3 the wall time.
4. **Reuse one broker in the pressure-boundary test** (F-11, M) — `control.rs:2719`
   builds 7 brokers; one roomy broker across all 7 cases removes 6 validations.
5. **Fold the recharge-denial triple** (F-20, M) — drop `control.rs:1876`, keep
   `control.rs:3482` (strongest oracle) + `control.rs:1911` (unique entry point).

## Coverage notes (no finding)

- Builder validation axes are well-paired: invalid-residency (`builder.rs:172`),
  soft>hard (`builder.rs:192`), missing-class (`builder.rs:238`), plus a positive
  pair (`builder.rs:214`) — exemplary deny/allow structure.
- Poison-path coverage is thorough and correctly distinguishes fail-closed
  mutation paths from recovering read paths (`pin.rs:181`, `control.rs:3796`–`control.rs:3865`).
- The `RecordingReclaimer` / `RecordingEventSink` harnesses
  (`control.rs:3920`, `control.rs:4127`) prove outside-the-lock execution by
  re-entering the broker from the callback — the strongest pattern in the file.
- Strongest single oracles: `control.rs:3482` (partition-axis restore),
  `control.rs:2541` (full-surface denied-reconfig atomicity),
  `control.rs:3191` (ledger == sum-of-admitted under contention).
