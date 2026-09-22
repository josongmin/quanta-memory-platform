#!/usr/bin/env python3
"""Freeze and verify the exact Git tree exported by the standalone extractor."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
from typing import Any


SCHEMA = "qmp-extraction-manifest-v1"
SUPPORTED_MODES = {"100644", "100755", "120000"}


class ManifestError(RuntimeError):
    pass


def git(repo: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=check,
    )


def resolve_commit(repo: Path, revision: str) -> str:
    return git(repo, "rev-parse", "--verify", f"{revision}^{{commit}}").stdout.decode().strip()


def tree_id(repo: Path, commit: str) -> str:
    return git(repo, "rev-parse", f"{commit}^{{tree}}").stdout.decode().strip()


def digest_fields(fields: list[bytes]) -> str:
    digest = hashlib.sha256()
    for field in fields:
        digest.update(len(field).to_bytes(8, "big"))
        digest.update(field)
    return digest.hexdigest()


def manifest_digest(manifest: dict[str, Any]) -> str:
    fields = [SCHEMA.encode(), manifest["tree"].encode()]
    for entry in sorted(manifest["entries"], key=lambda item: item["path"]):
        fields.extend(
            [
                entry["mode"].encode(),
                entry["oid"].encode(),
                os.fsencode(entry["path"]),
                entry["content_sha256"].encode(),
            ]
        )
    return digest_fields(fields)


def path_is_excluded(path: str, excludes: list[str]) -> bool:
    return any(
        path == exclude or path.startswith(exclude.rstrip("/") + "/")
        for exclude in excludes
    )


def build_manifest(
    repo: Path, revision: str, roots: list[str], excludes: list[str] | None = None
) -> dict[str, Any]:
    excludes = excludes or []
    commit = resolve_commit(repo, revision)
    command = ["ls-tree", "-r", "-z", "--full-tree", commit, "--", *roots]
    raw = git(repo, *command).stdout
    entries: list[dict[str, str]] = []
    for record in raw.split(b"\0"):
        if not record:
            continue
        metadata, raw_path = record.split(b"\t", 1)
        mode, object_type, oid = metadata.decode().split()
        path = os.fsdecode(raw_path)
        if path_is_excluded(path, excludes):
            continue
        if object_type != "blob" or mode not in SUPPORTED_MODES:
            raise ManifestError(
                f"unsupported extraction entry {os.fsdecode(raw_path)!r}: {mode} {object_type}"
            )
        content = git(repo, "cat-file", "blob", oid).stdout
        entries.append(
            {
                "mode": mode,
                "oid": oid,
                "path": path,
                "content_sha256": hashlib.sha256(content).hexdigest(),
            }
        )
    if not entries:
        raise ManifestError("extraction manifest is empty")
    manifest: dict[str, Any] = {
        "schema": SCHEMA,
        "commit": commit,
        "tree": tree_id(repo, commit),
        "entries": entries,
    }
    manifest["digest"] = manifest_digest(manifest)
    return manifest


def load_manifest(path: Path) -> dict[str, Any]:
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if manifest.get("schema") != SCHEMA:
        raise ManifestError(f"unsupported manifest schema: {manifest.get('schema')!r}")
    if manifest.get("digest") != manifest_digest(manifest):
        raise ManifestError("manifest digest does not match its entries")
    return manifest


def filesystem_mode(path: Path) -> str:
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode):
        return "120000"
    if not stat.S_ISREG(metadata.st_mode):
        raise ManifestError(f"extracted path is not a regular file or symlink: {path}")
    return "100755" if metadata.st_mode & stat.S_IXUSR else "100644"


def filesystem_content(path: Path, mode: str) -> bytes:
    if mode == "120000":
        return os.fsencode(os.readlink(path))
    return path.read_bytes()


def verify_destination(manifest: dict[str, Any], destination: Path) -> None:
    for entry in manifest["entries"]:
        path = destination / entry["path"]
        if not path.exists() and not path.is_symlink():
            raise ManifestError(f"missing canonical file: {path}")
        actual_mode = filesystem_mode(path)
        if actual_mode != entry["mode"]:
            raise ManifestError(
                f"mode mismatch for {entry['path']}: expected {entry['mode']}, got {actual_mode}"
            )
        actual_digest = hashlib.sha256(filesystem_content(path, actual_mode)).hexdigest()
        if actual_digest != entry["content_sha256"]:
            raise ManifestError(f"content mismatch for {entry['path']}")


def assert_clean(
    repo: Path, commit: str, roots: list[str], excludes: list[str] | None = None
) -> None:
    excludes = excludes or []
    if resolve_commit(repo, "HEAD") != commit:
        raise ManifestError(f"source HEAD moved after freeze: expected {commit}")
    result = git(
        repo,
        "diff",
        "--name-only",
        "--ignore-submodules=none",
        commit,
        "--",
        *roots,
        check=False,
    )
    if result.returncode != 0:
        raise ManifestError(result.stderr.decode(errors="replace").strip())
    changed = [
        os.fsdecode(path)
        for path in result.stdout.splitlines()
        if not path_is_excluded(os.fsdecode(path), excludes)
    ]
    if changed:
        raise ManifestError("tracked extraction input differs from frozen HEAD")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    subparsers = root.add_subparsers(dest="command", required=True)

    snapshot = subparsers.add_parser("snapshot")
    snapshot.add_argument("--repo", type=Path, required=True)
    snapshot.add_argument("--revision", required=True)
    snapshot.add_argument("--output", type=Path, required=True)
    snapshot.add_argument("--exclude", action="append", default=[])
    snapshot.add_argument("paths", nargs="+")

    verify = subparsers.add_parser("verify")
    verify.add_argument("--manifest", type=Path, required=True)
    verify.add_argument("--destination", type=Path, required=True)

    clean = subparsers.add_parser("assert-clean")
    clean.add_argument("--repo", type=Path, required=True)
    clean.add_argument("--commit", required=True)
    clean.add_argument("--exclude", action="append", default=[])
    clean.add_argument("paths", nargs="+")

    paths = subparsers.add_parser("paths")
    paths.add_argument("--manifest", type=Path, required=True)
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "snapshot":
            manifest = build_manifest(
                args.repo.resolve(), args.revision, args.paths, args.exclude
            )
            args.output.write_text(
                json.dumps(manifest, sort_keys=True, separators=(",", ":")),
                encoding="utf-8",
            )
            print(f"commit={manifest['commit']}")
            print(f"tree={manifest['tree']}")
            print(f"digest={manifest['digest']}")
        elif args.command == "verify":
            manifest = load_manifest(args.manifest)
            verify_destination(manifest, args.destination.resolve())
            print(manifest["digest"])
        elif args.command == "assert-clean":
            assert_clean(args.repo.resolve(), args.commit, args.paths, args.exclude)
        elif args.command == "paths":
            manifest = load_manifest(args.manifest)
            for entry in manifest["entries"]:
                sys.stdout.buffer.write(os.fsencode(entry["path"]) + b"\0")
        else:  # pragma: no cover - argparse enforces the command set.
            raise AssertionError(args.command)
    except (ManifestError, OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"extraction manifest rejected: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
