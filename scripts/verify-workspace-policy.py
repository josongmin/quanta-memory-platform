#!/usr/bin/env python3
"""Fail closed when a workspace member bypasses the Rust safety policy."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

try:
    import tomllib  # type: ignore[import-not-found]
except ImportError:
    try:
        import tomli as tomllib  # type: ignore[import-not-found,no-redef]
    except ImportError as error:
        raise SystemExit("workspace policy requires Python 3.11+ or tomli") from error


def cargo_metadata(root: Path) -> dict:
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise ValueError(f"cargo metadata failed: {result.stderr.strip()}")
    return json.loads(result.stdout)


def workspace_members(root: Path, metadata: dict) -> list[Path]:
    if Path(metadata["workspace_root"]).resolve() != root.resolve():
        raise ValueError("cargo metadata workspace root differs from policy root")
    packages = {package["id"]: package for package in metadata["packages"]}
    member_ids = metadata["workspace_members"]
    if not member_ids:
        raise ValueError("cargo metadata returned no workspace members")
    members = []
    for member_id in member_ids:
        manifest = Path(packages[member_id]["manifest_path"]).resolve()
        try:
            members.append(manifest.parent.relative_to(root.resolve()))
        except ValueError as error:
            raise ValueError(f"workspace member escapes policy root: {manifest}") from error
    return sorted(set(members))


def lint_policy_errors(root: Path, metadata: dict | None = None) -> list[str]:
    errors: list[str] = []
    try:
        root_manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        return [f"root Cargo.toml cannot be parsed: {error}"]
    if root_manifest.get("workspace", {}).get("lints", {}).get("rust", {}).get("unsafe_code") != "deny":
        errors.append("workspace.lints.rust.unsafe_code must be `deny`")

    try:
        members = workspace_members(root, metadata if metadata is not None else cargo_metadata(root))
    except (KeyError, TypeError, ValueError, OSError) as error:
        return [*errors, f"workspace member discovery failed: {error}"]
    for member in sorted(members):
        manifest_path = root / member / "Cargo.toml"
        if not manifest_path.is_file():
            errors.append(f"workspace member manifest is missing: {member}/Cargo.toml")
            continue
        try:
            manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            errors.append(f"workspace member manifest cannot be parsed: {member}: {error}")
            continue
        if manifest.get("lints", {}).get("workspace") is not True:
            errors.append(f"workspace member does not inherit lints: {member}")
        library_root = root / member / "src/lib.rs"
        if library_root.is_file() and not library_root.read_text().startswith(
            "#![forbid(unsafe_code)]\n"
        ):
            errors.append(f"library root does not forbid unsafe code: {member}")
        # Include cfg_attr(...) wrappers and lint lists. A crate-level
        # unsafe_code=deny can otherwise be lowered by an attribute in tests.
        exception = re.compile(
            r"#\s*!?\s*\[[^\]]*\b(?:allow|expect)\s*\([^)]*\bunsafe_code\b"
        )
        allowed_exception = root / "crates/quanta-memory-platform-facade/benches/scaling/mod.rs"
        for source in (root / member).rglob("*.rs"):
            if source != allowed_exception and exception.search(source.read_text()):
                errors.append(f"unauthorized unsafe-code exception: {source.relative_to(root)}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--metadata-stdin", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        metadata = json.load(sys.stdin) if args.metadata_stdin else None
    except ValueError as error:
        print(f"workspace lint policy failed: invalid metadata: {error}", file=sys.stderr)
        return 1
    errors = lint_policy_errors(root, metadata)
    if errors:
        for error in errors:
            print(f"workspace lint policy failed: {error}", file=sys.stderr)
        return 1
    print("workspace policy: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
