# Migration to 0.2.0

QMP 0.2.0 is a deliberate pre-1.0 breaking release. The workspace has not been
published to crates.io (`publish = false`, `UNLICENSED`), so there is no binary
or source compatibility window for the internal 0.1.x snapshots. Consumers
must migrate atomically and qualify their own host composition against the exact
QMP Git tree. No deprecated alias is retained when it would reopen an
unbudgeted or unbounded path.

## Required consumer changes

- Construct live authority through `MemoryPlatformBuilder`,
  `PortableMemoryPresets`, and platform-issued store builders. Raw ledger,
  broker, scratch allocator, ticket, and late `charged_against` constructors are
  not public.
- Treat `build_report_only()` as configuration data only. It cannot create a
  runtime view, scratch reservation, or budgeted store.
- Change payload reference types to implement `Eq + Hash`. Collision-safe
  canonical/reference digest buckets now retain two digest-index slots.
- Rebaseline semantic retained cost. The 32-byte payload conformance fixture is
  121 bytes in `RetainedCostV1`, not 89 bytes. This is a portable semantic-cost
  model, not allocator RSS.
- Replace `TranscriptRetentionPolicyV1` with bounded
  `TranscriptRetentionPolicyV2`. Saturation is a typed denial; no unbounded
  compatibility alias is provided.
- Use `LedgerSnapshot::checked_total_*` for externally assembled snapshots.
  Legacy infallible totals require an exact representable snapshot.
- Treat in-process revocable spill as an ownership transfer. Spill handles share
  one `Arc<StoreBindingHandle>` and carry the original managed charge; they do
  not deep-clone owner/runtime identity or create an uncharged destination copy.

## Feature policy

- Default: live accounting bridge only.
- Production aggregate: `full`; it excludes every unbudgeted escape hatch and
  third-party durable backing.
- Durable backing aggregate: `full-backings`; currently this adds only
  `spill-redb`.
- Explicit non-production escape hatches: `unbudgeted-stores` and core
  `unbudgeted_pin`. Neither is enabled by default, `full`, or `full-backings`.
- Removed/quarantined names: moka, scc, and bumpalo are not advertised features
  because no canonical public builder path implements those capabilities.
- There is no `advanced-unchecked` feature. Unsafe raw accounting authority is
  private rather than feature-gated.

## Toolchain and distribution policy

- MSRV is Rust 1.92, equal to `rust-toolchain.toml`; every crate declares the
  inherited `rust-version`.
- The repository is internal-only: `UNLICENSED` and `publish = false` are
  intentional. A distributable release requires an explicit license decision,
  crate metadata review, and a deliberately reviewed release change; changing
  only `publish` is invalid.

## Host qualification boundary

Standalone GREEN does not qualify Semantica. The host must prove that both its
top-level and nested Cargo workspaces resolve the same committed QMP root/tree,
then run its registered memory-host, production-composition, execution-budget,
and owning-product suites on that exact source.
