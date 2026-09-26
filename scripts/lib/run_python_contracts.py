#!/usr/bin/env python3
"""Run every discovered Python contract module and reject empty suites."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def cases(suite: unittest.TestSuite) -> list[unittest.TestCase]:
    return [case for item in suite for case in (cases(item) if isinstance(item, unittest.TestSuite) else [item])]


def run_suite(directory: Path, pattern: str = "test_*.py") -> bool:
    files = {path.stem for path in directory.glob(pattern)}
    if not files:
        print(f"Python contract suite has no modules: {directory}", file=sys.stderr)
        return False
    suite = unittest.TestLoader().discover(str(directory), pattern=pattern)
    found = cases(suite)
    modules = {case.__class__.__module__.rsplit(".", 1)[-1] for case in found}
    missing = files - modules
    if missing or not found:
        print(f"Python contract modules have no tests: {', '.join(sorted(missing))}", file=sys.stderr)
        return False
    print(f"Python contracts: {len(files)} modules, {len(found)} tests", flush=True)
    return unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful()


def main() -> int:
    if len(sys.argv) != 2 or sys.argv[1] not in {"scripts", "pm"}:
        print("usage: run_python_contracts.py [scripts|pm]", file=sys.stderr)
        return 2
    directory = ROOT / ("scripts/tests" if sys.argv[1] == "scripts" else "tools/pm/tests")
    return 0 if run_suite(directory) else 1


if __name__ == "__main__":
    raise SystemExit(main())
