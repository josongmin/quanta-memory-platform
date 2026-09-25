# Sep-26 engine implementation runbook

Status: **instructions only**. Base: QMP `438b869a36369021ee81560fccdb9692e3f39e35`.
Use with [the contract/priority plan](STRUCTURAL-REMEDIATION-PLAN.md) and
[the reproduced failures](STRUCTURAL-AUDIT-EVIDENCE.md). This file gives the
next implementer exact edit points and tests. No instruction here means that
the edit has already been made or qualified.

Paths below are relative to the QMP repository root. Work on the current
`main` checkout as requested; do not create a branch or worktree for this plan.
Existing dirty plan files belong to the prior audit. Preserve them. Before each
package, capture `git rev-parse HEAD` and `git status --short`; after it, inspect
`git diff --stat` and `git diff --check`. If another writer changes an owner file,
stop that package and re-read the file before proceeding.

Do packages in the order **STR-01 → STR-03** for their shared prepared service.
STR-02's scratch/controller edits and STR-04's broker edits can run alongside
STR-01 only while their workers use distinct files. The STR-02 broker-backed
test in `api/control.rs` waits until STR-04 releases that file. STR-05 and
final STR-07 belong to the
integrator. Do not let two workers edit `shared/builder.rs`, `api/control.rs`,
`domain/ledger.rs`, generated guidance, or benchmark configuration at once.

## File assignment sheet

The tables in each package below are the edit instructions. Use this sheet to
assign files without accidentally giving two workers the same owner:

| Package | Exclusive source/config owner | Tests and documentation to update |
| --- | --- | --- |
| STR-01 | `facade/src/shared/lifecycle.rs`; `facade/src/{payload_store,retained_map,transcript_store,publish_pipeline}/...service.rs` (the prepared file is named `prepared_publication.rs`) | The same four service test modules; no standalone integration test for the panic mechanism. |
| STR-02 | `core/src/ports/scratch.rs`, `core/src/application/controller.rs` | Their local tests; broker-backed scratch test in `core/src/api/control.rs` **after STR-04 releases that file**; `core/tests/public_api_surface.rs`, migration/changelog. |
| STR-03 | `facade/src/publish_pipeline/prepared_publication.rs` **after STR-01**, `facade/src/shared/measure.rs`, `core/src/domain/ledger.rs` rustdoc only | Prepared local tests, cross-store test at STR-07, README/verification/migration docs. |
| STR-04 | `core/src/api/control.rs`, `core/src/ports/reclaim.rs` rustdoc, `facade/src/revocable_store/service.rs` test | `facade/tests/cross_store_hellgate.rs` integration test; do not edit it concurrently with STR-07. |
| STR-05 | `.circleci/config.yml`, `tools/pm/sources/rules/architecture.md` | README, README-detailed, optional CI contract test; PM-generated outputs only by `pm sync`. |
| STR-07 | `facade/tests/cross_store_hellgate.rs` **after STR-04** | Final status/receipts in the two plan documents; frozen-source full gate. |

Here `core/` expands to `crates/quanta-memory-platform-core/` and `facade/`
to `crates/quanta-memory-platform-facade/`. If package ownership overlaps in
practice, serialize the edits; parallelism is not an acceptance criterion.

## First: freeze the contract (STR-00)

1. Read `docs/plans/bugbash-sep-26-engine/STRUCTURAL-REMEDIATION-PLAN.md`,
   especially D1–D8 and the store charge table. Read `AGENTS.md` and only the
   owning `tools/pm/sources/rules/*.md` needed by the package.
2. Treat ENG-01/11/12/13 as confirmed public API behavior: the diagnostic in
   `evidence/contract-observations.rs.txt` recorded current output. Create
   normal owner tests with the *desired* assertions below; see them fail before
   changing production code. The diagnostic itself is not a Cargo test.
3. Preserve these invariants in every mutation: all-or-nothing ledger update,
   retained object and ticket sharing one lifetime, rollback retaining the exact
   lease, final `total_owner_bytes()==0` **and** `live_ticket_count==0`.
4. One kind bucket is one owned lifetime component. `RetainedCostV1` keeps its
   component formula; the single managed ticket gets the bucket for the whole
   component. No new ticket type, second ledger, heuristic refund, `unwrap` on
   user error, or global mutex-poison bypass.

## STR-01: put every relevant store read through one owned boundary

Files to edit:

| File | Exact action |
| --- | --- |
| `crates/quanta-memory-platform-facade/src/shared/lifecycle.rs` | Add one internal `StoreLifecycle::with_read_state<T,R>(&self, state: &Mutex<T>, f: impl FnOnce(&T)->R) -> MemoryResult<R>`. It performs `ensure_readable`, `try_lock_state`, second `ensure_readable`, `catch_unwind(AssertUnwindSafe(|| f(&guard)))`, drops guard, then returns value or resumes unwind. Keep nonblocking reentry and real poison errors. Keep `invoke_client_read_while_locked` for mutation preflight calls that already hold a permit. |
| `.../src/payload_store/service.rs` | Replace the private guard-returning `lock_read()` and its callers: `charged_bytes`, `pin_count`, and trait `stats`. Move the whole read expression inside `with_read_state`; do not return a reference into state. Leave `pin`/`unpin`/delete mutation plans on their permit path. |
| `.../src/retained_map/service.rs` | Replace `lock_for_read()` use in `committed_bytes`, `stats_checked`, `get_checked`, `keys_checked`, `entries_checked`. Clone keys/values in the protected closure; keep deterministic `sort` outside the lock. Do not change `MapStatePlan`/`restore_state` ownership. |
| `.../src/transcript_store/service.rs` | Replace `lock_read()` use in `retention_policy`, `retained_bytes`, `retention_stats`, `session_bytes`, `checked_stats`, `load_versioned_snapshot`, trait `load`. `session_bytes` must execute `BTreeMap::get(session_id)` inside the common boundary and still return `0` for missing session. For nested `MemoryResult`, unwrap only the outer read result with `?`; preserve the inner typed error. Leave permit-owned `lock_state()` inspections separate. |
| `.../src/publish_pipeline/prepared_publication.rs` | Replace `lock_read()` use in `resolve`, `orphan_tokens`, `stats`. `resolve` still clones the payload and returns typed aborted/unknown errors. STR-03 will later move root normalization outside this read. Do not change prepare/abort/GC state plans in STR-01. |

The `.../` above is
`crates/quanta-memory-platform-facade`. `revocable_store/service.rs` has pure
counter/token/FIFO reads and no caller `Clone/Ord/Hash` in its current read
closure; record that inspection without adding a risky broad refactor there.
After editing, `rg -n 'lock_read\(|lock_for_read\('` on the four named service
files should find no old guard-returning public read path. Calls under a
mutation permit may still use `invoke_client_read_while_locked`.

Test changes, all in the named `service.rs` `#[cfg(test)]` modules:

- Transcript: add `session_bytes_ord_panic_preserves_store_and_ledger` beside
  `session_order_panic_before_mutation_keeps_public_transcript_available`.
  Reuse `ComparisonTrap`/`OrderedSessionId`, seed one row, arm exactly the
  lookup comparison, catch unwind, disarm, require `checked_stats`, `load`,
  `session_bytes`, a subsequent append, `close`, and zero owner bytes/tickets.
  Before production edit, this test must fail because reads return `Poisoned`.
- For payload, retained map, prepared service, reuse their existing client
  panic/reentry fixtures. Add one test per distinct read mechanism that panics
  inside the actual read closure, then assert same-store reuse and charge
  conservation. Do not make a test pass merely because it never reaches the
  callback. For plain counter reads, test correct Open/Busy/Closed images.
- Keep existing tests for mutation-in-progress Busy, close rollback, Drop
  containment, concurrent complete snapshots. A catch helper is allowed to
  resume caller panic; it must not convert it to success or erase a poison
  caused by genuine mutation corruption.

Focused command (test selector is a substring; require its reported count):

```bash
cargo test --locked -p quanta-memory-platform-facade --features transcript --lib session_bytes_ord_panic_preserves_store_and_ledger -- --nocapture
cargo test --locked -p quanta-memory-platform-facade --features full --lib
```

STR-01 is done only when the new RED test is GREEN, the four read inventories
are accounted for, and full-feature facade lib tests pass. This does **not**
complete RFC-0001's future requirement to execute *all* caller code outside
store locks; this boundary contains unwind under the current contract.

## STR-02: core scratch peak and validated pressure thresholds

Files to edit:

| File | Exact action |
| --- | --- |
| `crates/quanta-memory-platform-core/src/ports/scratch.rs` | In `ScratchReservation::grow_to_observed`, keep released and ceiling checks in their current order; then assign `self.observed_bytes = self.observed_bytes.max(observed_bytes)`. Update rustdoc to say a smaller valid observation is accepted without lowering the peak. |
| `crates/quanta-memory-platform-core/src/application/controller.rs` | Make `PressureThresholds`' three fields private. Keep `new(...) -> MemoryResult<Self>`, `validate`, `Default`, and `LadderPressureController::new` behavior. Add `soft_percent()`, `hard_percent()`, `fatal_percent()` read-only getters if consumers need these values. Replace the in-module literal assertion with getters/validated `new`. Do not add an unchecked constructor, silently normalize inputs, or tighten `<=` to `<`. |
| `crates/quanta-memory-platform-core/tests/public_api_surface.rs` | Add a positive external-path test for validated `PressureThresholds::new` and getters. A rustdoc `compile_fail` example adjacent to the type should show that an external struct literal cannot be constructed. The positive test proves the type/import exists, so a compile-fail for an unresolved name cannot be mistaken for protection. |
| `MIGRATION.md` and `CHANGELOG.md` | State that direct struct literals are no longer accepted and point to `PressureThresholds::new`. Treat the private-field change as a source API change in the next versioned release; do not call it an invisible patch. |

Add `scratch_peak_never_decreases_on_smaller_observation` in the `scratch.rs`
test module. Its sequence is:
`0 → 800 → 200 → 800 → 1000`, expected `0/800/800/800/1000`. A request of
`1001` returns `MemoryError::Capacity` and leaves 1000. After release, growth
returns `InvalidState`. Add
`scratch_peak_denial_release_and_drop_keep_authority_exact` in the
`api/control.rs` test module beside `live_runtime_scratch_charge_appears_in_ledger_snapshot`.
Use its `runtime_scratch_broker` and real runtime allocator: reserve 1000,
observe 800 then 200, deny 1001, release, and assert the broker snapshot's
`total_live_scratch_bytes` and `live_ticket_count` are both zero. Repeat in a
separate scope with Drop instead of explicit release. Do not use the
`scratch.rs` `unenforced` helper for this authority assertion.

In `controller.rs`, add an inverted and >100 constructor rejection test and
keep valid default 75/90/98 boundary, zero hard limit, equal-threshold and
`u64::MAX` classification oracles. Confirm every use of `PressureThresholds {`
inside this repo and the current Semantica host; only test literals should need
rewriting here. A public API that still accepts a caller-forged invalid value
does not close ENG-12.

```bash
cargo test --locked -p quanta-memory-platform-core --lib scratch_peak_never_decreases_on_smaller_observation
cargo test --locked -p quanta-memory-platform-core --lib scratch_peak_denial_release_and_drop_keep_authority_exact
cargo test --locked -p quanta-memory-platform-core --lib thresholds_new_rejects_inverted_or_out_of_range
cargo test --locked -p quanta-memory-platform-core --doc
cargo test --locked -p quanta-memory-platform-core --test public_api_surface
```

## STR-03: prepared accounting axis and bounded orphan input

Run after STR-01; both modify `prepared_publication.rs`. The fixed root limit
chosen in the plan requires **no builder field or setter**. Do not edit
`shared/builder.rs` for this package.

| File | Exact action |
| --- | --- |
| `crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs` | In `prepare`, change **only** the `persistent_charge` construction from `ChargeKind::RetainedPayload` to `RetainedMetadata`; keep `active_charge` as Payload. Strengthen `require_charge_shape` to receive the expected `ChargeKind` and check `ManagedCharge::kind()` when a charge exists. Pass Metadata for identity evidence and Payload for active entry at each close/abort/GC call site. Keep two exact leases and the old admission/rollback sequence. |
| Same file: `orphan_tokens`, `gc_orphans`, `classify_orphan_tokens` | Define one private `MAX_REFERENCED_ROOT_TOKENS: usize = 16_384`. Add `normalize_referenced_tokens(&[Token]) -> Result<BTreeSet<Token>, PreparedPublicationErrorV1>`: reject raw slice length above the limit with `Storage(MemoryError::Capacity(_))`, then clone/deduplicate the fixed-size tokens. Run this before `lock_read()` in `orphan_tokens`, and before `begin_mutation()` in `gc_orphans`. Make `classify_orphan_tokens` accept the normalized set by reference. Under the state lock, validate every normalized token as live/nonaborted **before** choosing/removing orphan entries. Preserve empty, duplicate, unknown and aborted semantics under the cap. |
| `crates/quanta-memory-platform-core/src/domain/ledger.rs` | Revise `ChargeKind` rustdoc from literal per-field byte language to D2's owned-lifetime charge bucket. Do not change ticket/ledger arithmetic or enum layout. |
| `crates/quanta-memory-platform-facade/src/shared/measure.rs` | Explain that `RetainedCostV1` components calculate one charge's total while `ChargeKind` classifies the owner component. Do not change formula-v1 constants or `total()`. |
| `README.md`, `docs/verification-scenarios.md`, `MIGRATION.md` | Describe public payload/metadata totals as semantic charge buckets that can include key/index overhead; note persistent prepared evidence now appears in Metadata. Note the new raw-root count cap and typed rejection. |

Extend `prepared_identity_index_and_tombstone_keep_exact_independent_cost`.
Use an independent numeric fixture: identity `[b'i']`, empty `Vec<u8>`.
Expected before prepare `(payload,metadata,scratch,total,tickets)=(0,0,0,0,0)`;
after prepare `(17,129,0,146,2)`; after abort `(0,129,0,129,1)`; after
close `(0,0,0,0,0)`. Assert the same breakdown on owner/class/context maps.
Do not call `active_cost()` or `persistent_cost()` to compute those expected
numbers; their arithmetic drift must fail the test. Also exercise identical
replay (no extra ticket), failed prepare (no charge), GC (active refund with
evidence retained), and failed release retry (original kinds and data intact).

Add `orphan_roots_bound_and_fail_closed` using 1 live entry, 16,384 duplicate
valid tokens, 16,385 duplicate tokens, and an unknown token mixed with valid
ones. At max: normal result. Above max: `Storage(Capacity)`, same payload,
stats and ledger. Unknown/aborted: typed prior error, no partial GC. Repeat
for `orphan_tokens` and `gc_orphans`. A same-thread reentrant read during GC
must be Busy; a valid call after denial must still succeed.

```bash
cargo test --locked -p quanta-memory-platform-facade --features publish_pipeline --lib prepared_identity_index_and_tombstone_keep_exact_independent_cost
cargo test --locked -p quanta-memory-platform-facade --features publish_pipeline --lib orphan_roots_bound_and_fail_closed
cargo test --locked -p quanta-memory-platform-facade --features full --lib
```

Do not rename existing public metric fields or split the persistent ticket into
several leases. Those changes alter management limits and failure semantics.

## STR-04: broker batch validation and reclaim contract

Files to edit:

| File | Exact action |
| --- | --- |
| `crates/quanta-memory-platform-core/src/api/control.rs` | In `release_managed_many`, perform impossibly large batch length, broker-affinity, active-lease and checked byte-sum validation before `Vec::with_capacity(charges.len())`. Walk the slice without cloning tickets during validation. Only then collect tickets and call the existing `release_many_on`; set every `charge.charge=None` only on full success. Emit one outcome on each path, after the broker lock is gone. Keep an empty batch successful. |
| `crates/quanta-memory-platform-core/src/domain/management_limits.rs` | Reuse `MAX_LIVE_TICKETS_PER_BROKER` for the impossibly large active batch check; do not add a second numeric copy. This file is **read-only** unless its documentation needs clarification. |
| `crates/quanta-memory-platform-core/src/ports/reclaim.rs` | Document callback contract: synchronous; same-store reentrant mutation may return `OperationInProgress`; returned claimed bytes are advisory and broker remeasures usage. No callback deadline or RSS guarantee. Do not change the trait signature. |
| `crates/quanta-memory-platform-facade/src/revocable_store/service.rs` | Add an owner test for same-store `reclaim_until` called while `insert_checked` holds its mutation permit. Expect typed Busy in the callback and unchanged FIFO/cost if admission is denied. Avoid permit release/reacquire inside `insert_checked`. |
| `crates/quanta-memory-platform-facade/tests/cross_store_hellgate.rs` | Add a public two-store test with one broker: a reclaimer evicts from the *other* revocable store and admission succeeds after actual ledger usage falls; compare before/after bytes and final ticket drain. Use `MemoryPlatformBuilder::with_reclaimer` and a test callback registered before platform construction. |

The source sequence in `release_managed_many` should be visibly:

```text
len / affinity / active / checked-sum preflight (no ticket vector)
    -> Vec::with_capacity and clone validated ticket identities
    -> one ledger release_many_on under broker lock
    -> clear owned leases after success
    -> one event outside lock
```

For the impossibly large slice, use the existing live-ticket cap. If its
early error changes which of two invalid conditions is reported, specify that
precedence in the API/test; do not let `bytes().unwrap_or(0)` turn an inactive
lease into an accepted zero. Preserve release failure atomicity in the tests
`managed_charge_batch_failure_retains_every_lease` and
`managed_charge_batch_rejects_foreign_broker_before_mutation`.

Add `managed_batch_invalid_lease_rejects_before_ticket_collection`: include
an inactive lease at the first and last positions in separate cases, one
foreign broker lease, a valid mixed zero/nonzero batch, and a rebuild-fenced
valid batch. Check exact IDs, active flags, owner totals, kind totals and
ticket count before/after; valid retry must drain. The unit test proves
authority preservation. Validate the source order for preallocation. A claim
about actual allocator call count requires the existing benchmark allocator
instrumentation; `testkit::AllocationProbe` alone installs no allocator and
must not be presented as proof of a zero-allocation rejection path.

For the same-store case, have the callback record the inner Busy result and
return `Err(OperationInProgress)`; the outer hard-cap admission should be
`Capacity` if no actual bytes were freed. Check the resident value/charge
remains. For the other-store case, return a `ReclaimOutcome` and show the
broker remeasures the ledger rather than trusting that number. Use a barrier
test in core if the broker-lock boundary needs proof; never use sleeps as a
timing oracle. Callback wall-time remains a host requirement.

```bash
cargo test --locked -p quanta-memory-platform-core --lib managed_batch_invalid_lease_rejects_before_ticket_collection
cargo test --locked -p quanta-memory-platform-facade --features revocable_store --lib same_store_reclaimer_returns_busy_without_losing_fifo
cargo test --locked -p quanta-memory-platform-facade --features full --test cross_store_hellgate
```

## STR-05: documentation and the existing CI job

This package edits only docs, PM source authority and CI configuration. Keep
the repo's main-only automatic workflow and existing machine/job. Do not add a
new paid workflow as part of this fix.

| File | Exact action |
| --- | --- |
| `.circleci/config.yml` | Inside the `full-extraction` job's final `run.command` block, immediately after `scripts/extract-to-standalone-repo.sh "$qmp_extract_dir"`, add the exact subshell shown below to run both examples inside the extracted checkout. Keep the final clean `git -C "$qmp_extract_dir" status --porcelain` assertion after it. Leave the `main` filter intact. |
| `README.md` | Make the push-CI statement true once the two commands are in CircleCI. Also explain the D2 bucket/overhead meaning without promising RSS accuracy. |
| `README-detailed.md` | Replace “no GitHub Actions workflow” with “manual `workflow_dispatch` only”. Correct the benchmark matrix: retained-map point read currently has one thread; other rows have their own 1/2/8/32 or pressure-profile thread sets. |
| `tools/pm/sources/rules/architecture.md` | Replace “ports/ contains traits only” with “pure ports contracts: traits, DTOs, and inert pure helpers; no host I/O/clocks”. Run `python3 tools/pm/pm.py sync`; generated `AGENTS.md` and other targets must be changed only by sync. |
| `scripts/tests/test_ci_workflows.py` (new) | Add a stdlib-only CI contract test. Extract the `full-extraction` job's `Verify actual standalone extraction and full workspace gate` command block by indentation, reject duplicate/missing extraction or example commands, and assert order: extraction → subshell `cd` → `hello_world` → `effective_budget_report` → clean-status check. Require both example lines to start with `cargo run --locked`. Add a negative fixture formed by deleting one example command from the parsed block and require the validator to fail. Inspect only the top-level event block of `.github/workflows/ci.yml`: require `workflow_dispatch`, reject `push`/`pull_request`; a fixture adding top-level `push:` must fail. This test checks policy text, not whether CircleCI executed it. |

Add these lines inside the existing CircleCI command block, preserving its
YAML indentation and shell `set -euo pipefail`:

```bash
(
  cd "$qmp_extract_dir"
  cargo run --locked -p quanta-memory-platform-facade --example hello_world
  cargo run --locked -p quanta-memory-platform-facade --example effective_budget_report
)
```

The PM authority change requires the generated-output sync. Check the actual
CircleCI result for the final committed source separately; local YAML/doc
checks do not claim the hosted job ran. `scripts/check-workspace.sh` discovers
`scripts/tests/test_*.py` automatically, so no separate registry edit is needed.

```bash
python3 tools/pm/pm.py sync
python3 tools/pm/pm.py lint
python3 -m unittest scripts.tests.test_ci_workflows
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
git diff --check
```

## STR-07: one narrow integration pass and final gate

| File | Exact action |
| --- | --- |
| `crates/quanta-memory-platform-facade/tests/cross_store_hellgate.rs` | Add `mixed_store_kind_lifetimes_balance_every_axis`. Extend its feature gate for `publish_pipeline` only around the new imports/test, so existing map/revocable selectors still compile alone. Use three distinct registered owners in a dedicated fixture, without changing older shared policy fixtures. One map entry, one revocable entry, one empty-payload prepared entry give Payload=17, Metadata=42+129=171, Scratch=34, total=222, live tickets=4 for the one-byte keys/values described below. After abort: Payload=0, Metadata=171, Scratch=34, total=205, tickets=3. Close each store and require every kind/owner/class/context total and ticket count zero. |
| `crates/quanta-memory-platform-facade/src/payload_store/service.rs` and `transcript_store/service.rs` tests | Pin their existing bucket roles separately. Payload store's composite entry stays Payload. Transcript active row stays Payload; session key/evidence stays Metadata. Avoid adding a huge all-store E2E just to restate local cost oracles. |
| `docs/plans/bugbash-sep-26-engine/README.md` and `STRUCTURAL-REMEDIATION-PLAN.md` | Change status only for the package actually implemented and evidenced. Record source SHA/tree, dirty state, focused RED/GREEN, full gate, and any host/benchmark gap. A plan file alone is not closure. |

Numeric cross-store fixture: map key `"m"` plus `vec![9]` costs
`1+1+24+16=42` Metadata; revocable key `"r"` plus `vec![8]` costs
`1+1+16+16=34` Scratch; prepared identity `[b'i']` plus empty `Vec<u8>`
costs 17 Payload +129 Metadata. Use public builders and snapshot fields,
not internal cost helpers. The prepared service's binding needs a budgeted
class/owner in the test fixture. Use the owner's own class/context breakdown
to detect correct grand totals with wrong attribution.

Final verification is performed **after all code/config/docs are frozen**:

```bash
cargo fmt --all --check
python3 tools/pm/pm.py sync
scripts/check-workspace.sh
git diff --check
git status --short
git rev-parse HEAD
git rev-parse 'HEAD^{tree}'
```

Run the extraction dry-run if Cargo features/dependencies or cross-crate use
paths changed, on a committed clean candidate only:

```bash
scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-check
```

The full workspace rail exercises optional Redb regression selectors; it is
not a declaration that Redb is a supported product. An in-memory development
run with `scripts/check-workspace.sh --in-memory` cannot replace the full
final gate. For a standalone-export claim, the actual clean extraction and
its rail/examples are a separate receipt. For host adoption, compile and run
the exact Semantica revision against the changed QMP source separately.

## STR-06: performance, only after the correctness source stops changing

There is currently no measured ENG-05/06 bottleneck or approved consumer SLO.
This is a measurement task, not permission to shard the ledger or remove full
invariant checks. Do it on a quiet machine with the same benchmark source and
configuration for baseline and candidate.

| File | Exact work when measurement is requested |
| --- | --- |
| `crates/quanta-memory-platform-facade/benches/scaling.rs` | Add actual public-path cells for retained-map overwrite/recharge, broker observe/reclaim planning, transcript stats under increasing sessions/rows/evidence, 1/2/8/32-thread retained-map reads/mix, and prepared GC at the raw-root cap. Emit attempts/successes/Busy and latency separately. Implement a benchmark-only uninstrumented timing mode if comparing scheduler tails; record the mode. |
| `scripts/verify-scaling-results.py` | Extend `expected_keys`, row schema/semantic-zero checks and release compatibility to cover new cells and mode. A missing/duplicated row must fail. If schema changes, version it and do not admit older receipts. |
| `scripts/tests/test_scaling_results.py` and `scripts/tests/test_scaling_build_config.py` | Add deliberate missing-row, fake-final-zero, incompatible-mode/harness, high-load/noise and regression negative fixtures. Keep the verifier's existing 3 baseline + 3 candidate requirement. |
| `docs/verification-scenarios.md` | Name actual new cells and conditions. Record the consumer workload (size, thread count, polling frequency), chosen p99/successful-throughput/Busy limits and machine envelope **before** collecting candidate results. |

The runner is `scripts/run-scaling-bench.sh`; it rejects a dirty checkout.
`--profile=in-memory-pressure` uses the in-memory `full` features, while the
standard profile currently compiles all features. Because the verifier binds
the hash of `scaling.rs`, baseline and candidate need the *same* new harness
and build configuration. Collect three independent full-process runs per
source and pass all `--release-*` limits explicitly to
`scripts/verify-scaling-results.py --release`. If run noise exceeds the
approved limit, label the comparison INCONCLUSIVE; do not loosen the bound
after seeing the candidate. Only then decide whether scheduler clone or
transcript stats traversal is a real bottleneck.

## Conditional 0.3 migration: do not mix into the immediate repair

ENG-03/04 and RFC-0001's full out-of-lock callback contract need a named
consumer and versioned API migration. The owning files and acceptance points
are recorded here so a later implementer does not patch only one facade call:

| File | Required future migration |
| --- | --- |
| `crates/quanta-memory-platform-core/src/domain/ledger.rs` | Replace boolean rebuild fence with broker/epoch-bound authority state; reject second begin and foreign/stale finish. Keep pending rollback/deferred release accounting exact. |
| `crates/quanta-memory-platform-core/src/api/control.rs` | Expose a non-Clone rebuild permit; remove public no-token finish path when migrating. On permit Drop, leave an observable abandoned fence rather than silently reopening writes. Distinguish fence completion from bounded deferred cleanup receipt. |
| `crates/quanta-memory-platform-core/src/domain/policy.rs`, `domain/context.rs`, `domain/config.rs`, `api/builder.rs` | Move host taxonomy to validated registered IDs, separate non-memory execution policy, make headroom authority non-forgeable, retain old cap/floor/burst math. |
| `crates/quanta-memory-platform-facade/src/shared/builder.rs`, five store services, `core/src/lib.rs`, `facade/src/lib.rs` | Add the builder-first replacement before removing broker escape; preserve stored object + charge lifetime when moving caller comparisons/clones outside a lock. `Arc<Value>` alone does not keep its managed charge alive. |
| `crates/quanta-memory-platform-core/tests/public_api_surface.rs`, `facade/tests/public_api_surface.rs`, relevant owner tests | Add compile-fail authority misuse, two broker clones' foreign/stale finish rejection, abandoned fence/recovery, 0-byte deferred cleanup, slow/reentrant/panicking caller callbacks versus close and reclaim. |
| `docs/rfcs/0001-sdk-interface-contract.md`, `MIGRATION.md`, real public examples | Update API contract and examples on the same candidate; do not call RFC acceptance an implementation receipt. |

No host source is edited by this runbook. Before declaring host adoption,
inventory `quanta-memory-host` users and the usage projection in
`quanta-runtime-memory-compose`; publish the changed kind-bucket meaning and
API migration there with its own exact-revision test receipt.

## Handoff format for each worker

Report the exact source HEAD/tree and own diff; file list; the RED test and
why it failed before production edit; GREEN selector/count; any existing
test failure; retained owner/ledger postimage; and what remains for the
integrator. A failing test caused by unresolved imports, skipped feature, or
setup error is not a behavioral RED. Do not write `DONE` from `cargo check`,
a single focused test, the diagnostic program, or a benchmark JSON verifier.
