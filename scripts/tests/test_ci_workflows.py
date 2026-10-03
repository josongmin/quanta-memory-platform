"""Fail-closed shape checks for the hosted QMP verification entry point.

These do not claim a CircleCI runner executed. The extracted full gate and
examples remain the only normal job command.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONFIG = ROOT / ".circleci/config.yml"
EXAMPLES = ROOT / "scripts/verify-extracted-examples.sh"
FULL_STEP = "Verify actual standalone extraction and full workspace gate"
FULL_COMMAND = "scripts/verify-extracted-examples.sh"


def section(text: str, heading: str, indent: int) -> str:
    lines = text.splitlines()
    marker = " " * indent + heading + ":"
    matches = [i for i, line in enumerate(lines) if line == marker]
    if len(matches) != 1:
        raise AssertionError(f"expected one {marker!r}, found {len(matches)}")
    start = matches[0]
    end = next(
        (i for i in range(start + 1, len(lines))
         if lines[i].strip() and not lines[i].startswith(" " * (indent + 1))),
        len(lines),
    )
    return "\n".join(lines[start:end])


def config_errors(text: str) -> list[str]:
    errors: list[str] = []
    try:
        params = section(text, "parameters", 0)
        normal = section(section(text, "workflows", 0), "normal", 2)
        heavy_flow = section(section(text, "workflows", 0), "heavy", 2)
        full = section(section(text, "jobs", 0), "full-extraction", 2)
        heavy = section(section(text, "jobs", 0), "heavy-benchmarks", 2)
    except AssertionError as error:
        return [str(error)]
    if params.strip() != "parameters:\n  run_heavy:\n    type: boolean\n    default: false":
        errors.append("heavy parameter must be boolean and default false")
    if normal != "  normal:\n    unless: << pipeline.parameters.run_heavy >>\n    jobs:\n      - full-extraction":
        errors.append("normal workflow must run the full gate only when heavy is false")
    if heavy_flow != "  heavy:\n    when: << pipeline.parameters.run_heavy >>\n    jobs:\n      - heavy-benchmarks":
        errors.append("heavy workflow must require explicit true")
    if "resource_class: medium" not in full or 'CARGO_BUILD_JOBS: "2"' not in full:
        errors.append("normal job must match two CPUs")
    if "resource_class: large" not in heavy or 'CARGO_BUILD_JOBS: "4"' not in heavy:
        errors.append("benchmark job must match four CPUs")
    if text.count(FULL_STEP) != 1 or not re.search(
        rf"(?m)^      - run:\n"
        rf"          name: {re.escape(FULL_STEP)}\n"
        rf"          no_output_timeout: 20m\n"
        rf"          command: \|\n"
        rf"            set -euo pipefail\n"
        rf"            {re.escape(FULL_COMMAND)}(?:\n|$)",
        full,
    ):
        errors.append("full gate must be a direct, strict, executable job step")
    if full.count(FULL_COMMAND) != 1 or "continue-on-error" in text or "when: on_fail" in text:
        errors.append("full gate may not be skipped or have ignored failures")
    if "- store_artifacts:\n          path: /tmp/qmp-benchmark\n          destination: qmp-scaling" not in heavy:
        errors.append("source-bound benchmark artifacts must be retained")
    if "for run in 1 2 3; do" not in heavy or "scripts/run-scaling-bench.sh --profile=in-memory-pressure" not in heavy:
        errors.append("three pressure receipts missing")
    if "scripts/run-scaling-bench.sh\n" not in heavy or "standard-full.jsonl" not in heavy:
        errors.append("standard full receipt missing")
    if any(token in text for token in ("schedule:", "cron:", "- target/", "store_test_results:")):
        errors.append("unexpected schedule or compiled-output cache")
    if text.count("cargo fetch --locked") != 2:
        errors.append("both jobs must fetch exact lock dependencies before saving")
    if text.count("{{ checksum \"Cargo.lock\" }}") != 4:
        errors.append("cache keys must include the lockfile")
    if text.count("- save_cache:") != 3 or text.count("- restore_cache:") != 3:
        errors.append("cache read/write shape changed")
    for match in re.finditer(r"(?m)^          command: \|\n((?:^            .*\n|^\n)*)", text):
        body = match.group(1)
        if not body.startswith("            set -euo pipefail\n"):
            errors.append("run step lacks strict shell failure handling")
    if "|| true" in text or "set +e" in text:
        errors.append("failure swallowing is forbidden")
    if (ROOT / ".github/workflows/ci.yml").exists() or (ROOT / ".circleci/benchmark.yml").exists():
        errors.append("legacy CI entry points remain")
    return errors


class CiEntryPointContracts(unittest.TestCase):
    def test_current_config_and_executable(self) -> None:
        self.assertEqual(config_errors(CONFIG.read_text()), [])
        script = EXAMPLES.read_text()
        self.assertIn("set -euo pipefail", script)
        self.assertIn('scripts/extract-to-standalone-repo.sh "${qmp_extract_dir}"', script)
        self.assertIn('scripts/check-workspace.sh', (ROOT / "scripts/extract-to-standalone-repo.sh").read_text())
        self.assertEqual(script.count('--manifest-path "${qmp_extract_dir}/Cargo.toml"'), 2)
        self.assertTrue(EXAMPLES.stat().st_mode & 0o111)

    def test_heavy_and_full_gate_tampering_fail(self) -> None:
        text = CONFIG.read_text()
        for before, after in (
            ("default: false", "default: true"),
            ("unless: << pipeline.parameters.run_heavy >>", "when: << pipeline.parameters.run_heavy >>"),
            ("- full-extraction", "- heavy-benchmarks"),
            (FULL_COMMAND, "echo skipped"),
            ("set -euo pipefail", "set +e"),
            ("- store_artifacts:", "- ignored_artifacts:"),
            ("cargo fetch --locked", "cargo fetch"),
        ):
            with self.subTest(before=before):
                self.assertTrue(config_errors(text.replace(before, after, 1)))


if __name__ == "__main__":
    unittest.main()
