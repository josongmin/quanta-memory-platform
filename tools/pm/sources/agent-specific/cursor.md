## Cursor Notes (mandatory)

### Generated rules

- `.cursorrules` is **generated** from `tools/pm/sources/**`.
- **Never** hand-edit `.cursorrules`, `AGENTS.md`, or `.codex/*`.
- After changing sources: `python3 tools/pm/pm.py sync` then `python3 tools/pm/pm.py lint`.

### Architecture is not negotiable

Read **`cursor-architecture.md`** (included in this file) before every task.
Cursor frequently violates boundaries when porting code from the Semantica
monorepo — treat that section as a checklist, not background reading.

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
| Ledger / policy / config | `core/src/domain/**` + that module's tests |
| Pressure / scheduler | `core/src/application/**` + tests |
| New port / adapter | `core/src/ports/**` + `core/src/adapters/**` |
| Public broker / builder | `core/src/api/**` + examples |
| Facade store / pipeline | `facade/src/<feature>/**` (service + backing) |
| Conformance / chaos tooling | `testkit/src/**` |

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
