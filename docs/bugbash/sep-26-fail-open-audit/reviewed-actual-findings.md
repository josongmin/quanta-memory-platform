# Sep-26 fail-open audit — filtered current-source review

> Remediation completed. This is the original pre-fix review record. See [final changes and verification](remediation-verification.md) for the corrected source, 19 rejected mutations and full gate. R12 was expanded from15 to16 guarded launch sites after cross-review found the revocable child wrapper.

Review base: `61e9c7c47fa7294f9a926fd867e4af0b116c429b` (2026-09-26).
Three parallel reviewers covered core-domain/core-api/app-ports, the five
facade scopes, and testkit. The integrator checked evidence and exclusions.
No production source was edited by this review. Runtime reproductions used
`/tmp/qmp-t9-review-61e9c7c`, created from a git archive of that HEAD.

## Decision and evidence boundary

Retain 10 work items: one reproduced testkit implementation defect and nine
test/oracle/harness defects. Two specific unproven dispatcher guard gaps are
kept separate and excluded from this actual-defect fix queue. No production
core/facade runtime defect was established. Do not interpret hypothetical
mutations as observed product failures.

- **REPRODUCED**: a desired invariant fails on the unmodified implementation.
- **MUTATION**: the accused test executes and stays green with named breakage.
- **SOURCE**: the current test's setup/oracle cannot establish its explicit
  claim. The proposed counterexample has not been run in this review.
- **GUARD GAP**: an identified dispatcher/contract lacks its specific guard;
  this is not a demonstrated failing implementation.

The separate critic is concurrently active. At the recorded checkpoint its
report contains three mutation rows: ledger scan, allocator-driven pressure,
and controller wrapping/saturation. This review does not complete
that critic's full mutation matrix or serve as its canonical `99-index.md`.

## Retained work

Paths below are relative to the repository root; line numbers refer to the
committed review base, not transient critic mutations.

| ID | Priority / evidence | Owning source and test | Actual problem | Required correction |
| --- | --- | --- | --- | --- |
| R01 | P1 / REPRODUCED | `crates/quanta-memory-platform-testkit/src/fake/broker.rs:433`, class removal `:484` | `reclaim_once` selects one owner but subtracts class/context charge kinds independently. Payload 20B in ArtifactCache/small and metadata 100B in EvidenceCache/large, reclaim 50B: class/context become payload=0, metadata=70; owner becomes payload=20, metadata=50. Totals still equal 70. Releasing the unrelated small payload ticket returns `InvalidState("fake broker: class release exceeds usage")`. Existing ticket authority is also left inconsistent with reclaimed amounts. | Model reclaim against actual owner/class/kind/ticket provenance, or explicitly restrict unsupported reclaim inputs before mutation. Assert per-kind cross-axis equality and release of unrelated surviving tickets; total equality alone is insufficient. |
| R02 | P1 / MUTATION | `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:110`, test `:214` | Pinned-delete classifier claims the entry survived using only nonzero unchanged charged bytes. A mutation removes the pinned entry, keeps its charge, and returns the original error: the real test still passes and the driver returns Contained. | After the rejected delete, read/pin the same reference and verify exact payload survival as well as ledger/store accounting. Feed the same classifier a losing-entry negative pole. |
| R05 | P2 / MUTATION reported by existing critic | `crates/quanta-memory-platform-core/src/domain/ledger.rs:901`, assertion `:1647`, test `:1666` | The release complexity counter measures refund-map cardinality, not actual probe work. One release reports three even when owner preflight is changed to a scan over every owner. The critic records the scan mutation GREEN. | Increment instrumentation at actual lookup/iteration sites, then require bounded work with 32 and 128 owners. Current keyed-lookup implementation is correct. |
| R06 | P2 / MUTATION reported by existing critic, arithmetic checked | `crates/quanta-memory-platform-core/src/application/controller.rs:421` | Overflow regression test only uses limit=1. Both u64 wrapping and premature saturation still classify MAX usage Fatal there. For limit=MAX and usage=MAX the correct ratio is 100%, while saturation gives 1% and wrapping gives 0%. | Add MAX-limit/MAX-usage Fatal assertion. Current production u128 computation at `:159` is correct. |
| R07 | P2 / SOURCE | `crates/quanta-memory-platform-core/src/application/scheduler.rs:520`, oracle `:529`–`:539` | Static-table reuse test compares table values after repeated admissions. Deterministic recomputation produces identical values, so the named reuse/work property is not measured. | Instrument precomputation calls/work with a scoped test hook; assert admissions add no recomputation. Current admissions borrow the cached table. |
| R08 | P2 / SOURCE | `crates/quanta-memory-platform-core/src/domain/config.rs:922` | Test of independent class/group reservation floors sets all class reservations to zero. Incorrectly combining both axes therefore still satisfies the fixture. | Use class reserved=5,000, group reserved=5,000, headroom=1,000, global=10,000 with valid per-axis caps. Each separate floor fits; the wrong joint floor 11,000 does not. |
| R09 | P2 / MUTATION reported by existing critic | `crates/quanta-memory-platform-core/src/api/control.rs:3434` | Semantic-vs-allocator pressure test never observes an allocator snapshot. The critic records current-test GREEN under allocator-driven classification, augmented-test RED (Fatal vs Soft), and clean-code augmented-test GREEN. | At semantic usage 800,000/global 1,000,000, observe resident=999,999 and require Soft pressure plus unchanged ledger. Current classify/observe implementation uses ledger authority correctly. |
| R10 | P2 / SOURCE | `crates/quanta-memory-platform-facade/src/payload_store/service.rs:880`, seed initialization `:142` | Per-store randomized-seed test constructs two standard-library RandomStates but no store. Store constructors sharing one process-global seed do not affect this oracle. | Construct two actual stores and compute digests through each store's own seed; also assert within-store stability. Finite digest collisions remain possible. |
| R11 | P2 / SOURCE | `crates/quanta-memory-platform-facade/src/spill/service.rs:646`, related tests `:675`, `:770` | Offline-transition test denies through a fresh empty controller after spilling into another. Actual toggle test checks cumulative accepted count, not retained payloads. Clearing existing inventory during a rejected offline accept is invisible to these assertions. | Seed the same target, toggle offline, deny accept, assert unchanged retained inventory, then toggle online and restore exact handles. Cover both target implementations. Current offline branches return before mutation. |
| R12 | P2 / SOURCE, libtest mechanism observed | Facade subprocess wrappers listed below | Fifteen wrappers accept successful child exit without proving their test body ran. A stale exact filter executes zero tests and exits successfully. All present filters match; no current skipped child was found. | Require exactly one selected child test or an explicit completion marker emitted after the child assertions. Preserve existing timeout bounds. |

R12 affected child-launch sites:

- `crates/quanta-memory-platform-facade/src/transcript_store/service.rs`:
  1463, 1556, 1633, 1726, 1823, 3525, 3699.
- `crates/quanta-memory-platform-facade/src/retained_map/service.rs`:
  1209, 1230, 1500, 1776, 3218.
- `crates/quanta-memory-platform-facade/src/payload_store/service.rs`:
  1527, 1685.
- `crates/quanta-memory-platform-facade/src/shared/lifecycle.rs`: 378.
- Exception: retained-map launch at 2681 already checks `1 passed` at 2690.
  Sixteen wrappers were reviewed in total; fifteen need the guard.

## Executed evidence

1. Fake reclaim reproduction on the archived HEAD:
   `cargo test -p quanta-memory-platform-testkit --all-features --test review_reclaim_repro -- --nocapture`.
   Exit 101: one defect-witness test passed; the desired per-kind equality test
   failed with class payload=0 versus owner payload=20. Test source:
   `/tmp/qmp-t9-review-61e9c7c/crates/quanta-memory-platform-testkit/tests/review_reclaim_repro.rs`.
   Log: `/tmp/qmp-t9-reclaim-repro.log`.
2. Pinned-entry-loss mutation in the isolated facade copy:
   `cargo test -p quanta-memory-platform-testkit --all-features --lib faults::scenarios::tests::pinned_delete_failure_is_contained_and_bytes_survive -- --exact --nocapture`.
   One test executed and passed while the entry was absent and charged bytes
   remained 103. Restore followed by the same baseline test also passed.
   Receipt: `/tmp/qmp-t9-pin-mutation-receipt.md`.
3. Arithmetic checks for R06 compared exact u128-style division with wrapping
   and saturating u64 products. No controller Cargo mutation was run here.
4. Existing facade test binary accepted a nonexistent exact filter with exit
   0 and zero tests. Binary provenance was not requalified; this establishes
   libtest behavior only, not a current-source facade pass.
5. Existing critic rows 1–3 record R05, R09, and R06 as CONFIRMED. R09 also
   records the augmented-test RED and clean-code augmented-test GREEN pair.
   This review inspected their mechanisms but did not independently rerun
   those mutations. These are critic-reported receipts, unlike the isolated
   reproductions in items 1–2.

No canonical workspace gate or extraction rail ran: this is a review and
isolated reproduction, not implementation or release qualification.

## Specific proof gaps — outside the actual-defect fix queue

Current implementations dispatch correctly. No direct mutation of these two
dispatcher paths was executed by this review; treat their proposed regressions as guard
coverage work, not established implementation failures.

| ID | Evidence | Owning source and test | Gap | Optional guard |
| --- | --- | --- | --- | --- |
| R03 | P1 / GUARD GAP | `crates/quanta-memory-platform-testkit/src/fixtures/mod.rs:59`, test `:119` | Public kit conformance entry is exercised only through honest Passed results. A dispatcher replaced with constant Passed is indistinguishable to its tests even though lower-level suite rejection tests still exist. | Drive a broken target through the same kit dispatch path and require Failed with the expected contract family. A standalone lower-level negative test does not test kit wiring. |
| R04 | P1 / GUARD GAP | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs:681`, test `fixtures/mod.rs:158` | Facade suite tests assert only final Passed. Removing payload/retained/transcript/spill slices leaves nonempty lifecycle checks, so the empty-suite guard does not establish completeness. | Assert the expected assertion-name set from the public facade-suite composition, and preserve a negative case through its dispatcher. Do not make the generic verdict fold reject arbitrary legitimate subsets. |

## Exclusions — do not put these into the fix queue

- Five Loom rows: module explicitly documents protocol models, not production
  mutex/storage execution. Their model assertions are meaningful; classifying
  them as five vacuous production tests is an overclaim.
- Metrics state mutation: changing the unit struct into an Arc-bearing struct
  does not compile unchanged `let emitter = MetricsEmitter` tests. The proposed
  GREEN is not established. Recorder coverage is separate optional work.
- Bounded facade events: bounded vocabulary/cardinality is not an exactly-once
  event-delivery contract. Duplicate-event assertions require a separate spec.
- Unbudgeted mode, broker identity, payload/broker builders, and reclaimer
  installation: type/compile contracts or stronger same-path sibling tests
  already cover the proposed broad breakages. In particular,
  `cross_store_hellgate.rs:330` drives the actual facade reclaimer at `:388`
  and checks its callback at `:410`.
- Spill rollback counter: controller has no independent mutable count; it reads
  target inventory. A fictitious controller counter is not an owning defect.
  Global constant-zero count mutations fail existing positive tests.
- Preset strict-cap, shared reclaim-lane, pending-ticket leak, and reconfigure
  ordering proposals: existing sibling authority assertions catch the stated
  broad mutations. Adding every assertion to every local test is optional.
- Unknown-group terminality: dangling mappings are rejected by validated public
  configuration. The audited branch is explicitly defensive/unreachable through
  that construction path.
- Non-Clone rebuild fences and report-only authority: legitimate type-level
  properties; runtime mutation suggestions assume unavailable construction paths.
- More class samples, exact ladder vectors, label uniqueness, and local builder
  round-trip assertions: optional coverage, not established current defects.
- Generic missing negative poles (testkit G3/G4 and facade slice rows), zero-test
  doctor/fake-view modules, and the dead family loop: proof gaps or cleanup.
  Keep them separate from reproduced false containment and dispatcher guards.
- Original fake-reclaim total-desynchronization claim: incorrect. Totals remain
  equal in the reproduced case; the real defect is charge-kind provenance and
  surviving ticket release (R01).
- Crash rails and redb backing: no retained current defect established by this
  review. This is not a fresh crash/durability qualification pass.

## Census and report quality

The nine reports contain 795 census rows, matching 795 raw attribute grep hits.
Actual anchored test attributes total 794: core-api's extra hit is a doc comment,
represented by a non-test compile pin. Report arithmetic also needs correction:
facade-stores has 148 STRONG rows, not its claimed 149; its scope has 15 subprocess
wrappers, not 12. These documentation errors are not product defects.

| Scope | Census rows / raw grep | Actual test attributes |
| --- | ---: | ---: |
| core-domain | 97 / 97 | 97 |
| core-api | 145 / 145 | 144 |
| app-ports | 55 / 55 | 55 |
| revocable-store | 72 / 72 | 72 |
| facade-stores | 151 / 151 | 151 |
| spill-redb | 75 / 75 | 75 |
| crash-rails | 29 / 29 | 29 |
| pipeline-shared | 84 / 84 | 84 |
| testkit | 87 / 87 | 87 |

The external critic was observed mutating `core/src/api/control.rs` during this
review. Those edits and its report belong to that writer; this review neither
restored nor accepted them. Source findings above are tied to committed HEAD.
Re-freeze source after that writer restores before implementing or qualifying
any fix. Only this review document was added in the shared repository.
