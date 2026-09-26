## Closeout Playbook

Run focused checks while iterating. For a meaningful code/configuration change,
close with the canonical rail:

```bash
python3 tools/pm/pm.py sync
scripts/check-workspace.sh
```

For guidance-only edits, run `python3 tools/pm/pm.py sync` and
`python3 tools/pm/pm.py lint`. For all prose-only changes, check affected links
and `git diff --check`; the full Rust rail is not required. Changes to generation
or gate behavior still require their owning contracts and the canonical rail.

If `Cargo.toml` or a cross-crate use path changed, run the extraction dry-run on
a committed, clean candidate:

```bash
scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-check
```

Report changed paths, exact verification run, anything not run, and remaining
risk. Bind verification claims to the tested source and configuration; historical
receipts and successful zero-test runs do not prove current behavior.
Do not treat focused checks as the canonical rail. If `pm lint` fails,
sync from source; if tests fail, fix behavior rather than weaken assertions.
