## Closeout Playbook

Run focused checks while iterating. For a meaningful code/configuration change,
close with the canonical rail:

```bash
python3 tools/pm/pm.py sync
scripts/check-workspace.sh
```

If `Cargo.toml` or a cross-crate use path changed, run the extraction dry-run on
a committed, clean candidate:

```bash
scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-check
```

Report changed paths, exact verification run, anything not run, and remaining
risk. Do not treat focused checks as the canonical rail. If `pm lint` fails,
sync from source; if tests fail, fix behavior rather than weaken assertions.
