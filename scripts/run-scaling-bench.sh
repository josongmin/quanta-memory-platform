#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

profile="$(python3 -c 'import runpy, sys; print(runpy.run_path("scripts/verify-scaling-results.py")["parse_benchmark_args"](sys.argv[1:]).profile)' "$@")"
if [[ "$profile" == in-memory-pressure ]]; then
  feature_set=full
  cargo_features=(--features full)
else
  feature_set=all-features
  cargo_features=(--all-features)
fi

source_sha="$(git rev-parse HEAD)"
source_tree="$(git rev-parse 'HEAD^{tree}')"
source_manifest="$(mktemp /tmp/qmp-benchmark-source.XXXXXX)"
trap 'rm -f -- "${source_manifest}"' EXIT
python3 scripts/lib/extraction_manifest.py snapshot --repo "${repo_root}" \
  --revision "${source_sha}" --output "${source_manifest}" . >/dev/null
python3 scripts/lib/extraction_manifest.py assert-clean --repo "${repo_root}" \
  --commit "${source_sha}" .
python3 scripts/lib/extraction_manifest.py verify --manifest "${source_manifest}" \
  --destination "${repo_root}" >/dev/null
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
benchmark_source_sha256="$(
  shasum -a 256 \
    crates/quanta-memory-platform-facade/benches/scaling/mod.rs \
    crates/quanta-memory-platform-facade/benches/scaling/pressure.rs \
    crates/quanta-memory-platform-facade/benches/scaling/throughput.rs \
    | shasum -a 256 | awk '{print $1}'
)"
bench_build_config_sha256="$(python3 scripts/lib/scaling_build_config.py)"

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

# Compile before sampling host load or timing the target. The second Cargo
# invocation should only launch the already-built binary; source identity is
# still checked again after it exits.
cargo bench --locked -p quanta-memory-platform-facade "${cargo_features[@]}" --bench scaling --no-run >/dev/null

run_id="$(python3 -c 'import uuid; print(uuid.uuid4())')"
python3 - "$source_sha" "$source_tree" "$rustc_version" "$cargo_version" "$machine" "$feature_graph_sha256" "$dependency_lock_sha256" "$benchmark_source_sha256" "$bench_build_config_sha256" "$feature_set" "$run_id" "$@" <<'PY' | tee "$result_file"
import json
import os
import sys
sha, tree, rustc, cargo, machine, feature_graph, lock_digest, benchmark_source, build_config, feature_set, run_id, *bench_args = sys.argv[1:]
logical_cpus = os.cpu_count()
if not logical_cpus:
    raise SystemExit("run-scaling-bench: logical CPU count is unavailable")
print(json.dumps({
    "schema": "qmp-scaling-run-v4",
    "source_sha": sha,
    "source_tree": tree,
    "rustc": rustc,
    "cargo": cargo,
    "machine": machine,
    "feature_set": feature_set,
    "feature_graph_sha256": feature_graph,
    "dependency_lock_sha256": lock_digest,
    "benchmark_source_sha256": benchmark_source,
    "bench_build_config_sha256": build_config,
    "package": "quanta-memory-platform-facade",
    "bench_target": "scaling",
    "benchmark_args": bench_args,
    "run_id": run_id,
    "logical_cpus": logical_cpus,
    "load_per_cpu_start": os.getloadavg()[0] / logical_cpus,
}, separators=(",", ":")))
PY

set +e
cargo bench --locked -p quanta-memory-platform-facade "${cargo_features[@]}" --bench scaling -- "$@" 2>&1 | tee -a "$result_file"
# Capture both statuses before any other command resets PIPESTATUS. A failed
# tee can otherwise leave an incomplete receipt while Cargo itself exits 0.
bench_pipe_status=("${PIPESTATUS[@]}")
bench_status=${bench_pipe_status[0]}
tee_status=${bench_pipe_status[1]}
set -e
if (( tee_status != 0 )); then
  echo "run-scaling-bench: could not retain complete benchmark output (tee exit ${tee_status}): ${result_file}" >&2
  exit 2
fi

end_source_sha="$(git rev-parse HEAD)"
end_source_tree="$(git rev-parse 'HEAD^{tree}')"
end_bench_build_config_sha256="$(python3 scripts/lib/scaling_build_config.py)"
source_clean=true
if ! python3 scripts/lib/extraction_manifest.py assert-clean --repo "${repo_root}" \
    --commit "${source_sha}" . || \
   ! python3 scripts/lib/extraction_manifest.py verify --manifest "${source_manifest}" \
    --destination "${repo_root}" >/dev/null; then
  source_clean=false
fi
python3 - "$end_source_sha" "$end_source_tree" "$end_bench_build_config_sha256" "$source_clean" "$run_id" "$bench_status" <<'PY' | tee -a "$result_file"
import json
import os
import sys
sha, tree, build_config, clean, run_id, bench_status = sys.argv[1:]
logical_cpus = os.cpu_count()
if not logical_cpus:
    raise SystemExit("run-scaling-bench: logical CPU count is unavailable")
print(json.dumps({
    "schema": "qmp-scaling-run-end-v4",
    "source_sha": sha,
    "source_tree": tree,
    "bench_build_config_sha256": build_config,
    "source_clean": clean == "true",
    "run_id": run_id,
    "bench_exit_code": int(bench_status),
    "load_per_cpu_end": os.getloadavg()[0] / logical_cpus,
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
if [[ "$bench_build_config_sha256" != "$end_bench_build_config_sha256" ]]; then
  echo "run-scaling-bench: build configuration changed during benchmark; artifact retained at $result_file" >&2
  exit 2
fi

verify_args=(--input "$result_file")
if [[ -n "${QMP_SCALING_BASELINE:-}" ]]; then
  verify_args+=(--baseline "$QMP_SCALING_BASELINE")
fi
python3 scripts/verify-scaling-results.py "${verify_args[@]}"
