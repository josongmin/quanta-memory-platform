#!/usr/bin/env bash
# Opt-in L2: inject EIO at real Redb finalize pwrite64/fdatasync syscalls.
# This is not a device-mapper test and never claims physical power-loss safety.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo_root}"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "UNAVAILABLE: L2 syscall injection requires Linux; no fallback is permitted" >&2
  exit 77
fi
if (( EUID != 0 )); then
  echo "UNAVAILABLE: L2 syscall injection requires effective UID 0" >&2
  exit 77
fi
for tool in strace cargo python3 timeout; do
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "UNAVAILABLE: ${tool} is required for the opt-in L2 rail" >&2
    exit 77
  fi
done
if [[ ! -f Cargo.lock || ! -f crates/quanta-memory-platform-facade/Cargo.toml ]]; then
  echo "error: run from the QMP workspace" >&2
  exit 2
fi
artifact_root_input="${QMP_DURABILITY_L2_ARTIFACT_ROOT:-${TMPDIR:-/tmp}/qmp-durability-l2-$$}"
artifact_root="$(realpath -m -- "${artifact_root_input}")"
case "${artifact_root}" in
  /|"${repo_root}"|"${repo_root}"/*|"${HOME}"|"${HOME}"/*)
    echo "error: L2 artifacts must use a new, narrow path outside the repository and home" >&2
    exit 2
    ;;
esac
if [[ -e "${artifact_root}" ]]; then
  echo "error: L2 artifact directory already exists: ${artifact_root}" >&2
  exit 2
fi
export QMP_DURABILITY_L2_ARTIFACT_ROOT="${artifact_root}"
run_timeout_secs="${QMP_DURABILITY_L2_RUN_TIMEOUT_SECS:-300}"
if [[ ! "${run_timeout_secs}" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: QMP_DURABILITY_L2_RUN_TIMEOUT_SECS must be a positive integer" >&2
  exit 2
fi

source_head="$(git rev-parse HEAD)"
# The Python digest filter below is intentionally a literal program.
# shellcheck disable=SC2016
compute_source_digest() {
  git ls-files -co --exclude-standard -z |
    python3 -c '
import sys
import os
excluded_roots = (b"docs/", b"tickets/")
excluded_files = {b"README.md", b"CHANGELOG.md", b"MIGRATION.md"}
for path in sys.stdin.buffer.read().split(b"\0"):
    # `git ls-files -co` also lists tracked paths deleted in the worktree.
    # Omit absent paths so digesting the present candidate is deterministic.
    if (path and os.path.isfile(path) and not path.startswith(excluded_roots)
            and path not in excluded_files):
        sys.stdout.buffer.write(path + b"\0")
' |
    sort -z |
    xargs -0 -r sha256sum |
    sha256sum |
    awk '{print $1}'
}

source_digest="$(compute_source_digest)"
export QMP_SOURCE_HEAD="${source_head}"
export QMP_SOURCE_DIGEST="${source_digest}"

echo "L2 source_head=${source_head} test_source_digest=${source_digest} (Rust/Cargo/scripts; docs and tickets excluded)"
if [[ "${QMP_DURABILITY_L2_MODE:-}" == device-mapper ]]; then
  echo "L2 mechanism=dm-error table swap; claim=block-device I/O disruption and filesystem recovery only"
else
  echo "L2 mechanism=strace EIO injection; claim=Linux syscall error behavior only"
fi
cargo_status=0
if timeout --signal=TERM --kill-after=10s "${run_timeout_secs}s" \
  cargo run --locked -p quanta-memory-platform-facade \
  --no-default-features --features durability-l2 \
  --example spill_redb_durability_l2; then
  :
else
  cargo_status=$?
fi

source_digest_after="$(compute_source_digest)"
receipt="${artifact_root}/receipt.txt"
source_stable=true
if [[ "${source_digest_after}" != "${source_digest}" ]]; then
  source_stable=false
fi
if [[ -f "${receipt}" ]]; then
  printf '\nrunner_cargo_status=%s\nsource_digest_after=%s\nsource_stable=%s\n' \
    "${cargo_status}" "${source_digest_after}" "${source_stable}" >>"${receipt}"
fi
if [[ "${source_stable}" != true ]]; then
  echo "error: executable inputs changed during L2 run (${source_digest} -> ${source_digest_after})" >&2
  exit 2
fi
if [[ ! -f "${receipt}" ]]; then
  echo "error: L2 run produced no receipt (cargo status ${cargo_status}): ${receipt}" >&2
  if (( cargo_status != 0 )); then
    exit "${cargo_status}"
  fi
  exit 2
fi
if (( cargo_status != 0 )); then
  echo "error: L2 scenario failed with cargo status ${cargo_status}; preserved receipt=${receipt}" >&2
  exit "${cargo_status}"
fi
echo "L2 source_stable=true effective_uid=${EUID} receipt=${receipt}"
