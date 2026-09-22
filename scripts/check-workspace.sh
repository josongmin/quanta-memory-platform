#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

echo "[check-workspace] fmt --check"
cargo fmt --all --check

echo "[check-workspace] clippy -D warnings"
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

echo "[check-workspace] regression-guards"
scripts/regression-guards.sh

echo "[check-workspace] cargo check --workspace --all-features"
cargo check --locked --workspace --all-features

echo "[check-workspace] cargo build --examples --all-features"
cargo build --locked --examples --all-features

echo "[check-workspace] feature-matrix"
tools/ci/feature-matrix.sh

echo "[check-workspace] advertised feature runtime capabilities"
scripts/verify-feature-capabilities.sh

echo "[check-workspace] documented test selectors"
python3 -m unittest -v scripts/tests/test_doc_test_selectors.py
scripts/verify-doc-test-selectors.py

if ! command -v cargo-deny >/dev/null 2>&1; then
  echo "check-workspace: cargo-deny unavailable" >&2
  exit 1
fi
echo "[check-workspace] cargo deny"
cargo deny --locked --all-features check
