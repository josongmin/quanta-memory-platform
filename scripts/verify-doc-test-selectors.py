#!/usr/bin/env python3
"""Reject stale test selectors named by the public verification documents."""

from __future__ import annotations

import argparse
import json
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
    "do_begin_rebuild",
    "allocator_observation_is_inert",
    "grow_to_observed",
    "hard_cap_bytes",
    "identity_evidence_bytes",
    "identity_evidence_count",
    "payload_dedup_refund",
    "pin_safe_delete",
    "release_managed_many",
    "retained_map_refunds",
    "run_facade_store_suite",
    "soft_cap_bytes",
    "spill_round_trip",
    "spill_unavailable_fail_closed",
    "total_retained_metadata_bytes",
    "total_retained_payload_bytes",
    "transcript_truncate_refund",
}


def owner_contract_errors(candidates: dict[str, set[str]], root: Path = ROOT) -> list[str]:
    """Pin each documented selector to the Rust source that owns its test."""
    def defines_test(lines: list[str], leaf: str) -> bool:
        function = re.compile(rf"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+{re.escape(leaf)}\s*\(")
        for index, line in enumerate(lines):
            if function.search(line) is None:
                continue
            cursor = index - 1
            attributes: list[str] = []
            while cursor >= 0:
                preceding = lines[cursor].strip()
                if preceding and not preceding.startswith("#["):
                    break
                attributes.append(preceding)
                cursor -= 1
            if any(re.fullmatch(r"#\[\s*(?:test|tokio::test)\s*\]", attr) for attr in attributes):
                return True
        return False

    def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate selector owner: {key}")
            result[key] = value
        return result

    try:
        owners = json.loads((root / "scripts/test-selector-owners.json").read_text(encoding="utf-8"),
                            object_pairs_hook=unique_object)
    except (OSError, ValueError) as error:
        return [f"selector owner manifest cannot be read: {error}"]
    if not isinstance(owners, dict):
        return ["selector owner manifest must be an object"]
    selectors = set(candidates) - NON_TEST_IDENTIFIERS
    errors = [f"selector owner inventory differs: missing={sorted(selectors - owners.keys())} "
              f"extra={sorted(owners.keys() - selectors)}"] if owners.keys() != selectors else []
    source_contents: dict[Path, list[str]] = {}
    for selector in sorted(selectors & owners.keys()):
        owner = owners[selector]
        if not isinstance(owner, str) or not owner.startswith("crates/") or ".." in Path(owner).parts or not owner.endswith(".rs"):
            errors.append(f"{selector}: invalid owner path {owner!r}")
            continue
        path = root / owner
        if path.is_symlink() or not path.is_file():
            errors.append(f"{selector}: owner source is missing: {owner}")
            continue
        leaf = selector.rsplit("::", 1)[-1]
        if path not in source_contents:
            source_contents[path] = path.read_text(encoding="utf-8").splitlines()
        if not defines_test(source_contents[path], leaf):
            errors.append(f"{selector}: owner source no longer defines an annotated test: {owner}")
    return errors


def documented_candidates(
    documents: tuple[Path, ...] = DOCUMENTS,
) -> dict[str, set[str]]:
    candidates: dict[str, set[str]] = {}
    for path in documents:
        text = path.read_text(encoding="utf-8")
        for token in re.findall(r"`([A-Za-z][A-Za-z0-9_:-]{5,})`", text):
            if "-" in token or ("::tests::" not in token and token.count("_") < 2):
                continue
            if "::" in token and "::tests::" not in token:
                continue
            candidates.setdefault(token, set()).add(path.relative_to(ROOT).as_posix())
    return candidates


def parse_listed_tests(output: str) -> set[str]:
    return {
        line.removesuffix(": test")
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
            tests.add(match.group("name"))
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
    owner_errors = owner_contract_errors(candidates)
    if owner_errors:
        for error in owner_errors:
            print(f"doc-test-selectors: {error}", file=sys.stderr)
        return 1
    registered = listed_tests()
    selectors = set(candidates) - NON_TEST_IDENTIFIERS
    if not selectors or not registered:
        print("doc-test-selectors: no documented or registered tests", file=sys.stderr)
        return 1
    tests = registered
    if args.test_output:
        # Cargo's successful output proves execution, while --list proves the
        # name was registered by the harness rather than printed by a test.
        tests = registered & parse_executed_tests(args.test_output.read_text(encoding="utf-8"))
    missing: list[str] = []
    ambiguous: list[str] = []
    for candidate in sorted(selectors):
        matches = {name for name in registered if (
            name == candidate if "::" in candidate else name.rsplit("::", 1)[-1] == candidate
        )}
        if len(matches) > 1:
            ambiguous.append(candidate)
        elif len(matches) != 1 or not matches <= tests:
            missing.append(candidate)
    for selector in ambiguous:
        print(f"ambiguous documented test selector: {selector}; use a qualified path", file=sys.stderr)
    if missing or ambiguous:
        for selector in missing:
            locations = ", ".join(sorted(candidates[selector]))
            print(f"stale documented test selector: {selector} ({locations})", file=sys.stderr)
        return 1
    print(
        "doc-test-selectors: OK "
        f"({len(selectors)} selectors resolve)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
