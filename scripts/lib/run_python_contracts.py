#!/usr/bin/env python3
"""Run every discovered Python contract and reject incomplete execution."""

from __future__ import annotations

import ast
import builtins
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def cases(suite: unittest.TestSuite) -> list[unittest.TestCase]:
    return [case for item in suite for case in (cases(item) if isinstance(item, unittest.TestSuite) else [item])]


class ContractLoader(unittest.TestLoader):
    def loadTestsFromModule(self, module: object, *args: object, **kwargs: object) -> unittest.TestSuite:
        # unittest calls load_tests after ordinary discovery. A hook can return
        # only the healthy cases while silently omitting a declared regression.
        if "load_tests" in vars(module):
            raise ValueError(f"custom load_tests is forbidden: {getattr(module, '__name__', module)}")
        return super().loadTestsFromModule(module, *args, **kwargs)


def run_suite(directory: Path, pattern: str = "test_*.py") -> bool:
    files = {
        ".".join(path.relative_to(directory).with_suffix("").parts): path
        for path in directory.rglob(pattern)
    }
    if not files:
        print(f"Python contract suite has no modules: {directory}", file=sys.stderr)
        return False
    preloaded = sorted(name for name in files if name in sys.modules)
    if preloaded:
        print(f"Python contract modules were loaded before discovery: {', '.join(preloaded)}", file=sys.stderr)
        return False
    # __module__ and __qualname__ are mutable. Capture each class body's code
    # origin while discovery imports candidates, before tests can relabel it.
    class_origins: dict[type, Path] = {}
    original_build_class = builtins.__build_class__

    def record_class(body: object, name: str, *bases: type, **keywords: object) -> object:
        result = original_build_class(body, name, *bases, **keywords)
        code = getattr(body, "__code__", None)
        if isinstance(result, type) and code is not None:
            class_origins[result] = Path(code.co_filename).resolve()
        return result

    builtins.__build_class__ = record_class
    try:
        try:
            suite = ContractLoader().discover(str(directory), pattern=pattern)
        except ValueError as error:
            print(f"Python contract discovery rejected: {error}", file=sys.stderr)
            return False
    finally:
        builtins.__build_class__ = original_build_class
    found = cases(suite)
    missing = []
    for name, path in sorted(files.items()):
        module = sys.modules.get(name)
        origin = getattr(module, "__file__", None)
        if not origin or Path(origin).resolve() != path.resolve():
            missing.append(name)
            continue
        # A class imported into another test module is not that module's own
        # contract. A local subclass with inherited tests still counts: its
        # declaration belongs to this file even when the method code does not.
        try:
            declarations = {
                node.name for node in ast.parse(path.read_text(encoding="utf-8")).body
                if isinstance(node, ast.ClassDef)
            }
        except (OSError, UnicodeError, SyntaxError):
            missing.append(name)
            continue
        registered = {
            value for value in vars(module).values()
            if isinstance(value, type) and value.__module__ == name
            and value.__qualname__ in declarations
            and class_origins.get(value) == path.resolve()
        }
        if not any(case.__class__ in registered for case in found):
            missing.append(name)
    if missing or not found:
        print(f"Python contract modules have no tests: {', '.join(sorted(missing))}", file=sys.stderr)
        return False
    identities = [case.id() for case in found]
    if len(identities) != len(set(identities)):
        print("Python contracts contain duplicate test identities", file=sys.stderr)
        return False
    print(f"Python contracts: {len(files)} modules, {len(found)} tests", flush=True)
    result = unittest.TextTestRunner(verbosity=1).run(suite)
    if result.skipped or result.expectedFailures or result.testsRun != len(found):
        print(
            f"Python contracts incomplete: selected={len(found)} executed={result.testsRun} "
            f"skipped={len(result.skipped)} expected_failures={len(result.expectedFailures)}",
            file=sys.stderr,
        )
        return False
    return result.wasSuccessful()


def main() -> int:
    if len(sys.argv) != 2 or sys.argv[1] not in {"scripts", "pm"}:
        print("usage: run_python_contracts.py [scripts|pm]", file=sys.stderr)
        return 2
    directory = ROOT / ("scripts/tests" if sys.argv[1] == "scripts" else "tools/pm/tests")
    return 0 if run_suite(directory) else 1


if __name__ == "__main__":
    raise SystemExit(main())
