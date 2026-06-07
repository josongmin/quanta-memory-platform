#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

source "${repo_root}/scripts/lib/self-hosting-manifest.sh"

self_hosting_assert_source_set
self_hosting_assert_copy_plan

echo "external repo scaffold is self-hosting and excludes tools/ci/gates"
