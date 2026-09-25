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
mkdir -p "${fixture}/bin"
cat > "${fixture}/bin/semgrep" <<'EOF'
#!/usr/bin/env bash
printf '{"paths":{"scanned":[]},"results":[],"errors":[]}\n'
EOF
chmod +x "${fixture}/bin/semgrep"
expect_failure "zero-target semgrep scan" env PATH="${fixture}/bin:${PATH}" scripts/regression-guards.sh

# Build one fixture corpus so Semgrep starts once. The result check below proves
# every historical rule ID bites and the documented clean/suppressed cases stay
# clean; merely expecting a non-zero scan would let one working rule mask five
# broken rules.
cat > "${fixture}/bad.rs" <<'EOF'
fn bad(bytes: u64, a: u64, b: u64, c: u64, counter: &std::sync::atomic::AtomicU64) {
    let _signed = bytes as i64;
    let _ratio = a.saturating_mul(b) / c;
    counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
}
EOF

cat > "${fixture}/allowed.rs" <<'EOF'
fn allowed(bytes: u64) {
    let _signed = bytes as i64; // nosemgrep: qmp-magnitude-as-signed -- reason: fixture proves scoped suppression
}
EOF

# Pure-layer architecture rules are path-scoped, so mirror the real crate path
# in the fixture and prove each forbidden dependency shape is rejected.
pure_fixture="${fixture}/crates/quanta-memory-platform-core/src/domain"
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

cat > "${pure_fixture}/good.rs" <<'EOF'
trait StatePort {
    fn read(&self) -> Result<Vec<u8>, String>;
}

fn read_state(port: &dyn StatePort) -> Result<Vec<u8>, String> {
    port.read()
}
EOF

cat > "${pure_fixture}/bypass.rs" <<'EOF'
pub use crate::{adapters::metrics::MetricsAdapter};
use crate::adapters as adapter_layer;
use std::{
    fs::File,
};
fn bad() {
    let _ = std::fs::File::open("state");
}
EOF

semgrep_json="${fixture}/semgrep-results.json"
semgrep_stderr="${fixture}/semgrep.stderr"
semgrep_status=0
semgrep --config tools/semgrep/qmp-regressions.yml --error --strict --metrics=off \
  --disable-version-check --jobs=1 --json "${fixture}" \
  >"${semgrep_json}" 2>"${semgrep_stderr}" || semgrep_status=$?
if [[ "${semgrep_status}" -ne 1 ]]; then
  echo "FAIL: consolidated Semgrep fixture scan returned ${semgrep_status}, expected findings exit 1" >&2
  sed -n '1,160p' "${semgrep_stderr}" >&2
  exit 1
fi
python3 -c '
import json
import pathlib
import sys

payload = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
expected = {
    "qmp-atomic-fetch-sub",
    "qmp-magnitude-as-signed",
    "qmp-pure-layer-no-adapter-import",
    "qmp-pure-layer-no-fs-env",
    "qmp-pure-layer-no-wall-clock",
    "qmp-saturating-mul-then-divide",
}
results = payload.get("results", [])
seen = {result["check_id"].rsplit(".", 1)[-1] for result in results}
errors = payload.get("errors", [])
clean_hits = sorted(
    result["path"]
    for result in results
    if pathlib.Path(result["path"]).name in {"allowed.rs", "good.rs"}
)
bypass_hits = {
    (result["check_id"].rsplit(".", 1)[-1], result["start"]["line"])
    for result in results
    if pathlib.Path(result["path"]).name == "bypass.rs"
}
failures = []
if seen != expected:
    failures.append(f"rule IDs differ: missing={sorted(expected - seen)} extra={sorted(seen - expected)}")
if errors:
    failures.append(f"Semgrep reported {len(errors)} parse/runtime errors")
if clean_hits:
    failures.append(f"clean/suppressed fixtures produced findings: {clean_hits}")
expected_bypass = {
    ("qmp-pure-layer-no-adapter-import", 1),
    ("qmp-pure-layer-no-adapter-import", 2),
    ("qmp-pure-layer-no-fs-env", 3),
    ("qmp-pure-layer-no-fs-env", 7),
}
if not expected_bypass <= bypass_hits:
    failures.append(f"scoped bypass fixtures missed: {sorted(expected_bypass - bypass_hits)}")
if failures:
    raise SystemExit("; ".join(failures))
' "${semgrep_json}"
echo "PASS: consolidated Semgrep fixtures exercised all 6 rules and clean cases"

# A failed feature-list parser must stop the matrix before any Clippy slice.
# Process substitution used to discard this failure and report a green matrix.
mkdir -p "${fixture}/matrix-bin"
cat > "${fixture}/matrix-bin/cargo" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  metadata) exec "${QMP_REAL_CARGO}" "$@" ;;
  clippy|tree) exit 0 ;;
esac
exit 2
EOF
cat > "${fixture}/matrix-bin/python3" <<'EOF'
#!/usr/bin/env bash
if [[ "$1" == "-c" && "$2" == *'get("default"'* ]]; then
  exit 37
fi
exec "${QMP_REAL_PYTHON}" "$@"
EOF
chmod +x "${fixture}/matrix-bin/cargo" "${fixture}/matrix-bin/python3"
matrix_status=0
env QMP_REAL_CARGO="$(command -v cargo)" QMP_REAL_PYTHON="$(command -v python3)" \
  PATH="${fixture}/matrix-bin:${PATH}" scripts/verify-feature-matrix.sh \
  >/dev/null 2>&1 || matrix_status=$?
if [[ "${matrix_status}" -ne 37 ]]; then
  echo "FAIL: feature-list failure returned ${matrix_status}, expected 37" >&2
  exit 1
fi
echo "PASS: feature-list parser failure stops feature matrix"

# A changed manifest must never rewrite the extracted lockfile implicitly.
mkdir -p "${fixture}/locked/src"
printf 'fn main() {}\n' > "${fixture}/locked/src/main.rs"
printf '[package]\nname = "qmp120-lock-fixture"\nversion = "0.1.0"\nedition = "2021"\n' > "${fixture}/locked/Cargo.toml"
(cd "${fixture}/locked" && cargo generate-lockfile --offline >/dev/null)
printf '\n[dependencies]\nlibc = "0.2"\n' >> "${fixture}/locked/Cargo.toml"
expect_failure "lockfile drift" cargo metadata --locked --offline --format-version 1 --manifest-path "${fixture}/locked/Cargo.toml"

echo "extraction-contracts: OK"
