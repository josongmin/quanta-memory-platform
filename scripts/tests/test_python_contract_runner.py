from __future__ import annotations

import contextlib
import importlib.util
import io
import tempfile
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


if __name__ == "__main__":
    unittest.main()
