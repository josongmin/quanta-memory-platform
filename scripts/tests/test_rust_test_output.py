from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "lib/verify_rust_test_output.py"
SPEC = importlib.util.spec_from_file_location("verify_rust_test_output", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class RustTestOutputTests(unittest.TestCase):
    def test_nonempty_test_execution_is_required(self) -> None:
        line = "test result: ok. {} passed; 0 failed; {} ignored; 0 measured; 0 filtered out\n"
        self.assertEqual(MODULE.counts(line.format(2, 0)), (2, 1))
        for output in ("", line.format(0, 0), line.format(0, 1), line.format(2, 1)):
            with self.subTest(output=output), self.assertRaises(ValueError):
                MODULE.counts(output)


if __name__ == "__main__":
    unittest.main()
