# T04 deep audit — `core/src/api/` rest (broker, builder) + `core/tests/public_api_surface.rs`

Scope: `crates/quanta-memory-platform-core/src/api/broker.rs`,
`crates/quanta-memory-platform-core/src/api/builder.rs`,
`crates/quanta-memory-platform-core/tests/public_api_surface.rs`.
Every test body opened and read (builder.rs:142–271, public_api_surface.rs:174–334).
No fix code — audit only.

## Census

Grep (attributes only; excludes the `#[test]` mention in the doc comment at
`public_api_surface.rs:11`):

```
grep -rn '^[[:space:]]*#\[test\]\|^[[:space:]]*#\[tokio::test\]' \
  crates/quanta-memory-platform-core/src/api/broker.rs \
  crates/quanta-memory-platform-core/src/api/builder.rs \
  crates/quanta-memory-platform-core/tests/public_api_surface.rs
```

Result: `broker.rs` 0, `builder.rs` 6, `public_api_surface.rs` 7 → **13 rows**.
(No `#[tokio::test]` in scope; all tests are synchronous.)

| # | test (file:line) | verdict | reason |
|---|------------------|---------|--------|
| 1 | `builder.rs:161` `build_without_manifest_returns_build_error` | KEEP | Root deny-case for both builder products (incl. `build()` under `live_control`); intentional double-coverage with #12 (v1 F-25) |
| 2 | `builder.rs:173` `build_with_invalid_residency_returns_config_error` | MERGE | One of three same-shape invalid-manifest `Config`-denial tests (#2, #3, #5); table-drive with #2 as keeper |
| 3 | `builder.rs:193` `build_with_soft_above_hard_returns_config_error` | MERGE | Same shape as #2 (build → `matches!(Err(Config))`); becomes row 2 of the table |
| 4 | `builder.rs:215` `build_with_valid_residency_succeeds_and_reports_that_residency` | KEEP | Sole positive pair proving validation does not reject honest policies; asserts both global limit and class cap verbatim |
| 5 | `builder.rs:239` `builder_rejects_manifest_missing_mandatory_class` | MERGE | Third same-shape `Config`-denial; becomes row 3 of the table (distinct axis preserved) |
| 6 | `builder.rs:255` `report_only_builder_applies_override_layers` | KEEP | Only override-precedence test in scope; asserts both value and provenance |
| 7 | `public_api_surface.rs:179` `memory_error_has_exactly_the_nine_documented_variants` | KEEP | Exhaustive-variant + constructor-helper + Display contract pin; strongest oracle in the integration file |
| 8 | `public_api_surface.rs:228` `memory_class_taxonomy_is_stable` | KEEP | Pins 14/4 partition counts, disjointness, no-dupes; load-bearing for presets |
| 9 | `public_api_surface.rs:253` `charge_kind_has_exactly_three_partitions` | KEEP | Exhaustive 3-partition pin; tiny and load-bearing |
| 10 | `public_api_surface.rs:271` `memory_preset_has_exactly_the_four_documented_variants` | KEEP | 4-variant exhaustive pin; ~17 lines, no cheaper form |
| 11 | `public_api_surface.rs:295` `builder_first_surface_produces_a_broker_from_a_preset` (`live_control` only) | KEEP | Only test in scope that exercises the live `build()` path end-to-end through the public surface |
| 12 | `public_api_surface.rs:319` `builder_without_manifest_fails_closed_with_build_variant` | KEEP | Boundary double-coverage of #1 via public path only; intentional (v1 F-25) |
| 13 | `public_api_surface.rs:328` `report_only_builder_returns_data_without_runtime_authority` | SHRINK | Thin oracle (asserts only `report.preset`); extend with one provenance assert or fold into #6's shape — see D-3 |

Row count 13 = grep count 13. ✅

## Findings

### A — cheap code

#### A-1 (S) — invalid-manifest denial triple shares one 15-line shape
Opened: `builder.rs:172`, `builder.rs:192`, `builder.rs:238`.
Oracle (representative, `builder.rs:183`–`builder.rs:187`):

```
let result = ProcessMemoryBrokerBuilder::new().with_manifest(manifest).build_report_only();
assert!(
    matches!(result, Err(MemoryError::Config(_))),
```

Mechanism: #2, #3, #5 differ only in how the `ResidencyPolicy` is made
invalid (hard-cap-above-global vs soft>hard vs removed mandatory class); the
build-assert tail is identical. Three copies of the manifest/resolver/boolean
scaffolding for one predicate ("invalid policy → `Config` error").
Action: table-drive into one `invalid_manifest_returns_config_error` test kept
at `builder.rs:172` (`#[test] fn cases() -> Vec<ResidencyPolicy>` or three
sub-cases in one body); keep all three invalid axes. Est. saving: ~30 lines.

### B — cheap tests (gaps)

#### B-1 (S) — `build_live` alias never called in any test
Opened: `builder.rs:109`–`builder.rs:113`.

```
/// Compatibility alias for callers migrating to the enforced default.
#[cfg(feature = "live_control")]
pub fn build_live(self) -> MemoryResult<LiveProcessMemoryBroker> {
```

Mechanism: `grep -rn build_live crates/ --include='*.rs'` hits only the
definition; no test exercises the alias, so a body that diverged from
`build()` (e.g. dropping the reclaimer/sink wiring) would go green.
Action: route #11 (`public_api_surface.rs:295`) — or a unit twin at
`builder.rs:160` — through `build_live()` instead of `build()`. Keeper:
`public_api_surface.rs:295`. Est. saving: none (coverage gain, +1 line
changed).

#### B-2 (S) — `with_reclaimer` / `with_event_sink` / `Debug` masking unasserted in scope
Opened: `builder.rs:40`–`builder.rs:50`, `builder.rs:76`–`builder.rs:90`.

```
debug.field("has_reclaimer", &self.reclaimer.is_some());
debug.field("event_sink", &"bounded MemoryEventSink");
```

Mechanism: in-scope tests never install a reclaimer or sink on the core
builder (the only `with_reclaimer`/`with_event_sink` test hits are the facade
builder at `facade/tests/public_api_surface.rs:480` and the testkit suite at
`testkit/src/conformance/suites.rs:1869`), and no test formats the builder
with `{:?}`, so the `has_reclaimer` masking could regress to a `Debug` dump
of the sink/reclaimer without failing anything in scope.
Action: one test at `builder.rs:254` building via `with_reclaimer` +
`with_event_sink` + `build_live()` with a recording sink (mirroring the
`RecordingEventSink` pattern v1 cites), plus a `format!("{:?}")` contains /
not-contains pair. Keeper: `builder.rs:254`. Est. cost: +~25 lines, closes
the last builder-surface gap.

### C — slow / measured

None in scope. `timing/top40.md` is present and was consulted: no test from
this scope appears in the top 40 (cutoff 0.225s), so all 13 tests run below
that bound by exclusion. The heaviest assertions here are a 14-element loop
(#8) and nine constructor calls (#7) — microseconds, no contention, no I/O.

### D — weak-oracle quality

#### D-1 (S) — stale "eight" comment above the nine-variant test
Opened: `public_api_surface.rs:174`–`public_api_surface.rs:179`.

```
/// All eight `MemoryError` variants remain constructible. The single error type is
/// a public contract
```

Mechanism: comment says eight, test name and body pin nine; a reader
"fixing" the body down to eight would break the exhaustive match the test is
designed to enforce.
Action: one-word fix at `public_api_surface.rs:174` ("eight" → "nine").
Est. saving: 0 lines (correctness of docs).

#### D-2 (S) — dead `assert_eq!(variants.len(), 9)` after literal construction
Opened: `public_api_surface.rs:180`–`public_api_surface.rs:191`.

```
let variants = [
    MemoryError::Build("b".to_string()),
    ...
];
assert_eq!(variants.len(), 9);
```

Mechanism: the array literal is written out in full, so `len() == 9` is true
by syntax, not by behavior — it pins nothing the exhaustive `match` at
`public_api_surface.rs:195`–`public_api_surface.rs:206` does not already pin.
Harmless; delete opportunistically, no dedicated churn. Keeper:
`public_api_surface.rs:195` (the exhaustive match — the real oracle). Est.
saving: 1 line.

#### D-3 (S) — #13 asserts only the preset, not the data it claims to return
Opened: `public_api_surface.rs:327`–`public_api_surface.rs:334`.

```
let report = ProcessMemoryBrokerBuilder::new()
    .with_preset(MemoryPreset::Production)
    .build_report_only()
    .expect("preset resolves");
assert_eq!(report.preset, MemoryPreset::Production);
```

Mechanism: the test name promises "returns data without runtime authority"
but the oracle checks only the preset echo — strictly weaker than #6
(`builder.rs:255`–`builder.rs:270`), which asserts the overridden value plus
`override_provenance`. A report that dropped provenance would still pass #13.
Action: add one provenance/limit assert to #13 mirroring
`builder.rs:266`–`builder.rs:269`, or fold #13's preset-echo line into #6.
Keeper: `builder.rs:255`. Est. saving: ~0–7 lines.

### E — duplicates

#### E-1 (S) — missing-manifest unit/integration pair is intentional
`builder.rs:161` (#1, both products incl. live `build()`) vs
`public_api_surface.rs:319` (#12, report-only via public path). Keep both:
unit pins the live path, integration pins the public error-variant contract.
No action.

#### E-2 (S) — valid-build unit/integration partial overlap, keep both
`builder.rs:215` (#4, custom residency, asserts caps verbatim) vs
`public_api_surface.rs:328` (#13, preset echo) vs
`public_api_surface.rs:295` (#11, live broker advertises preset + full
mandatory class set). Three distinct axes (custom-policy fidelity /
report-only echo / live end-to-end); the only redundancy is #13's thinness,
handled in D-3. No deletion.

## v1 re-verification (02-core-api-application.md findings in this scope)

| v1 | status | evidence |
|----|--------|----------|
| F-04 (S) stale "eight" comment `public_api_surface.rs:174`–`:176` | CONFIRM | Re-opened `public_api_surface.rs:174`: still reads "All eight `MemoryError` variants"; body still constructs nine (`public_api_surface.rs:180`–`:190`). See D-1 |
| F-06 (S) builder surface partially untested (`Debug`, `with_reclaimer`, `with_event_sink`, `build_live`) | CONFIRM | Re-opened `builder.rs:40`–`:50`, `:76`–`:90`, `:109`–`:113`; `grep build_live` still hits only the definition; no `{:?}` format of the builder in any test. Narrowed: `with_reclaimer`/`with_event_sink` ARE covered cross-crate (facade `:480`, testkit `:1869`) but not on the core builder in scope. See B-1, B-2 |
| F-08 (S) `broker.rs` has no direct tests (accepted gap) | CONFIRM | Re-opened `broker.rs:1`–`:31`: still 31 lines, two trait defs, zero tests. Proportionate; no action |
| F-25 (S) cross-file missing-manifest dup, keep both | CONFIRM | Re-opened `builder.rs:160`–`:166` and `public_api_surface.rs:318`–`:325`: still unit(both-products)/integration(report-only) split. Keep both. See E-1 |

v1 census cross-check: v1 counted `public_api_surface.rs` 7 tests
(`02-core-api-application.md:25`) — matches this audit's 7 (rows 7–13).
Note: a naive `grep -c '#\[test\]'` on that file returns 8 because the doc
comment at `public_api_surface.rs:11` mentions `` `#[test]` ``; the
attribute-anchored grep used above returns the true 7.

Out-of-scope v1 items touching a same-named file are NOT re-verified here:
`04-facade-pipeline-spill-shared.md` F07–F09 concern
`facade/tests/public_api_surface.rs` and `facade/src/shared/builder.rs`
(different files, track T08/T09); `05-testkit-integration.md` F-14 concerns
`testkit/tests/public_api_surface.rs` (track T12).

## Top quick wins

1. Table-drive the invalid-manifest triple (A-1) — ~30 lines saved, all three denial axes kept.
2. One-word "eight"→"nine" comment fix (D-1) — zero oracle risk.
3. Route one build through `build_live()` (B-1) — one-line change, closes alias gap.
4. Harden #13 with a provenance assert (D-3) — ~3 lines, makes the "returns data" name true.

## Unresolved

- Per-test `--report-time` values for the core binaries are not in the
captured log: `timing/full.txt` holds only the last 300 of 823 lines (no
`Running … core` block, no per-test lines for the 13 in-scope tests), so
lens C rests on top-40 exclusion (all in-scope tests < 0.225s) rather than
direct timings.
