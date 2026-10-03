#!/usr/bin/env python3
"""Check the live personal CircleCI trigger set against the checked-in full gate."""

from __future__ import annotations

import json
import subprocess
import sys


PROJECT_ID = "0869ae14-9b92-436d-9bba-8ac4b4cfafb5"
PROJECT_SLUG = "circleci/Q3G2VbitoZmaQSKihvptcF/23Fh6MNDBV4PLARDMMvDJc"
EXPECTED_REPO_ID = "1362792313"
EXPECTED_REPO = "josongmin/quanta-memory-platform"
EXPECTED_CONFIG = ".circleci/config.yml"
EXPECTED_RULES = {
    (("push", "event.branch == event.repository.default_branch", "push_ref"),),
    (("pull_request", 'event.action == "opened" or event.action == "reopened" or event.action == "synchronize"', "pr_head_ref"),),
}


def circleci(*arguments: str) -> object:
    result = subprocess.run(
        ["circleci", *arguments, "--json"], capture_output=True, text=True, check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"CircleCI query failed ({result.returncode}): {result.stderr or result.stdout}")
    return json.loads(result.stdout)


def circleci_api(path: str) -> object:
    result = subprocess.run(
        ["circleci", "api", path], capture_output=True, text=True, check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"CircleCI API query failed ({result.returncode}): {result.stderr or result.stdout}")
    return json.loads(result.stdout)


def binding_errors(definitions: object, triggers: object, settings: object) -> list[str]:
    if not isinstance(definitions, list) or not definitions:
        return ["no CircleCI pipeline definitions returned"]
    if not isinstance(triggers, dict) or not isinstance(triggers.get("data"), list):
        return ["invalid CircleCI v3 trigger response"]
    meta = triggers.get("meta")
    if not isinstance(meta, dict) or meta.get("total_count") != len(triggers["data"]):
        return ["CircleCI trigger response is incomplete"]
    errors: list[str] = []
    by_id: dict[str, dict] = {}
    for definition in definitions:
        if not isinstance(definition, dict) or not isinstance(definition.get("id"), str):
            return ["invalid CircleCI pipeline definition response"]
        by_id[definition["id"]] = definition
    seen: list[tuple] = []
    for trigger in triggers["data"]:
        if not isinstance(trigger, dict):
            return ["invalid CircleCI trigger entity"]
        attrs = trigger.get("attributes")
        refs = trigger.get("references")
        if not isinstance(attrs, dict) or not isinstance(refs, dict):
            return ["trigger is missing attributes or references"]
        if attrs.get("is_disabled") is True:
            continue
        project = refs.get("project")
        if not isinstance(project, dict) or project.get("id") != PROJECT_ID:
            return ["enabled trigger belongs to the wrong project"]
        pipeline = refs.get("pipeline")
        if not isinstance(pipeline, dict) or pipeline.get("id") not in by_id:
            return ["enabled trigger has no known pipeline"]
        definition = by_id[pipeline["id"]]
        config = definition.get("config_source")
        checkout = definition.get("checkout_source")
        if not isinstance(config, dict) or not isinstance(checkout, dict):
            return ["pipeline definition is missing source metadata"]
        config_repo = config.get("repo")
        checkout_repo = checkout.get("repo")
        if not isinstance(config_repo, dict) or not isinstance(checkout_repo, dict):
            return ["pipeline definition is missing repository metadata"]
        event = attrs.get("event")
        if not isinstance(event, dict) or not isinstance(event.get("vcs"), dict):
            return ["enabled trigger has no VCS source"]
        vcs = event["vcs"]
        if (
            config.get("file_path") != EXPECTED_CONFIG
            or config.get("provider") != "github_app"
            or config_repo.get("external_id") != EXPECTED_REPO_ID
            or checkout_repo.get("external_id") != EXPECTED_REPO_ID
            or config_repo.get("full_name") != EXPECTED_REPO
            or checkout_repo.get("full_name") != EXPECTED_REPO
            or event.get("type") != "vcs"
            or vcs.get("provider") != "github_app"
            or vcs.get("repo_id") != EXPECTED_REPO_ID
            or vcs.get("repo_full_name") != EXPECTED_REPO
        ):
            errors.append(f"enabled trigger {trigger.get('id')} uses the wrong full-gate config or repo")
        filters = event.get("filter")
        rules = filters.get("rules") if isinstance(filters, dict) else None
        if not isinstance(rules, list):
            return ["enabled trigger has no event rules"]
        seen.append(tuple(
            (rule.get("event_type"), rule.get("rule"), rule.get("event_ref"))
            for rule in rules if isinstance(rule, dict)
        ))
    if len(seen) != len(EXPECTED_RULES) or set(seen) != EXPECTED_RULES:
        errors.append(f"expected only main-push and PR-open/update rules, found {seen}")
    if not isinstance(settings, dict) or settings.get("enable_auto_cancel_redundant_workflows") is not True:
        errors.append("auto-cancel redundant workflows is disabled or unavailable")
    return errors


def main() -> int:
    try:
        definitions = circleci("pipeline", "list", "--project-id", PROJECT_ID)
        if not isinstance(definitions, list):
            raise RuntimeError("CircleCI pipeline list is not an array")
        triggers = circleci_api(f"triggers?filter[project_id]={PROJECT_ID}")
        settings = circleci("project", "setting", "list", "--project", PROJECT_SLUG)
        errors = binding_errors(definitions, triggers, settings)
    except (OSError, ValueError, KeyError, TypeError, RuntimeError) as error:
        errors = [str(error)]
    for error in errors:
        print(f"hosted CI binding: FAIL: {error}", file=sys.stderr)
    if errors:
        return 1
    print("hosted CI binding: OK (main push, PR open/update, auto-cancel)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
