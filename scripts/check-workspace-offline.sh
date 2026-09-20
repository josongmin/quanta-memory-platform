#!/usr/bin/env bash
# Explicit offline recheck. Run only after the online release rail has filled
# Cargo's crate cache and cargo-deny's advisory database.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

echo "[offline] cargo metadata --frozen"
cargo metadata --frozen --format-version 1 >/dev/null

echo "[offline] clippy --frozen -D warnings"
cargo clippy --frozen --workspace --all-targets --all-features -- -D warnings

echo "[offline] tests --frozen"
cargo test --frozen --workspace --all-features

if ! command -v cargo-deny >/dev/null 2>&1; then
  echo "offline rail: cargo-deny unavailable" >&2
  exit 1
fi
echo "[offline] cargo deny --frozen --disable-fetch"
cargo deny --frozen --all-features check --disable-fetch

echo "offline rail: OK"
