## Quick Reference

Portable, host-independent memory budgeting / accounting workspace
(`core <- facade <- testkit`), extractable into its own repo.

Start here:

- Read `README.md` (overview + dependency direction) and `EXTRACTION.md`.
- Public surface is builder-first: `MemoryPlatformBuilder`,
  `PortableMemoryPresets`, store/pipeline builders.
- Accounting SSOT + invariants: `core/src/domain/ledger.rs`.
- Architecture + lint cheatsheet: `.codex/CODEX-RULES.md`.

Most-used commands:

```bash
scripts/check-workspace.sh                                          # one-shot rail, includes feature matrix
scripts/memory-platform-doctor                                      # repo-local fmt/clippy/feature-matrix/check/test/examples/deny rail
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny --all-features check
python3 tools/pm/pm.py sync && python3 tools/pm/pm.py lint

cargo run -p quanta-memory-platform-facade --example hello_world
cargo run -p quanta-memory-platform-facade --example effective_budget_report
```

Hard rules:

- Direction is strict: `core <- facade <- testkit`; host -> platform, never the
  reverse.
- `domain/` & `ports/` are pure — no fs/env/clock, no `crate::adapters`.
- No Semantica nouns in the public API.
- Ledger ops are fail-closed; telemetry is advisory.
- Conformance must REJECT a broken fake, not just accept honest ones.
- Never hand-edit generated agent docs — edit `tools/pm/sources/**` and sync.
