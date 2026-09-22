#!/usr/bin/env python3
"""Fail closed when a workspace member does not inherit the Rust lint policy."""

from __future__ import annotations

import re
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.9 on the supported macOS builder.
    import tomli as tomllib


def load_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def lint_policy_errors(root: Path) -> list[str]:
    root_manifest = load_toml(root / "Cargo.toml")
    errors: list[str] = []
    unsafe_policy = (
        root_manifest.get("workspace", {})
        .get("lints", {})
        .get("rust", {})
        .get("unsafe_code")
    )
    if unsafe_policy != "deny":
        errors.append("workspace.lints.rust.unsafe_code must be `deny`")

    members = root_manifest.get("workspace", {}).get("members", [])
    if not isinstance(members, list):
        return [*errors, "workspace.members must be an array"]
    for member in sorted(members):
        manifest_path = root / member / "Cargo.toml"
        if not manifest_path.is_file():
            errors.append(f"workspace member manifest is missing: {member}/Cargo.toml")
            continue
        manifest = load_toml(manifest_path)
        if manifest.get("lints", {}).get("workspace") is not True:
            errors.append(f"workspace member does not inherit lints: {member}")
        library_root = root / member / "src/lib.rs"
        if library_root.is_file() and not library_root.read_text().startswith(
            "#![forbid(unsafe_code)]\n"
        ):
            errors.append(f"library root does not forbid unsafe code: {member}")
        exception = re.compile(r"#\s*!?\s*\[\s*(?:allow|expect)\s*\(\s*unsafe_code\s*\)")
        allowed_exception = root / "crates/quanta-memory-platform-facade/benches/scaling.rs"
        for source in (root / member).rglob("*.rs"):
            if source != allowed_exception and exception.search(source.read_text()):
                errors.append(f"unauthorized unsafe-code exception: {source.relative_to(root)}")
    return errors


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    errors = lint_policy_errors(root)
    if errors:
        for error in errors:
            print(f"workspace lint policy failed: {error}", file=sys.stderr)
        return 1
    print("workspace lint policy passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
