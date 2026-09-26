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
    def test_wrong_config_and_duplicate_push_triggers_fail(self) -> None:
        definition = {
            "id": "pipeline",
            "config_source": {"provider": "github_app", "repo": {"external_id": MODULE.EXPECTED_REPO_ID},
                              "file_path": MODULE.EXPECTED_CONFIG},
            "checkout_source": {"repo": {"external_id": MODULE.EXPECTED_REPO_ID}},
        }
        trigger = {"id": "push", "disabled": False, "event_preset": "all-pushes",
                   "repo_external_id": MODULE.EXPECTED_REPO_ID}
        self.assertEqual(MODULE.binding_errors([definition], {"pipeline": [trigger]}), [])
        definition["config_source"]["file_path"] = ".github/workflows/ci.yml"
        self.assertTrue(MODULE.binding_errors([definition], {"pipeline": [trigger]}))
        definition["config_source"]["file_path"] = MODULE.EXPECTED_CONFIG
        self.assertTrue(MODULE.binding_errors([definition], {"pipeline": [trigger, trigger]}))
        definition["config_source"]["file_path"] = ".github/workflows/ci.yml"
        other_push = {**trigger, "event_preset": "default-branch-pushes"}
        self.assertTrue(MODULE.binding_errors([definition], {"pipeline": [other_push]}))


if __name__ == "__main__":
    unittest.main()
