## Build & Verification Front Door

Plain `cargo` is the build front door. `scripts/check-workspace.sh` is the only
repository-wide verification front door. It owns formatting, shell syntax and
ShellCheck, resolved workspace-graph and safety policy, Python and extraction
contracts, Semgrep regression guards, the Cargo feature matrix, workspace
tests, isolated feature capability tests, documented selector validation,
generated-guidance drift, and dependency policy.

```bash
scripts/check-workspace.sh
```

Run focused commands while developing, but do not report them as equivalent to
the canonical rail. The feature matrix discovers feature names from Cargo
metadata and covers default, bare, every non-aggregate feature, `full`, and
all-features; adding a manifest feature therefore adds it to the matrix without
editing a shell list.

Examples use the real public APIs:

```bash
cargo run -p quanta-memory-platform-facade --example hello_world
cargo run -p quanta-memory-platform-facade --example effective_budget_report
```

Generated guidance:

```bash
python3 tools/pm/pm.py sync
python3 tools/pm/pm.py lint
python3 tools/pm/pm.py status
python3 -m unittest -q tools/pm/tests/test_pm.py
```

The prompt manager is Python-standard-library-only. Its JSON manifest rejects
orphan sources, duplicate outputs, path escapes, missing outputs, and drift.
The Rust toolchain version is owned solely by `rust-toolchain.toml`; dependency
policy is owned by `deny.toml`; formatting is owned by `rustfmt.toml`.

Extraction is mechanical and fail-closed:

```bash
scripts/extract-to-standalone-repo.sh --dry-run /path/to/dest   # pre-flight only
scripts/extract-to-standalone-repo.sh /path/to/dest             # copy + verify + commit
```
