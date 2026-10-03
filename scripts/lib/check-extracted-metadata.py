#!/usr/bin/env python3
"""Reject Cargo graphs that are not self-contained QMP workspaces."""

import argparse
import json
from pathlib import Path
import sys


EXPECTED_MEMBERS = {
    "quanta-memory-platform-core",
    "quanta-memory-platform-facade",
    "quanta-memory-platform-testkit",
}
FORBIDDEN_PACKAGES = {"quanta-memory-host", "quanta-runtime", "semantica"}
FORBIDDEN_EDGES = {
    "quanta-memory-platform-core": {
        "quanta-memory-platform-facade",
        "quanta-memory-platform-testkit",
    },
    "quanta-memory-platform-facade": {"quanta-memory-platform-testkit"},
}
REQUIRED_EDGES = {
    "quanta-memory-platform-facade": {"quanta-memory-platform-core"},
    "quanta-memory-platform-testkit": {
        "quanta-memory-platform-core",
        "quanta-memory-platform-facade",
    },
}


def check_metadata(
    metadata: dict, root: Path, extra_forbidden_packages: frozenset[str] = frozenset()
) -> None:
    root = root.resolve()
    if Path(metadata["workspace_root"]).resolve() != root:
        raise ValueError("metadata workspace root differs from extraction root")

    member_ids = metadata["workspace_members"]
    members = set(member_ids)
    packages_by_id = {package["id"]: package for package in metadata["packages"]}
    if len(members) != len(member_ids) or len(packages_by_id) != len(metadata["packages"]):
        raise ValueError("duplicate workspace member or package ID")
    if not members.issubset(packages_by_id):
        raise ValueError("workspace member ID is missing from packages")
    member_names = {packages_by_id[id_]["name"] for id_ in members}
    if len(members) != len(EXPECTED_MEMBERS) or member_names != EXPECTED_MEMBERS:
        raise ValueError(f"unexpected workspace members: {sorted(member_names)}")

    # The package name alone is not an identity: an external version can have
    # the same name as a local member. Pin every member to its exact local
    # manifest, then use its package ID to inspect resolved graph edges.
    member_id_by_name = {}
    for member_id in members:
        package = packages_by_id[member_id]
        name = package["name"]
        expected_manifest = root / "crates" / name / "Cargo.toml"
        if (
            package["source"] is not None
            or Path(package["manifest_path"]).resolve() != expected_manifest
        ):
            raise ValueError(f"workspace member is not the local crate: {name}")
        member_id_by_name[name] = member_id

    for member_id in members:
        package = packages_by_id[member_id]
        name = package["name"]
        forbidden = FORBIDDEN_EDGES.get(name, set())
        declared_required = set()
        # Declared rows include optional, dev/build and target-gated edges that
        # may not appear in this particular feature/target resolution.
        for dependency in package.get("dependencies", []):
            target_name = dependency["name"]
            if target_name in forbidden:
                raise ValueError(f"forbidden dependency direction: {name} -> {target_name}")
            if target_name in EXPECTED_MEMBERS:
                expected_path = root / "crates" / target_name
                path = dependency.get("path")
                if (
                    dependency.get("source") is not None
                    or path is None
                    or Path(path).resolve() != expected_path
                ):
                    raise ValueError(
                        f"declared member dependency is not local: {name} -> {target_name}"
                    )
                if (
                    dependency.get("kind") is None
                    and dependency.get("target") is None
                    and not dependency.get("optional", False)
                ):
                    declared_required.add(target_name)
        missing = REQUIRED_EDGES.get(name, set()) - declared_required
        if missing:
            raise ValueError(f"required normal dependency is missing: {name} -> {sorted(missing)}")

    resolved = metadata.get("resolve")
    if not isinstance(resolved, dict) or not isinstance(resolved.get("nodes"), list):
        raise ValueError("resolved Cargo dependency graph is missing")
    nodes_by_id = {node["id"]: node for node in resolved["nodes"]}
    if len(nodes_by_id) != len(resolved["nodes"]) or not members.issubset(nodes_by_id):
        raise ValueError("workspace member is missing from resolved Cargo graph")
    for member_id in members:
        name = packages_by_id[member_id]["name"]
        forbidden = FORBIDDEN_EDGES.get(name, set())
        resolved_required = set()
        for dependency in nodes_by_id[member_id].get("deps", []):
            target_id = dependency["pkg"]
            target = packages_by_id.get(target_id)
            if target is None:
                raise ValueError(f"resolved dependency package ID is missing: {target_id}")
            target_name = target["name"]
            if target_name in forbidden:
                raise ValueError(
                    f"forbidden resolved dependency direction: {name} -> {target_name}"
                )
            if target_name in EXPECTED_MEMBERS and target_id != member_id_by_name[target_name]:
                raise ValueError(f"resolved member dependency is not local: {name} -> {target_name}")
            if target_name in EXPECTED_MEMBERS and any(
                kind.get("kind") is None and kind.get("target") is None
                for kind in dependency.get("dep_kinds", [])
            ):
                resolved_required.add(target_name)
        missing = REQUIRED_EDGES.get(name, set()) - resolved_required
        if missing:
            raise ValueError(f"required resolved dependency is missing: {name} -> {sorted(missing)}")

    for package in metadata["packages"]:
        name = package["name"]
        if name in EXPECTED_MEMBERS and package["id"] != member_id_by_name[name]:
            raise ValueError(f"external duplicate of workspace member: {name}")
        if name in extra_forbidden_packages:
            raise ValueError(f"forbidden selected dependency: {name}")
        if name in FORBIDDEN_PACKAGES or name.startswith("semantica-"):
            raise ValueError(f"forbidden host dependency: {name}")
        if package["source"] is None:
            manifest = Path(package["manifest_path"]).resolve()
            if not manifest.is_relative_to(root):
                raise ValueError(f"external path dependency: {manifest}")
            if package["id"] not in members:
                raise ValueError(f"unexpected local package: {manifest}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("workspace_root", type=Path)
    parser.add_argument("--forbid-package", action="append", default=[])
    args = parser.parse_args()
    try:
        check_metadata(
            json.load(sys.stdin),
            args.workspace_root,
            frozenset(args.forbid_package),
        )
    except ValueError as error:
        raise SystemExit(f"extraction metadata rejected: {error}") from error
