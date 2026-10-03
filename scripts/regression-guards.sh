#!/usr/bin/env bash
# regression-guards — fail-closed guards against bug classes this repo already hit.
#
# Semgrep is required in the release rail. A grep approximation can miss rules,
# so a missing executable or config is a hard failure.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

config="tools/semgrep/qmp-regressions.yml"

if [[ ! -f "${config}" ]]; then
  echo "[regression-guards] missing required config: ${config}" >&2
  exit 1
fi

if ! command -v semgrep >/dev/null 2>&1; then
  echo "[regression-guards] semgrep unavailable" >&2
  exit 1
fi

echo "[regression-guards] semgrep ${config}"
python3 scripts/lib/verify_semgrep_suppressions.py
# --error makes any finding non-zero; --strict fails on rule/parse errors.
# Explicit targets include test modules under src/: Semgrep's directory scan
# can skip them even though the source-shape rail counts them as Rust source.
rust_targets=()
while IFS= read -r -d '' path; do
  rust_targets+=("${path}")
done < <(find crates -type f -name '*.rs' -print0)
if [[ "${#rust_targets[@]}" -eq 0 ]]; then
  echo "regression-guards: no Rust source targets" >&2
  exit 1
fi
scan_output="$(mktemp /tmp/qmp-semgrep-scan.XXXXXX)"
trap 'rm -f -- "${scan_output}"' EXIT
scan_status=0
semgrep --config "${config}" --error --strict --metrics=off \
  --disable-version-check --jobs=1 --timeout=30 --json "${rust_targets[@]}" \
  >"${scan_output}" || scan_status=$?
python3 - "${scan_output}" <<'PY'
import json
from pathlib import Path
import sys

scan = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
scanned = set(scan["paths"]["scanned"])
production = {
    path.as_posix() for path in Path("crates").glob("*/src/**/*.rs")
}
missing = sorted(production - scanned)
if not production or missing:
    raise SystemExit(
        "regression-guards: production Rust files not scanned: "
        + ", ".join(missing or ["<no production files found>"])
    )
if scan.get("errors") or scan.get("results"):
    for error in scan.get("errors", []):
        print(f"regression-guards: {error.get('type')}: {error.get('message')}", file=sys.stderr)
    for result in scan.get("results", []):
        print(f"regression-guards: {result['check_id']}: {result['path']}", file=sys.stderr)
    raise SystemExit("regression-guards: Semgrep reported findings or errors")
print(f"[regression-guards] scanned {len(scanned)} Rust files, including all {len(production)} production files")
PY
if [[ "${scan_status}" -ne 0 ]]; then
  echo "regression-guards: Semgrep failed (${scan_status})" >&2
  exit "${scan_status}"
fi
echo "[regression-guards] semgrep: clean"
