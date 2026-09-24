#!/usr/bin/env bash
# Opt-in Linux ENOSPC oracle on a disposable 8 MiB Docker tmpfs.
# It does not qualify the consuming host, its volume policy, or power loss.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
cargo_registry="${CARGO_HOME:-${HOME}/.cargo}/registry"
if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
  echo "UNAVAILABLE: Docker daemon is required for isolated Linux ENOSPC" >&2
  exit 77
fi
if [[ ! -d "${cargo_registry}" ]]; then
  echo "UNAVAILABLE: local Cargo registry is required for offline container build" >&2
  exit 77
fi

source_digest() {
  (cd "${repo_root}" && git ls-files -co --exclude-standard -z |
    LC_ALL=C sort -z |
    while IFS= read -r -d '' path; do
      if [[ -f "${path}" ]]; then
        shasum -a 256 -- "${path}"
      fi
    done |
    shasum -a 256 | awk '{print $1}')
}

source_head="$(git -C "${repo_root}" rev-parse HEAD)"
source_before="$(source_digest)"
echo "ENOSPC source_head=${source_head} source_digest=${source_before}"
echo "ENOSPC mechanism=Linux tmpfs size=8MiB; no host-volume or power-loss claim"

run_status=0
for mode in spill restore finalize charged charged-restore-commit; do
  echo "ENOSPC scenario=${mode}"
  features="spill-redb,revocable_store"
  if [[ "${mode}" == "charged-restore-commit" ]]; then
    features+=",durability-test-hooks"
  fi
  if docker run --rm --network none \
    --mount "type=bind,src=${repo_root},dst=/workspace,readonly" \
    --mount "type=bind,src=${cargo_registry},dst=/usr/local/cargo/registry,readonly" \
    --tmpfs /qmp-volume:rw,size=8m,mode=1777 \
    -w /workspace \
    -e QMP_ENOSPC_ISOLATED_TMPFS=1 \
    -e RUSTUP_TOOLCHAIN=1.92.0 \
    -e CARGO_TARGET_DIR=/tmp/qmp-linux-target \
    rust:1.92-bookworm \
    cargo run --offline --locked -p quanta-memory-platform-facade \
      --features "${features}" --example spill_redb_enospc_volume -- \
      "/qmp-volume/${mode}.redb" "${mode}"; then
    :
  else
    run_status=$?
    break
  fi
done

source_after="$(source_digest)"
if [[ "${source_before}" != "${source_after}" || "${source_head}" != "$(git -C "${repo_root}" rev-parse HEAD)" ]]; then
  echo "error: source changed during ENOSPC run (${source_before} -> ${source_after})" >&2
  exit 2
fi
echo "ENOSPC source_stable=true"
exit "${run_status}"
