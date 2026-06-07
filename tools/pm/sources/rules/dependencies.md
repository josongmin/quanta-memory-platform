## Dependency Policy

Runtime dependencies are declared in each crate's `Cargo.toml`, pinned in the
root `[workspace.dependencies]`, and audited by `cargo deny --all-features check`
against `deny.toml`.

**Shared versions** (pin in root `[workspace.dependencies]`, opt in per crate):

- `bumpalo` — facade `payload_store` scratch arena
- `moka`, `scc` — facade `revocable_store` backings
- `redb` — facade `spill` backing
- `hdrhistogram`, `metrics` — core `telemetry` adapters
- `tracking-allocator` — testkit `alloc_tracking` ONLY

**Rules:**

- Every non-trivial third-party crate is **optional** and behind a feature gate.
  `core` and `facade` build with `--no-default-features` and pull infra crates in
  only via their feature (`telemetry`, `spill`, `revocable_store`, …).
- `tracking-allocator` may appear ONLY under testkit `alloc_tracking`; it must
  never reach a production dependency graph.
- New runtime deps must pass `cargo deny --all-features check`. `deny.toml`
  allows only licenses actually encountered in the `--all-features` graph, so an
  unused allowance warns and a newly introduced license fails closed. If you add
  a dep with a new license, add it to the allow-list with a one-line rationale.
- Stay inside the architecture edges (`core <- facade <- testkit`). Do not add a
  dependency that pulls in Semantica/host crates — that inverts the direction.
- No dependency may introduce a path dep escaping the workspace (breaks
  extraction). Inter-member deps stay relative within `crates/`.

`deny.toml` notes: private (unpublished) members are license-exempt
(`license = "UNLICENSED"`); `multiple-versions = "allow"`. MPL-2.0
(tracking-allocator) and LGPL-2.1-or-later (transitive) are accepted for this
internal library — revisit if either reaches a distributed binary.
