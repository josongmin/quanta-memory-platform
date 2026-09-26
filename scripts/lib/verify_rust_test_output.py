#!/usr/bin/env python3
"""Require a nonempty, unignored libtest result from the workspace test rail."""

from __future__ import annotations

import re
import sys
from pathlib import Path


RESULT = re.compile(r"(?m)^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;")


def counts(output: str) -> tuple[int, int]:
    results = RESULT.findall(output)
    if not results:
        raise ValueError("no successful Rust test summary found")
    passed = sum(int(value) for value, _, _ in results)
    failed = sum(int(value) for _, value, _ in results)
    ignored = sum(int(value) for _, _, value in results)
    if failed or ignored or not passed:
        raise ValueError(f"Rust test summary is incomplete: passed={passed} failed={failed} ignored={ignored}")
    return passed, len(results)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: verify_rust_test_output.py <cargo-test-output>", file=sys.stderr)
        return 2
    try:
        passed, suites = counts(Path(sys.argv[1]).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        print(f"Rust test output: FAIL: {error}", file=sys.stderr)
        return 1
    print(f"Rust test output: OK ({passed} passed across {suites} suites, 0 ignored)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
