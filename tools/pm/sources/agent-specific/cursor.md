## Cursor Notes (mandatory)

### Generated rules

- `.cursorrules` is **generated** from `tools/pm/sources/**`.
- **Never** hand-edit `.cursorrules`, `AGENTS.md`, or `CLAUDE.md`.
- After changing sources: `python3 tools/pm/pm.py sync` then `python3 tools/pm/pm.py lint`.

### Architecture is not negotiable

Read the generated architecture, cheatsheet, build, test, determinism, error,
and dependency rules in this file before every task. Treat them as a checklist,
not background reading.

**Stop and re-plan** if your diff:

- Makes `core` name `facade`/`testkit`, or `facade` name `testkit` (inverts direction)
- Adds a dependency from this repo back to Semantica / the host adapter
- Adds `std::fs` / `std::env` / `SystemTime::now` to `domain/` or `ports/`
- Imports `crate::adapters` from `domain/` or `ports/`
- Introduces a Semantica noun (`Semantica*`, `quanta_runtime`, `codegraph`) into
  the public API
- Adds a raw broker constructor instead of going through the builder-first surface

### Scoped edits

| Task | Allowed touch set |
| --- | --- |
| Ledger / policy / config | `crates/quanta-memory-platform-core/src/domain/**` + tests |
| Pressure / scheduler | `crates/quanta-memory-platform-core/src/application/**` + tests |
| New port / adapter | `crates/quanta-memory-platform-core/src/{ports,adapters}/**` |
| Public broker / builder | `crates/quanta-memory-platform-core/src/api/**` + examples |
| Facade store / pipeline | `crates/quanta-memory-platform-facade/src/<feature>/**` |
| Conformance / chaos tooling | `crates/quanta-memory-platform-testkit/src/**` |

Do **not** "drive-by" refactor unrelated crates.

### Clippy / gate

- The lint bar is `cargo clippy --workspace --all-targets --all-features
  -- -D warnings`. Fix violations; do not blanket-`allow` without a one-line
  `-- reason: <…>` on that item.
- `map_err(|_| …)` is **always wrong** — include `{e}` in the message.
- Closeout for Rust changes: `scripts/check-workspace.sh` must pass, and the
  extraction dry-run if you changed any `Cargo.toml` or cross-crate `use` path.

### Portability

- The library must stay host-independent and extractable. When unsure, run
  `scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp`.
