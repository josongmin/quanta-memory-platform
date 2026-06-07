#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${repo_root}"

cargo check -p quanta-memory-platform-testkit --lib --no-default-features --features doctor
cargo check -p quanta-memory-platform-facade --example effective_budget_report
