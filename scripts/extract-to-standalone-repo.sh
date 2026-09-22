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
if [[ -z "${dest}" ]] || [[ $# -ne 1 ]]; then
  echo "usage: $0 [--dry-run] <destination-dir>" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/lib/self-hosting-manifest.sh"

SELF_HOSTING_EXTRACTION_PATHS=("${SELF_HOSTING_COPY_PATHS[@]}")

dest="$(python3 -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).resolve())' "${dest}")"
if [[ "${dest}" == "${repo_root}" ]] || [[ "${dest}" == "${repo_root}/"* ]] || \
   [[ "${repo_root}" == "${dest}/"* ]]; then
  echo "FAIL: destination overlaps source repository: ${dest}" >&2
  exit 2
fi
if [[ -e "${dest}" ]] && [[ -n "$(find "${dest}" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
  echo "FAIL: destination must be absent or empty: ${dest}" >&2
  exit 2
fi

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

# 0d. The lockfile and full local dependency graph must resolve without
# mutation, and no local package may escape this workspace.
cargo metadata --locked --format-version 1 | \
  python3 scripts/lib/check-extracted-metadata.py "${repo_root}"

# Freeze the exact committed source tree. The manifest binds tree ID, Git mode,
# blob ID, path, and a content SHA-256. Relevant staged, unstaged, or mode-only
# drift is rejected rather than silently copied under a misleading HEAD claim.
source_head="$(git rev-parse --verify 'HEAD^{commit}')"
source_tree="$(git rev-parse "${source_head}^{tree}")"
tree_manifest="$(mktemp /tmp/qmp-extract-tree.XXXXXX)"
path_manifest="$(mktemp /tmp/qmp-extract-paths.XXXXXX)"
trap 'rm -f "${tree_manifest}" "${path_manifest}"' EXIT
python3 scripts/lib/extraction_manifest.py assert-clean \
  --repo "${repo_root}" --commit "${source_head}" \
  "${SELF_HOSTING_COPY_PATHS[@]}"
manifest_output="$(python3 scripts/lib/extraction_manifest.py snapshot \
  --repo "${repo_root}" --revision "${source_head}" --output "${tree_manifest}" \
  "${SELF_HOSTING_COPY_PATHS[@]}")"
source_digest="$(sed -n 's/^digest=//p' <<<"${manifest_output}")"
if [[ -z "${source_digest}" ]]; then
  echo "FAIL: extraction manifest did not emit a canonical digest" >&2
  exit 1
fi
python3 scripts/lib/extraction_manifest.py paths --manifest "${tree_manifest}" > "${path_manifest}"

echo "      pre-flight OK: core <- facade <- testkit, no escaping path deps"
echo "      self-hosting OK: canonical extracted file set present, no tools/ci/gates in copy plan"
echo "      frozen HEAD: ${source_head}"
echo "      frozen tree: ${source_tree}"

# ---------------------------------------------------------------------------
# Step 1: create destination and copy the canonical set.
# ---------------------------------------------------------------------------

echo "[1/5] creating destination ${dest}"
run mkdir -p "${dest}"

# The extracted repo carries its own scripts/docs/pm sources. We deliberately
# EXCLUDE `tools/ci/gates/*`: those scripts are Semantica-monorepo rails, not
# standalone repo authority.

echo "[2/5] copying canonical files"
if [[ "${dry_run}" -eq 0 ]]; then
  git archive --format=tar "${source_head}" -- "${SELF_HOSTING_EXTRACTION_PATHS[@]}" | tar -xf - -C "${dest}"
else
  echo "DRY-RUN: export frozen tree ${source_tree} from HEAD ${source_head}"
fi

if [[ "${dry_run}" -eq 0 ]]; then
  self_hosting_assert_extracted_layout "${dest}"
  extracted_digest="$(python3 scripts/lib/extraction_manifest.py verify \
    --manifest "${tree_manifest}" --destination "${dest}")"
  if [[ "${source_digest}" != "${extracted_digest}" ]]; then
    echo "FAIL: source/extracted canonical digest mismatch" >&2
    exit 1
  fi
  echo "      canonical SHA-256: ${source_digest}"
else
  echo "DRY-RUN: assert extracted layout, modes, and canonical SHA-256 ${source_digest}"
fi

# ---------------------------------------------------------------------------
# Step 3: verify the extracted repo BEFORE creating any success commit.
# ---------------------------------------------------------------------------

echo "[3/5] verifying extracted workspace builds standalone"
if [[ "${dry_run}" -eq 0 ]]; then
  (
    cd "${dest}"
    cargo metadata --locked --format-version 1 | \
      python3 scripts/lib/check-extracted-metadata.py "${dest}"
    python3 -m unittest -v \
      scripts/tests/test_extraction_metadata.py \
      scripts/tests/test_extraction_manifest.py
    scripts/tests/extraction-contracts.sh
    scripts/scaffold-external-repo.sh
    scripts/check-workspace.sh
    scripts/memory-platform-doctor
    python3 tools/pm/pm.py lint
  )
  if [[ "$(python3 scripts/lib/extraction_manifest.py verify \
    --manifest "${tree_manifest}" --destination "${dest}")" != "${source_digest}" ]]; then
    echo "FAIL: verification changed canonical extracted files" >&2
    exit 1
  fi
else
  echo "DRY-RUN: (cd ${dest} && metadata --locked + scaffold + check-workspace + doctor + pm lint)"
fi

# ---------------------------------------------------------------------------
# Step 4: commit only the verified, canonical file set and assert clean state.
# ---------------------------------------------------------------------------

echo "[4/5] initializing verified git repo at ${dest}"
python3 scripts/lib/extraction_manifest.py assert-clean \
  --repo "${repo_root}" --commit "${source_head}" \
  "${SELF_HOSTING_COPY_PATHS[@]}"
run git -C "${dest}" init -q
run git -C "${dest}" add --pathspec-from-file="${path_manifest}" --pathspec-file-nul
if [[ "${dry_run}" -eq 0 ]] && \
   git -C "${dest}" status --porcelain --untracked-files=all | grep -q '^?? '; then
  echo "FAIL: verification left non-canonical untracked files in destination" >&2
  exit 1
fi
run git -C "${dest}" -c user.name="quanta-memory-platform bootstrap" \
  -c user.email="qmp-bootstrap@local.invalid" \
  commit -q -m "chore: bootstrap extracted quanta-memory-platform repo"
if [[ "${dry_run}" -eq 0 ]] && [[ -n "$(git -C "${dest}" status --porcelain)" ]]; then
  echo "FAIL: extracted git worktree is dirty after bootstrap" >&2
  exit 1
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
