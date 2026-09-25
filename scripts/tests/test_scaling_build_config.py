from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "lib" / "scaling_build_config.py"
SPEC = importlib.util.spec_from_file_location("scaling_build_config", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ScalingBuildConfigTests(unittest.TestCase):
    def test_build_flags_and_cargo_config_change_fingerprint(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "checkout"
            cargo_home = Path(directory) / "cargo-home"
            (root / "crates" / "core").mkdir(parents=True)
            cargo_home.mkdir()
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            (root / "crates" / "core" / "Cargo.toml").write_text(
                "[package]\nname = 'core'\nversion = '0.1.0'\n", encoding="utf-8",
            )
            baseline = MODULE.build_config_digest(root, cargo_home, {})
            self.assertEqual(len(baseline), 64)
            for override in (
                {"RUSTFLAGS": "-C target-cpu=native"},
                {"CARGO_PROFILE_BENCH_OPT_LEVEL": "1"},
                {"CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS": "-C target-feature=+neon"},
                {"CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER": "remote-runner"},
            ):
                with self.subTest(override=override):
                    self.assertNotEqual(
                        MODULE.build_config_digest(root, cargo_home, override), baseline,
                    )
            self.assertEqual(
                MODULE.build_config_digest(root, cargo_home, {"CARGO_TARGET_DIR": "/tmp/other"}),
                baseline,
            )
            (root / ".cargo").mkdir()
            (root / ".cargo" / "config.toml").write_text(
                "[build]\nrustflags = ['-C', 'target-cpu=native']\n", encoding="utf-8",
            )
            self.assertNotEqual(MODULE.build_config_digest(root, cargo_home, {}), baseline)
            (root / ".cargo" / "config.toml").unlink()
            (cargo_home / "config.toml").write_text("[build]\njobs = 2\n", encoding="utf-8")
            self.assertNotEqual(MODULE.build_config_digest(root, cargo_home, {}), baseline)
            (cargo_home / "config.toml").unlink()
            (root / "Cargo.toml").write_text("[workspace]\n[profile.bench]\nopt-level = 1\n", encoding="utf-8")
            self.assertNotEqual(MODULE.build_config_digest(root, cargo_home, {}), baseline)


if __name__ == "__main__":
    unittest.main()
