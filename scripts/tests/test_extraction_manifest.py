import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest


HELPER = Path(__file__).resolve().parents[1] / "lib/extraction_manifest.py"


class ExtractionManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="qmp120-manifest-")
        self.root = Path(self.temp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "-C", str(self.root), "config", "user.name", "QMP Test"], check=True)
        subprocess.run(
            ["git", "-C", str(self.root), "config", "user.email", "qmp-test@local.invalid"],
            check=True,
        )
        (self.root / "scripts").mkdir()
        self.source = self.root / "scripts/check.sh"
        self.source.write_text("#!/bin/sh\necho first\n", encoding="utf-8")
        self.source.chmod(0o755)
        subprocess.run(["git", "-C", str(self.root), "add", "scripts/check.sh"], check=True)
        subprocess.run(["git", "-C", str(self.root), "commit", "-qm", "fixture"], check=True)

    def tearDown(self):
        self.temp.cleanup()

    def helper(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(HELPER), *args],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=check,
        )

    def snapshot(self, name: str = "manifest.json") -> tuple[Path, dict]:
        manifest = self.root / name
        self.helper(
            "snapshot",
            "--repo",
            str(self.root),
            "--revision",
            "HEAD",
            "--output",
            str(manifest),
            "scripts",
        )
        return manifest, json.loads(manifest.read_text(encoding="utf-8"))

    def assert_dirty_rejected(self) -> None:
        head = subprocess.check_output(
            ["git", "-C", str(self.root), "rev-parse", "HEAD"], text=True
        ).strip()
        result = self.helper(
            "assert-clean",
            "--repo",
            str(self.root),
            "--commit",
            head,
            "scripts",
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("differs from frozen HEAD", result.stderr)

    def test_dirty_tracked_content_is_rejected(self):
        self.source.write_text("#!/bin/sh\necho dirty\n", encoding="utf-8")
        self.assert_dirty_rejected()

    def test_staged_only_content_is_rejected(self):
        self.source.write_text("#!/bin/sh\necho staged\n", encoding="utf-8")
        subprocess.run(["git", "-C", str(self.root), "add", "scripts/check.sh"], check=True)
        self.assert_dirty_rejected()

    def test_dirty_executable_mode_is_rejected(self):
        self.source.chmod(0o644)
        self.assert_dirty_rejected()

    def test_repeated_snapshot_has_identical_tree_and_digest(self):
        _, first = self.snapshot("first.json")
        _, second = self.snapshot("second.json")
        self.assertEqual(first["tree"], second["tree"])
        self.assertEqual(first["digest"], second["digest"])
        self.assertEqual(first["entries"][0]["mode"], "100755")

    def test_committed_mode_change_changes_manifest_digest(self):
        _, first = self.snapshot("before.json")
        self.source.chmod(0o644)
        subprocess.run(["git", "-C", str(self.root), "add", "scripts/check.sh"], check=True)
        subprocess.run(["git", "-C", str(self.root), "commit", "-qm", "change mode"], check=True)
        _, second = self.snapshot("after.json")
        self.assertNotEqual(first["tree"], second["tree"])
        self.assertNotEqual(first["digest"], second["digest"])
        self.assertEqual(second["entries"][0]["mode"], "100644")

    def test_destination_mode_drift_is_rejected(self):
        manifest_path, manifest = self.snapshot()
        destination = self.root / "destination"
        destination.mkdir()
        target = destination / manifest["entries"][0]["path"]
        target.parent.mkdir(parents=True)
        target.write_bytes(self.source.read_bytes())
        target.chmod(0o644)
        result = self.helper(
            "verify",
            "--manifest",
            str(manifest_path),
            "--destination",
            str(destination),
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("mode mismatch", result.stderr)


if __name__ == "__main__":
    unittest.main()
