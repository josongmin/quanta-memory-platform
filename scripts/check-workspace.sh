#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

echo "[check-workspace] fmt --check"
cargo fmt --all --check

echo "[check-workspace] shell syntax"
while IFS= read -r -d '' script; do
  bash -n "${script}"
done < <(find scripts -type f -name '*.sh' -print0)

echo "[check-workspace] resolved workspace graph"
cargo metadata --locked --format-version 1 | \
  python3 scripts/lib/check-extracted-metadata.py "${repo_root}"

echo "[check-workspace] workspace policy"
python3 scripts/verify-workspace-policy.py

echo "[check-workspace] Python contract tests"
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
python3 -m unittest -q tools/pm/tests/test_pm.py
scripts/tests/extraction-contracts.sh

echo "[check-workspace] regression-guards"
scripts/regression-guards.sh

echo "[check-workspace] feature-matrix"
scripts/verify-feature-matrix.sh

echo "[check-workspace] cargo test --workspace --all-features"
cargo test --locked --workspace --all-features

echo "[check-workspace] advertised feature runtime capabilities"
scripts/verify-feature-capabilities.sh

echo "[check-workspace] documented test selectors"
scripts/verify-doc-test-selectors.py

echo "[check-workspace] generated guidance"
python3 tools/pm/pm.py lint

if ! command -v cargo-deny >/dev/null 2>&1; then
  echo "check-workspace: cargo-deny unavailable" >&2
  exit 1
fi
echo "[check-workspace] cargo deny"
cargo deny --locked --all-features check -D warnings

echo "check-workspace: clean"
