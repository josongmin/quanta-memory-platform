## Claude Code Notes

- `CLAUDE.md` is generated from `tools/pm/sources/**`. Never hand-edit it; run
  `python3 tools/pm/pm.py sync` after source edits, then `python3 tools/pm/pm.py lint`.
- Self-check against the Rule Cheatsheet before claiming a slice is done.
- The two reflexes reviewers check first: (1) never invert `core <- facade <-
  testkit`, and (2) never let Semantica/host nouns into the public API. If a
  change makes you reach upward or import the host, stop and re-plan.
- Keep `domain/` pure — if you need fs/clock/env, add a port + adapter instead.
- For accounting changes, assert the ledger invariants in tests; "it returned Ok"
  is not a test.
- When unsure whether a change preserves portability, run
  `scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp`.
