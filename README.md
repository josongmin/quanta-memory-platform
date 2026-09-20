# quanta-memory-platform

Portable, host-independent memory-platform workspace. It is consumed by the
Semantica monorepo through a dedicated host adapter (`quanta-memory-host`) and is
designed to be extracted into its own repository without rewrites.

> **Current qualification status (2026-09-21): NOT production-ready and NOT
> extraction-ready at audit base `e103771670da3143e2ffebb8950a5dfc7e3d4515`.**
> Workspace rails pass, but the final audit confirmed accounting-authority and
> lifecycle gaps, and actual standalone extraction fails. The authoritative
> remediation inventory, status, dependencies, acceptance criteria, and receipts
> live in [`tickets/README.md`](./tickets/README.md). Passing a focused test or
> dry-run does not supersede that status.

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
  `PreparedPublicationServiceBuilder`; the QMP publish-pipeline migration owns
  removal of the legacy surface.
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
- Runtime scratch obtained through `LiveMemoryPlatform::attach_runtime` is
  charged against the same broker ledger, execution-scratch cap, class cap,
  owner/group admission, and release lifecycle as direct broker charges.

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
cargo check --workspace --all-features
cargo build --examples --all-features
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo deny --all-features check
# repo-local feature-sliced one-shot rail:
scripts/check-workspace.sh
# repo-local doctor rail:
scripts/memory-platform-doctor
```

## Extraction into a standalone repo

The split is mechanical, not folklore. See [EXTRACTION.md](./EXTRACTION.md) for
the full checklist. Bootstrap:

```bash
scripts/extract-to-standalone-repo.sh --dry-run /path/to/dest   # fail-closed pre-flight
scripts/extract-to-standalone-repo.sh /path/to/dest             # copy + git init + verify
```

The bootstrap runs a fail-closed pre-flight (member presence, dependency
direction, no escaping path deps, self-hosting file-set presence, copied-set
exclusion of `tools/ci/gates/*`, and `cargo metadata` resolution) BEFORE
copying, so a cyclic / mis-directed split is rejected up front.

## Repo-local rails

This repo now carries its own verification front doors:

- `scripts/check-workspace.sh` — fmt + clippy `-D warnings` + workspace check +
  example builds + per-feature matrix + `cargo deny`.
- `scripts/memory-platform-doctor` — repo-local doctor: fmt + clippy `-D warnings`
  + per-feature matrix + workspace check + workspace tests + example builds +
  `cargo deny` when available.
- `scripts/memory-platform-{fix,report}` — mechanical fix/report helpers.
- `scripts/scaffold-external-repo.sh` — asserts the extracted self-hosting file
  set is present in-source and that the copy plan excludes `tools/ci/gates`.

Semantica may run additional monorepo boundary gates around this repo, but the
portable repo's own scripts no longer depend on `tools/ci/gates/*`.
