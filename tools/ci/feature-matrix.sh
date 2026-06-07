#!/usr/bin/env bash
# feature-matrix.sh
#
# Per-feature build matrix for the portable memory platform. The one-shot rail
# (scripts/check-workspace.sh) only exercises `--all-features`, which can hide a
# feature-sliced break: a slice that fails to compile (or whose backing leaks a
# symbol) when ENABLED ALONE, or code that silently relies on another feature
# being on. This script checks each crate across the meaningful feature axes so
# such a break is caught BEFORE extraction.
#
# Axes checked, per member crate:
#   - --no-default-features                 (the bare, slice-free surface)
#   - --no-default-features --features <f>  (each individual feature, ALONE)
#   - --all-features                        (everything on together)
#   - --no-default-features --features full (the documented full feature set)
#
# Fail-closed: `set -euo pipefail` means the FIRST failing combo aborts the run
# with a non-zero exit. Each combo is echoed before it runs so the failing
# combination is obvious in the log.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

# Crate -> the individual features to check alone. The `full` aggregate and
# `default` are handled separately below; list only the leaf slices here.
CORE_CRATE="quanta-memory-platform-core"
FACADE_CRATE="quanta-memory-platform-facade"
TESTKIT_CRATE="quanta-memory-platform-testkit"

CORE_FEATURES=(pressure scheduler telemetry)
# Facade leaf slices PLUS the durable/concurrent BACKING features. Building a
# backing alone (`--no-default-features --features spill-redb`) is the only way to
# catch a backing module that fails to compile, leaks a symbol, or silently
# assumes another slice is on; the bare leaf sweep never links the heavy dep. The
# `full-backings` aggregate is swept too, since it is the only combo that links
# every backing at once (a cross-backing feature-unification break shows up there).
# Names are taken verbatim from facade/Cargo.toml — no invented variants.
FACADE_FEATURES=(
  payload_store
  retained_map
  revocable_store
  transcript
  publish_pipeline
  spill
  spill-redb
  revocable-moka
  revocable-scc
  full-backings
)
TESTKIT_FEATURES=(fixtures doctor conformance faults alloc_tracking)

# check_combo <crate> <human-label> <cargo args...>
check_combo() {
  local crate="$1"
  local label="$2"
  shift 2
  echo "==> [${crate}] checking: ${label}"
  cargo check -p "${crate}" --all-targets "$@"
}

# check_crate <crate> <feature...>
# Runs the full per-crate axis sweep, fail-closed on the first failure.
check_crate() {
  local crate="$1"
  shift
  local features=("$@")

  # 1. bare surface: no default, no features.
  check_combo "${crate}" "--no-default-features" --no-default-features

  # 2. each individual feature, enabled ALONE on top of no-default-features.
  local feature
  for feature in "${features[@]}"; do
    check_combo "${crate}" "feature '${feature}' alone" \
      --no-default-features --features "${feature}"
  done

  # 3. the documented full aggregate, alone.
  check_combo "${crate}" "--no-default-features --features full" \
    --no-default-features --features full

  # 4. everything on together (mirrors check-workspace.sh's surface).
  check_combo "${crate}" "--all-features" --all-features
}

echo "feature-matrix: per-crate feature-slice build sweep (fail-closed)"

check_crate "${CORE_CRATE}" "${CORE_FEATURES[@]}"
check_crate "${FACADE_CRATE}" "${FACADE_FEATURES[@]}"
check_crate "${TESTKIT_CRATE}" "${TESTKIT_FEATURES[@]}"

# Whole-workspace bookends: the two extremes must also resolve together, since a
# cross-crate feature unification bug only shows up at the workspace level.
echo "==> [workspace] checking: --workspace --no-default-features"
cargo check --workspace --all-targets --no-default-features
echo "==> [workspace] checking: --workspace --all-features"
cargo check --workspace --all-targets --all-features

echo "feature-matrix: OK (all feature combinations compiled)"
