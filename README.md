# quanta-memory-platform

Portable, host-independent memory-platform workspace. It is consumed by the
Semantica monorepo through a dedicated host adapter (`quanta-memory-host`) and is
designed to be extracted into its own repository without rewrites.

> **Qualification boundary (2026-09-22): NOT production-ready.** Repo-local
> static, test, conformance, performance, and standalone-extraction rails
> establish only this library's behavior. Exact-source host adoption,
> deployment, and activation are separate evidence; a focused test or dry-run
> does not establish host-product GREEN.

## Packages and dependency direction

```
quanta-memory-platform-core      (pure hexagonal: api/domain/application/ports/adapters)
        ^
        |  depends on
quanta-memory-platform-facade    (feature-sliced portable stores + pipelines)
        ^
        |  depends on
quanta-memory-platform-testkit   (feature-sliced conformance / chaos / alloc tooling)
```

The dependency direction is strictly `core <- facade <- testkit`. There are no
cycles, and no member depends upward. The host adapter lives OUTSIDE this repo:

```
quanta-memory-host (Semantica monorepo)  ->  quanta-memory-platform-{core,facade}
```

The host adapter never appears as a dependency of this repo; that would invert
the direction. The portable library carries no Semantica nouns
(`Semantica*`, `quanta_runtime`, `codegraph`) in its public API.

## Version, compatibility, and distribution

- `0.2.0` is a deliberate pre-1.0 breaking release. There is no deprecated
  compatibility window for an alias that would reopen unbounded or unbudgeted
  authority; internal consumers migrate atomically.
- Rust 1.92 is the declared MSRV and pinned toolchain, matching the current
  Semantica host toolchain.
- The workspace is internal-only (`UNLICENSED`, `publish = false`). It is not a
  crates.io release candidate.
- Exact breaking changes and consumer actions are in
  [MIGRATION.md](./MIGRATION.md); the release inventory is in
  [CHANGELOG.md](./CHANGELOG.md).

## Builder-first usage (recommended public surface)

Consumers use the builder-first API; raw ledger/broker constructors are not
public:

- `MemoryPlatformBuilder::build()` creates the enforced `LiveMemoryPlatform`
  from a preset/manifest. Live accounting is a default feature.
- `MemoryPlatformBuilder::build_report_only()` resolves and validates the same
  override chain but returns only `EffectiveMemoryBudgetReport`; it cannot
  attach a runtime, allocate scratch, or construct charged stores.
- `PortableMemoryPresets` for named budget manifests.
- `LiveMemoryPlatform::{payload_store_builder,retained_map_store_builder,
  revocable_store_builder,prepared_publication_builder,
  transcript_store_builder}` issues already-budgeted builders. Each retaining
  store has a different concrete type from its unbudgeted counterpart; an
  unset builder cannot `build()` or acquire a sink after construction.
- `PublishPipelineBuilder` and its non-retaining `stage`/`commit` transition are
  legacy compatibility surfaces. New retaining publication code uses
  `PreparedPublicationServiceBuilder`; remove the legacy surface only through
  an explicit public-API migration.
- Prepared-publication orphan enumeration and GC require the caller's complete
  canonical-root token snapshot, stabilized against concurrent activation for
  the duration of the call. QMP validates and excludes referenced tokens; it
  never invents or owns the caller's activation state.
- For retained maps, `LiveMemoryPlatform::retained_map_store_builder()` returns
  the budgeted builder; `bind(...)` moves into a bound typestate whose `build()`
  is infallible. No store has a late accounting attachment or user-supplied
  accounting callback. The explicit `unbudgeted-stores` feature permits a
  distinct `Unbudgeted` store via `.unbudgeted()`, but it is absent from
  `default`, `full`, and `full-backings`. `--all-features` enables it for coverage;
  that does not change the budgeted platform factories or their product types.
- Store reads, writes, and `close()` return typed `OperationInProgress` on
  lifecycle or container-lock contention. This deliberately fails fast on
  same-store reentry from client `Hash`/`Clone`/`CanonicalKey` callbacks;
  callers may retry after the in-flight operation. A successful `close()` is
  terminal and repeated close is idempotent; later writes return `Closed`.
- Payload entries use collision-safe canonical/reference digest indexes and
  stable reusable slots. `PayloadRef` must implement `Eq + Hash`; exact value
  equality is still checked inside a digest bucket. `RetainedCostV1` charges
  both retained digest-index slots, so this semantic cost is not allocator RSS.
  Revocable entries use FIFO `VecDeque` ownership; rollback restores the front
  and reclaim drains the front without repeatedly shifting all survivors.
- Transcript idempotency evidence is finite by default. Configure
  `TranscriptRetentionPolicyV2::fail_closed(count, bytes)` on the transcript
  builder when the default 65,536-entry/64-MiB horizon is unsuitable. Exact
  retries and identity conflicts remain deterministic at saturation; a new
  identity returns typed `RetentionSaturated` without charging or mutation.
  `retention_stats()` separates active, evidence, and session-key cost, and
  `close()` explicitly terminates the process-lifetime replay horizon.
- Runtime scratch obtained through `LiveMemoryPlatform::attach_runtime` is
  charged against the same broker ledger, execution-scratch cap, class cap,
  owner/group admission, and release lifecycle as direct broker charges.
- Runtime policy fields are active, not report-only vocabulary:
  - `reserved_bytes` protects each class axis and resource-group axis
    independently. The axes may describe the same bytes, so their floors are
    each checked against `global - headroom` and are not added together.
  - `soft_cap_bytes` crossing emits one bounded reclaim request. Configure the
    callback before construction with `MemoryPlatformBuilder::with_reclaimer`.
    It runs without the broker mutex; callback-reported bytes are advisory and
    the broker remeasures the ledger before admission.
  - `hard_cap_bytes` remains the deny boundary after at most one reclaim retry.
    Class borrowing is bounded by the shared pool. Group borrowing is bounded
    by `min(shared_pool * burst_percent / 100, weighted_share)`, where weighted
    shares use deterministic largest-remainder allocation so no pool byte is
    silently stranded.
  - reclaim candidates are ordered `Last`, then `Normal`, then protected
    `First`, with stable class/group identity as the tie break.
  - accepted live reconfiguration preserves legal class borrow and group burst,
    applies the same headroom/floor postimage rules, increments
    `policy_revision`, and records `RuntimeReconfigure` provenance atomically.

## Observation and operational events

- `LiveProcessMemoryBroker::observe()` is the authoritative observation API. It
  returns a typed `Result<BrokerObservation, MemoryError>` containing one-lock
  ledger, effective-policy revision/provenance, pressure, reclaim counters and
  semantic bytes. Poison or an invalid snapshot is never converted to zero.
- `ProcessMemoryBroker::effective_report()` remains an infallible compatibility
  view of the last effective policy. It is not an accounting/control oracle;
  callers that need availability and ledger truth must use `observe()`.
- Public `LedgerSnapshot::checked_total_*` methods are the fallible surface for
  externally constructed snapshots. The legacy `total_*` convenience methods
  require an exact/representable snapshot and may panic when that precondition
  is violated.
- `MemoryError::Capacity` is caller-actionable by reducing demand or changing a
  validated policy. `OperationInProgress` is retryable after the current
  lifecycle transition. `Poisoned`, `InvalidState`, and `Report` require
  operator/recovery handling and are never equivalent to empty state.
- Install `MemoryEventSink` with `MemoryPlatformBuilder::with_event_sink`.
  Operations and outcomes are closed enums; owner, runtime, key, session and
  error strings cannot become metric labels. Events are advisory, emitted
  outside broker/store data locks, and sink panics are contained. They never
  mutate semantic ledger counters.

Examples (compile against the real public APIs):

```bash
# enforced runtime scratch: success, cap denial, release, final ledger zero
cargo run -p quanta-memory-platform-facade --example hello_world

# report-only resolution; no runtime authority
cargo run -p quanta-memory-platform-facade --example effective_budget_report
```

The host-adapter hello world (`SemanticaMemoryKitBuilder`) lives in the
Semantica monorepo:

```bash
cargo run -p quanta-memory-host --example hello_host_adapter
```

## Baseline commands

```bash
# canonical repo-local verification rail:
scripts/check-workspace.sh

# source-bound smoke: repeated 1/2-thread distributions + allocator drift gate
scripts/run-scaling-bench.sh --smoke

# full 1/2/8/32-thread read/write/mixed matrix (five repetitions)
scripts/run-scaling-bench.sh

# optional same-host baseline comparison
QMP_SCALING_BASELINE=/path/to/baseline.jsonl scripts/run-scaling-bench.sh

```

The scaling rail exercises public budgeted store and broker paths and records
source SHA/tree, toolchain, machine, repeated p50/p95/p99, throughput, lifecycle
busy retries, semantic bytes, allocator high-water and exact final drift. The
verifier rejects missing matrix cells, insufficient repetitions, semantic or
allocator drift, counter underflow and absolute latency regressions. A supplied
same-host baseline also gates stable-lane throughput and p99 regressions. These
wall-clock thresholds are host-local; they are not portable performance claims.

## Extraction into a standalone repo

The split is mechanical, not folklore. See [EXTRACTION.md](./EXTRACTION.md) for
the full checklist. Bootstrap:

```bash
scripts/extract-to-standalone-repo.sh --dry-run /path/to/dest   # fail-closed pre-flight
scripts/extract-to-standalone-repo.sh /path/to/dest             # copy + git init + verify
```

The bootstrap runs a fail-closed pre-flight (member presence, dependency
direction, no escaping path deps, self-hosting file-set presence, and
`cargo metadata` resolution) BEFORE copying, so a cyclic / mis-directed split
is rejected up front.

## Repo-local rails

This repo carries one authoritative verification front door:

- `scripts/check-workspace.sh` — fmt, ShellCheck, resolved Cargo-graph and
  workspace safety policy, Python and extraction contracts, Semgrep regression
  guards, metadata-driven feature matrix, workspace tests, isolated feature
  runtime tests, documented-selector validation, prompt-manager tests/drift
  lint, and warning-free `cargo deny`.

The rail requires Python 3.9+, ShellCheck, Semgrep, and `cargo-deny`; missing
tooling fails closed rather than silently skipping a check.
