#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

workspace_test_output=""
cleanup() {
  if [[ -n "${workspace_test_output}" && -f "${workspace_test_output}" ]]; then
    rm -f -- "${workspace_test_output}"
  fi
}
trap cleanup EXIT

echo "[check-workspace] fmt --check"
cargo fmt --all --check

echo "[check-workspace] shellcheck"
shell_scripts=()
while IFS= read -r -d '' script; do
  shell_scripts+=("${script}")
done < <(find scripts -type f -name '*.sh' -print0)
if ! command -v shellcheck >/dev/null 2>&1; then
  echo "check-workspace: shellcheck unavailable" >&2
  exit 1
fi
if [[ "${#shell_scripts[@]}" -eq 0 ]]; then
  echo "check-workspace: no shell scripts found" >&2
  exit 1
fi
shellcheck "${shell_scripts[@]}"

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

echo "[check-workspace] production error-discard and lint-exception policy"
cargo clippy --locked -p quanta-memory-platform-core \
  -p quanta-memory-platform-facade --lib --all-features -- \
  -D warnings -D clippy::let_underscore_must_use \
  -D clippy::allow_attributes_without_reason

echo "[check-workspace] feature-matrix"
scripts/verify-feature-matrix.sh

echo "[check-workspace] cargo test --workspace --all-features"
workspace_test_output="$(mktemp /tmp/qmp-workspace-tests.XXXXXX)"
cargo test --locked --workspace --all-features 2>&1 | tee "${workspace_test_output}"

echo "[check-workspace] advertised feature runtime capabilities"
scripts/verify-feature-capabilities.sh

echo "[check-workspace] documented test selectors"
scripts/verify-doc-test-selectors.py --test-output "${workspace_test_output}"

echo "[check-workspace] generated guidance"
python3 tools/pm/pm.py lint

if ! command -v cargo-deny >/dev/null 2>&1; then
  echo "check-workspace: cargo-deny unavailable" >&2
  exit 1
fi
echo "[check-workspace] cargo deny"
cargo deny --locked --all-features check -D warnings

echo "check-workspace: clean"
