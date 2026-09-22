# tools/pm — generated guidance manager

`tools/pm` is the source of truth for generated agent/operator guidance.
Markdown sections under `sources/**` are assembled according to `targets.json`.
The renderer uses only the Python standard library.

## Source-of-truth rule

- Edit `sources/**` for guidance content.
- Edit `targets.json` or `pm.py` only when the generation mechanism changes.
- Never hand-edit generated outputs. `pm lint` rejects drift and missing files.
- Every Markdown source must be referenced by a target; orphan sources, duplicate
  outputs, and paths escaping the repo are rejected.

| Target | Output |
| --- | --- |
| `agents` | `AGENTS.md` |
| `claude` | `CLAUDE.md` |
| `cursor_rules` | `.cursorrules` |

## Commands

```bash
python3 tools/pm/pm.py sync
python3 tools/pm/pm.py lint
python3 tools/pm/pm.py status
python3 tools/pm/pm.py preview --target agents
python3 -m unittest -q tools/pm/tests/test_pm.py
```

## Layout

```text
tools/pm/
  pm.py
  targets.json
  sources/
    rules/
    operational/
    agent-specific/
  tests/
```
