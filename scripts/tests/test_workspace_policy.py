from __future__ import annotations

import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts/verify-workspace-policy.py"
SPEC = importlib.util.spec_from_file_location("verify_workspace_policy", CHECKER)
assert SPEC and SPEC.loader
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def policy_errors_for(root: Path) -> list[str]:
    result = subprocess.run(
        ["cargo", "metadata", "--offline", "--no-deps", "--format-version", "1"],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    )
    return checker.lint_policy_errors(root, json.loads(result.stdout))


class WorkspaceLintPolicyTest(unittest.TestCase):
    def test_domain_root_super_glob_cannot_admit_bare_adapters_path(self) -> None:
        relative = Path("crates/quanta-memory-platform-core/src/domain/mod.rs")
        for source in (
            "use super::*; pub fn leaked(_: adapters::Clock) {}\n",
            "use super::*; use adapters as backend; pub fn leaked(_: backend::Clock) {}\n",
            "use super::*; use r#adapters as backend; pub fn leaked(_: backend::Clock) {}\n",
            "use super::*; use {{adapters as backend}}; pub fn leaked(_: backend::Clock) {}\n",
        ):
            with self.subTest(source=source), tempfile.TemporaryDirectory(prefix="qmp-super-glob-") as directory:
                root = Path(directory)
                (root / "domain").mkdir()
                (root / "lib.rs").write_text(
                    "pub mod adapters { pub struct Clock; }\npub mod domain;\n", encoding="utf-8",
                )
                (root / "domain/mod.rs").write_text(source, encoding="utf-8")
                compiled = subprocess.run(
                    ["rustc", "--crate-type=lib", "--edition=2021", "--emit=metadata",
                     "-o", str(root / "lib.rmeta"), str(root / "lib.rs")],
                    capture_output=True, text=True, check=False,
                )
                self.assertEqual(compiled.returncode, 0, compiled.stderr)
                self.assertIn("pure layer depends on adapters", "\n".join(
                    checker.pure_layer_adapter_errors(source, relative)))
        self.assertEqual(
            checker.pure_layer_adapter_errors(
                "#[cfg(test)] mod tests { use super::*; fn local() {} }\n", relative,
            ), [],
        )

    def test_pure_layer_rejects_direct_and_aliased_adapter_paths(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-pure-layer-") as directory:
            root = Path(directory)
            core = root / "crates/quanta-memory-platform-core"
            domain = core / "src/domain"
            domain.mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/quanta-memory-platform-core"]\n'
                '[workspace.lints.rust]\nunsafe_code = "deny"\n', encoding="utf-8",
            )
            (core / "Cargo.toml").write_text(
                '[package]\nname = "qmp-fixture-core"\nversion = "0.1.0"\n'
                '[lints]\nworkspace = true\n', encoding="utf-8",
            )
            (core / "src/lib.rs").write_text("#![forbid(unsafe_code)]\n", encoding="utf-8")
            source = domain / "leak.rs"
            bad = (
                "pub fn type_only(_: crate::adapters::Clock) {}\n",
                "pub fn value() { let _ = crate::adapters::Clock; }\n",
                "use crate as local; pub fn aliased(_: local::adapters::Clock) {}\n",
                "pub use crate as local; use local as renamed; type T = renamed::adapters::Clock;\n",
                "use crate::{self as local}; pub fn grouped_alias(_: local::adapters::Clock) {}\n",
                "use crate::{foo, self as local}; pub fn middle_alias(_: local::adapters::Clock) {}\n",
                "use crate::{{self as local}, foo}; "
                "pub fn anonymous_alias(_: local::adapters::Clock) {}\n",
                "use {crate as local}; pub fn list_alias(_: local::adapters::Clock) {}\n",
                "use crate::{self as r#local, foo}; use r#local::{self as root}; "
                "pub fn chained_alias(_: root::adapters::Clock) {}\n",
                "use crate as r#type; pub fn raw_alias(_: r#type::adapters::Clock) {}\n",
                "use crate::*; pub fn glob(_: adapters::Clock) {}\n",
                "use crate::{{*}}; pub fn anonymous_glob(_: adapters::Clock) {}\n",
                "use {crate::*}; pub fn list_glob(_: adapters::Clock) {}\n",
                "use crate::{foo, *}; pub fn grouped_glob(_: adapters::Clock) {}\n",
                "use crate as local; use local::*; pub fn aliased_glob(_: adapters::Clock) {}\n",
                "type T = crate::r#adapters::Clock;\n",
                "use crate::{adapters as concrete};\n",
            )
            for code in bad:
                with self.subTest(code=code):
                    source.write_text(code, encoding="utf-8")
                    self.assertIn("pure layer depends on adapters", "\n".join(policy_errors_for(root)))
            for index, code in enumerate(bad[5:15]):
                with self.subTest(compiled_code=code):
                    fixture = root / f"alias_{index}.rs"
                    fixture.write_text(
                        "pub mod foo {}\npub mod adapters { pub struct Clock; }\n"
                        "pub mod domain { " + code + " }\n", encoding="utf-8",
                    )
                    compiled = subprocess.run(
                        ["rustc", "--crate-type=lib", "--edition=2021", "--emit=metadata",
                         "-o", str(root / f"alias_{index}.rmeta"), str(fixture)],
                        capture_output=True, text=True, check=False,
                    )
                    self.assertEqual(compiled.returncode, 0, compiled.stderr)
            source.write_text(
                'const DOC: &str = "crate::adapters::Clock"; '
                'pub fn pure(_: crate::ports::Clock) {}\n'
                '// use crate::adapters::Clock;\n'
                'const C: &std::ffi::CStr = cr#"use crate::adapters::Clock"#;\n',
                encoding="utf-8",
            )
            self.assertEqual(policy_errors_for(root), [])
            source.write_text(
                'use crate::{foo::{self as local}}; '
                'pub fn pure(_: local :: adapters::Clock) {}\n', encoding="utf-8",
            )
            self.assertEqual(policy_errors_for(root), [], "nested self alias must not mean crate root")
            source.write_text(
                'const DOC: &str = "crate::adapters::Clock"; '
                'pub fn leaked(_: crate::adapters::Clock) {}\n', encoding="utf-8",
            )
            self.assertIn("pure layer depends on adapters", "\n".join(policy_errors_for(root)))
            (core / "src/lib.rs").write_text(
                "#![forbid(unsafe_code)]\npub mod adapters { pub struct Clock; }\n"
                "pub mod domain;\n", encoding="utf-8",
            )
            (domain / "mod.rs").write_text("pub mod leak;\n", encoding="utf-8")
            source.write_text(
                'use crate as local; pub fn leaked(_: local::adapters::Clock) {}\n',
                encoding="utf-8",
            )
            compiled = subprocess.run(
                ["rustc", "--crate-type=lib", "--edition=2021", "--emit=metadata",
                 "-o", str(root / "fixture.rmeta"), str(core / "src/lib.rs")],
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(compiled.returncode, 0, compiled.stderr)
            self.assertIn("pure layer depends on adapters", "\n".join(policy_errors_for(root)))
            ports = core / "src/ports"
            ports.mkdir()
            source.unlink()
            (ports / "leak.rs").write_text(
                "pub fn leaked(_: crate::adapters::Clock) {}\n", encoding="utf-8",
            )
            self.assertIn("pure layer depends on adapters", "\n".join(policy_errors_for(root)))

    def test_benchmark_unsafe_exception_is_item_scoped(self) -> None:
        bench = Path("crates/quanta-memory-platform-facade/benches/scaling/mod.rs")
        reason = 'GlobalAlloc instrumentation delegates to System'
        approved = (
            f'#[allow(unsafe_code, reason = "{reason}")]\n'
            'unsafe impl GlobalAlloc for BenchAllocator {}\n'
        )
        self.assertEqual(checker.unsafe_exception_errors(approved, bench, bench), [])
        self.assertEqual(
            checker.unsafe_exception_errors(
                '// #[allow(unsafe_code)]\n'
                'const COMMENT: &std::ffi::CStr = cr#"#[allow(unsafe_code)]"#;\n',
                Path("crates/quanta-memory-platform-core/src/lib.rs"), bench,
            ), [],
        )
        for source in (
            '#![allow(unsafe_code)]\n' + approved,
            '#[allow(unsafe_code)]\nunsafe fn rogue() {}\n' + approved,
            '#[allow(unsafe_code, reason = "wide")]\nmod other {}\n' + approved,
            '#[cfg_attr(all(), allow(unsafe_code))]\n' + approved,
            '#[allow(unsafe_code)]\nunsafe impl GlobalAlloc for BenchAllocator {}\n',
        ):
            with self.subTest(source=source):
                self.assertIn("unauthorized unsafe-code exception", "\n".join(
                    checker.unsafe_exception_errors(source, bench, bench)))
        with tempfile.TemporaryDirectory(prefix="qmp-item-unsafe-") as directory:
            source = Path(directory) / "scope.rs"
            real_impl = (
                'struct BenchAllocator;\n'
                f'#[allow(unsafe_code, reason = "{reason}")]\n'
                'unsafe impl std::alloc::GlobalAlloc for BenchAllocator {\n'
                ' unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 { std::ptr::null_mut() }\n'
                ' unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) {}\n'
                '}\n'
            )
            source.write_text(real_impl, encoding="utf-8")
            command = ["rustc", "--crate-type=lib", "--edition=2021", "-Dunsafe_code",
                       "--emit=metadata", "-o", str(Path(directory) / "scope.rmeta"), str(source)]
            accepted = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(accepted.returncode, 0, accepted.stderr)
            source.write_text(real_impl + "unsafe fn rogue() {}\n", encoding="utf-8")
            rejected = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertIn("unsafe", rejected.stderr)

    def test_toml_string_cannot_spoof_workspace_lint_table(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-lint-string-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\n'
                '[workspace.metadata]\ndescription = """\n[workspace.lints.rust]\nunsafe_code = "deny"\n"""\n'
                '[workspace.lints.rust]\nunsafe_code = "allow"\n', encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\n[lints]\nworkspace = true\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text("#![forbid(unsafe_code)]\n", encoding="utf-8")
            self.assertIn("workspace.lints.rust.unsafe_code must be `deny`", policy_errors_for(root))

    def test_new_non_inheriting_member_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-lints-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\n\n'
                '[workspace.lints.rust]\nunsafe_code = "deny"\n',
                encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text(
                "#![forbid(unsafe_code)]\n", encoding="utf-8"
            )
            self.assertEqual(
                policy_errors_for(root),
                ["workspace member does not inherit lints: member"],
            )

    def test_library_root_without_non_overridable_forbid_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-missing-forbid-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\n\n'
                '[workspace.lints.rust]\nunsafe_code = "deny"\n',
                encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\n\n'
                '[lints]\nworkspace = true\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text("pub fn safe() {}\n", encoding="utf-8")
            self.assertEqual(
                policy_errors_for(root),
                ["library root does not forbid unsafe code: member"],
            )

    def test_inherited_policy_rejects_unsafe_code(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-unsafe-lint-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\nresolver = "2"\n\n'
                '[workspace.lints.rust]\nunsafe_code = "deny"\n',
                encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\nedition = "2021"\n\n'
                '[lints]\nworkspace = true\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text(
                "#![forbid(unsafe_code)]\npub unsafe fn forbidden() {}\n",
                encoding="utf-8",
            )
            result = subprocess.run(
                ["cargo", "check", "--offline", "-q"],
                cwd=root,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("declaration of an `unsafe` function", result.stderr)

    def test_unapproved_unsafe_exception_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-unsafe-exception-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "member/tests").mkdir()
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\n\n'
                '[workspace.lints.rust]\nunsafe_code = "deny"\n',
                encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\n\n'
                '[lints]\nworkspace = true\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text(
                "#![forbid(unsafe_code)]\n", encoding="utf-8"
            )
            (root / "member/tests/rogue.rs").write_text(
                "#![allow(unsafe_code)]\n", encoding="utf-8"
            )
            (root / "member/tests/conditional.rs").write_text(
                "#![cfg_attr(all(), allow(unused, unsafe_code))]\n",
                encoding="utf-8",
            )
            self.assertCountEqual(
                policy_errors_for(root),
                [
                    "unauthorized unsafe-code exception: member/tests/rogue.rs",
                    "unauthorized unsafe-code exception: member/tests/conditional.rs",
                ],
            )

    def test_single_quoted_cargo_members_are_checked(self) -> None:
        with tempfile.TemporaryDirectory(prefix="qmp-single-quote-members-") as directory:
            root = Path(directory)
            (root / "member/src").mkdir(parents=True)
            (root / "Cargo.toml").write_text(
                "[workspace]\nmembers = ['member']\n\n"
                '[workspace.lints.rust]\nunsafe_code = "deny"\n',
                encoding="utf-8",
            )
            (root / "member/Cargo.toml").write_text(
                '[package]\nname = "member"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            (root / "member/src/lib.rs").write_text("pub fn safe() {}\n", encoding="utf-8")
            self.assertCountEqual(
                policy_errors_for(root),
                [
                    "workspace member does not inherit lints: member",
                    "library root does not forbid unsafe code: member",
                ],
            )


if __name__ == "__main__":
    unittest.main()
