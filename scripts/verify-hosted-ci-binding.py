#!/usr/bin/env python3
"""Check the live CircleCI push trigger against the checked-in full gate."""

from __future__ import annotations

import json
import subprocess
import sys


PROJECT_ID = "0869ae14-9b92-436d-9bba-8ac4b4cfafb5"
EXPECTED_REPO_ID = "1362792313"
EXPECTED_CONFIG = ".circleci/config.yml"
PUSH_PRESETS = {
    "all-pushes", "default-branch-pushes", "only-build-pushes-to-non-draft-prs",
    "pushes-to-merge-queues",
}


def circleci(*arguments: str) -> object:
    result = subprocess.run(
        ["circleci", *arguments, "--json"], capture_output=True, text=True, check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"CircleCI query failed ({result.returncode}): {result.stderr or result.stdout}")
    return json.loads(result.stdout)


def binding_errors(definitions: object, triggers: dict[str, object]) -> list[str]:
    if not isinstance(definitions, list) or not definitions:
        return ["no CircleCI pipeline definitions returned"]
    errors: list[str] = []
    canonical = 0
    for definition in definitions:
        if not isinstance(definition, dict) or not isinstance(definition.get("id"), str):
            return ["invalid CircleCI pipeline definition response"]
        config = definition.get("config_source")
        checkout = definition.get("checkout_source")
        if not isinstance(config, dict) or not isinstance(checkout, dict):
            return ["pipeline definition is missing source metadata"]
        config_repo = config.get("repo")
        checkout_repo = checkout.get("repo")
        if not isinstance(config_repo, dict) or not isinstance(checkout_repo, dict):
            return ["pipeline definition is missing repository metadata"]
        events = triggers.get(definition["id"])
        if not isinstance(events, list):
            return [f"triggers unavailable for {definition['id']}"]
        for event in events:
            if not isinstance(event, dict):
                return ["invalid CircleCI trigger response"]
            if event.get("disabled") is True or (
                event.get("event_preset") not in PUSH_PRESETS and event.get("event_name") != "push"
            ):
                continue
            if (
                config.get("file_path") != EXPECTED_CONFIG
                or config.get("provider") != "github_app"
                or config_repo.get("external_id") != EXPECTED_REPO_ID
                or checkout_repo.get("external_id") != EXPECTED_REPO_ID
                or event.get("repo_external_id") != EXPECTED_REPO_ID
            ):
                errors.append(f"enabled push trigger {event.get('id')} uses the wrong full-gate config or repo")
            elif event.get("event_preset") == "all-pushes":
                canonical += 1
    if canonical != 1:
        errors.append(f"expected exactly one canonical full-gate push trigger, found {canonical}")
    return errors


def main() -> int:
    try:
        definitions = circleci("pipeline", "list", "--project-id", PROJECT_ID)
        if not isinstance(definitions, list):
            raise RuntimeError("CircleCI pipeline list is not an array")
        triggers = {
            item["id"]: circleci(
                "project", "trigger", "list", "--project-id", PROJECT_ID,
                "--pipeline-definition-id", item["id"],
            )
            for item in definitions
        }
        errors = binding_errors(definitions, triggers)
    except (OSError, ValueError, KeyError, TypeError, RuntimeError) as error:
        errors = [str(error)]
    for error in errors:
        print(f"hosted CI binding: FAIL: {error}", file=sys.stderr)
    if errors:
        return 1
    print("hosted CI binding: OK (one full-gate push trigger)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
