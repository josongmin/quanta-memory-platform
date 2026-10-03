from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "verify-hosted-ci-binding.py"
SPEC = importlib.util.spec_from_file_location("verify_hosted_ci_binding", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def trigger(rule: tuple[str, str, str]) -> dict:
    event_type, expression, ref = rule
    return {
        "id": event_type,
        "attributes": {
            "is_disabled": False,
            "event": {
                "type": "vcs",
                "vcs": {
                    "provider": "github_app",
                    "repo_id": MODULE.EXPECTED_REPO_ID,
                    "repo_full_name": MODULE.EXPECTED_REPO,
                },
                "filter": {"rules": [{
                    "event_type": event_type, "rule": expression, "event_ref": ref,
                }]},
            },
        },
        "references": {"pipeline": {"id": "pipeline"}, "project": {"id": MODULE.PROJECT_ID}},
    }


class HostedCiBindingTests(unittest.TestCase):
    def test_exact_main_and_pr_rules_and_auto_cancel(self) -> None:
        definition = {
            "id": "pipeline",
            "config_source": {"provider": "github_app", "repo": {
                "external_id": MODULE.EXPECTED_REPO_ID, "full_name": MODULE.EXPECTED_REPO,
            }, "file_path": MODULE.EXPECTED_CONFIG},
            "checkout_source": {"repo": {
                "external_id": MODULE.EXPECTED_REPO_ID, "full_name": MODULE.EXPECTED_REPO,
            }},
        }
        events = [trigger(rule[0]) for rule in sorted(MODULE.EXPECTED_RULES)]
        settings = {"enable_auto_cancel_redundant_workflows": True}
        response = {"data": events, "meta": {"total_count": len(events)}}
        self.assertEqual(MODULE.binding_errors([definition], response, settings), [])
        for altered in (
            events + [events[0]],
            events[:1],
            [{**events[0], "attributes": {**events[0]["attributes"],
                "event": {**events[0]["attributes"]["event"], "filter": {"rules": [
                    {"event_type": "push", "rule": "true", "event_ref": "push_ref"},
                ]}}}}, events[1]],
        ):
            with self.subTest(altered=altered):
                self.assertTrue(MODULE.binding_errors([definition], {
                    "data": altered, "meta": {"total_count": len(altered)},
                }, settings))
        self.assertTrue(MODULE.binding_errors([definition], response, {
            "enable_auto_cancel_redundant_workflows": False,
        }))
        definition["config_source"]["file_path"] = ".github/workflows/ci.yml"
        self.assertTrue(MODULE.binding_errors([definition], response, settings))


if __name__ == "__main__":
    unittest.main()
