#!/usr/bin/env bash
# regression-guards — fail-closed guards against bug classes this repo already hit.
#
# Primary engine is semgrep (tools/semgrep/qmp-regressions.yml). When semgrep is
# unavailable (e.g. a minimal CI image) it falls back to a line-based grep that
# approximates the same patterns, so the guard still bites. Wired into
# scripts/check-workspace.sh and scripts/memory-platform-doctor.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

config="tools/semgrep/qmp-regressions.yml"

if command -v semgrep >/dev/null 2>&1; then
  echo "[regression-guards] semgrep ${config}"
  # --error makes any finding a non-zero exit; --strict fails on rule/parse errors.
  semgrep --config "${config}" --error --strict --quiet crates/
  echo "[regression-guards] semgrep: clean"
  exit 0
fi

echo "[regression-guards] semgrep unavailable; using grep fallback" >&2
status=0

# Grep fallback. Each check mirrors a semgrep rule. `nosemgrep` lines are honored
# so an annotated, justified exception passes both engines. Comments are skipped.
scan() {
  local id="$1" desc="$2" pattern="$3"
  local hits
  # Strip each line's `//...` comment BEFORE re-matching, so the pattern only
  # counts when it appears in actual code (not in doc-comment prose), then drop
  # any `nosemgrep`-annotated line.
  hits="$(grep -rnE "${pattern}" crates/*/src --include='*.rs' 2>/dev/null \
    | grep -v 'nosemgrep' \
    | sed -E 's,//.*,,' \
    | grep -E "${pattern}" || true)"
  if [ -n "${hits}" ]; then
    echo "[regression-guards] FAIL ${id}: ${desc}" >&2
    echo "${hits}" >&2
    status=1
  fi
}

# qmp-magnitude-as-signed: `x as i64/i32/i16/i8` sign-flips a large magnitude.
scan "qmp-magnitude-as-signed" \
  "casting a magnitude to a signed int (the 'bytes as i64' wrap bug)" \
  ' as i(8|16|32|64)\b'

# qmp-saturating-mul-then-divide: saturates before the divide -> under-reports.
scan "qmp-saturating-mul-then-divide" \
  "saturating_mul(...) / ... rounds a ratio down; compute in u128" \
  'saturating_mul\(.*\) /'

# qmp-atomic-fetch-sub: can underflow/wrap; use a saturating fetch_update.
scan "qmp-atomic-fetch-sub" \
  "Atomic::fetch_sub can wrap; decrement with saturating fetch_update" \
  '\.fetch_sub\('

if [ "${status}" -eq 0 ]; then
  echo "[regression-guards] grep fallback: clean"
fi
exit "${status}"
