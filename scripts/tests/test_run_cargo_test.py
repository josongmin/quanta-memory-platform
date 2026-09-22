from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "lib/run_cargo_test.py"
SPEC = importlib.util.spec_from_file_location("run_cargo_test", SCRIPT)
assert SPEC and SPEC.loader
module = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = module
SPEC.loader.exec_module(module)


class CargoTestOutputTest(unittest.TestCase):
    def test_sums_multiple_result_lines(self) -> None:
        output = "\n".join(
            [
                "test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured;",
                "test result: ok. 3 passed; 0 failed; 0 ignored; 1 measured;",
            ]
        )
        counts = module.parse_cargo_test_counts(output)
        self.assertEqual(counts.selected, 5)
        self.assertEqual(counts.passed, 5)
        self.assertEqual(counts.result_lines, 2)

    def test_zero_selected_is_rejected(self) -> None:
        output = "test result: ok. 0 passed; 0 failed; 2 ignored; 0 measured;"
        with self.assertRaisesRegex(module.CargoTestEvidenceError, "zero executable"):
            module.parse_cargo_test_counts(output)

    def test_ignored_only_is_rejected(self) -> None:
        output = "test result: ok. 0 passed; 0 failed; 9 ignored; 1 measured;"
        with self.assertRaises(module.CargoTestEvidenceError):
            module.parse_cargo_test_counts(output)

    def test_failed_test_is_counted_as_selected(self) -> None:
        output = "test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured;"
        counts = module.parse_cargo_test_counts(output)
        self.assertEqual(counts.selected, 3)
        self.assertEqual(counts.failed, 2)

    def test_malformed_output_is_rejected(self) -> None:
        with self.assertRaisesRegex(module.CargoTestEvidenceError, "no test result"):
            module.parse_cargo_test_counts("cargo emitted a new format")


if __name__ == "__main__":
    unittest.main()
