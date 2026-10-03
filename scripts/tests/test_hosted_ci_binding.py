from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "verify-hosted-ci-binding.py"
SPEC = importlib.util.spec_from_file_location("verify_hosted_ci_binding", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class HostedCiBindingTests(unittest.TestCase):
    def test_exact_main_and_pr_triggers_and_auto_cancel(self) -> None:
        definition = {
            "id": "pipeline",
            "config_source": {"provider": "github_app", "repo": {
                "external_id": MODULE.EXPECTED_REPO_ID, "full_name": MODULE.EXPECTED_REPO,
            }, "file_path": MODULE.EXPECTED_CONFIG},
            "checkout_source": {"repo": {
                "external_id": MODULE.EXPECTED_REPO_ID, "full_name": MODULE.EXPECTED_REPO,
            }},
        }
        events = [
            {"id": preset, "disabled": False, "event_preset": preset,
             "repo_external_id": MODULE.EXPECTED_REPO_ID,
             "repo_full_name": MODULE.EXPECTED_REPO}
            for preset in sorted(MODULE.REQUIRED_PRESETS)
        ]
        settings = {"enable_auto_cancel_redundant_workflows": True}
        self.assertEqual(MODULE.binding_errors([definition], {"pipeline": events}, settings), [])
        for altered in (
            events + [events[0]],
            events[:2],
            [{**events[0], "event_preset": "all-pushes"}, *events[1:]],
            [{**events[0], "repo_external_id": "wrong"}, *events[1:]],
        ):
            self.assertTrue(MODULE.binding_errors([definition], {"pipeline": altered}, settings))
        self.assertTrue(MODULE.binding_errors([definition], {"pipeline": events}, {
            "enable_auto_cancel_redundant_workflows": False,
        }))
        definition["config_source"]["file_path"] = ".github/workflows/ci.yml"
        self.assertTrue(MODULE.binding_errors([definition], {"pipeline": events}, settings))


if __name__ == "__main__":
    unittest.main()
