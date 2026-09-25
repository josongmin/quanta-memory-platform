# Engine memory substrate architecture bugbash — 2026-09-26

Status: **AUDIT ONLY — findings recorded, fixes not implemented or qualified**.

Audit source: `main` HEAD `96d0a256c05f1e74995e912571546bfc60ed89f1`, tree
`a3ad563845a170377168aaf201c7c1285d3fa1df`. Source code was clean;
the worktree had a documentation-only overlay (`docs/plans/README.md` and this
new directory) during the second pass. This document covers the
Sep-26 architecture/internal-logic reviews of QMP's in-memory product scope.
It does **not** reopen every historical checklist item, qualify the Semantica
host, or promote the optional Redb backing to a supported product capability.
Re-freeze HEAD and dirty state before implementing or accepting any receipt.

## Verdict and triage

The `core <- facade <- testkit` dependency direction, pure admission decision,
single ledger accounting authority, owned charge lifecycle, and feature isolation
are structurally sound. No new P0/P1 ledger corruption or fail-open path was
confirmed in the inspected source. That is not an exhaustive correctness proof.
The findings below distinguish a source-confirmed defect from a conditional
host contract, known open work, and measurement/documentation gaps.

| ID | Priority / classification | Finding | Disposition |
| --- | --- | --- | --- |
| ENG-01 | **P2 / source-confirmed availability defect** | Transcript `session_bytes()` can poison its store mutex if caller `SessionId::Ord` panics. | Fix and add owner-local regression. |
| ENG-02 | **P2 if self-reclaim is required / conditional liveness** | A writer's store cannot be synchronously reclaimed by its own broker callback while its mutation permit is held. | Decide host contract; then add the matching integration test and fix only if required. |
| ENG-03 | **P2 if rebuild has independent coordinators / authority contract** | Any broker clone may finish another caller's boolean rebuild fence. | Specify single owner or use token/epoch authority. |
| ENG-04 | **P2 SDK roadmap / existing design debt** | Closed, host-shaped class/context taxonomy and caller-selectable `Headroom` limit generic SDK and privilege claims. | Follow RFC-0001 when broadening the consumer/API contract; not a current ledger bug. |
| ENG-05 | **P2 if policy is dynamic; otherwise P3 / known open bound** | Policy-map cardinality is unbounded; recharge and reclaim/observe do group/owner-scale work under the broker lock. Transcript statistics also scan all sessions/rows under their store lock. | Decide count and polling contracts; measure actual paths. B07/J06 already track the broader issue. |
| ENG-06 | **P2 proof gap / no product perf defect established** | Current bench receipts do not prove an approved SLO, an uninstrumented peak, or SOTA concurrency. | Add controlled consumer workload and release comparison before optimization claims. |
| ENG-07 | **Conditional host operating contract** | Synchronous reclaimer/event-sink duration and pre-admission transient allocation/RSS are not bounded by semantic accounting. | Bound callbacks at the host and input/OS boundary; do not label QMP as an RSS limiter. |
| ENG-08 | **P3 / host-controlled batch input** | `release_managed_many()` allocates for the whole caller slice before rejecting an invalid lease. | Validate/bound batch before allocation if large batches are in scope. |
| ENG-09 | **P3 / documentation and CI proof scope** | Example-execution, GitHub Actions, `ports/`, and scaling-matrix prose misdescribe current source; repository CI has no automatic PR/non-main full-extraction trigger. | Correct claims and explicitly choose premerge versus main-only qualification. |
| ENG-10 | **P3 / conditional service availability** | Prepared-publication orphan APIs build a set from the entire caller-supplied root-token slice while holding the store lock. | Set a root-input contract and move/bound normalization outside the critical section; test oversized/duplicate inputs. |
| ENG-11 | **P3 / source-confirmed API semantic mismatch** | Scratch `grow_to_observed()` can lower the value documented as peak observed usage. | Make the value monotonic (or reject a lower observation) and add a negative owner-local test. |
| ENG-12 | **P3 / public low-level policy contract** | A caller can bypass pressure-threshold validation with public fields, and the public controller constructor accepts the invalid value. | Make validation unavoidable at the controller boundary or explicitly narrow the low-level API contract. |
| ENG-13 | **P3 / source-confirmed kind-axis classification** | Prepared-publication identity/tombstone metadata is charged as `RetainedPayload`, so the public payload/metadata breakdown is misleading after abort. | Decide per-byte versus store-level kind semantics; pin the chosen axis with a tombstone oracle. |

## Findings and acceptance oracles

### ENG-01 — transcript read panic poisons authoritative state

- Evidence: [`session_bytes()`](../../../crates/quanta-memory-platform-facade/src/transcript_store/service.rs#L461)
  calls `BTreeMap::get(session_id)` while retaining the guard from `lock_read()`.
  `SessionId: Ord` is caller-implemented. Unlike other read paths, this one
  bypasses [`invoke_client_read_while_locked`](../../../crates/quanta-memory-platform-facade/src/shared/lifecycle.rs#L45),
  which releases the guard before resuming a panic. The lifecycle maps a
  poisoned mutex to a persistent typed error. The path is source-confirmed;
  no new executable reproducer was run in this audit.
- Owner-local RED/GREEN: seed one session; arm a custom `SessionId::cmp` to
  panic on the `session_bytes()` lookup; catch the unwind; then require
  `checked_stats`, `load`, `append`, `close`, and final broker ledger drain to
  work. Protect the lookup with the existing read-side helper or an equivalent
  outside-lock comparison. Do not weaken poison detection globally.

### ENG-02 — same-store callback cannot reclaim its active writer

- Evidence: [revocable insert](../../../crates/quanta-memory-platform-facade/src/revocable_store/service.rs#L1266)
  acquires `StoreLifecycle::begin_mutation` before `accounting.charge` may call
  the host reclaimer. Both [`reclaim_one`](../../../crates/quanta-memory-platform-facade/src/revocable_store/service.rs#L1295)
  and [`reclaim_until`](../../../crates/quanta-memory-platform-facade/src/revocable_store/service.rs#L220)
  need the same nonblocking permit. A callback targeting that store receives
  `OperationInProgress`, so a hard-cap insert can fail despite resident FIFO
  victims. This is bounded denial, not deadlock or ledger corruption. No
  same-store reclaimer integration test exists; the inspected Semantica host
  source does not install a QMP reclaimer (see host spot-check below).
- Decision: if active-writer self-reclaim is **unsupported**, document that the
  host must choose another victim or retry after the writer exits; test the
  bounded denial. If **required**, design pre-admission reclaim plus revalidation
  or a structured reservation/retry protocol and test admission, victim
  identity, exact refund, and final zero. Merely dropping the mutation permit
  around `charge` is unsafe for stores with detached in-flight state.

### ENG-03 — rebuild fence has no owner token

- Evidence: public [`begin_rebuild`/`finish_rebuild`](../../../crates/quanta-memory-platform-core/src/api/control.rs#L1570)
  change one [ledger boolean](../../../crates/quanta-memory-platform-core/src/domain/ledger.rs#L433).
  The existing double-begin/single-finish [test](../../../crates/quanta-memory-platform-core/src/api/control.rs#L3233)
  deliberately pins idempotence; a distinct broker clone can therefore open
  another caller's fence. The inspected Semantica rebuild call sites were
  test-only; no independent-coordinator production host failure was found.
- Acceptance: establish a single-coordinator public contract, or introduce an
  owned token/epoch whose finish/drop can affect only its own rebuild. A
  two-clone test must prove the chosen semantics and deferred charge cleanup.

### ENG-04 — portable taxonomy and privileged headroom are still 0.2 debt

- Evidence: core's closed [`MemoryClass`](../../../crates/quanta-memory-platform-core/src/domain/policy.rs#L125)
  contains PTA, reasoning-graph, trace, and IDE nouns; [`ExecutionBudget`](../../../crates/quanta-memory-platform-core/src/domain/policy.rs#L15)
  includes subquery/trace-edge caps. A second host cannot add its own class
  without changing core. The facade exposes its [broker](../../../crates/quanta-memory-platform-facade/src/shared/builder.rs#L172),
  whose public `reserve_managed_charge` accepts caller-selected `Headroom`.
  A current public-surface [test](../../../crates/quanta-memory-platform-core/tests/policy_contracts.rs#L208)
  proves that class can enter the reserved band. This is a trusted-caller
  authority footgun, not an external exploit or accounting bypass.
- Existing owner: [RFC-0001](../../rfcs/0001-sdk-interface-contract.md)
  already proposes host-declared class IDs, moving non-memory execution policy,
  and a non-forgeable headroom role. Do not count this plan as implementation.
  Before 0.3 migration, inventory actual host uses and keep the current
  headroom/floor math and no-raw-authority guarantees in public negative tests.

### ENG-05 — management cardinality and lock-held work

- Evidence: policy exposes public owner/group maps, while
  [validation](../../../crates/quanta-memory-platform-core/src/domain/config.rs#L255)
  limits identifier byte length but not map count. Custom policies may contain
  arbitrarily many zero-reserved groups. Retained-map overwrite reaches
  [`recharge`](../../../crates/quanta-memory-platform-core/src/api/control.rs#L703),
  which [clones the entire scheduler](../../../crates/quanta-memory-platform-core/src/api/control.rs#L724)
  (groups, usage, fair shares) under the global broker mutex. A soft/hard
  [reclaim request](../../../crates/quanta-memory-platform-core/src/api/control.rs#L898)
  clones a ledger snapshot and builds/sorts every group candidate under that
  mutex; [`observe`](../../../crates/quanta-memory-platform-core/src/api/control.rs#L1782)
  also clones report state. Separately, transcript [`checked_stats()`](../../../crates/quanta-memory-platform-facade/src/transcript_store/service.rs#L473)
  and [`retention_stats()`](../../../crates/quanta-memory-platform-facade/src/transcript_store/service.rs#L323)
  call `validate_component_counters` while holding the store mutex; the check
  traverses every session, active row, and evidence record. The default preset
  has no groups, and no high-cardinality transcript polling latency has been
  measured. These are conditional risks, not measured normal-host regressions.
- Existing tracking: [FUP-007 clarification](../sep-23-followup-misc/FUP-007-zero-byte-ticket-scope.md#L125)
  and [receipt](../sep-23-followup-misc/RECEIPT.md#L427) leave broader B07/J06
  metadata/backlog bounds open. The [plan index](../README.md) says they are
  implementation/test contracts rather than open plan items; reconcile that
  status before assigning new duplicate tickets.
- Acceptance: decide whether policy maps are trusted small configuration or
  dynamic/user-derived input. If dynamic, set per-map count limits with negative
  validation tests. Measure public retained-map overwrite/recharge and
  pressure-plan p95/p99 plus broker lock wait at 0/100/1,000 groups. Optimize
  the transactional delta or precompute victim order only if the consumer SLO
  shows a real bottleneck. The existing scheduler `admits()` cardinality bench
  does **not** time the overwrite/recharge path. For transcript, compare
  `checked_stats`/`retention_stats` and concurrent append p99 at increasing
  session, row, and evidence counts under the intended polling frequency;
  retain a complete invariant check somewhere if the hot stats path is changed.

### ENG-06 — SOTA performance remains unproven

- Evidence: one [global broker mutex](../../../crates/quanta-memory-platform-core/src/api/control.rs#L127)
  serializes accounting mutations; each store has one mutation lane; retained-map
  [point read](../../../crates/quanta-memory-platform-facade/src/retained_map/service.rs#L509)
  clones its value under the state mutex. The [scaling benchmark](../../../crates/quanta-memory-platform-facade/benches/scaling.rs#L1279)
  measures retained-map hit/miss only at one thread. Its process-global tracked
  allocator performs shared atomic updates inside timed multithreaded workloads,
  so measured tails include instrumentation contention. The ordinary v4
  receipts (three 60-row pressure runs and one 220-row standard run on this
  source) passed structural/absolute verification, but recorded high host load
  and have no approved consumer SLO or compatible 3+3 release baseline.
  [Verification scenarios](../../verification-scenarios.md#L83) explicitly
  distinguish ordinary validation from release comparison. The manually run
  [CircleCI benchmark](../../../.circleci/benchmark.yml) does not supply a
  release baseline. No new benchmark process was run for this document.
- Acceptance: define the consumer workload and p99/throughput/Busy SLO; collect
  three baseline and three candidate full-process receipts on a controlled
  host; measure broker lock wait, callback latency, multithreaded reads and a
  read/write mix; quantify allocation-tracker overhead with a separate
  uninstrumented timing mode. Compare the **same accounting semantics**, not
  raw QMP throughput to a cache's hit-rate/data-path benchmark. Caffeine's
  [striped, lossy read buffer](https://github.com/ben-manes/caffeine/wiki/Design)
  is valid for advisory eviction metadata but cannot replace lossless ledger
  writes; [CacheBench](https://cachelib.org/docs/Cache_Library_User_Guides/Cachebench_Overview/)
  demonstrates workload/trace-based cache evaluation, not a QMP score.

### ENG-07 — explicit operating boundary, not a QMP correctness defect

- The [`MemoryReclaimer`](../../../crates/quanta-memory-platform-core/src/ports/reclaim.rs#L54)
  callback is synchronous and inline on admission. Single-flight and panic
  quarantine bound reentry, **not** wall time: a stuck host callback stalls
  that reservation. The host must install a bounded/nonblocking reclaimer and
  measure its latency/fallback behavior.
- [`MemoryEventSink::emit`](../../../crates/quanta-memory-platform-core/src/api/control.rs#L528)
  is also synchronous after broker operations. Its single-flight and panic
  quarantine do not impose a deadline; a stuck host sink can delay the caller's
  response even after the accounting postimage is committed. A host-installed
  sink must be nonblocking or separately bounded. Neither callback was found
  installed in the inspected Semantica host source at its recorded HEAD below.
- `CanonicalKey` may materialize an owned buffer before broker admission;
  [payload insertion](../../../crates/quanta-memory-platform-facade/src/payload_store/service.rs#L291)
  and an existing [1 MiB transient test](../../../crates/quanta-memory-platform-facade/src/payload_store/service.rs#L798)
  make the semantic-versus-transient distinction explicit. QMP does not limit
  arbitrary caller allocations or RSS. Untrusted inputs need host input/scratch
  bounds and, where applicable, OS-level physical-memory control such as
  [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html).

### ENG-08 — batch validation before allocation

Public [`release_managed_many`](../../../crates/quanta-memory-platform-core/src/api/control.rs#L1510)
does `Vec::with_capacity(charges.len())` before checking broker affinity or
whether each lease is active. A host can retain many inactive leases beyond
the broker's 16,384 **live** ticket cap and pass a large invalid slice, causing
an avoidable allocation before typed rejection. This is a low-priority
host-controlled input path, not a proved OOM. If kept, add a focused invalid
batch/allocation oracle; otherwise document a trusted small-batch contract.

### ENG-09 — documentation and CI contract corrections

- [README](../../../README.md#L129) says push CI executes both public examples
  on the extracted source. [CircleCI main](../../../.circleci/config.yml#L34)
  runs extraction and its full workspace rail but not the two `cargo run`
  examples. The manual [GitHub workflow](../../../.github/workflows/ci.yml#L57)
  does run them. Add examples to push CI or narrow the README claim.
- [README-detailed](../../../README-detailed.md#L419) says no GitHub Actions
  workflow exists, despite the manual workflow. It also describes a
  1/2/8/32-thread read/write/mixed matrix broadly, while retained-map point
  reads are only single-thread. State the actual profile precisely.
- The [architecture rule](../../../tools/pm/sources/rules/architecture.md#L44)
  says `ports/` contains only traits. It also contains contract DTOs and pure
  `NoopMemoryEventSink`/`StaticAllocatorTelemetry` implementations. Clarify
  the rule; their presence is not an I/O or dependency-direction leak.
- Repository-configured automatic [CircleCI full extraction](../../../.circleci/config.yml#L44)
  is filtered to `main`. The [GitHub workflow](../../../.github/workflows/ci.yml#L3)
  is dispatch-only, and the separate benchmark is manual. No repo-defined
  PR/non-main automatic full-extraction trigger was found. This is a
  **conditional premerge proof gap**, not a demonstrated bad merge: external
  required checks and branch protection were not verified. If premerge proof
  is promised, add an affordable candidate gate and a config-level negative
  trigger test; otherwise document main-only/postmerge qualification explicitly.

### ENG-10 — orphan root normalization inside the store critical section

- Evidence: both [`orphan_tokens()`](../../../crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs#L765)
  and [`gc_orphans()`](../../../crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs#L780)
  call [`classify_orphan_tokens`](../../../crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs#L938)
  under the prepared-state mutex. It clones every supplied token into a
  `BTreeSet` before validating whether the root exists. The caller can supply
  an arbitrarily long or duplicate-heavy slice even when the service owns one
  prepared entry. Time is at least proportional to input length and distinct
  roots allocate additional memory while the store is unavailable to other
  calls (`OperationInProgress`). This is a source-confirmed complexity path;
  no high-cardinality host input or latency failure was measured.
- Acceptance: define whether root snapshots are trusted and their maximum
  cardinality. Normalize and reject excess input before acquiring the state
  mutex, or provide an explicitly bounded/streaming contract. Keep the
  existing atomic rule that an unknown/aborted reference cannot partially GC
  valid entries. Owner-local tests should cover 1 live entry with a large
  duplicate/unknown root slice, exact error, unchanged payload/ledger, and a
  bounded concurrent read/mutation outcome. Measure critical-section p99 at
  the approved maximum; do not infer a product SLO from asymptotic analysis.

### ENG-11 — scratch observation is not monotonic

- Evidence: [`grow_to_observed()`](../../../crates/quanta-memory-platform-core/src/ports/scratch.rs#L115)
  is documented as recording the **peak** observed scratch usage, but assigns
  `self.observed_bytes = observed_bytes` after only checking release state and
  the reservation ceiling. `grow_to_observed(800)` followed by
  `grow_to_observed(200)` therefore reports 200. The current [positive test](../../../crates/quanta-memory-platform-core/src/ports/scratch.rs#L214)
  calls it only in increasing order. A repository search found no production
  consumer of `observed_bytes()`; ledger reservation/refund stays unchanged.
  This is a public observation-contract defect, not a capacity bypass.
- Acceptance: preserve 800 on a lower observation or reject the call with a
  typed error, then pin the chosen semantics in the port test. Prove an
  over-reservation attempt leaves the peak unchanged and `release`/Drop still
  drain the exact live scratch ticket.

### ENG-12 — public pressure controller accepts an invalid ladder

- Evidence: [`PressureThresholds`](../../../crates/quanta-memory-platform-core/src/application/controller.rs#L38)
  exposes all three percentage fields. Its [`new()`](../../../crates/quanta-memory-platform-core/src/application/controller.rs#L58)
  correctly rejects non-monotonic or >100 values, but a struct literal bypasses
  that check. Public [`LadderPressureController::new()`](../../../crates/quanta-memory-platform-core/src/application/controller.rs#L103)
  stores the supplied thresholds without calling `validate()`. For example,
  `(soft, hard, fatal) = (90, 75, 50)` classifies 50% usage as `Fatal`, making
  `Soft` and `Hard` unreachable. Both types are [publicly re-exported](../../../crates/quanta-memory-platform-core/src/lib.rs#L97)
  under `live_control`. The live broker itself constructs only the valid
  default thresholds, so this is **not** evidence of a broker-ledger or current
  host failure. The existing negative test exercises `PressureThresholds::new`
  only, not the controller's acceptance boundary.
- Acceptance: make thresholds non-forgeable or make controller construction
  fallible with `validate()` at its boundary; do not replace invalid input with
  an unreported default or panic. Pin a public-API negative test for an inverted
  and >100 literal and a valid 75/90/98 transition test. Decide whether the
  separately exported scheduler/controller are supported SDK surfaces before
  changing signatures in 0.2 versus the planned API migration.

### ENG-13 — persistent prepared metadata is reported as payload

- Evidence: [`persistent_cost`](../../../crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs#L269)
  contains identity, index, and fixed metadata, with no payload component.
  The [persistent lease](../../../crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs#L610)
  is nevertheless reserved as `ChargeKind::RetainedPayload` and remains after
  abort as the identity tombstone. Core's [`ChargeKind`](../../../crates/quanta-memory-platform-core/src/domain/ledger.rs#L51)
  documents payload and metadata byte categories; the public ledger snapshot
  and Semantica usage projection expose separate totals. Thus an empty-payload
  `prepare` → `abort` leaves a metadata-only tombstone in the payload axis.
  Existing [tombstone accounting test](../../../crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs#L1493)
  checks total bytes and drain but not the kind split. Total admission and
  refund remain exact; no capacity bypass was found.
- Scope caveat: other facade stores charge composite payload/key/index costs
  as one kind, so fixing this requires an explicit axis contract, not an
  isolated enum swap without checking all consumers. If `ChargeKind` means
  store-level purpose rather than the documented byte category, revise its
  documentation and projected metric names instead of claiming byte-accurate
  payload/metadata telemetry.
- Acceptance: with empty payload, assert the prepared and aborted ledger's
  payload/metadata kind totals against the chosen contract, then close and
  verify every kind and owner total is zero. Include a cross-store axis table
  for payload, retained map, revocable, transcript, and prepared publication
  before changing charge construction. This is P3 observability semantics, not
  a P1 accounting total defect.

## Verified boundaries and non-findings

- Resolved Cargo graph respects `core <- facade <- testkit`. The facade bare
  graph does not activate core live control; `full` excludes Redb, unbudgeted
  stores, and durability hooks. `spill-redb` and disk-failure rails remain
  explicit opt-ins, outside the current in-memory product release claim.
- Admission preflight is a pure decision and the ledger is the mutation
  authority. Reclaim callbacks run outside the broker mutex and admission
  remeasures actual ledger usage. Owned charges and deferred cleanup retain
  release authority. The static group scheduler is weighted **capacity
  apportionment**, not WFQ, a rate scheduler, or an eviction algorithm.
- A revocable entry always incurs fixed index/state cost, so its zero-payload
  entry is **not** a zero-byte ticket; `reclaim_until` skipping zero-byte
  entries is not a reachable revocable-store bug. Generic broker zero-byte
  tickets have their own count and identifier bounds.
- No new default/full Redb dependency leak, cross-store ledger fail-open,
  or additional in-memory verifier false-green was found in this audit.
  Absence of a finding is limited to the inspected paths, not a 100-point
  correctness or release claim.
- A max-scale `u64::MAX` charge followed by a 1-byte request is deliberately
  denied before callback construction because the prospective total is not
  representable. The [explicit test](../../../crates/quanta-memory-platform-core/src/api/control.rs#L6748)
  pins this fail-closed extreme; it is not a newly discovered normal-workload
  reclaim defect.

### Current Semantica host reachability spot-check — not host qualification

Read-only inspection of Semantica HEAD `a6b1e65ad89432ffbe66f81e88ce70cda7b60f97`
found unrelated dirty files, but the integration files cited below were clean.
Its workspace depends on this QMP repository by path. No Semantica build,
runtime, or E2E was run, so this narrows reachability only:

- `quanta-memory-host/src/session/transcript_adapter.rs` uses a workspace/session
  key with derived `Ord` over `String`s and calls QMP `session_bytes`. ENG-01
  remains a generic caller-implemented-`Ord` defect, not a demonstrated panic
  path for this host key.
- No `.with_reclaimer` or `.with_event_sink` call was found in the inspected
  Semantica Rust source. ENG-02 and the callback portion of ENG-07 are therefore
  unactivated in this observed host wiring; installing either callback changes
  that assessment. Rebuild begin/finish call sites found were inside tests, so
  ENG-03 is likewise a public authority contract rather than a proved active
  multi-coordinator failure.
- The feature-gated manifest composer supplies root-token snapshots to
  `gc_orphan_manifests_v1`; no maximum root count or latency receipt was found
  for ENG-10. The composer projects both `retained_payload_bytes` and
  `retained_metadata_bytes` from QMP snapshots into public usage stats, making
  ENG-13's kind split observable when that feature is active.

## Verification and closeout rules

Observed at the audit HEAD: `scripts/check-workspace.sh --in-memory` exited 0
with `check-workspace: clean (in-memory scope only; full repository gate not
run)`. It checked formatting, shell scripts, the resolved no-Redb graph,
workspace policy, 96 script contract tests, 18 PM tests, extraction negative
fixtures, regression guards (75 Rust files including all 67 production files,
zero findings), in-memory Clippy/workspace tests, eight feature-capability
selectors, `pm lint`, and `cargo deny`. Earlier focused checks at the same
HEAD also passed the all-feature metadata boundary, 66 selector/scaling-verifier
Python tests, and the public Headroom contract 1/1. These checks do not execute
a `session_bytes` panic reproducer or same-store self-reclaimer E2E. No full
`scripts/check-workspace.sh`, fresh clean benchmark, standalone extraction,
Semantica-host test, or release SLO comparison was run for this document.

### Second-pass coverage and limits

| Angle | Rechecked evidence | Residual |
| --- | --- | --- |
| Graph and portability | Resolved in-memory graph, `domain/`/`ports/` dependency and I/O scan, public taxonomy, headroom and standalone controller surface. | ENG-04 is an SDK migration contract; ENG-12 is a low-level constructor validation gap. No new dependency leak found. |
| Accounting and concurrency | Broker lock/reclaim boundary, live charge/deferred lifecycle, rebuild fence, batch release and scratch-reservation observation. | ENG-02/03/08 remain conditional or low-priority; ENG-11 is observational, not a ledger bypass. The in-memory rail is not a host callback qualification. |
| Store failure atomicity | Read-side client-callback guards across payload, retained map, transcript and prepared publication; transcript `session_bytes` is the unguarded lookup. Prepared orphan GC validates roots before removing entries. | ENG-01 needs an executable panic/poison RED test; ENG-10 needs an oversized-root negative test. No second source-confirmed poison path found in inspected reads. |
| Observability and scaling | Broker report/scheduler copies, transcript stats traversal, prepared orphan-root normalization, store serialization, benchmark workload/instrumentation. | ENG-05/06/10 require cardinality and consumer SLO measurements; a green benchmark verifier does not establish latency headroom. |
| Tests and delivery | In-memory canonical rail and existing owner/e2e selector inventory; CI/example claims compared to config. | ENG-09; full backing/crash rail, current host pair, and exact-target release receipt not run here. Latest local `qmp-local-qualified-*` tag does not target this HEAD. |

This second pass adds the transcript-statistics path to ENG-05, not a new
confirmed P0/P1 or a new standalone ticket. The table is an inspected-path
boundary, not a claim that all possible generic callback implementations,
host wiring, or fault schedules have been exercised.

### Additional pass for request (2)

The subsequent ownership/critical-section pass added ENG-10 and ENG-11.
`cargo test --locked -p quanta-memory-platform-core --lib
ports::scratch::tests::grow_to_observed_updates_within_reserved_ceiling --
--exact` and the facade `publish_pipeline` root-reference selector each passed
1/1 at the same HEAD. They cover ordinary increasing observations and valid
roots, **not** the two new negative cases. No production code or test was
changed and the full repository rail was not rerun after this documentation
edit. No additional P0/P1 failure was established by this pass.

### Additional pass for request (3)

Rechecked configuration validation, the pure admission/pressure path,
resource-group apportionment, event-sink quarantine, and semantic
cost/measurement contracts. Only ENG-12 was added. At HEAD `96d0a25`, the
selected valid-classification and `PressureThresholds::new` rejection tests
each passed 1/1; neither exercises a caller-built invalid literal passed to
`LadderPressureController::new`. The broker path uses
`PressureThresholds::default()`, and no new default-product P0/P1 or fail-open
ledger mutation was established. This was a source/test audit, not a new full
workspace, host, benchmark or Redb qualification run.

### Parallel adversarial/objective final pass

Three read-only owner lanes independently covered core accounting/policy,
facade ownership/failure paths, and test/CI/benchmark evidence; the integrator
also checked the host boundary above and reclassified conditional findings.
At the frozen QMP HEAD/tree, core found no new in-memory fail-open or P0/P1
defect. Facade added ENG-13's kind-axis semantics; proof review added the
premerge scope to ENG-09. ENG-07 now includes event-sink latency. The max-scale
reclaim case was explicitly rejected as a new finding because a test pins the
intentional fail-closed behavior.

The existing local ordinary benchmark artifacts were rechecked with
`python3 scripts/verify-scaling-results.py --input <results.jsonl>`:

| Local ignored artifact under `target/scaling-results/` | Rows / result | SHA-256 of `results.jsonl` |
| --- | --- | --- |
| `qmp-scaling.MLHJ4z` | 60 / OK | `ef42fd8400620f069721134fbc560f0c93e4715c6eafeb5b85f803d7b1e1599f` |
| `qmp-scaling.z7seiq` | 60 / OK | `58f846cd85237fb6def1ef2f55bf70454e1bbe7c9be012734caa314e20371e34` |
| `qmp-scaling.TfL6g3` | 60 / OK | `e901052959702a5187de9d8eb56f9b97cc4ba22db858065b88329b32c2d925c6` |
| `qmp-scaling.tU5B7T` | 220 / OK | `13d15a18cf2220f5459568e736640c93658611ae2beb228b774723bb08c86192` |

Each JSONL header names the frozen QMP SHA/tree, `source_clean=true`, and
`bench_exit_code=0`. The files are local ignored artifacts, not committed
portable receipts. Ordinary verification does not apply the release host-load
gate; the recorded load was high, so ENG-06 remains open. The existing
prepared-publication tombstone-total selector was also rerun 1/1 GREEN; it
does not assert ENG-13's kind axis. No new full workspace/Redb rail, host E2E,
or baseline/candidate performance qualification was run for this pass.

Each implemented behavior change needs its owner-local RED/GREEN oracle and
the canonical repository rail on the changed source. Cross-store reclaim or
host authority changes additionally need the smallest real integration test.
Performance acceptance needs controlled baseline/candidate receipts, not a
focused microbenchmark. If Cargo/features or cross-crate edges change, run the
committed clean-candidate extraction dry-run per repository rules. Record exact
HEAD/tree, dirty state, selected test count, raw receipt and remaining host
activation boundary. Do not mark this audit `DONE` because its document exists.
