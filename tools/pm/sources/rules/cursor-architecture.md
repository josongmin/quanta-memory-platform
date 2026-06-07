## Cursor — Architecture Enforcement (read before every edit)

This repo is a **portable, host-independent** memory-platform workspace, not a
flat Rust project and not an application. Cursor must not paste code from the
Semantica monorepo (or any host) without re-homing it to the **allowed crate
graph** below. Violations fail clippy `-D warnings`, `cargo deny`, the doctor
gate, and the extraction pre-flight.

### Crate graph (only these workspace edges — strict `core <- facade <- testkit`)

```text
quanta-memory-platform-core    -> (none — pure hexagonal kernel)
quanta-memory-platform-facade  -> quanta-memory-platform-core
quanta-memory-platform-testkit -> quanta-memory-platform-core, quanta-memory-platform-facade
quanta-memory-host (Semantica) -> quanta-memory-platform-{core,facade}   # OUTSIDE this repo
```

**Hard bans:**

| Action | Why |
| --- | --- |
| `core` names `facade`/`testkit`, or `facade` names `testkit` | Inverts the direction; fails extraction pre-flight |
| Add a dependency from this repo back to Semantica / the host | Direction is host -> platform, never the reverse |
| `domain/` or `ports/` imports `crate::adapters` | Hexagonal direction is inward-only |
| `std::fs` / `std::env` / `SystemTime::now` in `domain/` or `ports/` | Kernel is pure; I/O & clocks belong behind ports/adapters |
| Semantica nouns (`Semantica*`, `quanta_runtime`, `codegraph`) in public API | Library must stay host-independent |
| Raw broker constructor in public code | Use the builder-first surface (`MemoryPlatformBuilder`, `ProcessMemoryBrokerBuilder`) |
| A path dependency escaping `crates/` | Breaks standalone extraction |
| Storage backing (redb/moka/scc/bumpalo) leaking outside its feature gate | Backings stay confined + feature-gated |

### Where new code goes

| You are implementing… | Put it in… | Do NOT put it in… |
| --- | --- | --- |
| Budget policy, presets, manifests, residency | `core/src/domain/policy.rs`, `config.rs` | `facade` |
| Accounting algebra (reserve/commit/release, invariants) | `core/src/domain/ledger.rs` | `api`, `facade` |
| Pressure / reclaim / scheduling | `core/src/application/{controller,scheduler}.rs` | `domain` |
| New capability trait (a port) | `core/src/ports/{pin,scratch,store,telemetry}.rs` | `domain`, `adapters` |
| Port implementation (infra) | `core/src/adapters/` | `domain`, `ports` |
| Public broker surface / builder | `core/src/api/{broker,builder,control}.rs` | `domain` |
| A portable store / pipeline (feature-sliced) | `facade/src/<feature>/service.rs` + builder in `shared/` | `core` |
| A storage backing (redb/moka/scc) | `facade/src/<feature>/backings/<name>.rs` (feature-gated) | service code |
| Conformance / fault / fixture / doctor tooling | `testkit/src/{conformance,faults,fixtures,doctor}/` | `core`, `facade` |

### Core kernel rules (common mistakes)

- **`domain/` stays pure** — no I/O, no clocks, no env, no `crate::adapters`.
- **One error type** — return `MemoryError` / `MemoryResult<T>`; never
  `map_err(|_| …)` (drops the source). Preserve `{e}` in the message.
- **Fail-closed before mutation** — validate the whole operation first; every
  mutating ledger op either fully succeeds or leaves the ledger untouched.
- **Telemetry is advisory** — allocator telemetry never mutates ledger counters.
- **Public API is the `lib.rs` re-exports** — keep internals `pub(crate)`; don't
  add raw constructor backdoors past the builder-first surface.

### Before submitting a Cursor diff

1. List every file you touched and which crate/layer owns it.
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo check --workspace --all-features` and, if you added a feature path,
   build that feature alone.
4. `scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-check` if you changed
   any `Cargo.toml` or cross-crate `use` path.
5. If you edited `tools/pm/sources/**`, run `python3 tools/pm/pm.py sync` then
   `python3 tools/pm/pm.py lint`.
