#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

python3 -m unittest -v tools/tickets/test_verify_tickets.py
python3 tools/tickets/verify_tickets.py --root "${repo_root}"
