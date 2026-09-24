#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

python3 -c 'import runpy, sys; runpy.run_path("scripts/verify-scaling-results.py")["parse_benchmark_args"](sys.argv[1:])' "$@"

if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
  echo "run-scaling-bench: source tree is dirty; commit/freeze the benchmark source first" >&2
  exit 2
fi

source_sha="$(git rev-parse HEAD)"
source_tree="$(git rev-parse 'HEAD^{tree}')"
rustc_version="$(rustc --version)"
cargo_version="$(cargo --version)"
# A uname-only label aliases different hosts with the same OS/architecture.
# Keep the local host identity private while rejecting cross-host baselines.
host_digest="$(hostname | shasum -a 256 | cut -c 1-16)"
machine="$(uname -srm)|host=${host_digest}"
feature_graph_sha256="$(cargo metadata --locked --no-deps --format-version 1 | python3 -c '
import hashlib, json, sys
metadata = json.load(sys.stdin)
names = {"quanta-memory-platform-core", "quanta-memory-platform-facade", "quanta-memory-platform-testkit"}
graph = sorted(
    (package["name"], package["version"], package["features"])
    for package in metadata["packages"] if package["name"] in names
)
print(hashlib.sha256(json.dumps(graph, sort_keys=True, separators=(",", ":")).encode()).hexdigest())
')"
dependency_lock_sha256="$(shasum -a 256 Cargo.lock | awk '{print $1}')"

if [[ -n "${QMP_SCALING_ARTIFACT:-}" ]]; then
  result_file="$QMP_SCALING_ARTIFACT"
  mkdir -p "$(dirname "$result_file")"
  if [[ -e "$result_file" ]]; then
    echo "run-scaling-bench: artifact already exists: $result_file" >&2
    exit 2
  fi
else
  mkdir -p "$repo_root/target/scaling-results"
  # BSD mktemp does not replace XXXXXX when a suffix follows it. Give each
  # retained JSONL receipt its own randomly named directory on both platforms.
  result_dir="$(mktemp -d "$repo_root/target/scaling-results/qmp-scaling.XXXXXX")"
  result_file="$result_dir/results.jsonl"
fi
echo "run-scaling-bench: raw artifact: $result_file" >&2

python3 - "$source_sha" "$source_tree" "$rustc_version" "$cargo_version" "$machine" "$feature_graph_sha256" "$dependency_lock_sha256" "$@" <<'PY' | tee "$result_file"
import json
import sys
sha, tree, rustc, cargo, machine, feature_graph, lock_digest, *bench_args = sys.argv[1:]
print(json.dumps({
    "schema": "qmp-scaling-run-v3",
    "source_sha": sha,
    "source_tree": tree,
    "rustc": rustc,
    "cargo": cargo,
    "machine": machine,
    "feature_set": "all-features",
    "feature_graph_sha256": feature_graph,
    "dependency_lock_sha256": lock_digest,
    "package": "quanta-memory-platform-facade",
    "bench_target": "scaling",
    "benchmark_args": bench_args,
}, separators=(",", ":")))
PY

set +e
cargo bench -p quanta-memory-platform-facade --all-features --bench scaling -- "$@" 2>&1 | tee -a "$result_file"
bench_status=${PIPESTATUS[0]}
set -e

end_source_sha="$(git rev-parse HEAD)"
end_source_tree="$(git rev-parse 'HEAD^{tree}')"
source_clean=true
if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
  source_clean=false
fi
python3 - "$end_source_sha" "$end_source_tree" "$source_clean" <<'PY' | tee -a "$result_file"
import json
import sys
sha, tree, clean = sys.argv[1:]
print(json.dumps({
    "schema": "qmp-scaling-run-end-v3",
    "source_sha": sha,
    "source_tree": tree,
    "source_clean": clean == "true",
}, separators=(",", ":")))
PY

if (( bench_status != 0 )); then
  echo "run-scaling-bench: cargo bench failed ($bench_status); artifact retained at $result_file" >&2
  exit "$bench_status"
fi
if [[ "$source_sha" != "$end_source_sha" || "$source_tree" != "$end_source_tree" || "$source_clean" != true ]]; then
  echo "run-scaling-bench: source changed during benchmark; artifact retained at $result_file" >&2
  exit 2
fi

verify_args=(--input "$result_file")
if [[ -n "${QMP_SCALING_BASELINE:-}" ]]; then
  verify_args+=(--baseline "$QMP_SCALING_BASELINE")
fi
python3 scripts/verify-scaling-results.py "${verify_args[@]}"
