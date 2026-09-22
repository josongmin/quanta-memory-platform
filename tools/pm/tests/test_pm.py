"""Hermetic tests for the standard-library prompt manager."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import pm  # noqa: E402


class PromptManagerTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="qmp-pm-")
        root = Path(self.temp.name)
        self.pm_dir = root / "pm"
        self.repo = root / "repo"
        (self.pm_dir / "sources").mkdir(parents=True)
        self.repo.mkdir()
        self.targets_path = self.pm_dir / "targets.json"
        self.targets_path.write_text(
            json.dumps(
                {
                    "targets": {
                        "demo": {
                            "output": "out/DEMO.md",
                            "title": "Demo",
                            "sections": ["sources/a.md", "sources/b.md"],
                        }
                    }
                }
            ),
            encoding="utf-8",
        )
        (self.pm_dir / "sources/a.md").write_text("alpha", encoding="utf-8")
        (self.pm_dir / "sources/b.md").write_text("beta", encoding="utf-8")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def target(self) -> pm.Target:
        return pm.load_targets(self.targets_path)["demo"]

    def test_load_and_render(self) -> None:
        target = self.target()
        self.assertEqual(target.output, "out/DEMO.md")
        rendered = pm.render_target(target, self.pm_dir)
        self.assertIn("DO NOT EDIT", rendered)
        self.assertIn("# Demo", rendered)
        self.assertIn("alpha\n\nbeta", rendered)

    def test_missing_source_is_rejected(self) -> None:
        target = self.target()
        (self.pm_dir / "sources/a.md").unlink()
        with self.assertRaisesRegex(pm.PmError, "missing source file"):
            pm.render_target(target, self.pm_dir)

    def test_orphan_source_is_rejected(self) -> None:
        (self.pm_dir / "sources/orphan.md").write_text("orphan", encoding="utf-8")
        with self.assertRaisesRegex(pm.PmError, "orphaned source"):
            pm.load_targets(self.targets_path)

    def test_duplicate_output_is_rejected(self) -> None:
        config = json.loads(self.targets_path.read_text(encoding="utf-8"))
        config["targets"]["duplicate"] = {
            "output": "out/DEMO.md",
            "title": "Duplicate",
            "sections": ["sources/a.md"],
        }
        self.targets_path.write_text(json.dumps(config), encoding="utf-8")
        with self.assertRaisesRegex(pm.PmError, "duplicate generated output"):
            pm.load_targets(self.targets_path)

    def test_path_escape_is_rejected(self) -> None:
        config = json.loads(self.targets_path.read_text(encoding="utf-8"))
        config["targets"]["demo"]["output"] = "../outside.md"
        self.targets_path.write_text(json.dumps(config), encoding="utf-8")
        with self.assertRaisesRegex(pm.PmError, "escapes its root"):
            pm.load_targets(self.targets_path)

    def test_drift_states(self) -> None:
        target = self.target()
        self.assertEqual(pm.check_drift(target, self.pm_dir, self.repo), "missing")
        pm.sync_target(target, self.pm_dir, self.repo)
        self.assertEqual(pm.check_drift(target, self.pm_dir, self.repo), "ok")
        pm.output_path(target, self.repo).write_text("tampered\n", encoding="utf-8")
        self.assertEqual(pm.check_drift(target, self.pm_dir, self.repo), "drift")

    def test_lint_fails_closed_on_drift(self) -> None:
        parser = pm.build_parser()
        base = [
            "--targets",
            str(self.targets_path),
            "--pm-dir",
            str(self.pm_dir),
            "--repo-root",
            str(self.repo),
        ]
        lint_args = parser.parse_args(base + ["lint"])
        with redirect_stdout(StringIO()), redirect_stderr(StringIO()):
            self.assertEqual(lint_args.func(lint_args), 1)
        sync_args = parser.parse_args(base + ["sync"])
        with redirect_stdout(StringIO()):
            self.assertEqual(sync_args.func(sync_args), 0)
            self.assertEqual(lint_args.func(lint_args), 0)

    def test_preview_is_read_only(self) -> None:
        args = pm.build_parser().parse_args(
            [
                "--targets",
                str(self.targets_path),
                "--pm-dir",
                str(self.pm_dir),
                "--repo-root",
                str(self.repo),
                "preview",
                "--target",
                "demo",
            ]
        )
        output = StringIO()
        with redirect_stdout(output):
            self.assertEqual(args.func(args), 0)
        self.assertIn("alpha", output.getvalue())
        self.assertFalse(pm.output_path(self.target(), self.repo).exists())


if __name__ == "__main__":
    unittest.main()
