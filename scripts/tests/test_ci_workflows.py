"""CI configuration contract tests (stdlib only).

These tests check the *policy text* of the repository's CI configuration:
the CircleCI `full-extraction` job must run the standalone extraction, then
both public examples inside the extracted checkout, then the clean-status
assertion, in that order; the GitHub workflow must stay dispatch-only with
no top-level `push`/`pull_request` trigger. They do not claim that a hosted
job actually executed — hosted results are recorded separately.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CIRCLECI_CONFIG = ROOT / ".circleci" / "config.yml"
GITHUB_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"

STEP_NAME = "Verify actual standalone extraction and full workspace gate"
EXTRACTION_COMMAND = 'scripts/extract-to-standalone-repo.sh "$qmp_extract_dir"'
EXTRACT_CD_COMMAND = 'cd "$qmp_extract_dir"'
CLEAN_STATUS_COMMAND = 'test -z "$(git -C "$qmp_extract_dir" status --porcelain)"'
EXAMPLE_HELLO = "cargo run --locked -p quanta-memory-platform-facade --example hello_world"
EXAMPLE_REPORT = (
    "cargo run --locked -p quanta-memory-platform-facade --example effective_budget_report"
)


def extract_step_command_block(text: str, step_name: str) -> list[str]:
    """Return the shell body of the named CircleCI `run` step.

    The step is located by its `name:` line; the body starts after the
    `command: |` literal and is parsed by the indentation of its first
    non-blank line until a non-blank line dedents to or below the `command`
    key.
    """
    lines = text.splitlines()
    for index, line in enumerate(lines):
        if line.strip() != f"name: {step_name}":
            continue
        for cursor in range(index + 1, min(index + 5, len(lines))):
            if lines[cursor].strip() != "command: |":
                continue
            body: list[str] = []
            body_indent: int | None = None
            for body_line in lines[cursor + 1 :]:
                if not body_line.strip():
                    body.append("")
                    continue
                indent = len(body_line) - len(body_line.lstrip())
                if body_indent is None:
                    body_indent = indent
                elif indent < body_indent:
                    break
                body.append(body_line)
            return body
    raise AssertionError(f"CircleCI step {step_name!r} with a literal command block not found")


def exactly_one(lines: list[str], predicate, label: str, errors: list[str]) -> int | None:
    """Require exactly one matching line; record missing/duplicate errors."""
    hits = [position for position, line in enumerate(lines) if predicate(line)]
    if not hits:
        errors.append(f"missing {label}")
        return None
    if len(hits) > 1:
        errors.append(f"duplicate {label} ({len(hits)} occurrences)")
        return None
    return hits[0]


def circleci_extraction_gate_errors(command_block: list[str]) -> list[str]:
    """Validate the extraction/example/clean-status sequence of one job body."""
    stripped = [line.strip() for line in command_block]
    errors: list[str] = []

    extraction = exactly_one(
        stripped, lambda line: line == EXTRACTION_COMMAND, "extraction command", errors
    )
    enter = exactly_one(
        stripped, lambda line: line == EXTRACT_CD_COMMAND, "subshell cd into the extraction", errors
    )
    hello = exactly_one(
        stripped,
        lambda line: line.startswith("cargo run --locked") and line == EXAMPLE_HELLO,
        "hello_world example command",
        errors,
    )
    report = exactly_one(
        stripped,
        lambda line: line.startswith("cargo run --locked") and line == EXAMPLE_REPORT,
        "effective_budget_report example command",
        errors,
    )
    clean = exactly_one(
        stripped,
        lambda line: line == CLEAN_STATUS_COMMAND,
        "clean-status assertion",
        errors,
    )
    # Subshell membership: the subshell body (its `cd` and both example
    # commands) shares one indentation level strictly deeper than the step
    # body. Dedenting an example to the body level structurally removes it
    # from the extraction subshell even though presence and order still hold.
    def indent_of(line: str) -> int:
        return len(line) - len(line.lstrip())

    extraction_line = next(
        (l for l in command_block if l.strip() == EXTRACTION_COMMAND), None
    )
    cd_line = next((l for l in command_block if l.strip() == EXTRACT_CD_COMMAND), None)
    if extraction_line is None or cd_line is None or hello is None or report is None:
        errors.append("cannot verify subshell nesting without extraction/cd/example lines")
    else:
        body_indent = indent_of(extraction_line)
        if indent_of(cd_line) <= body_indent:
            errors.append("the subshell cd must be indented deeper than the step body")
        for label, position in (("hello_world", hello), ("effective_budget_report", report)):
            if indent_of(command_block[position]) <= body_indent:
                errors.append(
                    f"{label} example must be indented deeper than the step body to stay inside the extraction subshell"
                )
    for example in (hello, report):
        if example is not None:
            position = stripped[example]
            if not position.startswith("cargo run --locked"):
                errors.append("example command must start with `cargo run --locked`")

    positions = {
        "extraction": extraction,
        "subshell cd": enter,
        "hello_world example": hello,
        "effective_budget_report example": report,
        "clean-status assertion": clean,
    }
    resolved = {label: index for label, index in positions.items() if index is not None}
    order = [
        "extraction",
        "subshell cd",
        "hello_world example",
        "effective_budget_report example",
        "clean-status assertion",
    ]
    present_order = [label for label in order if label in resolved]
    ranked = sorted(present_order, key=lambda label: resolved[label])
    if ranked != present_order:
        errors.append(f"command order must be {' then '.join(order)}; got {ranked}")
    return errors


def github_workflow_event_errors(text: str) -> list[str]:
    """Require a dispatch-only top-level event block for the GitHub workflow."""
    lines = text.splitlines()
    errors: list[str] = []
    events: list[str] = []
    in_events = False
    for line in lines:
        if re.fullmatch(r"on:\s*", line):
            in_events = True
            continue
        if not in_events:
            continue
        if not line.strip():
            continue
        if not line.startswith(" "):
            break
        match = re.match(r"([A-Za-z_]+):", line.strip())
        if match:
            events.append(match.group(1))
    if "workflow_dispatch" not in events:
        errors.append("top-level event block must include workflow_dispatch")
    for banned in ("push", "pull_request"):
        if banned in events:
            errors.append(f"top-level event block must not declare {banned}")
    return errors


class CircleCiExtractionGateTest(unittest.TestCase):
    def test_current_config_passes_the_gate(self) -> None:
        block = extract_step_command_block(CIRCLECI_CONFIG.read_text(encoding="utf-8"), STEP_NAME)
        self.assertEqual(circleci_extraction_gate_errors(block), [])

    def test_missing_example_command_is_rejected(self) -> None:
        block = extract_step_command_block(CIRCLECI_CONFIG.read_text(encoding="utf-8"), STEP_NAME)
        negative = [line for line in block if line.strip() != EXAMPLE_HELLO]
        errors = circleci_extraction_gate_errors(negative)
        self.assertTrue(any("hello_world example command" in error for error in errors))

    def test_duplicate_extraction_command_is_rejected(self) -> None:
        block = extract_step_command_block(CIRCLECI_CONFIG.read_text(encoding="utf-8"), STEP_NAME)
        negative = list(block)
        negative.append(" " * 12 + EXTRACTION_COMMAND)
        errors = circleci_extraction_gate_errors(negative)
        self.assertTrue(any("duplicate extraction command" in error for error in errors))

    def test_dedented_examples_leave_the_subshell_and_are_rejected(self) -> None:
        block = extract_step_command_block(CIRCLECI_CONFIG.read_text(encoding="utf-8"), STEP_NAME)
        cd_index = next(
            index for index, line in enumerate(block) if line.strip() == EXTRACT_CD_COMMAND
        )
        hello_index = next(
            index for index, line in enumerate(block) if line.strip() == EXAMPLE_HELLO
        )
        report_index = next(
            index for index, line in enumerate(block) if line.strip() == EXAMPLE_REPORT
        )
        negative = list(block)
        # Dedent both examples to the step-body level: structurally outside
        # the extraction subshell even though presence and order still hold.
        body_indent = len(block[cd_index]) - len(block[cd_index].lstrip()) - 2
        negative[hello_index] = " " * body_indent + EXAMPLE_HELLO
        negative[report_index] = " " * body_indent + EXAMPLE_REPORT
        assert cd_index < hello_index, "fixture sanity: cd precedes examples"
        errors = circleci_extraction_gate_errors(negative)
        self.assertTrue(any("subshell" in error for error in errors), errors)

    def test_swapped_example_order_is_rejected(self) -> None:
        block = extract_step_command_block(CIRCLECI_CONFIG.read_text(encoding="utf-8"), STEP_NAME)
        negative = [line for line in block if line.strip() not in (EXAMPLE_HELLO, EXAMPLE_REPORT)]
        insert_at = next(
            index
            for index, line in enumerate(negative)
            if line.strip() == EXTRACT_CD_COMMAND
        ) + 1
        negative[insert_at:insert_at] = [
            " " * 16 + EXAMPLE_REPORT,
            " " * 16 + EXAMPLE_HELLO,
        ]
        errors = circleci_extraction_gate_errors(negative)
        self.assertTrue(any("command order" in error for error in errors))


class GitHubWorkflowExamplesTest(unittest.TestCase):
    """Mirror the CircleCI example gate for the manual GitHub workflow.

    The dispatch-only workflow runs the same two public examples inside its
    own extraction step; drift there must not be invisible to the contract.
    """

    def _example_lines(self) -> list[str]:
        text = GITHUB_WORKFLOW.read_text(encoding="utf-8")
        return [line.strip() for line in text.splitlines() if line.strip().startswith("cargo run")]

    def test_both_examples_run_locked_in_the_workflow(self) -> None:
        lines = self._example_lines()
        self.assertIn(EXAMPLE_HELLO, lines)
        self.assertIn(EXAMPLE_REPORT, lines)
        for line in lines:
            self.assertTrue(
                line.startswith("cargo run --locked"),
                f"workflow example must be --locked: {line}",
            )


class GitHubWorkflowDispatchOnlyTest(unittest.TestCase):
    def test_current_workflow_is_dispatch_only(self) -> None:
        self.assertEqual(github_workflow_event_errors(GITHUB_WORKFLOW.read_text(encoding="utf-8")), [])

    def test_fixture_adding_top_level_push_is_rejected(self) -> None:
        text = GITHUB_WORKFLOW.read_text(encoding="utf-8")
        fixture, replaced = re.subn(
            r"(^on:\s*$)",
            r"\1\n  push:\n    branches: [main]",
            text,
            count=1,
            flags=re.MULTILINE,
        )
        assert replaced == 1
        errors = github_workflow_event_errors(fixture)
        self.assertTrue(any("must not declare push" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
