# Extraction Checklist — quanta-memory-platform

Mechanical, executable steps to split this library into its own git repository
while preserving the package split (`core <- facade <- testkit`) and the
dependency direction. Not prose: every step is a command or a verifiable gate.

## Invariants preserved by extraction

1. Package split stays `core <- facade <- testkit` (three members).
2. The Semantica host adapter (`quanta-memory-host`) is NOT moved; it stays in
   the Semantica monorepo and depends on this repo (direction:
   `quanta-memory-host -> quanta-memory-platform-{core,facade}`).
3. No cyclic dependency among the three members.
4. No path dependency escaping the workspace after extraction.

## Inputs / Outputs

- Input: this repo (`quanta-memory-platform/`) with committed `Cargo.lock`,
  pinned `rust-toolchain.toml`, and installed `cargo-deny` and `semgrep`.
- Output: a standalone git repo at `<dest>` that builds in isolation with its
  own repo-local rails, plus a one-line manual rewire on the Semantica host
  adapter manifest.

## Step-by-step

1. Dry-run the bootstrap (no writes):

   ```bash
   scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-extracted
   ```

   This runs the fail-closed pre-flight:
   - all three member manifests exist,
   - `core` has no upward dep on facade/testkit,
   - `facade` has no upward dep on testkit,
   - no member has a path dep escaping the workspace,
   - `cargo metadata --locked` resolves without changing the lockfile,
   - the self-hosting extracted file set is present in-source,
   - the copy plan excludes `tools/ci/gates`.

   If any check fails, STOP and resolve before extracting (the pre-flight is the
   cyclic-dependency / direction guard).

2. Perform the extraction:

   ```bash
   scripts/extract-to-standalone-repo.sh /path/to/new/quanta-memory-platform
   ```

   This freezes `HEAD` and its tree ID, rejects relevant staged, unstaged, or
   executable-bit drift, and exports bytes directly from that committed tree.
   The canonical manifest binds every path to its Git mode, blob ID, and
   content SHA-256. Source and extracted manifest digests must match. It
   verifies the extracted workspace before `git init` or any bootstrap commit,
   using:

   ```bash
   scripts/check-workspace.sh
   scripts/memory-platform-doctor
   python3 tools/pm/pm.py lint
   ```

   Campaign ticket admission (`scripts/verify-tickets.sh` and `tickets/**`)
   remains source-checkout governance only and is intentionally excluded from
   the standalone product extraction.

   Both build rails require the lockfile, `semgrep`, and `cargo-deny`; missing
   tooling fails the extraction. After verification, the script commits the
   extracted tree and requires `git status --porcelain` to be empty. A failed
   verification leaves no success commit.

   CI then runs `scripts/check-workspace-offline.sh` as a separate `--frozen`
   lane against the warmed Cargo/advisory caches. It is not a substitute for
   the online locked release rail.

3. Validate repo-local rails in the extracted repo:

   ```bash
   cd /path/to/new/quanta-memory-platform
   scripts/scaffold-external-repo.sh   # required self-hosting file set present, no copied gates
   scripts/check-workspace.sh          # fmt + clippy -D warnings + check + examples + feature matrix + deny
   scripts/memory-platform-doctor      # repo-local doctor rail, includes workspace tests, no tools/ci/gates dependency
   scripts/check-workspace-offline.sh  # separate frozen/cache-only recheck
   scripts/tests/extraction-contracts.sh # negative release-rail contracts
   python3 -m unittest -v scripts/tests/test_extraction_metadata.py scripts/tests/test_extraction_manifest.py # dependency/tree negative fixtures
   python3 -m unittest -v scripts/tests/test_scaling_results.py # performance-gate negative fixtures
   scripts/verify-feature-capabilities.sh # every advertised feature executes a non-zero runtime test
   scripts/verify-doc-test-selectors.py # documented selectors resolve against cargo --list
   scripts/run-scaling-bench.sh --smoke # public store/broker + allocator drift smoke
   cargo metadata --locked --format-version 1 # no re-resolution
   ```

4. Rewire the Semantica host adapter (manual, one edit):

   In `packages/analysis/quanta-v2/crates/quanta-memory-host/Cargo.toml`,
   repoint:

   ```toml
   quanta-memory-platform-core   = { path = "../../../../../../quanta-memory-platform/crates/quanta-memory-platform-core" }
   quanta-memory-platform-facade = { path = "../../../../../../quanta-memory-platform/crates/quanta-memory-platform-facade" }
   ```

   to the extracted location (path, git, or registry). Do NOT add a dependency
   from the extracted repo back to Semantica — that would invert the direction.

5. Host integration proof is separate from extraction. After the rewire, first
   prove that both Semantica workspace roots resolve the same QMP root/tree and
   that the gate target matches Cargo resolution. Then run the registered
   QMP-130 suite bundle from the Semantica root:

   ```bash
   just gate-qmp130-host-adoption
   ```

   Resolve the suite IDs from the host's current suite registry. Do not replace
   the registered QBC receipts with raw Cargo commands or names copied from an
   older Semantica snapshot.

## Standalone extraction gate (QMP-120)

- Extracted repo: `scripts/check-workspace.sh`, `scripts/memory-platform-doctor`,
  `scripts/scaffold-external-repo.sh`, and PM lint all pass. Campaign ticket
  admission remains a source-checkout governance rail and is not an extracted
  product dependency.
- Source/extracted tree-bound canonical digests and file modes match, the source
  stays at the frozen clean `HEAD`, and the extracted git worktree is clean.
- All Cargo resolution is locked; missing Semgrep or cargo-deny fails closed.
- Qualification runs the fresh actual extraction command directly from a clean,
  frozen source checkout. This repository intentionally has no GitHub Actions
  workflow; a nonexistent remote job is not an acceptance dependency.

Host adapter compilation and Semantica static guards are QMP-130 adoption
proof, not evidence that this repo-local extraction rail passed.
