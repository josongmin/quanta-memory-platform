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
# --error makes any finding non-zero; --strict fails on rule/parse errors.
semgrep --config "${config}" --error --strict --metrics=off \
  --disable-version-check --jobs=1 --include 'crates/**/*.rs' .
echo "[regression-guards] semgrep: clean"
