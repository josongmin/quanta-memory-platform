#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

mode="full"
if [[ "${1:-}" == "--in-memory" ]]; then
  mode="in-memory"
  shift
fi
if [[ "$#" -ne 0 ]]; then
  echo "usage: scripts/check-workspace.sh [--in-memory]" >&2
  exit 2
fi

in_memory_features="quanta-memory-platform-core/full,quanta-memory-platform-facade/full,quanta-memory-platform-testkit/full"
# The capability rail also exercises the explicit unbudgeted fixture slice.
# Check the union so no selected test can quietly pull in Redb.
in_memory_graph_features="${in_memory_features},quanta-memory-platform-facade/unbudgeted-stores"
workspace_test_output=""
regression_output=""
regression_pid=""
shell_script_list=""
cleanup() {
  if [[ -n "${regression_pid}" ]]; then
    wait "${regression_pid}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${regression_output}" && -f "${regression_output}" ]]; then
    cat "${regression_output}"
  fi
  if [[ -n "${workspace_test_output}" && -f "${workspace_test_output}" ]]; then
    rm -f -- "${workspace_test_output}"
  fi
  [[ -z "${regression_output}" ]] || rm -f -- "${regression_output}"
  if [[ -n "${shell_script_list}" && -f "${shell_script_list}" ]]; then
    rm -f -- "${shell_script_list}"
  fi
}
trap cleanup EXIT

echo "[check-workspace] fmt --check"
cargo fmt --all --check

echo "[check-workspace] shellcheck"
shell_scripts=()
shell_script_list="$(mktemp /tmp/qmp-shell-scripts.XXXXXX)"
find scripts -type f -name '*.sh' -print0 >"${shell_script_list}"
while IFS= read -r -d '' script; do
  shell_scripts+=("${script}")
done <"${shell_script_list}"
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
if [[ "${mode}" == "full" ]]; then
  metadata_features=(--all-features)
else
  metadata_features=(--no-default-features --features "${in_memory_graph_features}")
fi
workspace_metadata="$(cargo metadata --locked --format-version 1 "${metadata_features[@]}")"
if [[ "${mode}" == "in-memory" ]]; then
  python3 scripts/lib/check-extracted-metadata.py "${repo_root}" \
    --forbid-package redb <<<"${workspace_metadata}"
else
  python3 scripts/lib/check-extracted-metadata.py "${repo_root}" \
    <<<"${workspace_metadata}"
fi

echo "[check-workspace] workspace policy"
python3 scripts/verify-workspace-policy.py --metadata-stdin <<<"${workspace_metadata}"

if [[ "${mode}" == "full" ]]; then
  # Crash tests build child examples with --offline to prove binary freshness.
  # Fetch host dependencies before those tests so a cold cache fails here
  # instead of after compiling and running most of the workspace suite.
  echo "[check-workspace] host dependency cache"
  host_target="$(rustc -vV | sed -n 's/^host: //p')"
  if [[ -z "${host_target}" ]]; then
    echo "check-workspace: could not determine Rust host target" >&2
    exit 1
  fi
  cargo fetch --locked --target "${host_target}"
fi

echo "[check-workspace] regression-guards (parallel with Python contract tests)"
regression_output="$(mktemp /tmp/qmp-regression-guards.XXXXXX)"
scripts/regression-guards.sh >"${regression_output}" 2>&1 &
regression_pid=$!

echo "[check-workspace] Python contract tests"
python3 scripts/lib/run_python_contracts.py scripts
python3 scripts/lib/run_python_contracts.py pm
scripts/tests/extraction-contracts.sh

regression_status=0
wait "${regression_pid}" || regression_status=$?
regression_pid=""
cat "${regression_output}"
rm -f -- "${regression_output}"
regression_output=""
if [[ "${regression_status}" -ne 0 ]]; then
  echo "check-workspace: regression-guards failed (${regression_status})" >&2
  exit "${regression_status}"
fi

if [[ "${mode}" == "full" ]]; then
  echo "[check-workspace] feature-matrix"
  scripts/verify-feature-matrix.sh

  echo "[check-workspace] cargo test --workspace --all-features"
  workspace_test_output="$(mktemp /tmp/qmp-workspace-tests.XXXXXX)"
  cargo test --locked --workspace --all-features 2>&1 | tee "${workspace_test_output}"
  python3 scripts/lib/verify_rust_test_output.py "${workspace_test_output}"

  echo "[check-workspace] advertised feature runtime capabilities"
  scripts/verify-feature-capabilities.sh

  echo "[check-workspace] documented test selectors"
  scripts/verify-doc-test-selectors.py --test-output "${workspace_test_output}"
else
  echo "[check-workspace] in-memory workspace Clippy"
  cargo clippy --locked --workspace --all-targets --no-default-features \
    --features "${in_memory_features}" -- -D warnings

  echo "[check-workspace] in-memory workspace tests"
  workspace_test_output="$(mktemp /tmp/qmp-workspace-tests.XXXXXX)"
  cargo test --locked --workspace --no-default-features \
    --features "${in_memory_features}" 2>&1 | tee "${workspace_test_output}"
  python3 scripts/lib/verify_rust_test_output.py "${workspace_test_output}"

  echo "[check-workspace] in-memory feature capabilities"
  scripts/verify-feature-capabilities.sh --in-memory
fi

echo "[check-workspace] generated guidance"
python3 tools/pm/pm.py lint

if ! command -v cargo-deny >/dev/null 2>&1; then
  echo "check-workspace: cargo-deny unavailable" >&2
  exit 1
fi
echo "[check-workspace] cargo deny"
cargo deny --locked --all-features check -D warnings

if [[ "${mode}" == "full" ]]; then
  echo "check-workspace: clean (full repository gate)"
else
  echo "check-workspace: clean (in-memory scope only; full repository gate not run)"
fi
