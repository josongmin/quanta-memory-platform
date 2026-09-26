from __future__ import annotations

import importlib.util
import json
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


def policy_errors_for(root: Path) -> list[str]:
    result = subprocess.run(
        ["cargo", "metadata", "--offline", "--no-deps", "--format-version", "1"],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    )
    return checker.lint_policy_errors(root, json.loads(result.stdout))


class WorkspaceLintPolicyTest(unittest.TestCase):
    def test_toml_string_cannot_spoof_workspace_lint_table(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-lint-string-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\n'
                '[workspace.metadata]\ndescription = """\n[workspace.lints.rust]\nunsafe_code = "deny"\n"""\n'
                '[workspace.lints.rust]\nunsafe_code = "allow"\n', encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\n[lints]\nworkspace = true\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text("#![forbid(unsafe_code)]\n", encoding="utf-8")
            self.assertIn("workspace.lints.rust.unsafe_code must be `deny`", policy_errors_for(root))

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
                policy_errors_for(root),
                ["workspace member does not inherit lints: member"],
            )

    def test_library_root_without_non_overridable_forbid_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-missing-forbid-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
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
            (root / "member/src/lib.rs").write_text("pub fn safe() {}\n", encoding="utf-8")
            self.assertEqual(
                policy_errors_for(root),
                ["library root does not forbid unsafe code: member"],
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
            (root / "member/tests/conditional.rs").write_text(
                "#![cfg_attr(all(), allow(unused, unsafe_code))]\n",
                encoding="utf-8",
            )
            self.assertCountEqual(
                policy_errors_for(root),
                [
                    "unauthorized unsafe-code exception: member/tests/rogue.rs",
                    "unauthorized unsafe-code exception: member/tests/conditional.rs",
                ],
            )

    def test_single_quoted_cargo_members_are_checked(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-single-quote-members-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                "[workspace]\nmembers = ['member']\n\n"
                '[workspace.lints.rust]\nunsafe_code = "deny"\n',
                encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text("pub fn safe() {}\n", encoding="utf-8")
            self.assertCountEqual(
                policy_errors_for(root),
                [
                    "workspace member does not inherit lints: member",
                    "library root does not forbid unsafe code: member",
                ],
            )


if __name__ == "__main__":
    unittest.main()
