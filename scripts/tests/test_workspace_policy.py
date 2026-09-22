from __future__ import annotations

import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts/verify-workspace-policy.py"
SPEC = importlib.util.spec_from_file_location("verify_workspace_policy", CHECKER)
assert SPEC and SPEC.loader
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


class WorkspaceLintPolicyTest(unittest.TestCase):
    def test_live_workspace_forbids_product_unsafe(self) -> None:
        self.assertEqual(checker.lint_policy_errors(ROOT), [])

    def test_new_non_inheriting_member_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-lints-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\n\n'
                '[workspace.lints.rust]\nunsafe_code = "deny"\n',
                encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text(
                "#![forbid(unsafe_code)]\n", encoding="utf-8"
            )
            self.assertEqual(
                checker.lint_policy_errors(root),
                ["workspace member does not inherit lints: member"],
            )

    def test_inherited_policy_rejects_unsafe_code(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-unsafe-lint-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\nresolver = "2"\n\n'
                '[workspace.lints.rust]\nunsafe_code = "deny"\n',
                encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\nedition = "2021"\n\n'
                '[lints]\nworkspace = true\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text(
                "#![forbid(unsafe_code)]\npub unsafe fn forbidden() {}\n",
                encoding="utf-8",
            )
            result = subprocess.run(
                ["cargo", "check", "--offline", "-q"],
                cwd=root,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("declaration of an `unsafe` function", result.stderr)

    def test_unapproved_unsafe_exception_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-unsafe-exception-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "member/tests").mkdir()
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\n\n'
                '[workspace.lints.rust]\nunsafe_code = "deny"\n',
                encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\n\n'
                '[lints]\nworkspace = true\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text(
                "#![forbid(unsafe_code)]\n", encoding="utf-8"
            )
            (root / "member/tests/rogue.rs").write_text(
                "#![allow(unsafe_code)]\n", encoding="utf-8"
            )
            self.assertEqual(
                checker.lint_policy_errors(root),
                ["unauthorized unsafe-code exception: member/tests/rogue.rs"],
            )


if __name__ == "__main__":
    unittest.main()
