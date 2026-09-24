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
  pinned `rust-toolchain.toml`, Python 3.9+, and installed ShellCheck,
  `cargo-deny`, and `semgrep`.
- Output: a standalone git repo at `<dest>` that builds in isolation with its
  own repo-local rails, plus a one-line manual rewire on the Semantica host
  adapter manifest.

## Step-by-step

1. Dry-run the bootstrap (no repository or destination writes; temporary
   manifest files are created and removed under `/tmp`):

   ```bash
   scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-extracted
   ```

   This runs the fail-closed pre-flight:
   - all three member manifests exist,
   - the resolved Cargo graph has no upward core/facade dependency,
   - the resolved graph contains no host back-edge or local path dependency
     escaping the workspace, including renamed dependencies,
   - `cargo metadata --locked` resolves without changing the lockfile,
   - the self-hosting extracted file set is present in-source.

   If any check fails, STOP and resolve before extracting (the pre-flight is the
   cyclic-dependency / direction guard).

2. Perform the extraction:

   ```bash
   scripts/extract-to-standalone-repo.sh /path/to/new/quanta-memory-platform
   ```

   This freezes `HEAD` and its tree ID, rejects relevant staged, unstaged,
   untracked, or executable-bit drift, and exports bytes directly from that
   committed tree.
   The canonical manifest binds every path to its Git mode, blob ID, and
   content SHA-256. Source and extracted manifest digests must match. It
   verifies the extracted workspace before `git init` or any bootstrap commit,
   using:

   ```bash
   scripts/check-workspace.sh
   ```

   The canonical rail requires the lockfile, ShellCheck, `semgrep`, and
   `cargo-deny`; missing tooling fails the extraction. After verification, the
   script commits the extracted tree and requires `git status --porcelain` to be
   empty. A failed verification leaves no success commit.

3. Validate repo-local rails in the extracted repo:

   ```bash
   cd /path/to/new/quanta-memory-platform
   scripts/check-workspace.sh          # canonical static, contract, test, feature, selector, PM, and deny rail
   scripts/run-scaling-bench.sh --smoke # public store/broker + allocator drift smoke
   cargo metadata --locked --format-version 1 # no re-resolution
   ```

4. Rewire the Semantica dependency contract (manual, coordinated change):

   Keep consumer manifests such as `quanta-memory-host/Cargo.toml` on
   `workspace = true`. Update the `quanta-memory-platform-core` and
   `quanta-memory-platform-facade` entries in **both** Semantica workspace
   roots (`packages/analysis/quanta-v2/Cargo.toml` and the top-level
   `Cargo.toml`), their matching
   `[workspace.metadata.quanta_memory_platform_contract]` blocks, and the
   Semantica dependency-contract document together. Change the two mirrored
   entries to the chosen path, pinned Git revision, or published version. Do
   NOT add a dependency from this extracted repo back to Semantica or move
   dependency ownership into a consumer manifest.

5. Host integration proof is separate from extraction. After the rewire, use
   `cargo metadata` from both Semantica workspace roots to prove that Cargo
   resolves the intended identical QMP source. Then run the host's currently
   registered adoption suites from its suite registry. Do not treat an
   external-repository validation override as Cargo dependency resolution.

## Standalone extraction qualification

- Extracted repo: `scripts/check-workspace.sh` passes. It includes ShellCheck,
  resolved dependency-graph, static, contract, test, feature, selector,
  prompt-manager, and dependency checks.
- Source/extracted tree-bound canonical digests and file modes match, the source
  stays at the frozen clean `HEAD`, and the extracted git worktree is clean.
- All Cargo resolution is locked; missing ShellCheck, Semgrep, or cargo-deny
  fails closed.
- Qualification runs the fresh actual extraction command directly from a clean,
  frozen source checkout. This repository intentionally has no GitHub Actions
  workflow; a nonexistent remote job is not an acceptance dependency.

Host adapter compilation and Semantica static guards are host-adoption proof,
not evidence that this repo-local extraction rail passed.
