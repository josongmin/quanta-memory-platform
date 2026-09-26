from __future__ import annotations

import contextlib
import importlib.util
import io
import tempfile
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "lib/run_python_contracts.py"
SPEC = importlib.util.spec_from_file_location("run_python_contracts", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PythonContractRunnerTests(unittest.TestCase):
    def test_empty_and_testless_modules_fail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertFalse(MODULE.run_suite(root))
            (root / "test_missing.py").write_text("# no contracts\n", encoding="utf-8")
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertFalse(MODULE.run_suite(root))

    def test_disabled_contracts_cannot_turn_a_broken_suite_green(self) -> None:
        for decorator in ("unittest.skip('disabled')", "unittest.expectedFailure"):
            with self.subTest(decorator=decorator), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                name = "test_contract_" + root.name
                (root / f"{name}.py").write_text(
                    "import unittest\nclass Contracts(unittest.TestCase):\n"
                    " def test_healthy(self): pass\n"
                    f" @{decorator}\n"
                    " def test_broken(self): self.fail('regression')\n",
                    encoding="utf-8",
                )
                try:
                    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                        self.assertFalse(MODULE.run_suite(root))
                finally:
                    sys.modules.pop(name, None)

    def test_executed_passing_contract_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            name = "test_contract_" + root.name
            (root / f"{name}.py").write_text(
                "import unittest\nclass Contracts(unittest.TestCase):\n"
                " def test_healthy(self): pass\n", encoding="utf-8",
            )
            try:
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertTrue(MODULE.run_suite(root))
            finally:
                sys.modules.pop(name, None)


if __name__ == "__main__":
    unittest.main()
