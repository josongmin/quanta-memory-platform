#!/usr/bin/env python3
"""Fingerprint benchmark build inputs not covered by Cargo.lock or rustc version."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
from typing import Mapping


BUILD_ENV_KEYS = {
    "AR", "CC", "CFLAGS", "CXX", "CXXFLAGS", "RANLIB", "SDKROOT",
    "MACOSX_DEPLOYMENT_TARGET", "RUSTC", "RUSTC_BOOTSTRAP", "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER", "RUSTFLAGS", "RUSTUP_TOOLCHAIN",
    "CARGO_BUILD_RUSTC", "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTFLAGS",
    "CARGO_BUILD_TARGET", "CARGO_ENCODED_RUSTFLAGS", "CARGO_INCREMENTAL",
}


def build_environment(environment: Mapping[str, str]) -> dict[str, str]:
    return {
        key: value for key, value in sorted(environment.items())
        if key in BUILD_ENV_KEYS
        or key.startswith(("CARGO_PROFILE_", "CARGO_UNSTABLE_"))
        or key.startswith("CARGO_TARGET_") and key.endswith(("_RUSTFLAGS", "_LINKER", "_RUNNER"))
    }


def file_digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build_config_digest(
    root: Path, cargo_home: Path, environment: Mapping[str, str],
) -> str:
    manifests = [root / "Cargo.toml", *sorted((root / "crates").glob("*/Cargo.toml"))]
    if len(manifests) < 2:
        raise RuntimeError("benchmark workspace manifests are missing")
    manifest_digests = {
        path.relative_to(root).as_posix(): file_digest(path) for path in manifests
    }
    # Cargo reads directory configs from the invocation directory upward.
    # Preserve precedence without encoding worktree-specific absolute paths.
    directory_configs: list[tuple[int, str, str]] = []
    for depth, directory in enumerate((root, *root.parents)):
        for name in ("config.toml", "config"):
            path = directory / ".cargo" / name
            if path.is_file():
                directory_configs.append((depth, name, file_digest(path)))
    home_configs = {
        name: file_digest(cargo_home / name)
        for name in ("config.toml", "config") if (cargo_home / name).is_file()
    }
    inputs = {
        "manifests": manifest_digests,
        "directory_configs": directory_configs,
        "cargo_home_configs": home_configs,
        "build_environment": build_environment(environment),
    }
    encoded = json.dumps(inputs, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    cargo_home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
    if not cargo_home.is_absolute():
        cargo_home = root / cargo_home
    print(build_config_digest(root, cargo_home, os.environ))


if __name__ == "__main__":
    main()
