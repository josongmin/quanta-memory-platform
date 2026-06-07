#!/usr/bin/env bash
# Canonical self-hosting file set for the extracted quanta-memory-platform repo.
#
# These are the repo-local paths the standalone workspace must carry so its own
# scripts, docs, and prompt-manager sources remain functional without depending
# on Semantica's monorepo-local `tools/ci/gates/*`.

SELF_HOSTING_COPY_PATHS=(
  "Cargo.toml"
  "README.md"
  "EXTRACTION.md"
  "rustfmt.toml"
  "clippy.toml"
  "deny.toml"
  ".cargo"
  ".gitignore"
  "crates"
  "scripts"
  "tools/pm"
  "tools/ci/feature-matrix.sh"
  "docs/verification-scenarios.md"
)

SELF_HOSTING_REQUIRED_PATHS=(
  "Cargo.toml"
  "README.md"
  "EXTRACTION.md"
  "rustfmt.toml"
  "clippy.toml"
  "deny.toml"
  ".cargo/config.toml"
  "crates/quanta-memory-platform-core/Cargo.toml"
  "crates/quanta-memory-platform-facade/Cargo.toml"
  "crates/quanta-memory-platform-testkit/Cargo.toml"
  "crates/quanta-memory-platform-facade/examples/hello_world.rs"
  "scripts/check-workspace.sh"
  "scripts/extract-to-standalone-repo.sh"
  "scripts/scaffold-external-repo.sh"
  "scripts/memory-platform-doctor"
  "scripts/memory-platform-fix"
  "scripts/memory-platform-report"
  "scripts/lib/self-hosting-manifest.sh"
  "tools/pm/pm.py"
  "tools/pm/targets.yaml"
  "tools/pm/templates/agents.md.j2"
  "tools/pm/sources/operational/agent-core.md"
  "tools/ci/feature-matrix.sh"
  "docs/verification-scenarios.md"
)

SELF_HOSTING_FORBIDDEN_PATHS=(
  "tools/ci/gates"
  "tools/ci/gates/gate-memory-platform-doctor.sh"
  "tools/ci/gates/gate-memory-platform-lint.sh"
)

self_hosting_assert_source_set() {
  local missing=0
  local path

  for path in "${SELF_HOSTING_REQUIRED_PATHS[@]}"; do
    if [[ ! -e "${path}" ]]; then
      echo "missing self-hosting source path: ${path}" >&2
      missing=1
    fi
  done

  for path in "${SELF_HOSTING_COPY_PATHS[@]}"; do
    if [[ ! -e "${path}" ]]; then
      echo "missing self-hosting copy root: ${path}" >&2
      missing=1
    fi
  done

  if [[ "${missing}" -ne 0 ]]; then
    return 1
  fi
}

self_hosting_assert_copy_plan() {
  local forbidden
  local copied

  for forbidden in "${SELF_HOSTING_FORBIDDEN_PATHS[@]}"; do
    for copied in "${SELF_HOSTING_COPY_PATHS[@]}"; do
      if [[ "${copied}" == "${forbidden}" ]] || [[ "${copied}" == "${forbidden}/"* ]]; then
        echo "forbidden self-hosting copy path declared: ${copied}" >&2
        return 1
      fi
    done
  done
}

self_hosting_assert_extracted_layout() {
  local dest="$1"
  local missing=0
  local path

  for path in "${SELF_HOSTING_REQUIRED_PATHS[@]}"; do
    if [[ ! -e "${dest}/${path}" ]]; then
      echo "missing extracted self-hosting path: ${path}" >&2
      missing=1
    fi
  done

  for path in "${SELF_HOSTING_FORBIDDEN_PATHS[@]}"; do
    if [[ -e "${dest}/${path}" ]]; then
      echo "forbidden extracted path present: ${path}" >&2
      missing=1
    fi
  done

  if [[ "${missing}" -ne 0 ]]; then
    return 1
  fi
}
