#!/usr/bin/env bash
# verify-feature-matrix.sh
#
# Per-feature build matrix for the portable memory platform. A single
# `--all-features` build can hide a
# feature-sliced break: a slice that fails to compile (or whose backing leaks a
# symbol) when ENABLED ALONE, or code that silently relies on another feature
# being on. This script checks each crate across the meaningful feature axes so
# such a break is caught BEFORE extraction.
#
# Logical axes covered, per member crate (identical Cargo feature sets run once):
#   - default features                      (the consumer-default surface)
#   - --no-default-features                 (the bare, slice-free surface)
#   - --no-default-features --features <f>  (each individual feature, ALONE)
#   - --no-default-features --features full (the documented full feature set)
#   - workspace --all-features              (everything on together)
#
# Fail-closed: `set -euo pipefail` means the FIRST failing combo aborts the run
# with a non-zero exit. Each combo is echoed before it runs so the failing
# combination is obvious in the log.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

# Cargo metadata is the feature-list authority. Every public feature except the
# `default` and `full` aggregates is checked alone or by an identical singleton
# default, so a new feature cannot be added to a manifest without entering this
# matrix automatically.
CORE_CRATE="quanta-memory-platform-core"
FACADE_CRATE="quanta-memory-platform-facade"
TESTKIT_CRATE="quanta-memory-platform-testkit"

metadata="$(cargo metadata --locked --no-deps --format-version 1)"

crate_features() {
  local crate="$1"
  python3 -c '
import json, sys
crate = sys.argv[1]
metadata = json.load(sys.stdin)
package = next((item for item in metadata["packages"] if item["name"] == crate), None)
if package is None:
    raise SystemExit(f"missing workspace package: {crate}")
print("\n".join(sorted(name for name in package["features"] if name not in {"default", "full"})))
' "${crate}" <<<"${metadata}"
}

crate_default_features() {
  local crate="$1"
  python3 -c '
import json, sys
crate = sys.argv[1]
metadata = json.load(sys.stdin)
package = next((item for item in metadata["packages"] if item["name"] == crate), None)
if package is None:
    raise SystemExit(f"missing workspace package: {crate}")
print("\n".join(package["features"].get("default", [])))
' "${crate}" <<<"${metadata}"
}

# check_combo <crate> <human-label> <cargo args...>
check_combo() {
  local crate="$1"
  local label="$2"
  shift 2
  echo "==> [${crate}] checking: ${label}"
  cargo clippy --locked -p "${crate}" --all-targets "$@" -- -D warnings
}

# check_crate <crate> <feature...>
# Runs the full per-crate axis sweep, fail-closed on the first failure.
check_crate() {
  local crate="$1"
  local features=()
  local default_features=()
  while IFS= read -r feature; do
    [[ -n "${feature}" ]] && features+=("${feature}")
  done < <(crate_features "${crate}")
  while IFS= read -r feature; do
    [[ -n "${feature}" ]] && default_features+=("${feature}")
  done < <(crate_default_features "${crate}")

  # 1. consumer-default surface.
  check_combo "${crate}" "default features"

  # 2. bare surface: no default, no features. An empty default is the same
  # Cargo feature set and is already covered by step 1.
  if [[ "${#default_features[@]}" -eq 0 ]]; then
    echo "==> [${crate}] covered without rerun: --no-default-features (default is empty)"
  else
    check_combo "${crate}" "--no-default-features" --no-default-features
  fi

  # 3. Each individual feature, enabled ALONE on top of no-default-features.
  # A singleton default is the same Cargo feature set and is not run twice.
  local feature
  for feature in "${features[@]}"; do
    if [[ "${#default_features[@]}" -eq 1 ]] && \
       [[ "${feature}" == "${default_features[0]}" ]]; then
      echo "==> [${crate}] covered without rerun: feature '${feature}' alone (singleton default)"
      continue
    fi
    check_combo "${crate}" "feature '${feature}' alone" \
      --no-default-features --features "${feature}"
  done

  # 4. the documented full aggregate, alone.
  check_combo "${crate}" "--no-default-features --features full" \
    --no-default-features --features full

}

echo "feature-matrix: per-crate feature-slice build sweep (fail-closed)"

check_crate "${CORE_CRATE}"
check_crate "${FACADE_CRATE}"
check_crate "${TESTKIT_CRATE}"

# Facade's no-default graph must not activate core's default/live-control
# surface through dependency defaults. Compilation alone cannot detect this
# feature leak because the extra symbols still compile successfully.
facade_bare_tree="$(cargo tree --locked -p "${FACADE_CRATE}" --no-default-features -e features)"
if rg -q 'quanta-memory-platform-core feature "(default|live_control)"' <<<"${facade_bare_tree}"; then
  echo "feature-matrix: facade --no-default-features leaked core default/live_control" >&2
  printf '%s\n' "${facade_bare_tree}" >&2
  exit 1
fi

# Production/default aggregates must not expose the explicit unbudgeted escape
# hatch. Resolve the graph instead of parsing Cargo.toml text: dependency feature
# forwarding and renamed edges are visible only in Cargo's resolved graph.
assert_facade_escape_hatch_absent() {
  local label="$1"
  shift
  local tree
  tree="$(cargo tree --locked -p "${FACADE_CRATE}" "$@" -e features -i "${FACADE_CRATE}")"
  if rg -q 'quanta-memory-platform-facade feature "unbudgeted-stores"' <<<"${tree}"; then
    echo "feature-matrix: facade ${label} must not enable unbudgeted-stores" >&2
    printf '%s\n' "${tree}" >&2
    exit 1
  fi
}

assert_facade_escape_hatch_absent "default features"
assert_facade_escape_hatch_absent "feature 'full'" --no-default-features --features full
assert_facade_escape_hatch_absent \
  "feature 'full-backings'" --no-default-features --features full-backings

# Every crate's bare all-target surface was checked above. Their union is the
# workspace no-default surface because all intra-workspace dependency edges
# disable default features, so rerunning that union adds no coverage. Keep the
# all-features workspace run because it uniquely catches feature unification.
echo "==> [workspace] checking: --workspace --all-features"
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

echo "feature-matrix: OK (all feature combinations compiled warning-free)"
