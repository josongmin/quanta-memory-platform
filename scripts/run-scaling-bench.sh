#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
  echo "run-scaling-bench: source tree is dirty; commit/freeze the benchmark source first" >&2
  exit 2
fi

source_sha="$(git rev-parse HEAD)"
source_tree="$(git rev-parse 'HEAD^{tree}')"
rustc_version="$(rustc --version)"
cargo_version="$(cargo --version)"
machine="$(uname -srm)"

result_file="$(mktemp "${TMPDIR:-/tmp}/qmp-scaling.XXXXXX.jsonl")"
trap 'rm -f "$result_file"' EXIT

printf '{"schema":"qmp-scaling-run-v2","source_sha":"%s","source_tree":"%s","rustc":"%s","cargo":"%s","machine":"%s"}\n' \
  "$source_sha" "$source_tree" "$rustc_version" "$cargo_version" "$machine" | tee "$result_file"

if [[ "${1:-}" == "--smoke" ]]; then
  cargo bench -p quanta-memory-platform-facade --all-features --bench scaling -- --smoke | tee -a "$result_file"
  verify_args=(--input "$result_file" --smoke)
else
  cargo bench -p quanta-memory-platform-facade --all-features --bench scaling -- "$@" | tee -a "$result_file"
  verify_args=(--input "$result_file")
fi

if [[ -n "${QMP_SCALING_BASELINE:-}" ]]; then
  verify_args+=(--baseline "$QMP_SCALING_BASELINE")
fi
python3 scripts/verify-scaling-results.py "${verify_args[@]}"
