"""Exercise the source-shape guard on repository-shaped Rust files."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/verify-source-shape.py"
SPEC = importlib.util.spec_from_file_location("verify_source_shape", SCRIPT)
assert SPEC and SPEC.loader
shape = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(shape)

LEGACY = "crates/quanta-memory-platform-core/src/api/control.rs"
NEW = "crates/quanta-memory-platform-core/src/api/new_module.rs"


class SourceShapeContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="qmp-source-shape-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "crates/quanta-memory-platform-core/src/api").mkdir(parents=True)
        (self.root / "scripts").mkdir()
        self.baseline = self.root / "scripts/source-shape-baseline.json"
        self.write_baseline({})

    def write_baseline(self, limits: dict[str, int]) -> None:
        self.baseline.write_text(json.dumps({"version": 1, "grandfathered": limits}), encoding="utf-8")

    def write_source(self, relative: str, lines: int, tail: str = "") -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("// fixture\n" * lines + tail, encoding="utf-8")

    def check(self) -> tuple[list[str], list[str]]:
        return shape.verify(self.root, self.baseline)

    def test_new_file_at_cap_passes_and_reports_advisory(self) -> None:
        self.write_source(NEW, 1_000)
        errors, advisory = self.check()
        self.assertEqual(errors, [])
        self.assertEqual(advisory, [f"{NEW}: 1000 lines (advisory at 600)"])

    def test_new_file_over_cap_is_rejected_even_after_rename(self) -> None:
        self.write_source(NEW, 1_001)
        self.assertIn(f"{NEW}: 1001 lines exceeds 1000-line ceiling", self.check()[0])
        renamed = "crates/quanta-memory-platform-core/src/api/renamed.rs"
        (self.root / NEW).rename(self.root / renamed)
        self.assertIn(f"{renamed}: 1001 lines exceeds 1000-line ceiling", self.check()[0])

    def test_existing_large_file_can_only_shrink_with_exact_baseline_update(self) -> None:
        with patch.dict(shape.INITIAL_OVERSIZED_CEILINGS, {LEGACY: 1_004}, clear=True):
            self.write_source(LEGACY, 1_004)
            self.write_baseline({LEGACY: 1_004})
            self.assertEqual(self.check()[0], [])
            self.write_source(LEGACY, 1_005)
            self.assertIn("1005 lines exceeds 1004-line ceiling", "\n".join(self.check()[0]))
            self.write_source(LEGACY, 1_003)
            self.assertIn("stale 1004-line limit", "\n".join(self.check()[0]))
            self.write_baseline({LEGACY: 1_003})
            self.assertEqual(self.check()[0], [])
            self.write_source(LEGACY, 1_000)
            self.write_baseline({})
            self.assertEqual(self.check()[0], [])

    def test_baseline_inflation_and_new_exception_are_rejected(self) -> None:
        with patch.dict(shape.INITIAL_OVERSIZED_CEILINGS, {LEGACY: 1_004}, clear=True):
            self.write_source(LEGACY, 1_004)
            self.write_baseline({LEGACY: 1_005})
            self.assertIn("exceeds frozen initial ceiling", "\n".join(self.check()[0]))
            self.write_baseline({NEW: 1_005})
            self.assertIn("cannot grandfather it", "\n".join(self.check()[0]))

    def test_missing_or_removed_legacy_baseline_fails(self) -> None:
        with patch.dict(shape.INITIAL_OVERSIZED_CEILINGS, {LEGACY: 1_004}, clear=True):
            self.write_source(LEGACY, 1_004)
            self.assertIn("1004 lines exceeds 1000-line ceiling", "\n".join(self.check()[0]))
            self.write_baseline({LEGACY: 1_004})
            (self.root / LEGACY).unlink()
            self.write_source(NEW, 1)
            self.assertIn("grandfathered source file is missing", "\n".join(self.check()[0]))

    def test_symlinks_cannot_hide_rust_files_or_modules(self) -> None:
        self.write_source(NEW, 1)
        (self.root / "crates/quanta-memory-platform-core/src/api/link.rs").symlink_to(self.root / NEW)
        self.assertIn("symlinks under crates are not allowed", "\n".join(self.check()[0]))
        (self.root / "crates/quanta-memory-platform-core/src/api/link.rs").unlink()
        (self.root / "crates/quanta-memory-platform-core/src/linked").symlink_to(self.root / "crates/quanta-memory-platform-core/src/api")
        self.assertIn("symlinks under crates are not allowed", "\n".join(self.check()[0]))

    def test_include_and_path_module_bypasses_are_rejected(self) -> None:
        self.write_source(NEW, 0, 'include!("hidden.rs");\n#[cfg_attr(test, path = "hidden.rs")] mod hidden;\n')
        errors, _ = self.check()
        self.assertIn("include! bypasses", "\n".join(errors))
        self.assertIn("#[path] bypasses", "\n".join(errors))

    def test_comments_and_literals_do_not_spoof_bypass_tokens(self) -> None:
        self.write_source(
            NEW, 0,
            '// include!("ignored.rs");\n'
            '/* #[path = "ignored.rs"] */\n'
            'const A: &str = "include!(\\\"ignored.rs\\\")";\n'
            'const B: &str = r#"#[path = \"ignored.rs\"]"#;\n'
            'mod normal;\n'
            '#[cfg(test)] mod tests;\n',
        )
        self.assertEqual(self.check()[0], [])

    def test_invalid_baseline_and_duplicate_keys_fail_closed(self) -> None:
        self.write_source(NEW, 1)
        self.baseline.write_text('{"version":1,"grandfathered":{},"grandfathered":{}}', encoding="utf-8")
        self.assertIn("duplicate JSON key", "\n".join(self.check()[0]))
        self.baseline.write_text('{"version":1,"grandfathered":{"' + LEGACY + '":true}}', encoding="utf-8")
        self.assertIn("must be an integer", "\n".join(self.check()[0]))

if __name__ == "__main__":
    unittest.main()
