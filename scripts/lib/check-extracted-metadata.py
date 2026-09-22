#!/usr/bin/env python3
"""Reject Cargo graphs that are not self-contained QMP workspaces."""

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


def check_metadata(metadata: dict, root: Path) -> None:
    root = root.resolve()
    members = set(metadata["workspace_members"])
    member_names = {
        package["name"] for package in metadata["packages"] if package["id"] in members
    }
    if member_names != EXPECTED_MEMBERS:
        raise ValueError(f"unexpected workspace members: {sorted(member_names)}")

    packages_by_id = {package["id"]: package for package in metadata["packages"]}
    for member_id in members:
        package = packages_by_id[member_id]
        forbidden = FORBIDDEN_EDGES.get(package["name"], set())
        for dependency in package.get("dependencies", []):
            if dependency["name"] in forbidden:
                raise ValueError(
                    "forbidden dependency direction: "
                    f"{package['name']} -> {dependency['name']}"
                )

    for package in metadata["packages"]:
        name = package["name"]
        if name in FORBIDDEN_PACKAGES or name.startswith("semantica-"):
            raise ValueError(f"forbidden host dependency: {name}")
        if package["source"] is None:
            manifest = Path(package["manifest_path"]).resolve()
            if not manifest.is_relative_to(root):
                raise ValueError(f"external path dependency: {manifest}")
            if package["id"] not in members:
                raise ValueError(f"unexpected local package: {manifest}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: check-extracted-metadata.py <workspace-root>")
    try:
        check_metadata(json.load(sys.stdin), Path(sys.argv[1]))
    except ValueError as error:
        raise SystemExit(f"extraction metadata rejected: {error}") from error
