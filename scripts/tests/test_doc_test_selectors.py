import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "verify-doc-test-selectors.py"
SPEC = importlib.util.spec_from_file_location("verify_doc_test_selectors", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class DocTestSelectorTests(unittest.TestCase):
    def test_candidate_parser_keeps_test_like_identifiers_only(self) -> None:
        with tempfile.TemporaryDirectory(dir=MODULE.ROOT) as directory:
            document = Path(directory) / "doc.md"
            document.write_text(
                "`exact_test_selector` `two_parts` `TypeName` `feature-name`\n",
                encoding="utf-8",
            )
            self.assertEqual(
                MODULE.documented_candidates((document,)),
                {"exact_test_selector": {document.relative_to(MODULE.ROOT).as_posix()}},
            )

    def test_cargo_list_parser_returns_leaf_selector(self) -> None:
        output = (
            "module::exact_test_selector: test\n"
            "module::ignored_benchmark: benchmark\n"
        )
        self.assertEqual(MODULE.parse_listed_tests(output), {"exact_test_selector"})

    def test_cargo_execution_parser_reuses_passed_and_ignored_test_names(self) -> None:
        output = (
            "running 3 tests\n"
            "test module::exact_test_selector ... ok\n"
            "test module::ignored_test_selector ... ignored\n"
            "test module::failed_test_selector ... FAILED\n"
            "test result: FAILED. 1 passed; 1 failed; 1 ignored\n"
        )
        self.assertEqual(
            MODULE.parse_executed_tests(output),
            {
                "exact_test_selector",
                "failed_test_selector",
                "ignored_test_selector",
            },
        )

    def test_cargo_execution_parser_handles_interleaved_test_lines(self) -> None:
        output = (
            "test module::first_selector ... test module::second_selector ... ok\n"
            "\nok\n"
            "test result: ok. 2 passed; 0 failed; 0 ignored\n"
        )
        self.assertEqual(
            MODULE.parse_executed_tests(output),
            {"first_selector", "second_selector"},
        )

    def test_cargo_execution_parser_ignores_unstructured_prose(self) -> None:
        output = (
            "warning: someone wrote test fake_selector ... ok in a log\n"
            "test module::real_selector ... ok\n"
        )
        self.assertEqual(MODULE.parse_executed_tests(output), {"real_selector"})


if __name__ == "__main__":
    unittest.main()
