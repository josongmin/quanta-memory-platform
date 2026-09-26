"""Fail-closed local contracts for the checked-in hosted CI entry points.

These assertions prove configuration shape, not that a hosted runner executed.
The actual standalone extraction and examples remain a separate release gate.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CIRCLECI_CONFIG = ROOT / ".circleci" / "config.yml"
GITHUB_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
EXAMPLE_SCRIPT = ROOT / "scripts" / "verify-extracted-examples.sh"
STEP_NAME = "Verify actual standalone extraction and full workspace gate"
SCRIPT_COMMAND = "scripts/verify-extracted-examples.sh"


def named_step(text: str, name: str) -> list[str]:
    """Return one YAML step, bounded by the next step at the same indentation."""
    lines = text.splitlines()
    matches = [i for i, line in enumerate(lines) if line.strip() == f"- name: {name}"]
    # CircleCI spells `- run:` followed by `name:` one level deeper.
    if not matches:
        matches = [i for i, line in enumerate(lines) if line.strip() == f"name: {name}"]
    if len(matches) != 1:
        raise AssertionError(f"expected one {name!r} step, got {len(matches)}")
    start = matches[0]
    indent = len(lines[start]) - len(lines[start].lstrip())
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if not line.strip():
            continue
        level = len(line) - len(line.lstrip())
        if level < indent or (level == indent and line.lstrip().startswith("- ")):
            end = i
            break
    return lines[start:end]


def executable_step_errors(text: str, name: str, body_key: str) -> list[str]:
    try:
        step = named_step(text, name)
    except AssertionError as error:
        return [str(error)]
    stripped = [line.strip() for line in step if line.strip()]
    errors: list[str] = []
    if any(re.fullmatch(r"if:\s*(false|0|\$\{\{\s*false\s*\}\})", line) for line in stripped):
        errors.append("extraction step is disabled")
    if f"{body_key}: |" not in stripped:
        errors.append("extraction step has no executable command block")
    else:
        body = stripped[stripped.index(f"{body_key}: |") + 1 :]
        if body != ["set -euo pipefail", SCRIPT_COMMAND]:
            errors.append("extraction step must execute only the strict script entry point")
    return errors


def github_event_errors(text: str) -> list[str]:
    events: list[str] = []
    in_events = False
    for line in text.splitlines():
        if re.fullmatch(r"on:\s*", line):
            in_events = True
            continue
        if not in_events or not line.strip():
            continue
        if not line.startswith(" "):
            break
        if re.match(r"^  [A-Za-z_]+:\s*", line):
            events.append(line.strip().split(":", 1)[0])
    return [] if events == ["workflow_dispatch"] else [f"expected dispatch-only events, got {events}"]


def github_full_job_errors(text: str) -> list[str]:
    jobs = text.split("\njobs:\n", 1)
    if len(jobs) != 2:
        return ["missing GitHub jobs"]
    match = re.search(r"(?m)^  full-extraction:\s*\n    if: (.+)$", jobs[1])
    expected = "github.event_name == 'workflow_dispatch' && inputs.verification == 'full'"
    return [] if match and match.group(1) == expected else ["full extraction must require explicit full dispatch"]


def circleci_main_filter_errors(text: str) -> list[str]:
    workflow = text.split("\nworkflows:\n", 1)
    if len(workflow) != 2:
        return ["missing CircleCI workflows"]
    expected = re.compile(
        r"(?m)^\s+- full-extraction:\s*\n\s+filters: pipeline\.git\.branch == \"main\"\s*$"
    )
    return [] if len(expected.findall(workflow[1])) == 1 else ["full-extraction must be main-filtered"]


def example_script_errors(text: str) -> list[str]:
    # The short owned script is the executable SSOT. Exact shape disallows an
    # early subshell close, a no-op example, or source-checkout execution.
    lines = [line.strip() for line in text.splitlines() if line.strip() and not line.startswith("#")]
    expected = [
        "set -euo pipefail",
        'repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"',
        'cd "${repo_root}"',
        'qmp_extract_dir="$(mktemp -d /tmp/qmp-ci-extraction.XXXXXX)"',
        'echo "Source SHA: $(git rev-parse --verify HEAD)"',
        'scripts/extract-to-standalone-repo.sh "${qmp_extract_dir}"',
        'cargo run --locked --manifest-path "${qmp_extract_dir}/Cargo.toml" -p quanta-memory-platform-facade --example hello_world',
        'cargo run --locked --manifest-path "${qmp_extract_dir}/Cargo.toml" -p quanta-memory-platform-facade --example effective_budget_report',
        'test -z "$(git -C "${qmp_extract_dir}" status --porcelain)"',
        'echo "Verified extracted checkout: ${qmp_extract_dir}"',
    ]
    return [] if lines == expected else ["extraction/example/clean command sequence drifted"]


class CiEntryPointContracts(unittest.TestCase):
    def test_current_configs_and_script(self) -> None:
        circle = CIRCLECI_CONFIG.read_text(encoding="utf-8")
        github = GITHUB_WORKFLOW.read_text(encoding="utf-8")
        script = EXAMPLE_SCRIPT.read_text(encoding="utf-8")
        self.assertEqual(executable_step_errors(circle, STEP_NAME, "command"), [])
        self.assertEqual(executable_step_errors(github, STEP_NAME, "run"), [])
        self.assertEqual(circleci_main_filter_errors(circle), [])
        self.assertEqual(github_event_errors(github), [])
        self.assertEqual(github_full_job_errors(github), [])
        self.assertEqual(example_script_errors(script), [])
        self.assertTrue(EXAMPLE_SCRIPT.stat().st_mode & 0o111)

    def test_schedule_trigger_and_missing_main_filter_are_rejected(self) -> None:
        github = GITHUB_WORKFLOW.read_text(encoding="utf-8")
        mutated = github.replace("on:\n", "on:\n  schedule:\n    - cron: '0 0 * * *'\n", 1)
        self.assertTrue(github_event_errors(mutated))
        self.assertTrue(github_full_job_errors(github.replace("github.event_name == 'workflow_dispatch' && inputs.verification == 'full'", "github.event_name != 'workflow_dispatch' || inputs.verification == 'full'")))
        circle = CIRCLECI_CONFIG.read_text(encoding="utf-8")
        self.assertTrue(circleci_main_filter_errors(circle.replace('filters: pipeline.git.branch == "main"', 'filters: pipeline.git.branch == "dev"')))

    def test_disabled_step_and_missing_command_are_rejected(self) -> None:
        github = GITHUB_WORKFLOW.read_text(encoding="utf-8")
        self.assertTrue(executable_step_errors(github.replace(f"- name: {STEP_NAME}", f"- name: {STEP_NAME}\n        if: false"), STEP_NAME, "run"))
        self.assertTrue(executable_step_errors(github.replace(SCRIPT_COMMAND, "echo skipped"), STEP_NAME, "run"))
        self.assertTrue(executable_step_errors(github.replace(SCRIPT_COMMAND, "exit 0\n          " + SCRIPT_COMMAND), STEP_NAME, "run"))

    def test_early_close_and_source_checkout_examples_are_rejected(self) -> None:
        script = EXAMPLE_SCRIPT.read_text(encoding="utf-8")
        self.assertTrue(example_script_errors(script.replace('cd "${repo_root}"', 'cd "${repo_root}"\n)')))
        self.assertTrue(example_script_errors(script.replace('--manifest-path "${qmp_extract_dir}/Cargo.toml" ', '')))


if __name__ == "__main__":
    unittest.main()
