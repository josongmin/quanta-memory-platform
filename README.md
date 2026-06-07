# quanta-memory-platform

Portable, host-independent memory-platform workspace. It is consumed by the
Semantica monorepo through a dedicated host adapter (`quanta-memory-host`) and is
designed to be extracted into its own repository without rewrites.

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

Consumers use the builder-first API, never low-level broker backdoors:

- `MemoryPlatformBuilder` (facade) to build a broker from a preset/manifest.
- `PortableMemoryPresets` for named budget manifests.
- Store/pipeline builders (`PayloadStoreBuilder`, `RetainedMapStoreBuilder`,
  `RevocableStoreBuilder`, `PublishPipelineBuilder`,
  `SessionTranscriptStoreBuilder`).
- Charged store assembly stays builder-first too: the relevant store builders
  expose `charged_against(...)` so consumers do not have to drop to
  `Simple*Store::new(...).charged_against(...)`.

Examples (compile against the real public APIs):

```bash
# library-only hello world
cargo run -p quanta-memory-platform-facade --example hello_world

# effective budget report
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
