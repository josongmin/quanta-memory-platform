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
# shellcheck disable=SC2016 # The child bash intentionally expands the copied arrays.
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

# The regression rules must bite on each historical bug shape and accept the
# documented scoped exception mechanism.
cat > "${fixture}/bad.rs" <<'EOF'
fn bad(bytes: u64, a: u64, b: u64, c: u64, counter: &std::sync::atomic::AtomicU64) {
    let _signed = bytes as i64;
    let _ratio = a.saturating_mul(b) / c;
    counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
}
EOF
expect_failure "semgrep historical bug fixtures" \
  semgrep --config tools/semgrep/qmp-regressions.yml --error --strict --metrics=off \
    "${fixture}/bad.rs"

cat > "${fixture}/allowed.rs" <<'EOF'
fn allowed(bytes: u64) {
    let _signed = bytes as i64; // nosemgrep: qmp-magnitude-as-signed -- reason: fixture proves scoped suppression
}
EOF
semgrep --config tools/semgrep/qmp-regressions.yml --error --strict --metrics=off \
  "${fixture}/allowed.rs" >/dev/null

# Pure-layer architecture rules are path-scoped, so mirror the real crate path
# in the fixture and prove each forbidden dependency shape is rejected.
pure_fixture="${fixture}/pure/crates/quanta-memory-platform-core/src/domain"
mkdir -p "${pure_fixture}"
cat > "${pure_fixture}/bad.rs" <<'EOF'
use crate::adapters::metrics::MetricsAdapter;
use std::time::SystemTime;

fn bad() {
    let _ = std::fs::read("state");
    let _ = std::env::var("QMP_STATE");
    let _ = SystemTime::now();
}
EOF
expect_failure "semgrep pure-layer architecture fixtures" \
  semgrep --config tools/semgrep/qmp-regressions.yml --error --strict --metrics=off \
    "${fixture}/pure"

cat > "${pure_fixture}/good.rs" <<'EOF'
trait StatePort {
    fn read(&self) -> Result<Vec<u8>, String>;
}

fn read_state(port: &dyn StatePort) -> Result<Vec<u8>, String> {
    port.read()
}
EOF
semgrep --config tools/semgrep/qmp-regressions.yml --error --strict --metrics=off \
  "${fixture}/pure" --exclude "**/bad.rs" >/dev/null

# A changed manifest must never rewrite the extracted lockfile implicitly.
mkdir -p "${fixture}/locked/src"
printf 'fn main() {}\n' > "${fixture}/locked/src/main.rs"
printf '[package]\nname = "qmp120-lock-fixture"\nversion = "0.1.0"\nedition = "2021"\n' > "${fixture}/locked/Cargo.toml"
(cd "${fixture}/locked" && cargo generate-lockfile --offline >/dev/null)
printf '\n[dependencies]\nlibc = "0.2"\n' >> "${fixture}/locked/Cargo.toml"
expect_failure "lockfile drift" cargo metadata --locked --offline --format-version 1 --manifest-path "${fixture}/locked/Cargo.toml"

echo "extraction-contracts: OK"
