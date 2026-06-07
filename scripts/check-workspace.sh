#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

echo "[check-workspace] fmt --check"
cargo fmt --all --check

echo "[check-workspace] clippy -D warnings"
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo "[check-workspace] regression-guards"
scripts/regression-guards.sh

echo "[check-workspace] cargo check --workspace --all-features"
cargo check --workspace --all-features

echo "[check-workspace] cargo build --examples --all-features"
cargo build --examples --all-features

echo "[check-workspace] feature-matrix"
tools/ci/feature-matrix.sh

if command -v cargo-deny >/dev/null 2>&1; then
  echo "[check-workspace] cargo deny"
  cargo deny --all-features check
else
  echo "check-workspace: cargo-deny unavailable; skipping deny check" >&2
fi
