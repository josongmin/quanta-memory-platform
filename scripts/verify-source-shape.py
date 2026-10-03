#!/usr/bin/env python3
"""Reject Rust source files above the post-split 1,000-line ceiling."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path


NEW_FILE_MAX_LINES = 1_000
ADVISORY_LINES = 600
RAW_LITERAL = re.compile(r"(?:br|rb|r)(#*)\"")
CHAR_LITERAL = re.compile(r"(?:b)?'(?:\\.|[^\\'\n])'")
def _unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _code_only(source: str) -> str:
    """Mask Rust comments and literals while preserving code positions."""
    chars = list(source)

    def mask(start: int, end: int) -> None:
        for offset in range(start, end):
            if chars[offset] != "\n":
                chars[offset] = " "

    offset = 0
    while offset < len(source):
        if source.startswith("//", offset):
            end = source.find("\n", offset)
            end = len(source) if end == -1 else end
            mask(offset, end)
            offset = end
            continue
        if source.startswith("/*", offset):
            start = offset
            depth = 1
            offset += 2
            while offset < len(source) and depth:
                if source.startswith("/*", offset):
                    depth += 1
                    offset += 2
                elif source.startswith("*/", offset):
                    depth -= 1
                    offset += 2
                else:
                    offset += 1
            mask(start, offset)
            continue

        raw = RAW_LITERAL.match(source, offset) if source[offset] in "br" else None
        if raw and (offset == 0 or not (source[offset - 1].isalnum() or source[offset - 1] == "_")):
            start = offset
            terminator = '"' + raw.group(1)
            end = source.find(terminator, offset + len(raw.group()))
            offset = len(source) if end == -1 else end + len(terminator)
            mask(start, offset)
            continue

        char_literal = CHAR_LITERAL.match(source, offset) if source[offset] in "b'" else None
        if char_literal and (offset == 0 or not (source[offset - 1].isalnum() or source[offset - 1] == "_")):
            end = char_literal.end()
            mask(offset, end)
            offset = end
            continue

        if source[offset] == '"':
            start = offset
            offset += 1
            while offset < len(source):
                if source[offset] == "\\":
                    offset += 2
                elif source[offset] == '"':
                    offset += 1
                    break
                else:
                    offset += 1
            mask(start, min(offset, len(source)))
            continue
        offset += 1
    return "".join(chars)


def _bypass_errors(source: str, path: str) -> list[str]:
    code = _code_only(source)
    errors: list[str] = []
    if re.search(r"\b(?:r#)?include\s*!\s*[({\[]", code):
        errors.append(f"{path}: include! bypasses file-size and module ownership checks")
    for attribute in re.finditer(r"#\s*!?\s*\[", code):
        cursor = attribute.end()
        depth = 1
        while cursor < len(code) and depth:
            if code[cursor] == "[":
                depth += 1
            elif code[cursor] == "]":
                depth -= 1
            cursor += 1
        if re.search(r"\bpath\s*=", code[attribute.end():cursor]):
            errors.append(f"{path}: #[path] bypasses module ownership checks")
            break
    return errors


def _rust_files(root: Path) -> tuple[dict[str, Path], list[str]]:
    crates = root / "crates"
    if not crates.is_dir() or crates.is_symlink():
        return {}, ["crates directory is missing or is a symlink"]
    files: dict[str, Path] = {}
    errors: list[str] = []
    for directory, dirs, names in os.walk(crates, followlinks=False):
        for name in sorted(dirs + names):
            path = Path(directory) / name
            relative = path.relative_to(root).as_posix()
            if path.is_symlink():
                errors.append(f"{relative}: symlinks under crates are not allowed")
        dirs[:] = sorted(name for name in dirs if not (Path(directory) / name).is_symlink())
        for name in sorted(names):
            path = Path(directory) / name
            if path.suffix == ".rs" and not path.is_symlink():
                files[path.relative_to(root).as_posix()] = path
    if not files:
        errors.append("no Rust files found under crates")
    return files, errors


def verify(root: Path, baseline_path: Path) -> tuple[list[str], list[str]]:
    """Return errors and sorted advisory lines; all checks use the working tree."""
    errors: list[str] = []
    advisory: list[str] = []
    try:
        baseline = json.loads(baseline_path.read_text(encoding="utf-8"), object_pairs_hook=_unique_object)
    except (OSError, UnicodeError, ValueError) as error:
        return [f"baseline cannot be read: {error}"], advisory
    if not isinstance(baseline, dict) or set(baseline) != {"version", "grandfathered"}:
        return ["baseline must contain only version and grandfathered"], advisory
    if type(baseline["version"]) is not int or baseline["version"] != 1:
        errors.append("baseline version must be 1")
    limits = baseline["grandfathered"]
    if not isinstance(limits, dict):
        return [*errors, "grandfathered must be an object"], advisory
    # The split at 14cf034 eliminated every oversized source file. Keep the
    # baseline schema for old checkouts, but never let an old path regain an
    # exception: a single-commit extracted checkout has no trusted base to
    # compare against a lower historical limit.
    if limits:
        errors.append("grandfathered must remain empty after the source split")

    files, file_errors = _rust_files(root)
    errors.extend(file_errors)

    for path, source_path in sorted(files.items()):
        try:
            data = source_path.read_bytes()
            source = data.decode("utf-8")
        except (OSError, UnicodeError) as error:
            errors.append(f"{path}: source cannot be read as UTF-8: {error}")
            continue
        lines = data.count(b"\n") + bool(data and not data.endswith(b"\n"))
        if lines > NEW_FILE_MAX_LINES:
            errors.append(f"{path}: {lines} lines exceeds {NEW_FILE_MAX_LINES}-line ceiling")
        if lines >= ADVISORY_LINES:
            advisory.append(f"{path}: {lines} lines (advisory at {ADVISORY_LINES})")
        errors.extend(_bypass_errors(source, path))
    return errors, advisory


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--baseline", type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    baseline = args.baseline or root / "scripts/source-shape-baseline.json"
    errors, advisory = verify(root, baseline)
    for item in advisory:
        print(f"source-shape advisory: {item}")
    for item in errors:
        print(f"source-shape rejected: {item}", file=sys.stderr)
    if errors:
        return 1
    print(f"source-shape: OK ({len(advisory)} files at or above {ADVISORY_LINES} lines)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
