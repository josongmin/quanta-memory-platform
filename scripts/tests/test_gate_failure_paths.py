"""Exercise shell gate failure paths with isolated command wrappers."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


def executable(path: Path, contents: str) -> None:
    path.write_text(contents, encoding="utf-8")
    path.chmod(0o755)


class GateFailurePathsTests(unittest.TestCase):
    def test_extracted_checkout_status_error_cannot_report_success(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-extract-git-failure-") as directory:
            root = Path(directory)
            scripts = root / "scripts"
            binary = root / "bin"
            scripts.mkdir()
            binary.mkdir()
            shutil.copy2(ROOT / "scripts/verify-extracted-examples.sh", scripts)
            executable(scripts / "extract-to-standalone-repo.sh", "#!/bin/sh\nexit 0\n")
            executable(binary / "cargo", "#!/bin/sh\nexit 0\n")
            executable(binary / "git", "#!/bin/sh\n"
                       'if [ "$1" = "-C" ]; then echo "git status failed" >&2; exit 37; fi\n'
                       'if [ "$1" = "rev-parse" ]; then echo "fixture-head"; exit 0; fi\n'
                       "exit 38\n")
            result = subprocess.run(
                ["bash", str(scripts / "verify-extracted-examples.sh")],
                cwd=root,
                env={**os.environ, "PATH": f"{binary}:{os.environ['PATH']}"},
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("Verified extracted checkout", result.stdout)
            self.assertIn("Failed to inspect extracted checkout", result.stderr)

    def test_feature_matrix_rejects_missing_ripgrep_before_cargo(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-missing-rg-") as directory:
            root = Path(directory)
            scripts = root / "scripts"
            binary = root / "bin"
            scripts.mkdir()
            binary.mkdir()
            shutil.copy2(ROOT / "scripts/verify-feature-matrix.sh", scripts)
            for command in ("bash", "dirname"):
                source = shutil.which(command)
                self.assertIsNotNone(source)
                (binary / command).symlink_to(source)
            executable(binary / "cargo", "#!/bin/sh\nexit 99\n")
            result = subprocess.run(
                [str(binary / "bash"), str(scripts / "verify-feature-matrix.sh")],
                cwd=root,
                env={**os.environ, "PATH": str(binary)},
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("feature-matrix: rg unavailable", result.stderr)

    def test_partial_find_failure_stops_before_shellcheck(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-find-failure-") as directory:
            root = Path(directory)
            scripts = root / "scripts"
            binary = root / "bin"
            scripts.mkdir()
            binary.mkdir()
            shutil.copy2(ROOT / "scripts/check-workspace.sh", scripts)
            executable(
                binary / "cargo",
                "#!/bin/sh\n"
                'if [ "$1" = fmt ]; then exit 0; fi\n'
                'if [ "$1" = metadata ]; then echo "metadata reached"; exit 37; fi\n'
                "exit 38\n",
            )
            executable(
                binary / "find",
                "#!/bin/sh\nprintf 'scripts/check-workspace.sh\\000'\nexit 42\n",
            )
            executable(binary / "shellcheck", "#!/bin/sh\nexit 0\n")
            env = {**os.environ, "PATH": f"{binary}:{os.environ['PATH']}"}
            result = subprocess.run(
                ["bash", str(scripts / "check-workspace.sh")],
                cwd=root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 42, result.stdout + result.stderr)
            self.assertNotIn("resolved workspace graph", result.stdout)

    def test_feature_matrix_runs_bare_and_singleton_default_slice(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-feature-matrix-") as directory:
            root = Path(directory)
            scripts = root / "scripts"
            binary = root / "bin"
            scripts.mkdir()
            binary.mkdir()
            shutil.copy2(ROOT / "scripts/verify-feature-matrix.sh", scripts)
            packages = [
                {
                    "name": "quanta-memory-platform-core",
                    "features": {"default": ["live_control"], "live_control": [], "full": []},
                },
                {
                    "name": "quanta-memory-platform-facade",
                    "features": {
                        "default": ["core-live-accounting"],
                        "core-live-accounting": [],
                        "full": [],
                    },
                },
                {
                    "name": "quanta-memory-platform-testkit",
                    "features": {"default": [], "full": []},
                },
            ]
            metadata = root / "metadata.json"
            metadata.write_text(json.dumps({"packages": packages}), encoding="utf-8")
            log = root / "cargo.log"
            executable(
                binary / "cargo",
                "#!/bin/sh\n"
                'case "$1" in\n'
                '  metadata) cat "$QMP_MATRIX_METADATA" ;;\n'
                '  clippy) printf "%s\\n" "$*" >> "$QMP_MATRIX_LOG" ;;\n'
                "  tree) exit 0 ;;\n"
                "  *) exit 38 ;;\n"
                "esac\n",
            )
            env = {
                **os.environ,
                "PATH": f"{binary}:{os.environ['PATH']}",
                "QMP_MATRIX_METADATA": str(metadata),
                "QMP_MATRIX_LOG": str(log),
            }
            result = subprocess.run(
                ["bash", str(scripts / "verify-feature-matrix.sh")],
                cwd=root,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            commands = log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(
                any(
                    "-p quanta-memory-platform-core --all-targets"
                    " --no-default-features --features live_control" in command
                    for command in commands
                ),
                commands,
            )
            self.assertTrue(
                any(
                    "-p quanta-memory-platform-testkit --all-targets"
                    " --no-default-features -- -D warnings" in command
                    for command in commands
                ),
                commands,
            )


if __name__ == "__main__":
    unittest.main()
