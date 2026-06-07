## Codex Notes

- Cite the exact commands you ran in closeout (`cargo clippy … -D warnings`,
  `cargo test --all-features`, `scripts/check-workspace.sh`,
  `scripts/memory-platform-doctor`).
- Edit agent guidance in `tools/pm/sources/**` then `python3 tools/pm/pm.py sync`;
  never patch generated files (`AGENTS.md`, `CLAUDE.md`, `.codex/*`, `.cursorrules`).
- Respect the dependency direction `core <- facade <- testkit`. If a change needs
  a new crate edge or reaches toward the host, stop — do not add the import to
  make it compile.
- Keep `domain/`/`ports/` pure (no fs/env/clock, no `crate::adapters`).
- Use the one `MemoryError` enum; preserve source errors (`{e}`), never
  `map_err(|_| …)`.
- For any accounting/policy change, run the extraction dry-run if you touched a
  `Cargo.toml` or cross-crate `use` path.
