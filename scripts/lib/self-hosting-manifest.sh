#!/usr/bin/env bash
# Canonical self-hosting file set for the extracted quanta-memory-platform repo.
#
# These are the repo-local paths the standalone workspace must carry so its own
# scripts, docs, and prompt-manager sources remain functional.

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
  ".circleci"
  ".github"
  ".cursor"
  "rustfmt.toml"
  "deny.toml"
  ".gitignore"
  "crates"
  "scripts"
  "tools/pm"
  "tools/semgrep"
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
  ".circleci/config.yml"
  ".github/workflows/ci.yml"
  ".cursor/rules/architecture.mdc"
  ".cursor/rules/build.mdc"
  ".cursor/rules/dependencies.mdc"
  ".cursor/rules/determinism.mdc"
  ".cursor/rules/error-handling.mdc"
  ".cursor/rules/rust-review.mdc"
  ".cursor/rules/tests.mdc"
  "rustfmt.toml"
  "deny.toml"
  "crates/quanta-memory-platform-core/Cargo.toml"
  "crates/quanta-memory-platform-facade/Cargo.toml"
  "crates/quanta-memory-platform-testkit/Cargo.toml"
  "crates/quanta-memory-platform-facade/examples/hello_world.rs"
  "scripts/check-workspace.sh"
  "scripts/extract-to-standalone-repo.sh"
  "scripts/verify-extracted-examples.sh"
  "scripts/run-scaling-bench.sh"
  "scripts/verify-scaling-results.py"
  "scripts/verify-feature-matrix.sh"
  "scripts/verify-feature-capabilities.sh"
  "scripts/verify-doc-test-selectors.py"
  "scripts/verify-workspace-policy.py"
  "scripts/lib/self-hosting-manifest.sh"
  "scripts/lib/extraction_manifest.py"
  "scripts/lib/check-extracted-metadata.py"
  "scripts/tests/extraction-contracts.sh"
  "scripts/tests/test_extraction_manifest.py"
  "scripts/tests/test_extraction_metadata.py"
  "scripts/tests/test_scaling_results.py"
  "scripts/tests/test_doc_test_selectors.py"
  "scripts/tests/test_gate_failure_paths.py"
  "scripts/tests/test_run_cargo_test.py"
  "scripts/tests/test_workspace_policy.py"
  "scripts/regression-guards.sh"
  "scripts/lib/run_cargo_test.py"
  "tools/pm/pm.py"
  "tools/pm/targets.json"
  "tools/pm/sources/operational/agent-core.md"
  "tools/pm/tests/test_pm.py"
  "tools/semgrep/qmp-regressions.yml"
  "docs/verification-scenarios.md"
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
  local copied
  local required
  local covered

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

  if [[ "${missing}" -ne 0 ]]; then
    return 1
  fi
}
