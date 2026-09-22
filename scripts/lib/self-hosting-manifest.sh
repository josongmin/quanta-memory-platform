#!/usr/bin/env bash
# Canonical self-hosting file set for the extracted quanta-memory-platform repo.
#
# These are the repo-local paths the standalone workspace must carry so its own
# scripts, docs, and prompt-manager sources remain functional without depending
# on Semantica's monorepo-local `tools/ci/gates/*`.

SELF_HOSTING_COPY_PATHS=(
  "Cargo.toml"
  "Cargo.lock"
  "rust-toolchain.toml"
  "README.md"
  "EXTRACTION.md"
  "CHANGELOG.md"
  "MIGRATION.md"
  "AGENTS.md"
  "CLAUDE.md"
  ".codex"
  ".cursorrules"
  "rustfmt.toml"
  "clippy.toml"
  "deny.toml"
  ".cargo"
  ".gitignore"
  "crates"
  "scripts"
  "tools/pm"
  "tools/semgrep"
  "tools/ci/feature-matrix.sh"
  "docs"
)

SELF_HOSTING_REQUIRED_PATHS=(
  "Cargo.toml"
  "Cargo.lock"
  "rust-toolchain.toml"
  "README.md"
  "EXTRACTION.md"
  "CHANGELOG.md"
  "MIGRATION.md"
  "AGENTS.md"
  "CLAUDE.md"
  ".codex/CODEX-RULES.md"
  ".codex/CODEX-START-PROMPT.md"
  ".cursorrules"
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
  "scripts/run-scaling-bench.sh"
  "scripts/verify-scaling-results.py"
  "scripts/verify-feature-capabilities.sh"
  "scripts/verify-doc-test-selectors.py"
  "scripts/lib/self-hosting-manifest.sh"
  "scripts/lib/extraction_manifest.py"
  "scripts/lib/check-extracted-metadata.py"
  "scripts/tests/extraction-contracts.sh"
  "scripts/tests/test_extraction_manifest.py"
  "scripts/tests/test_extraction_metadata.py"
  "scripts/tests/test_scaling_results.py"
  "scripts/tests/test_doc_test_selectors.py"
  "scripts/regression-guards.sh"
  "scripts/run-checked-cargo-test"
  "scripts/lib/run_cargo_test.py"
  "tools/pm/pm.py"
  "tools/pm/targets.yaml"
  "tools/pm/templates/agents.md.j2"
  "tools/pm/sources/operational/agent-core.md"
  "tools/ci/feature-matrix.sh"
  "tools/semgrep/qmp-regressions.yml"
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
  local required
  local covered

  for forbidden in "${SELF_HOSTING_FORBIDDEN_PATHS[@]}"; do
    for copied in "${SELF_HOSTING_COPY_PATHS[@]}"; do
      if [[ "${copied}" == "${forbidden}" ]] || [[ "${copied}" == "${forbidden}/"* ]]; then
        echo "forbidden self-hosting copy path declared: ${copied}" >&2
        return 1
      fi
    done
  done

  for required in "${SELF_HOSTING_REQUIRED_PATHS[@]}"; do
    covered=0
    for copied in "${SELF_HOSTING_COPY_PATHS[@]}"; do
      if [[ "${required}" == "${copied}" ]] || [[ "${required}" == "${copied}/"* ]]; then
        covered=1
        break
      fi
    done
    if [[ "${covered}" -eq 0 ]]; then
      echo "required self-hosting path omitted from copy plan: ${required}" >&2
      return 1
    fi
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
