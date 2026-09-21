#!/usr/bin/env python3
"""Run a Cargo test selection and fail if it executes zero tests."""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
from dataclasses import asdict, dataclass


RESULT_LINE = re.compile(
    r"test result: (?:ok|FAILED)\. "
    r"(?P<passed>\d+) passed; (?P<failed>\d+) failed; "
    r"(?P<ignored>\d+) ignored; (?P<measured>\d+) measured;"
)


class CargoTestEvidenceError(RuntimeError):
    pass


@dataclass(frozen=True)
class CargoTestCounts:
    selected: int
    passed: int
    failed: int
    ignored: int
    measured: int
    result_lines: int


def parse_cargo_test_counts(output: str) -> CargoTestCounts:
    matches = list(RESULT_LINE.finditer(output))
    if not matches:
        raise CargoTestEvidenceError("cargo output contains no test result lines")
    passed = sum(int(match.group("passed")) for match in matches)
    failed = sum(int(match.group("failed")) for match in matches)
    ignored = sum(int(match.group("ignored")) for match in matches)
    measured = sum(int(match.group("measured")) for match in matches)
    selected = passed + failed
    if selected == 0:
        raise CargoTestEvidenceError("cargo test selected zero executable tests")
    return CargoTestCounts(
        selected=selected,
        passed=passed,
        failed=failed,
        ignored=ignored,
        measured=measured,
        result_lines=len(matches),
    )


def main(argv: list[str] | None = None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    if args[:1] == ["--"]:
        args = args[1:]
    if not args:
        print("usage: run-ticket-test [cargo test arguments...]", file=sys.stderr)
        return 2

    command = ["cargo", "test", *args]
    completed = subprocess.run(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    sys.stdout.write(completed.stdout)
    output_digest = hashlib.sha256(completed.stdout.encode("utf-8")).hexdigest()
    if completed.returncode != 0:
        evidence = {
            "schema": "qmp-cargo-test-evidence-v1",
            "command": command,
            "exit": completed.returncode,
            "selected": None,
            "output_sha256": output_digest,
        }
        print("QMP_TEST_EVIDENCE=" + json.dumps(evidence, separators=(",", ":")))
        return completed.returncode

    try:
        counts = parse_cargo_test_counts(completed.stdout)
    except CargoTestEvidenceError as exc:
        evidence = {
            "schema": "qmp-cargo-test-evidence-v1",
            "command": command,
            "exit": completed.returncode,
            "selected": 0,
            "output_sha256": output_digest,
            "admission_error": str(exc),
        }
        print("QMP_TEST_EVIDENCE=" + json.dumps(evidence, separators=(",", ":")))
        print(f"run-ticket-test: FAIL: {exc}", file=sys.stderr)
        return 3

    evidence = {
        "schema": "qmp-cargo-test-evidence-v1",
        "command": command,
        "exit": completed.returncode,
        **asdict(counts),
        "output_sha256": output_digest,
    }
    print("QMP_TEST_EVIDENCE=" + json.dumps(evidence, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
