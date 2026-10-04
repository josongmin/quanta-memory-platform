from __future__ import annotations

import contextlib
import builtins
import importlib.util
import io
import tempfile
import sys
import unittest
from pathlib import Path
from unittest import mock


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

    def test_nested_package_contract_is_executed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            nested = root / "nested"
            nested.mkdir()
            (nested / "__init__.py").write_text("", encoding="utf-8")
            (nested / "test_nested.py").write_text(
                "import unittest\nclass Contracts(unittest.TestCase):\n"
                " def test_broken(self): self.fail('nested regression')\n",
                encoding="utf-8",
            )
            try:
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertFalse(MODULE.run_suite(root))
            finally:
                sys.modules.pop("nested.test_nested", None)
                sys.modules.pop("nested", None)

    def test_undiscoverable_nested_contract_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "test_healthy.py").write_text(
                "import unittest\nclass Contracts(unittest.TestCase):\n"
                " def test_healthy(self): pass\n", encoding="utf-8",
            )
            nested = root / "nested"
            nested.mkdir()
            (nested / "test_hidden.py").write_text(
                "import unittest\nclass Contracts(unittest.TestCase):\n"
                " def test_broken(self): self.fail('undiscovered regression')\n", encoding="utf-8",
            )
            try:
                output = io.StringIO()
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(output):
                    self.assertFalse(MODULE.run_suite(root))
                self.assertIn("nested.test_hidden", output.getvalue())
            finally:
                sys.modules.pop("test_healthy", None)

    def test_spoofed_class_module_cannot_register_undiscovered_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "test_healthy.py").write_text(
                'import unittest\n'
                'class Healthy(unittest.TestCase):\n def test_ok(self): pass\n'
                'class Spoofed(unittest.TestCase):\n'
                ' __module__ = "nested.test_hidden"\n def test_ok(self): pass\n',
                encoding="utf-8",
            )
            nested = root / "nested"
            nested.mkdir()
            (nested / "test_hidden.py").write_text(
                'import unittest\nclass Broken(unittest.TestCase):\n'
                ' def test_broken(self): self.fail("hidden")\n', encoding="utf-8",
            )
            try:
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertFalse(MODULE.run_suite(root))
            finally:
                sys.modules.pop("test_healthy", None)

    def test_imported_class_with_spoofed_module_is_not_a_local_contract(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "helper.py").write_text(
                'import unittest\nclass Imported(unittest.TestCase):\n'
                ' def test_ok(self): pass\n', encoding="utf-8",
            )
            (root / "test_alias.py").write_text(
                'from helper import Imported\nImported.__module__ = "test_alias"\n',
                encoding="utf-8",
            )
            try:
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertFalse(MODULE.run_suite(root))
            finally:
                sys.modules.pop("test_alias", None)
                sys.modules.pop("helper", None)

    def test_imported_class_cannot_replace_a_declared_local_class(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "helper.py").write_text(
                'import unittest\nclass Imported(unittest.TestCase):\n'
                ' def test_ok(self): pass\n', encoding="utf-8",
            )
            (root / "test_alias.py").write_text(
                'import unittest\nfrom helper import Imported\n'
                'class Local(unittest.TestCase): pass\n'
                'Local = Imported\ndel Imported\n'
                'Local.__module__ = "test_alias"\nLocal.__qualname__ = "Local"\n',
                encoding="utf-8",
            )
            try:
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertFalse(MODULE.run_suite(root))
            finally:
                sys.modules.pop("test_alias", None)
                sys.modules.pop("helper", None)

    def test_duplicate_test_identity_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "test_duplicate.py").write_text(
                'import unittest\nclass First(unittest.TestCase):\n'
                ' def test_ok(self): pass\n'
                'class Second(unittest.TestCase):\n'
                ' def test_ok(self): pass\n'
                'Second.__qualname__ = "First"\n',
                encoding="utf-8",
            )
            try:
                output = io.StringIO()
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(output):
                    self.assertFalse(MODULE.run_suite(root))
                self.assertIn("duplicate test identities", output.getvalue())
            finally:
                sys.modules.pop("test_duplicate", None)

    def test_load_tests_cannot_hide_a_declared_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "test_hidden.py").write_text(
                'import unittest\nclass Contracts(unittest.TestCase):\n'
                ' def test_healthy(self): pass\n'
                ' def test_broken(self): self.fail("omitted regression")\n'
                'def load_tests(loader, tests, pattern):\n'
                ' return unittest.TestSuite([Contracts("test_healthy")])\n',
                encoding="utf-8",
            )
            try:
                output = io.StringIO()
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(output):
                    self.assertFalse(MODULE.run_suite(root))
                self.assertIn("custom load_tests is forbidden", output.getvalue())
            finally:
                sys.modules.pop("test_hidden", None)

    def test_local_subclass_inheriting_contract_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "helper.py").write_text(
                'import unittest\nclass Base(unittest.TestCase):\n'
                ' def test_inherited(self): pass\n', encoding="utf-8",
            )
            (root / "test_child.py").write_text(
                'from helper import Base\nclass Child(Base): pass\n', encoding="utf-8",
            )
            try:
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertTrue(MODULE.run_suite(root))
            finally:
                sys.modules.pop("test_child", None)
                sys.modules.pop("helper", None)

    def test_class_origin_hook_is_restored_after_discovery_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "test_error.py").write_text("# fixture\n", encoding="utf-8")
            original = builtins.__build_class__
            with mock.patch.object(MODULE.unittest.TestLoader, "discover", side_effect=RuntimeError("boom")):
                with self.assertRaisesRegex(RuntimeError, "boom"):
                    MODULE.run_suite(root)
            self.assertIs(builtins.__build_class__, original)


if __name__ == "__main__":
    unittest.main()
