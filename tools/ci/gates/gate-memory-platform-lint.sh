#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

# One canonical matrix owns the default/bare/leaf/full/all axes and enforces
# `-D warnings` on every combination, including feature-leak assertions.
tools/ci/feature-matrix.sh

if ! command -v cargo-deny >/dev/null 2>&1; then
  echo "cargo-deny is required for gate-memory-platform-lint.sh" >&2
  exit 1
fi

cargo deny --all-features check
