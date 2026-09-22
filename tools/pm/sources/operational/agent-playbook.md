## Closeout Playbook

Every meaningful slice should close with:

1. The code change (smallest surface that proves the behavior).
2. Tests in the owning crate module (`#[cfg(test)]`), asserting invariants — not
   just `Ok`.
3. A conformance / contract test if the slice touches accounting or a public
   contract (and the suite still REJECTS a broken fake).
4. Updated `lib.rs` re-exports + examples if the public API changed.
5. Doc updates (`README.md` / `EXTRACTION.md`) if direction, surface, or build
   steps changed.

Before closeout, run:

```bash
# Regenerate agent docs if you edited tools/pm/sources/**
python3 tools/pm/pm.py sync
scripts/check-workspace.sh          # canonical static/test/feature/selector/PM/deny rail

# if you changed any Cargo.toml or cross-crate use path:
scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-check
```

Manual smoke (examples compile against the real public API):

```bash
cargo run -p quanta-memory-platform-facade --example hello_world
cargo run -p quanta-memory-platform-facade --example effective_budget_report
```

Closeout report format:

```text
Changed:
- ...

Verified:
- scripts/check-workspace.sh
- extraction dry-run (if Cargo.toml/use paths changed)

Not run:
- ...

Known risk:
- ...
```

Fail-closed principle: if `pm lint` fails, sync — do not patch generated files.
If tests fail, fix behavior; do not weaken assertions to greenwash. Never invert
the `core <- facade <- testkit` direction to make something compile.
