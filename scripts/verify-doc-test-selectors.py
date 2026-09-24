#!/usr/bin/env python3
"""Reject stale test selectors named by the public verification documents."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DOCUMENTS = (
    ROOT / "README.md",
    ROOT / "EXTRACTION.md",
    ROOT / "docs/verification-scenarios.md",
    ROOT / "MIGRATION.md",
    ROOT / "CHANGELOG.md",
)

# These are documented fields, suite/function names, or conformance case labels,
# not Cargo test selectors. Keeping the set explicit prevents the linter from
# silently treating an arbitrary stale selector as prose.
NON_TEST_IDENTIFIERS = {
    "FACADE_STORE_SUITE",
    "allocator_observation_is_inert",
    "hard_cap_bytes",
    "payload_dedup_refund",
    "pin_safe_delete",
    "retained_map_refunds",
    "run_facade_store_suite",
    "soft_cap_bytes",
    "spill_round_trip",
    "spill_unavailable_fail_closed",
    "transcript_truncate_refund",
}


def documented_candidates(
    documents: tuple[Path, ...] = DOCUMENTS,
) -> dict[str, set[str]]:
    candidates: dict[str, set[str]] = {}
    for path in documents:
        text = path.read_text(encoding="utf-8")
        for token in re.findall(r"`([A-Za-z][A-Za-z0-9_:-]{5,})`", text):
            if token.count("_") < 2 or "::" in token or "-" in token:
                continue
            candidates.setdefault(token, set()).add(path.relative_to(ROOT).as_posix())
    return candidates


def parse_listed_tests(output: str) -> set[str]:
    return {
        line.removesuffix(": test").split("::")[-1]
        for line in output.splitlines()
        if line.endswith(": test")
    }


def parse_executed_tests(output: str) -> set[str]:
    tests: set[str] = set()
    has_ignored = re.search(r"\.\.\. ignored\b|;\s*[1-9][0-9]* ignored\b", output) is not None
    marker = re.compile(r"(?:^|(?<=\.\.\. ))test (?P<name>[A-Za-z0-9_:]+) \.\.\.")
    for line in output.splitlines():
        # libtest output can interleave when tests spawn subprocesses: one
        # line may contain "test A ... test B ... ok" and A's "ok" later.
        # The caller only reuses output after cargo test exits successfully.
        matches = list(marker.finditer(line))
        for index, match in enumerate(matches):
            result_end = matches[index + 1].start() if index + 1 < len(matches) else len(line)
            result = line[match.end():result_end]
            if re.search(r"\bignored\b", result):
                continue
            if has_ignored and not re.search(r"\b(?:ok|FAILED)\b", result):
                # An interleaved marker with no attached result may itself be
                # ignored, so do not certify it from a later standalone "ok".
                continue
            tests.add(match.group("name").split("::")[-1])
    return tests


def listed_tests() -> set[str]:
    command = [
        "cargo",
        "test",
        "--locked",
        "--workspace",
        "--all-features",
        "--",
        "--list",
    ]
    completed = subprocess.run(
        command,
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if completed.returncode != 0:
        sys.stderr.write(completed.stdout)
        sys.stderr.write(completed.stderr)
        raise RuntimeError("cargo test -- --list failed")
    return parse_listed_tests(completed.stdout)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--test-output",
        type=Path,
        help="require completed cargo test output as well as Cargo's registered test list",
    )
    args = parser.parse_args(argv)

    candidates = documented_candidates()
    registered = listed_tests()
    tests = registered
    if args.test_output:
        # Cargo's successful output proves execution, while --list proves the
        # name was registered by the harness rather than printed by a test.
        tests = registered & parse_executed_tests(args.test_output.read_text(encoding="utf-8"))
    missing = sorted(set(candidates) - tests - NON_TEST_IDENTIFIERS)
    if missing:
        for selector in missing:
            locations = ", ".join(sorted(candidates[selector]))
            print(f"stale documented test selector: {selector} ({locations})", file=sys.stderr)
        return 1
    print(
        "doc-test-selectors: OK "
        f"({len(set(candidates) - NON_TEST_IDENTIFIERS)} selectors resolve)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
