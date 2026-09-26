#!/usr/bin/env bash
# Run the public examples against the freshly extracted, self-hosted checkout.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

qmp_extract_dir="$(mktemp -d /tmp/qmp-ci-extraction.XXXXXX)"
echo "Source SHA: $(git rev-parse --verify HEAD)"
scripts/extract-to-standalone-repo.sh "${qmp_extract_dir}"
cargo run --locked --manifest-path "${qmp_extract_dir}/Cargo.toml" -p quanta-memory-platform-facade --example hello_world
cargo run --locked --manifest-path "${qmp_extract_dir}/Cargo.toml" -p quanta-memory-platform-facade --example effective_budget_report
test -z "$(git -C "${qmp_extract_dir}" status --porcelain)"
echo "Verified extracted checkout: ${qmp_extract_dir}"
