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
BENCHMARK_CONFIG = ROOT / ".circleci" / "benchmark.yml"
GITHUB_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
EXAMPLE_SCRIPT = ROOT / "scripts" / "verify-extracted-examples.sh"
STEP_NAME = "Verify actual standalone extraction and full workspace gate"
SCRIPT_COMMAND = "scripts/verify-extracted-examples.sh"


def job_block(text: str, name: str) -> str:
    """Select a direct child of jobs, never a step in another job."""
    lines = text.splitlines()
    starts = [i for i, line in enumerate(lines) if line == "jobs:"]
    if len(starts) != 1:
        raise AssertionError(f"expected one jobs section, got {len(starts)}")
    section = starts[0] + 1
    end = next((i for i in range(section, len(lines)) if lines[i] and not lines[i].startswith(" ")), len(lines))
    matches = [i for i in range(section, end) if lines[i] == f"  {name}:"]
    if len(matches) != 1:
        raise AssertionError(f"expected one {name!r} job, got {len(matches)}")
    start = matches[0]
    job_end = next((i for i in range(start + 1, end) if re.fullmatch(r"  [A-Za-z][\w-]*:", lines[i])), end)
    return "\n".join(lines[start:job_end])


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
        step = named_step(job_block(text, "full-extraction"), name)
    except AssertionError as error:
        return [str(error)]
    stripped = [line.strip() for line in step if line.strip()]
    errors: list[str] = []
    expected_indent = 10 if body_key == "command" else 6
    actual_indent = len(step[0]) - len(step[0].lstrip())
    if actual_indent != expected_indent:
        errors.append("extraction step is nested below a conditional or other wrapper")
    if any(re.fullmatch(r"if:\s*(false|0|\$\{\{\s*false\s*\}\})", line) for line in stripped):
        errors.append("extraction step is disabled")
    if any(line.startswith(("when:", "continue-on-error:")) for line in stripped):
        errors.append("extraction step has conditional or ignored failure")
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
    if events != ["workflow_dispatch"]:
        return [f"expected dispatch-only events, got {events}"]
    sentinel = re.search(
        r"(?ms)^  reject-unexpected-trigger:\n"
        r"    if: github\.event_name != 'workflow_dispatch'\n"
        r"    runs-on: ubuntu-24\.04\n"
        r"    steps:\n"
        r"      - name: Reject CI configuration bound to a push trigger\n"
        r"        run: \|\n"
        r"          echo [^\n]+\n"
        r"          exit 1\n",
        text,
    )
    return [] if sentinel else ["dispatch config must fail closed when bound to a push trigger"]


def github_full_job_errors(text: str) -> list[str]:
    jobs = text.split("\njobs:\n", 1)
    if len(jobs) != 2:
        return ["missing GitHub jobs"]
    match = re.search(r"(?m)^  full-extraction:\s*\n    if: (.+)$", jobs[1])
    expected = "github.event_name == 'workflow_dispatch' && inputs.verification == 'full'"
    if not match or match.group(1) != expected:
        return ["full extraction must require explicit full dispatch"]
    job = re.split(r"(?m)^  [A-Za-z][\w-]*:\s*$", jobs[1][match.end():], maxsplit=1)[0]
    if re.search(r"(?m)^    (?:needs|continue-on-error):", job):
        return ["full extraction must not depend on a skipped job or ignore failures"]
    return []


def circleci_push_workflow_errors(text: str) -> list[str]:
    workflow = text.split("\nworkflows:\n", 1)
    if len(workflow) != 2:
        return ["missing CircleCI workflows"]
    lines = [line.strip() for line in workflow[1].splitlines() if line.strip()]
    return [] if lines == ["push-verification:", "jobs:", "- full-extraction"] else [
        "every push must schedule exactly one unconditional full-extraction job"
    ]


def benchmark_artifact_errors(text: str) -> list[str]:
    required = (
        "path: /tmp/qmp-benchmark\n          destination: qmp-in-memory-pressure",
        "path: /tmp/qmp-benchmark/results.jsonl\n          destination: qmp-standard-full/results.jsonl",
    )
    return ["benchmark receipt artifact is not retained"] if (
        text.count("- store_artifacts:") != 2 or any(item not in text for item in required)
    ) else []


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
        'extracted_status="$(git -C "${qmp_extract_dir}" status --porcelain)" || {',
        'echo "Failed to inspect extracted checkout: ${qmp_extract_dir}" >&2',
        'exit 1',
        '}',
        'test -z "${extracted_status}"',
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
        self.assertEqual(circleci_push_workflow_errors(circle), [])
        self.assertEqual(benchmark_artifact_errors(BENCHMARK_CONFIG.read_text(encoding="utf-8")), [])
        self.assertEqual(github_event_errors(github), [])
        self.assertEqual(github_full_job_errors(github), [])
        self.assertEqual(example_script_errors(script), [])
        self.assertTrue(EXAMPLE_SCRIPT.stat().st_mode & 0o111)

    def test_schedule_trigger_and_missing_main_filter_are_rejected(self) -> None:
        github = GITHUB_WORKFLOW.read_text(encoding="utf-8")
        mutated = github.replace("on:\n", "on:\n  schedule:\n    - cron: '0 0 * * *'\n", 1)
        self.assertTrue(github_event_errors(mutated))
        self.assertTrue(github_event_errors(github.replace("          exit 1\n", "          exit 0\n", 1)))
        self.assertTrue(github_full_job_errors(github.replace("github.event_name == 'workflow_dispatch' && inputs.verification == 'full'", "github.event_name != 'workflow_dispatch' || inputs.verification == 'full'")))
        circle = CIRCLECI_CONFIG.read_text(encoding="utf-8")
        self.assertTrue(circleci_push_workflow_errors(circle.replace("- full-extraction", "- full-extraction:\n          filters: pipeline.git.branch == \"main\"")))

    def test_disabled_step_and_missing_command_are_rejected(self) -> None:
        github = GITHUB_WORKFLOW.read_text(encoding="utf-8")
        self.assertTrue(executable_step_errors(github.replace(f"- name: {STEP_NAME}", f"- name: {STEP_NAME}\n        if: false"), STEP_NAME, "run"))
        self.assertTrue(executable_step_errors(github.replace(SCRIPT_COMMAND, "echo skipped"), STEP_NAME, "run"))
        self.assertTrue(executable_step_errors(github.replace(SCRIPT_COMMAND, "exit 0\n          " + SCRIPT_COMMAND), STEP_NAME, "run"))
        self.assertTrue(executable_step_errors(github.replace(f"- name: {STEP_NAME}", f"- name: {STEP_NAME}\n        continue-on-error: true"), STEP_NAME, "run"))
        circle = CIRCLECI_CONFIG.read_text(encoding="utf-8")
        self.assertTrue(executable_step_errors(circle.replace(f"name: {STEP_NAME}", f"name: {STEP_NAME}\n          when: on_fail"), STEP_NAME, "command"))

    def test_workflow_and_job_skip_paths_are_rejected(self) -> None:
        circle = CIRCLECI_CONFIG.read_text(encoding="utf-8")
        self.assertTrue(circleci_push_workflow_errors(circle.replace("workflows:\n", "workflows:\n  when: false\n", 1)))
        github = GITHUB_WORKFLOW.read_text(encoding="utf-8")
        self.assertTrue(github_full_job_errors(github.replace("  full-extraction:\n", "  full-extraction:\n    needs: billing-probe\n", 1)))

    def test_extraction_step_must_be_directly_in_the_scheduled_job(self) -> None:
        circle = CIRCLECI_CONFIG.read_text(encoding="utf-8")
        direct = (
            "      - run:\n"
            f"          name: {STEP_NAME}\n"
            "          no_output_timeout: 20m\n"
            "          command: |\n"
            "            set -euo pipefail\n"
            f"            {SCRIPT_COMMAND}\n"
        )
        self.assertEqual(circle.count(direct), 1)
        conditional = (
            "      - when:\n"
            "          condition: false\n"
            "          steps:\n"
            "            - run:\n"
            f"                name: {STEP_NAME}\n"
            "                no_output_timeout: 20m\n"
            "                command: |\n"
            "                  set -euo pipefail\n"
            f"                  {SCRIPT_COMMAND}\n"
        )
        self.assertTrue(executable_step_errors(circle.replace(direct, conditional), STEP_NAME, "command"))
        unused_job = (
            "  unused-verification:\n"
            "    machine:\n"
            "      image: ubuntu-2404:2026.05.1\n"
            "    steps:\n" + direct
        )
        moved = circle.replace(direct, "").replace("workflows:\n", unused_job + "\nworkflows:\n")
        self.assertTrue(executable_step_errors(moved, STEP_NAME, "command"))

    def test_benchmark_artifact_steps_cannot_disappear(self) -> None:
        text = BENCHMARK_CONFIG.read_text(encoding="utf-8")
        self.assertTrue(benchmark_artifact_errors(text.replace("- store_artifacts:", "- ignored_artifact:", 1)))

    def test_early_close_and_source_checkout_examples_are_rejected(self) -> None:
        script = EXAMPLE_SCRIPT.read_text(encoding="utf-8")
        self.assertTrue(example_script_errors(script.replace('cd "${repo_root}"', 'cd "${repo_root}"\n)')))
        self.assertTrue(example_script_errors(script.replace('--manifest-path "${qmp_extract_dir}/Cargo.toml" ', '')))


if __name__ == "__main__":
    unittest.main()
