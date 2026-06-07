## Build & Front Door

Plain `cargo` is the front door — there is no `just`/`nextest`/`uv` in this repo.
`scripts/check-workspace.sh` is the one-shot lint rail and includes the
per-feature matrix; `scripts/memory-platform-doctor` is the repo-local doctor
rail for fmt/clippy/feature-matrix/check/test/examples/deny. Closeout reports
cite the exact commands run.

Baseline commands:

```bash
cargo check --workspace --all-features
cargo build --examples --all-features
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo deny --all-features check
cargo test --workspace --all-features

# one-shot lint rail (fmt + clippy -D warnings + check + examples + deny):
scripts/check-workspace.sh

# repo-local doctor rail (fmt + clippy -D warnings + feature matrix + check + test + examples + deny):
scripts/memory-platform-doctor
```

The extracted repo is self-hosting: its own scripts do not depend on
`tools/ci/gates/*`. Semantica may still wrap this repo with additional external
gates after host-adapter rewires.

Examples (compile against the real public APIs):

```bash
cargo run -p quanta-memory-platform-facade --example hello_world
cargo run -p quanta-memory-platform-facade --example effective_budget_report
```

Agent docs:

```bash
python3 tools/pm/pm.py sync     # regenerate AGENTS.md / CLAUDE.md / .codex/* / .cursorrules
python3 tools/pm/pm.py lint     # fail on generated-doc drift
python3 tools/pm/pm.py status   # show targets + ok/drift/missing state
python3 tools/pm/tests          # via: python3 -m pytest tools/pm/tests -q
```

Config lives at the repo root: `clippy.toml` (`too-many-arguments-threshold = 8`,
`type-complexity-threshold = 300`), `rustfmt.toml` (edition 2021, `max_width = 100`,
`use_small_heuristics = "Max"`), `deny.toml` (license allow-list; private members
exempt), `.cargo/config.toml`. Toolchain: cargo 1.95.0.

Feature flags (always test `--all-features`, but features must also build alone):

- **core:** `pressure`, `scheduler`, `telemetry` (= `metrics` + `hdrhistogram`), `full`
- **facade:** `payload_store` (bumpalo), `retained_map`, `revocable_store` (moka + scc),
  `transcript`, `publish_pipeline`, `spill` (redb), `full`
- **testkit:** `fixtures`, `doctor`, `conformance`, `faults`,
  `alloc_tracking` (tracking-allocator), `full`

Extraction into a standalone repo is mechanical and fail-closed:

```bash
scripts/extract-to-standalone-repo.sh --dry-run /path/to/dest   # pre-flight only
scripts/extract-to-standalone-repo.sh /path/to/dest             # copy + git init + verify
```
