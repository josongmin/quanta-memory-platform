#!/usr/bin/env python3
"""Reject unscoped or unexplained Semgrep suppressions in Rust sources."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
RULES = ROOT / "tools/semgrep/qmp-regressions.yml"
SUPPRESSION = re.compile(r"\bnosem(?:grep)?\b", re.IGNORECASE)
REASON = re.compile(r":\s*(qmp-[a-z0-9-]+)\s+--\s+reason:\s*(\S.*)$")


def errors_for(root: Path, rules: Path) -> list[str]:
    ids = set(re.findall(r"(?m)^\s+- id: (qmp-[a-z0-9-]+)\s*$", rules.read_text(encoding="utf-8")))
    if not ids:
        return ["no Semgrep rule IDs found"]
    errors: list[str] = []
    for path in (root / "crates").rglob("*.rs"):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            match = SUPPRESSION.search(line)
            if not match:
                continue
            reason = REASON.fullmatch(line[match.end():])
            canonical = match.group() == "nosemgrep" and line[:match.start()].rstrip().endswith("//")
            if not canonical or reason is None or reason.group(1) not in ids:
                errors.append(f"{path.relative_to(root)}:{number}: suppression requires a known rule ID and reason")
    return errors


def main() -> int:
    errors = errors_for(ROOT, RULES)
    for error in errors:
        print(f"regression-guards: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
