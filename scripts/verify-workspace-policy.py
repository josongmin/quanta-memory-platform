#!/usr/bin/env python3
"""Fail closed when a workspace member bypasses the Rust safety policy."""

from __future__ import annotations

import re
import sys
from pathlib import Path


def section(text: str, name: str) -> str:
    match = re.search(
        rf"^\[{re.escape(name)}\]\s*$\n(?P<body>.*?)(?=^\[|\Z)",
        text,
        flags=re.MULTILINE | re.DOTALL,
    )
    return match.group("body") if match else ""


def workspace_members(text: str) -> list[str] | None:
    body = section(text, "workspace")
    match = re.search(r"^members\s*=\s*\[(?P<items>.*?)\]", body, re.MULTILINE | re.DOTALL)
    if not match:
        return None
    return re.findall(r'"([^"\n]+)"', match.group("items"))


def lint_policy_errors(root: Path) -> list[str]:
    root_manifest = (root / "Cargo.toml").read_text(encoding="utf-8")
    errors: list[str] = []
    rust_lints = section(root_manifest, "workspace.lints.rust")
    if not re.search(r'^unsafe_code\s*=\s*"deny"\s*$', rust_lints, re.MULTILINE):
        errors.append("workspace.lints.rust.unsafe_code must be `deny`")

    members = workspace_members(root_manifest)
    if members is None:
        return [*errors, "workspace.members must be an array"]
    for member in sorted(members):
        manifest_path = root / member / "Cargo.toml"
        if not manifest_path.is_file():
            errors.append(f"workspace member manifest is missing: {member}/Cargo.toml")
            continue
        manifest = manifest_path.read_text(encoding="utf-8")
        member_lints = section(manifest, "lints")
        if not re.search(r"^workspace\s*=\s*true\s*$", member_lints, re.MULTILINE):
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
    root = Path(__file__).resolve().parents[1]
    errors = lint_policy_errors(root)
    if errors:
        for error in errors:
            print(f"workspace lint policy failed: {error}", file=sys.stderr)
        return 1
    print("workspace policy: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
