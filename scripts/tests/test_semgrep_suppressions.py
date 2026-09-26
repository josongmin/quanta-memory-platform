from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "lib/verify_semgrep_suppressions.py"
SPEC = importlib.util.spec_from_file_location("verify_semgrep_suppressions", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class SemgrepSuppressionTests(unittest.TestCase):
    def test_bare_unknown_and_unexplained_suppressions_fail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "crates/core/src/lib.rs"
            source.parent.mkdir(parents=True)
            rules = root / "rules.yml"
            rules.write_text("rules:\n  - id: qmp-test-rule\n", encoding="utf-8")
            for suffix in ("", ": qmp-test-rule", ": qmp-unknown -- reason: test"):
                with self.subTest(suffix=suffix):
                    source.write_text(f"let x = 1; // nosemgrep{suffix}\n", encoding="utf-8")
                    self.assertEqual(len(MODULE.errors_for(root, rules)), 1)
            source.write_text(
                "let x = 1; // nosemgrep: qmp-test-rule -- reason: bounded conversion\n",
                encoding="utf-8",
            )
            self.assertEqual(MODULE.errors_for(root, rules), [])

    def test_engine_recognized_alternative_suppressions_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "crates/core/src/lib.rs"
            source.parent.mkdir(parents=True)
            rules = root / "rules.yml"
            rules.write_text("rules:\n  - id: qmp-test-rule\n", encoding="utf-8")
            for comment in (
                "/* nosemgrep */", "// NOSEMGREP", "// nosem",
                "// nosem: qmp-test-rule -- reason: bypass",
                "// NOSEMGREP: qmp-test-rule -- reason: bypass",
                "/* nosemgrep: qmp-test-rule -- reason: bypass */",
            ):
                with self.subTest(comment=comment):
                    source.write_text(f"let x = value as i64; {comment}\n", encoding="utf-8")
                    self.assertEqual(len(MODULE.errors_for(root, rules)), 1)


if __name__ == "__main__":
    unittest.main()
