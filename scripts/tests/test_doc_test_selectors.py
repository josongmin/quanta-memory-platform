import importlib.util
import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "verify-doc-test-selectors.py"
SPEC = importlib.util.spec_from_file_location("verify_doc_test_selectors", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
REAL_OWNER_CONTRACT_ERRORS = MODULE.owner_contract_errors


class DocTestSelectorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.owner_patch = patch.object(MODULE, "owner_contract_errors", return_value=[])
        self.owner_patch.start()

    def tearDown(self) -> None:
        self.owner_patch.stop()

    def test_owner_inventory_rejects_wrong_module_replacement(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            core = root / "crates/core/src/domain/ledger.rs"
            facade = root / "crates/facade/src/ledger.rs"
            core.parent.mkdir(parents=True)
            facade.parent.mkdir(parents=True)
            core.write_text("#[test]\nfn exact_test_selector() {}\n", encoding="utf-8")
            facade.write_text("#[test]\nfn exact_test_selector() {}\n", encoding="utf-8")
            (root / "scripts/test-selector-owners.json").write_text(
                json.dumps({"exact_test_selector": "crates/core/src/domain/ledger.rs"}),
                encoding="utf-8",
            )
            candidates = {"exact_test_selector": {"doc.md"}}
            self.assertEqual(REAL_OWNER_CONTRACT_ERRORS(candidates, root), [])
            core.write_text("// test removed\n", encoding="utf-8")
            self.assertTrue(REAL_OWNER_CONTRACT_ERRORS(candidates, root))

    def test_current_owner_inventory_matches_documented_tests(self) -> None:
        self.assertEqual(REAL_OWNER_CONTRACT_ERRORS(MODULE.documented_candidates()), [])

    def test_zero_documented_or_registered_tests_is_rejected(self) -> None:
        with (
            patch.object(MODULE, "documented_candidates", return_value={}),
            patch.object(MODULE, "listed_tests", return_value=set()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            self.assertEqual(MODULE.main([]), 1)

    def test_ambiguous_leaf_cannot_be_satisfied_by_wrong_owner(self) -> None:
        with tempfile.TemporaryDirectory(dir=MODULE.ROOT) as directory:
            output = Path(directory) / "cargo.log"
            output.write_text("test retained_map::tests::same_counter_test ... ok\n", encoding="utf-8")
            with (
                patch.object(MODULE, "documented_candidates", return_value={"same_counter_test": {"doc.md"}}),
                patch.object(MODULE, "listed_tests", return_value={
                    "retained_map::tests::same_counter_test",
                    "payload_store::tests::same_counter_test",
                }),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                self.assertEqual(MODULE.main(["--test-output", str(output)]), 1)

    def test_candidate_parser_keeps_test_like_identifiers_only(self) -> None:
        with tempfile.TemporaryDirectory(dir=MODULE.ROOT) as directory:
            document = Path(directory) / "doc.md"
            document.write_text(
                "`exact_test_selector` `fixtures::tests::qualified_test_selector` "
                "`Widget::non_test_method` `two_parts` `TypeName` `feature-name`\n",
                encoding="utf-8",
            )
            self.assertEqual(
                MODULE.documented_candidates((document,)),
                {
                    "exact_test_selector": {document.relative_to(MODULE.ROOT).as_posix()},
                    "fixtures::tests::qualified_test_selector": {
                        document.relative_to(MODULE.ROOT).as_posix()
                    },
                },
            )

    def test_cargo_list_parser_preserves_full_selector(self) -> None:
        output = (
            "module::exact_test_selector: test\n"
            "module::ignored_benchmark: benchmark\n"
        )
        self.assertEqual(MODULE.parse_listed_tests(output), {"module::exact_test_selector"})

    def test_cargo_execution_parser_does_not_count_ignored_test_names(self) -> None:
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
                "module::exact_test_selector",
                "module::failed_test_selector",
            },
        )

    def test_ignored_interleaving_does_not_certify_ambiguous_names(self) -> None:
        output = (
            "test module::ambiguous_selector ... test module::ignored_selector ... ignored\n"
            "ok\n"
            "test result: ok. 1 passed; 0 failed; 1 ignored\n"
        )
        self.assertEqual(MODULE.parse_executed_tests(output), set())

    def test_cargo_execution_parser_handles_interleaved_test_lines(self) -> None:
        output = (
            "test module::first_selector ... test module::second_selector ... ok\n"
            "\nok\n"
            "test result: ok. 2 passed; 0 failed; 0 ignored\n"
        )
        self.assertEqual(
            MODULE.parse_executed_tests(output),
            {"module::first_selector", "module::second_selector"},
        )

    def test_cargo_execution_parser_ignores_unstructured_prose(self) -> None:
        output = (
            "warning: someone wrote test fake_selector ... ok in a log\n"
            "test module::real_selector ... ok\n"
        )
        self.assertEqual(MODULE.parse_executed_tests(output), {"module::real_selector"})

    def test_logged_marker_without_cargo_registration_is_not_a_selector(self) -> None:
        with tempfile.TemporaryDirectory(dir=MODULE.ROOT) as directory:
            output = Path(directory) / "cargo.log"
            output.write_text("test module::fake_test_selector ... ok\n", encoding="utf-8")
            with (
                patch.object(
                    MODULE, "documented_candidates",
                    return_value={"fake_test_selector": {"doc.md"}},
                ),
                patch.object(MODULE, "listed_tests", return_value={"module::real_test_selector"}),
                patch("sys.argv", ["verify-doc-test-selectors.py", "--test-output", str(output)]),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                self.assertEqual(MODULE.main(), 1)

    def test_logged_and_registered_selector_resolves(self) -> None:
        with tempfile.TemporaryDirectory(dir=MODULE.ROOT) as directory:
            output = Path(directory) / "cargo.log"
            output.write_text("test module::real_test_selector ... ok\n", encoding="utf-8")
            with (
                patch.object(
                    MODULE, "documented_candidates",
                    return_value={"real_test_selector": {"doc.md"}},
                ),
                patch.object(MODULE, "listed_tests", return_value={"module::real_test_selector"}),
                patch("sys.argv", ["verify-doc-test-selectors.py", "--test-output", str(output)]),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                self.assertEqual(MODULE.main(), 0)

    def test_qualified_selector_requires_the_exact_registered_and_executed_path(self) -> None:
        qualified = "fixtures::tests::qualified_test_selector"
        with tempfile.TemporaryDirectory(dir=MODULE.ROOT) as directory:
            output = Path(directory) / "cargo.log"
            output.write_text(
                "test other::tests::qualified_test_selector ... ok\n",
                encoding="utf-8",
            )
            with (
                patch.object(MODULE, "documented_candidates", return_value={qualified: {"doc.md"}}),
                patch.object(MODULE, "listed_tests", return_value={
                    qualified, "other::tests::qualified_test_selector",
                }),
                patch("sys.argv", ["verify-doc-test-selectors.py", "--test-output", str(output)]),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                self.assertEqual(MODULE.main(), 1)

            output.write_text(f"test {qualified} ... ok\n", encoding="utf-8")
            with (
                patch.object(MODULE, "documented_candidates", return_value={qualified: {"doc.md"}}),
                patch.object(MODULE, "listed_tests", return_value={qualified}),
                patch("sys.argv", ["verify-doc-test-selectors.py", "--test-output", str(output)]),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                self.assertEqual(MODULE.main(), 0)


if __name__ == "__main__":
    unittest.main()
