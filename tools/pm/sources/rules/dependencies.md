## Dependency Policy

Runtime dependencies are declared in each crate's `Cargo.toml`; shared version
requirements live in the root `[workspace.dependencies]`. Exact resolved
versions are owned by `Cargo.lock`; use `--locked` for verification. Audit the
resolved graph with `cargo deny --locked --all-features check -D warnings`
against `deny.toml`.

**Shared versions** (declare in root `[workspace.dependencies]`, opt in per crate):

- `blake3` — facade transcript and publish-pipeline identity hashing
- `hashlink` — facade retained-map storage
- `hdrhistogram`, `metrics` — core `telemetry` adapters
- `redb` — facade `spill-redb` backing
- `tracking-allocator` — testkit `alloc_tracking` ONLY
- `loom` — testkit concurrency models under the `loom` feature
- `proptest` — testkit dev-only property tests

**Rules:**

- Third-party runtime crates are **optional** and behind a feature gate.
  `core` and `facade` build with `--no-default-features` and pull infra crates in
  only via their feature (`telemetry`, `retained_map`, `transcript`,
  `publish_pipeline`, `spill-redb`). Dev-only test dependencies are not runtime
  graph members.
- `tracking-allocator` may appear ONLY under testkit `alloc_tracking`; it must
  never reach a production dependency graph.
- New runtime deps must pass `cargo deny --locked --all-features check -D warnings`. `deny.toml`
  allows only licenses actually encountered in the `--all-features` graph, so an
  unused allowance warns and a newly introduced license fails closed. If you add
  a dep with a new license, add it to the allow-list with a one-line rationale.
- Stay inside the architecture edges (`core <- facade <- testkit`). Do not add a
  dependency that pulls in Semantica/host crates — that inverts the direction.
- No dependency may introduce a path dep escaping the workspace (breaks
  extraction). Inter-member deps stay relative within `crates/`.

`deny.toml` notes: private unpublished members are license-exempt and duplicate
versions fail. MPL-2.0 is accepted only for the testkit tracking-allocator path.
Revisit that allowance before any distributed binary release.
