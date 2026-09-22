## Architecture & Crate Boundaries

`quanta-memory-platform` is a portable, host-independent memory budgeting /
accounting workspace. It is consumed by the Semantica monorepo through a host
adapter (`quanta-memory-host`) that lives **outside this repo**, and it is
designed to be extracted into its own git repository with no rewrites. The crate
graph is the architecture authority.

Allowed workspace edges (strict, acyclic — `core <- facade <- testkit`):

```text
quanta-memory-platform-core    -> (none — pure)
quanta-memory-platform-facade  -> quanta-memory-platform-core
quanta-memory-platform-testkit -> quanta-memory-platform-core, quanta-memory-platform-facade
quanta-memory-host (Semantica) -> quanta-memory-platform-{core,facade}   # OUTSIDE this repo
```

No member depends upward. `core` must not name `facade`/`testkit`; `facade` must
not name `testkit`. The host adapter never appears as a dependency of this repo —
that would invert the direction.

Layout:

```text
crates/
  quanta-memory-platform-core/     # pure hexagonal kernel
    src/api/         # public broker surface: broker, builder, control
    src/application/ # use-case services: controller (pressure), scheduler
    src/domain/      # config, context, errors, ledger (accounting SSOT), policy, stats
    src/ports/       # trait definitions: pin, scratch, store, telemetry
    src/adapters/    # port impls: broker-backed scratch, hdrhistogram, metrics; explicit unbudgeted pin
  quanta-memory-platform-facade/   # feature-sliced portable stores + pipelines
    src/{payload_store,retained_map,revocable_store,transcript_store,publish_pipeline,spill}/
    src/{presets,shared}/
  quanta-memory-platform-testkit/  # conformance / chaos / alloc tooling (never a prod dep)
    src/{conformance,faults,fixtures,doctor,fake,alloc_tracking}/
```

Boundary rules (hexagonal direction is inward-only):

- **`domain/`** is pure: no I/O, no `std::fs`, no `std::env`, no wall-clock
  (`SystemTime::now` / `Instant::now`), and it must NOT import `crate::adapters`.
  It depends on its own types plus the `ports/` trait shapes.
- **`ports/`** holds trait definitions only — no concrete adapters, no I/O.
- **`adapters/`** implement port traits; infra types (hdrhistogram, metrics) live
  here. Allocator telemetry from adapters is **advisory**: it never mutates ledger
  counters. The in-memory pin registry is explicitly named `UnbudgetedPinRegistry`
  and exists only behind the non-default `unbudgeted_pin` feature.
- **`application/`** orchestrates domain + ports; carries no host nouns.
- **`api/`** is the public broker surface, builder-first. Low-level broker
  backdoors stay `pub(crate)` / unexported.
- **`facade`** composes core behind builders; production storage backings such
  as redb are feature-gated and confined to their `*/backings/` modules.
- **`testkit`** is NEVER a mandatory production dependency. `alloc_tracking` (the
  only `tracking-allocator` integration) is feature-gated so it can never enter a
  production graph.

Forbidden usages (fail-closed under `-D warnings` + repo-local doctor + deny):

- Semantica nouns in the public API (`Semantica*`, `quanta_runtime`, `codegraph`).
  The portable library carries none.
- `core` depending on `facade`/`testkit`; `facade` depending on `testkit`.
- A path dependency escaping the workspace (breaks extraction).
- `crate::adapters` imported from `domain/` or `ports/`.
- Raw constructor backdoors leaking past the builder-first surface; stale
  `V1`/`V2` / renamed-symbol naming (the doctor rail flags both).

Enforcement: `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
`cargo deny --all-features check`, `scripts/memory-platform-doctor`, and the
extraction pre-flight in `scripts/extract-to-standalone-repo.sh` (member
presence, dependency direction, no escaping path deps, self-hosting file set,
and `cargo metadata` resolves). If a change needs a NEW crate edge, stop and
reconsider — do not add the import to make it compile.
