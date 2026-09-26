"""Execute real Semgrep rules against bad and permitted Rust owner shapes.

Fixtures are copied to a repository-shaped root: path-scoped rules must actually
fire there, rather than merely parsing successfully or scanning zero files.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CONFIG = ROOT / "tools/semgrep/qmp-regressions.yml"
FIXTURES = Path(__file__).parent / "fixtures/semgrep"


class RegressionRuleContracts(unittest.TestCase):
    def test_every_rule_rejects_its_regression_and_accepts_permitted_shapes(self) -> None:
        self.assertIsNotNone(shutil.which("semgrep"), "Semgrep is required; do not skip rule contracts")
        ids = set(re.findall(r"(?m)^  - id: (qmp-[\w-]+)$", CONFIG.read_text()))
        expected: list[tuple[str, str, int]] = []
        sources = sorted(FIXTURES.rglob("*.rs"))
        self.assertTrue(sources, "rule fixture inventory must not be empty")
        for source in sources:
            relative = source.relative_to(FIXTURES).as_posix()
            for line, text in enumerate(source.read_text().splitlines(), 1):
                marker = re.fullmatch(r"\s*// ruleid: (qmp-[\w-]+)\s*", text)
                if marker:
                    expected.append((marker.group(1), relative, line + 1))
        self.assertEqual({rule for rule, _, _ in expected}, ids, "every configured rule needs a rejecting fixture")
        with tempfile.TemporaryDirectory(prefix="qmp-rule-contract-") as directory:
            root = Path(directory)
            shutil.copytree(FIXTURES / "crates", root / "crates")
            result = subprocess.run(
                ["semgrep", "--config", str(CONFIG), "--strict", "--metrics=off",
                 "--disable-version-check", "--jobs=1", "--timeout=30", "--json", "."],
                cwd=root, capture_output=True, text=True, timeout=180, check=False,
            )
            scan = json.loads(result.stdout)
            self.assertEqual(scan.get("errors"), [], result.stderr)
            self.assertEqual(result.returncode, 0, result.stderr)
            scanned = set(scan["paths"]["scanned"])
            self.assertEqual(scanned, {source.relative_to(FIXTURES).as_posix() for source in sources})
            findings = [
                (finding["check_id"].split(".")[-1], finding["path"],
                 finding["start"]["line"], finding["end"]["line"])
                for finding in scan["results"]
            ]
            for rule, path, line in expected:
                self.assertTrue(
                    any(rule == found_rule and path == found_path and start <= line <= end
                        for found_rule, found_path, start, end in findings),
                    f"regression escaped: {rule} {path}:{line}; findings={findings}",
                )
            for rule, path, start, end in findings:
                self.assertTrue(
                    any(rule == expected_rule and path == expected_path and start <= line <= end
                        for expected_rule, expected_path, line in expected),
                    f"permitted shape rejected: {rule} {path}:{start}-{end}",
                )


if __name__ == "__main__":
    unittest.main()
