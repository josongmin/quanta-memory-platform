# quanta-memory-platform

`quanta-memory-platform` is a portable Rust library for **semantic memory
budgeting, accounting, and lifecycle control**. A host uses it to decide
whether retaining data is allowed, charge that data to an owner/class/context,
and release or transfer that authority without counter drift.

It is not an allocator, an RSS limiter, a general-purpose database, or the
Semantica host adapter. Allocator telemetry is advisory; the ledger is the
accounting source of truth.

## Status and boundary

This workspace is an internal, pre-1.0 library (`0.2.0`, `publish = false`).
Repo-local checks establish library behavior only. Semantica host adoption,
deployment, and activation require separate exact-source evidence.

Current product scope is in-memory accounting and reclaim. Redb-backed durable
spill, its codec, and disk-recovery qualification are **excluded from the active
QMP/SDK/Semantica plan**: no current consumer requirement has been identified.
The opt-in code still exists in this 0.2 workspace, but is **not
production-qualified or supported as a current product capability**. This
scope decision does not remove its API or turn historical test results into a
release claim.
Reconsider it only with a named consumer, measured reuse benefit, and an owned
disk-failure and invalidation contract.

The host adapter is deliberately outside this repository:

```text
Semantica quanta-memory-host  ->  quanta-memory-platform-facade  ->  core
                                                        ^
                                                        |
                                                     testkit
```

Dependency direction is strict: `core <- facade <- testkit`. No package here
may depend on a host, and public APIs do not expose Semantica-specific nouns.

## Start here

Consumers should construct the facade, not a raw ledger or broker:

```rust
use quanta_memory_platform_facade::{MemoryPlatformBuilder, PortableMemoryPresets};

let platform = MemoryPlatformBuilder::new()
    .with_manifest(PortableMemoryPresets::development())
    .build()?;

// Create budgeted stores from this one platform/ledger.
// let store = platform.payload_store_builder::<PayloadRef, Vec<u8>>().build()?;
# Ok::<(), quanta_memory_platform_core::MemoryError>(())
```

For a configuration-only caller, use `build_report_only()`; it returns an
effective budget report but creates no runtime or accounting authority.

Runnable public examples:

```bash
cargo run -p quanta-memory-platform-facade --example hello_world
cargo run -p quanta-memory-platform-facade --example effective_budget_report
```

## What it covers

| Area | Contract |
| --- | --- |
| Budget control | Global, class, owner, context, execution-scratch, headroom, reserved/soft/hard caps, resource groups, and bounded shared borrowing. |
| Accounting | Every budgeted mutation is fail-closed; a charge is reserved, committed, released, recharged, or restored atomically. |
| Retaining data | Canonical payloads, retained maps, FIFO revocable stores, prepared publication, and session transcripts all route bytes through the same broker. |
| Pressure | Soft-cap reclaim requests run outside the broker lock; hard-cap admission retries at most once after remeasurement. |
| Transfer | In-process transfer preserves accounting authority; the existing optional durable path is outside the active product plan. |
| Correctness tooling | Conformance fakes, deterministic fault scenarios, allocation tracking, Loom models, feature-matrix checks, and standalone extraction checks. |

Management metadata has independent public bounds because a zero-byte ticket
still owns single-use authority: one broker admits at most
MAX_LIVE_TICKETS_PER_BROKER live tickets, owner and resource-group identifiers
are limited to 256 UTF-8 bytes, and deferred ticket cleanup processes at most
MAX_DEFERRED_TICKET_DRAIN_BATCH authorities per broker-lock acquisition. These
are logical bookkeeping bounds, not an RSS guarantee.

Public payload/metadata totals are semantic **charge buckets per owned
lifetime component**, not per-field byte telemetry: a bucket covers the
retained object together with the key/index/entry overhead held under the
same lease. The prepared-publication service splits its two leases across
buckets — the active payload component is charged as payload and the
persistent identity/index/tombstone evidence is charged as metadata, so an
aborted entry leaves a metadata-only tombstone. Prepared-publication
orphan enumeration and GC also bound the raw caller-supplied
reference-root slice at 16,384 tokens (duplicates counted) and reject a
larger slice with a typed storage capacity error before any state lock.

The authority, lifecycle, failure and verification boundaries are in the
[architecture decisions](docs/adr/README.md).

## Feature selection

`quanta-memory-platform-facade` defaults to live accounting. Enable only the
store slices a consumer needs:

| Feature | Enables |
| --- | --- |
| `payload_store` | Canonical retained payload store. |
| `retained_map` | Typed retained key/value map. |
| `revocable_store` | FIFO reclaimable store. |
| `transcript` | Idempotent session transcript with bounded replay evidence. |
| `publish_pipeline` | Publication staging/commit and prepared-publication lifecycle. |
| `spill` | Portable in-process/durable spill contracts. |
| `spill-redb` | Existing opt-in Redb backing; not production-qualified or supported as a current product capability. |
| `full` | All in-memory facade slices; no durable backing dependency. |
| `full-backings` | Existing aggregate feature that adds `spill-redb`; not a current release target or production qualification claim. |

The Redb/codec implementation remains documented in the detailed current-code
guide for maintainers, but is not an SDK integration or release prerequisite.

`unbudgeted-stores` is an explicit, distinct product type for fixtures or
deliberate non-accounted use. It is absent from `default`, `full`, and
`full-backings`; `--all-features` enables it only to test that isolated surface.

## Repository checks

```bash
# authoritative repo-local static, contract, feature, test, and dependency rail
scripts/check-workspace.sh

# shorter local in-memory check; does not build/test Redb or qualify extraction
scripts/check-workspace.sh --in-memory

# public broker/store scaling and final semantic/allocator-drift checks
scripts/run-scaling-bench.sh --smoke
scripts/run-scaling-bench.sh

# fail-closed standalone-repository preflight
scripts/extract-to-standalone-repo.sh --dry-run /path/to/qmp-extracted
```

The scaling results are same-host signals, not portable latency guarantees.
Push CI verifies a real clean standalone extraction, its full workspace rail,
and both public examples on the extracted source. The separate, manually run
CircleCI benchmark collects three independent full 60-row `in-memory-pressure`
v4 process receipts and one 220-row standard scaling receipt at one pinned commit,
retaining each raw JSONL artifact. Passing those receipts establishes only the
profiles' absolute/structural guards; release regression or SLO claims still
require an approved policy and compatible baseline receipts as described in
[verification scenarios](docs/verification-scenarios.md).
Extraction and host adoption are separate operations; see
[EXTRACTION.md](EXTRACTION.md).

## Documentation map

| Need | Document |
| --- | --- |
| Architecture and ownership decisions | [docs/adr/README.md](docs/adr/README.md) |
| Standalone-repository extraction procedure | [EXTRACTION.md](EXTRACTION.md) |
| Consumer breaking changes | [MIGRATION.md](MIGRATION.md) |
| Release inventory | [CHANGELOG.md](CHANGELOG.md) |
| Accepted architecture decisions | [docs/adr/README.md](docs/adr/README.md) |
| Current plan status and remaining gates | [docs/plans/README.md](docs/plans/README.md) |
| Executable verification contract | [docs/verification-scenarios.md](docs/verification-scenarios.md) |
