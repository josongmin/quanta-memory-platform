#!/usr/bin/env bash
# extract-to-standalone-repo.sh
#
# MECHANICAL extraction of the portable memory platform into its own git repo.
# Another worker runs this with no folklore: it preserves the package split
# (`core <- facade <- testkit`) and the dependency direction, and it fails closed
# on any cyclic-dependency ambiguity BEFORE copying anything.
#
# What it does NOT move: the Semantica host adapter (`quanta-memory-host`). That
# crate lives in the Semantica monorepo and depends on this repo by path/registry
# AFTER extraction. Moving it here would invert the dependency direction.
#
# Usage:
#   scripts/extract-to-standalone-repo.sh <destination-dir>
#   scripts/extract-to-standalone-repo.sh --dry-run <destination-dir>
set -euo pipefail

dry_run=0
if [[ "${1:-}" == "--dry-run" ]]; then
  dry_run=1
  shift
fi

dest="${1:-}"
if [[ -z "${dest}" ]]; then
  echo "usage: $0 [--dry-run] <destination-dir>" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/lib/self-hosting-manifest.sh"

run() {
  if [[ "${dry_run}" -eq 1 ]]; then
    echo "DRY-RUN: $*"
  else
    "$@"
  fi
}

# ---------------------------------------------------------------------------
# Step 0: fail-closed pre-flight — the split must be acyclic and self-contained.
# ---------------------------------------------------------------------------

MEMBERS=(
  "crates/quanta-memory-platform-core"
  "crates/quanta-memory-platform-facade"
  "crates/quanta-memory-platform-testkit"
)

echo "[0/5] pre-flight: dependency direction + cycle check"
self_hosting_assert_source_set
self_hosting_assert_copy_plan

# 0a. Every member manifest must exist.
for m in "${MEMBERS[@]}"; do
  if [[ ! -f "${m}/Cargo.toml" ]]; then
    echo "FAIL: missing member manifest ${m}/Cargo.toml" >&2
    exit 1
  fi
done

# 0b. Dependency-direction fence: `core` must not depend on facade/testkit;
#     `facade` must not depend on testkit. (core <- facade <- testkit)
core_manifest="$(cat crates/quanta-memory-platform-core/Cargo.toml)"
facade_manifest="$(cat crates/quanta-memory-platform-facade/Cargo.toml)"
if grep -q "quanta-memory-platform-facade\|quanta-memory-platform-testkit" <<<"${core_manifest}"; then
  echo "FAIL: core depends upward on facade/testkit (cyclic / wrong direction)" >&2
  exit 1
fi
if grep -q "quanta-memory-platform-testkit" <<<"${facade_manifest}"; then
  echo "FAIL: facade depends upward on testkit (cyclic / wrong direction)" >&2
  exit 1
fi

# 0c. No member may carry a path dependency that escapes the workspace (would
#     break after extraction). Inter-member path deps must stay relative within
#     `crates/`.
for m in "${MEMBERS[@]}"; do
  if grep -E 'path = "\.\./\.\./\.\./' "${m}/Cargo.toml" >/dev/null 2>&1; then
    echo "FAIL: ${m} has a path dependency escaping the workspace; resolve before extraction" >&2
    exit 1
  fi
done

# 0d. cargo metadata must resolve the workspace with no errors.
cargo metadata --format-version 1 --no-deps >/dev/null

echo "      pre-flight OK: core <- facade <- testkit, no escaping path deps"
echo "      self-hosting OK: canonical extracted file set present, no tools/ci/gates in copy plan"

# ---------------------------------------------------------------------------
# Step 1: create destination and copy the canonical set.
# ---------------------------------------------------------------------------

echo "[1/5] creating destination ${dest}"
run mkdir -p "${dest}"

# The extracted repo carries its own scripts/docs/pm sources. We deliberately
# EXCLUDE `tools/ci/gates/*`: those scripts are Semantica-monorepo rails, not
# standalone repo authority.

echo "[2/5] copying canonical files"
for p in "${SELF_HOSTING_COPY_PATHS[@]}"; do
  if [[ -e "${p}" ]]; then
    run mkdir -p "${dest}/$(dirname "${p}")"
    run cp -R "${p}" "${dest}/${p}"
  fi
done

if [[ "${dry_run}" -eq 0 ]]; then
  self_hosting_assert_extracted_layout "${dest}"
else
  echo "DRY-RUN: assert extracted self-hosting layout (required paths present, tools/ci/gates absent)"
fi

# ---------------------------------------------------------------------------
# Step 3: initialize git in the new repo.
# ---------------------------------------------------------------------------

echo "[3/5] initializing git repo at ${dest}"
run git -C "${dest}" init -q
run git -C "${dest}" add -A
run git -C "${dest}" -c user.name="quanta-memory-platform bootstrap" \
  -c user.email="qmp-bootstrap@local.invalid" \
  commit -q -m "chore: bootstrap extracted quanta-memory-platform repo"

# ---------------------------------------------------------------------------
# Step 4: verify the extracted repo builds in isolation.
# ---------------------------------------------------------------------------

echo "[4/5] verifying extracted workspace builds standalone"
if [[ "${dry_run}" -eq 0 ]]; then
  (
    cd "${dest}"
    scripts/check-workspace.sh
    scripts/memory-platform-doctor
  )
else
  echo "DRY-RUN: (cd ${dest} && scripts/check-workspace.sh && scripts/memory-platform-doctor)"
fi

# ---------------------------------------------------------------------------
# Step 5: print the host-adapter rewire instruction (NOT performed here).
# ---------------------------------------------------------------------------

echo "[5/5] done"
cat <<EOF

Extraction complete. In the extracted repo, the standalone front doors are:

  scripts/check-workspace.sh
  scripts/memory-platform-doctor

Remaining MANUAL step (Semantica monorepo side):

  In packages/analysis/quanta-v2/crates/quanta-memory-host/Cargo.toml, repoint
  the path dependencies from the in-tree sibling checkout to the extracted repo
  (path or git or registry). Dependency direction stays:

      quanta-memory-host  ->  quanta-memory-platform-{core,facade}

  After the rewire, optional Semantica monorepo gates:
      tools/ci/gates/gate-memory-platform-doctor.sh
      tools/ci/gates/gate-memory-platform-lint.sh
EOF
