# Short entry points. scripts/check-workspace.sh remains the verification authority.
set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

check:
    scripts/check-workspace.sh

in-memory:
    scripts/check-workspace.sh --in-memory

extract-dry-run destination:
    scripts/extract-to-standalone-repo.sh --dry-run {{quote(destination)}}

hosted-binding:
    python3 scripts/verify-hosted-ci-binding.py

scaling-smoke:
    scripts/run-scaling-bench.sh --smoke
