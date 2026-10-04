#!/usr/bin/env python3
"""Fail closed when a workspace member bypasses the Rust safety policy."""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import subprocess
import sys
from pathlib import Path


SHAPE_SPEC = importlib.util.spec_from_file_location(
    "qmp_source_shape", Path(__file__).with_name("verify-source-shape.py")
)
if SHAPE_SPEC is None or SHAPE_SPEC.loader is None:
    raise RuntimeError("Rust source masker is unavailable")
SHAPE = importlib.util.module_from_spec(SHAPE_SPEC)
SHAPE_SPEC.loader.exec_module(SHAPE)

UNSAFE_EXCEPTION = re.compile(r"#\s*!?\s*\[[^\]]*\b(?:allow|expect)\s*\([^)]*\bunsafe_code\b")
BENCH_EXCEPTION = re.compile(
    r'#\s*\[\s*allow\s*\(\s*unsafe_code\s*,\s*reason\s*=\s*"[^"\n]+"\s*\)\s*\]'
)
BENCH_IMPL = re.compile(r"\s*unsafe\s+impl\s+GlobalAlloc\s+for\s+BenchAllocator\s*\{")
ADAPTER_NAME = r"(?:r#)?adapters\b"
BARE_ADAPTER_PATH = re.compile(r"\b(?:r#)?adapters\s*::")
BARE_ADAPTER_IMPORT = re.compile(r"\buse\s+(?:r#)?adapters\b")
IDENT = r"(?:r#)?[A-Za-z_]\w*"
ROOT_ALIAS = re.compile(rf"\buse\s+({IDENT})\s+as\s+({IDENT})\s*;")
EXTERN_ROOT_ALIAS = re.compile(rf"\bextern\s+crate\s+self\s+as\s+({IDENT})\s*;")
USE_TREE = re.compile(rf"\buse\s+({IDENT})\s*::\s*(\*|\{{)")
ANON_USE = re.compile(r"\buse\s*\{")
USE_TREE_ITEM = re.compile(rf"({IDENT})\s*::\s*(\*|\{{)")
SIMPLE_ALIAS_ITEM = re.compile(rf"({IDENT})\s+as\s+({IDENT})\s*")
GROUP_SELF_ALIAS = re.compile(rf"self\s+as\s+({IDENT})\s*")


def _top_level_use_items(code: str, opening: int) -> list[str]:
    """Split one Rust use group without treating nested groups as root items."""
    items: list[str] = []
    depth = 1
    start = opening + 1
    for cursor in range(start, len(code)):
        if code[cursor] == "{":
            depth += 1
        elif code[cursor] == "}":
            depth -= 1
            if depth == 0:
                items.append(code[start:cursor].strip())
                flattened: list[str] = []
                for item in items:
                    # Anonymous braces preserve the same import root. A named
                    # subtree (foo::{...}) has a different root and stays local.
                    if item.startswith("{") and item.endswith("}"):
                        flattened.extend(_top_level_use_items(item, 0))
                    else:
                        flattened.append(item)
                return flattened
        elif code[cursor] == "," and depth == 1:
            items.append(code[start:cursor].strip())
            start = cursor + 1
    return []


def unsafe_exception_errors(source: str, relative: Path, allowed_exception: Path) -> list[str]:
    code = SHAPE._code_only(source)
    exceptions = list(UNSAFE_EXCEPTION.finditer(code))
    if not exceptions:
        return []
    if relative != allowed_exception or len(exceptions) != 1:
        return [f"unauthorized unsafe-code exception: {relative}"]
    match = BENCH_EXCEPTION.match(source, exceptions[0].start())
    if match is None or not BENCH_IMPL.match(code, match.end()):
        return [f"unauthorized unsafe-code exception: {relative}"]
    return []


def pure_layer_adapter_errors(source: str, relative: Path) -> list[str]:
    """Guard direct and locally aliased adapter paths after masking comments/literals.

    Rust name resolution remains the compiler's authority; this lexical guard
    rejects source-level layer edges without interpreting arbitrary macros.
    """
    code = SHAPE._code_only(source)
    aliases = {"crate", "self", "super"}
    crate_roots = {"crate"}
    crate_roots.update(EXTERN_ROOT_ALIAS.findall(code))
    aliases.update(crate_roots)
    simple_aliases = ROOT_ALIAS.findall(code)
    use_trees = [
        (match.group(1), ["*"] if match.group(2) == "*" else _top_level_use_items(code, match.end() - 1))
        for match in USE_TREE.finditer(code)
    ]
    anonymous_items = [
        item
        for match in ANON_USE.finditer(code)
        for item in _top_level_use_items(code, match.end() - 1)
    ]
    for item in anonymous_items:
        alias = SIMPLE_ALIAS_ITEM.fullmatch(item)
        if alias:
            simple_aliases.append((alias.group(1), alias.group(2)))
        tree = USE_TREE_ITEM.match(item)
        if tree:
            use_trees.append((
                tree.group(1), ["*"] if tree.group(2) == "*"
                else _top_level_use_items(item, tree.end() - 1),
            ))
    while True:
        extended = aliases.copy()
        extended_roots = crate_roots.copy()
        for origin, target in simple_aliases:
            if origin in aliases:
                extended.add(target)
            if origin in crate_roots:
                extended_roots.add(target)
        for origin, items in use_trees:
            if origin not in aliases:
                continue
            for item in items:
                match = GROUP_SELF_ALIAS.fullmatch(item)
                if match:
                    extended.add(match.group(1))
                    if origin in crate_roots:
                        extended_roots.add(match.group(1))
        if extended == aliases and extended_roots == crate_roots:
            break
        aliases = extended
        crate_roots = extended_roots
    qualified = re.compile(
        r"\b(?:" + "|".join(re.escape(alias) for alias in sorted(aliases))
        + r")\s*::\s*" + ADAPTER_NAME
    )
    root_glob = any(origin in crate_roots and "*" in items for origin, items in use_trees)
    grouped_adapter = any(
        origin in aliases and any(re.match(ADAPTER_NAME, item) for item in items)
        for origin, items in use_trees
    )
    anonymous_adapter = any(re.match(ADAPTER_NAME, item) for item in anonymous_items)
    bare_adapter_path = any(
        not code[:match.start()].rstrip().endswith("::")
        for match in BARE_ADAPTER_PATH.finditer(code)
    )
    if (qualified.search(code) or grouped_adapter or anonymous_adapter or root_glob
            or bare_adapter_path or BARE_ADAPTER_IMPORT.search(code)):
        return [f"pure layer depends on adapters: {relative}"]
    return []

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
        allowed_exception = Path("crates/quanta-memory-platform-facade/benches/scaling/mod.rs")
        for source in (root / member).rglob("*.rs"):
            relative = source.relative_to(root)
            text = source.read_text(encoding="utf-8")
            errors.extend(unsafe_exception_errors(text, relative, allowed_exception))
            if (relative.parts[:4] == ("crates", "quanta-memory-platform-core", "src", "domain")
                    or relative.parts[:4] == ("crates", "quanta-memory-platform-core", "src", "ports")):
                errors.extend(pure_layer_adapter_errors(text, relative))
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
