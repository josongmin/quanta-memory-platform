#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Per-feature build matrix: the workspace advertises feature-sliced facade/testkit
# crates, so every slice MUST also compile in isolation (not only under
# --all-features). Without this, a single broken slice would pass CI and only
# break a consumer that enables exactly that feature. Each slice is checked alone.
echo "== per-feature build matrix =="
cargo check -p quanta-memory-platform-facade --no-default-features
for feat in payload_store retained_map revocable_store transcript publish_pipeline spill \
  spill-redb revocable-moka revocable-scc full full-backings; do
  echo "-- facade: ${feat}"
  cargo check -p quanta-memory-platform-facade --no-default-features --features "${feat}"
done
cargo check -p quanta-memory-platform-core --no-default-features
for feat in pressure scheduler telemetry full; do
  echo "-- core: ${feat}"
  cargo check -p quanta-memory-platform-core --no-default-features --features "${feat}"
done
cargo check -p quanta-memory-platform-testkit --no-default-features
for feat in fixtures doctor conformance faults alloc_tracking full; do
  echo "-- testkit: ${feat}"
  cargo check -p quanta-memory-platform-testkit --no-default-features --features "${feat}"
done

if ! command -v cargo-deny >/dev/null 2>&1; then
  echo "cargo-deny is required for gate-memory-platform-lint.sh" >&2
  exit 1
fi

cargo deny --all-features check
