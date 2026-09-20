#!/usr/bin/env bash
# Fast negative contracts for the standalone extraction release rail.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"
source scripts/lib/self-hosting-manifest.sh

expect_failure() {
  local label="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "FAIL: ${label} unexpectedly succeeded" >&2
    exit 1
  fi
  echo "PASS: ${label} rejected"
}

# A required executable/config path may not silently disappear from the copy
# plan, even when its source still exists.
expect_failure "omitted semgrep config" bash -c '
  source scripts/lib/self-hosting-manifest.sh
  SELF_HOSTING_COPY_PATHS=("${SELF_HOSTING_COPY_PATHS[@]/tools\/semgrep/}")
  self_hosting_assert_copy_plan
'

expect_failure "missing lockfile source" bash -c '
  source scripts/lib/self-hosting-manifest.sh
  SELF_HOSTING_REQUIRED_PATHS+=("Cargo.lock.missing-fixture")
  self_hosting_assert_source_set
'

# Regression guards must reject both an absent config and an absent engine.
fixture="$(mktemp -d /tmp/qmp120-guards.XXXXXX)"
trap 'rm -rf "${fixture}"' EXIT
mkdir -p "${fixture}/scripts"
cp scripts/regression-guards.sh "${fixture}/scripts/regression-guards.sh"
expect_failure "missing semgrep config" "${fixture}/scripts/regression-guards.sh"
expect_failure "missing semgrep binary" env PATH=/usr/bin:/bin scripts/regression-guards.sh

# A changed manifest must never rewrite the extracted lockfile implicitly.
mkdir -p "${fixture}/locked/src"
printf 'fn main() {}\n' > "${fixture}/locked/src/main.rs"
printf '[package]\nname = "qmp120-lock-fixture"\nversion = "0.1.0"\nedition = "2021"\n' > "${fixture}/locked/Cargo.toml"
(cd "${fixture}/locked" && cargo generate-lockfile --offline >/dev/null)
printf '\n[dependencies]\nlibc = "0.2"\n' >> "${fixture}/locked/Cargo.toml"
expect_failure "lockfile drift" cargo metadata --locked --offline --format-version 1 --manifest-path "${fixture}/locked/Cargo.toml"

echo "extraction-contracts: OK"
