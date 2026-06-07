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

- Input: this repo (`quanta-memory-platform/`) with green crates 05/06/07.
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
   - `cargo metadata` resolves cleanly,
   - the self-hosting extracted file set is present in-source,
   - the copy plan excludes `tools/ci/gates`.

   If any check fails, STOP and resolve before extracting (the pre-flight is the
   cyclic-dependency / direction guard).

2. Perform the extraction:

   ```bash
   scripts/extract-to-standalone-repo.sh /path/to/new/quanta-memory-platform
   ```

   This copies the canonical file set, `git init` + initial commit, then
   verifies the extracted workspace with:

   ```bash
   scripts/check-workspace.sh
   scripts/memory-platform-doctor
   ```

3. Validate repo-local rails in the extracted repo:

   ```bash
   cd /path/to/new/quanta-memory-platform
   scripts/scaffold-external-repo.sh   # required self-hosting file set present, no copied gates
   scripts/check-workspace.sh          # fmt + clippy -D warnings + check + examples + feature matrix + deny
   scripts/memory-platform-doctor      # repo-local doctor rail, includes workspace tests, no tools/ci/gates dependency
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

5. If you need host-integration proof, run the Semantica boundary gates from the
   Semantica monorepo root after the rewire:

   ```bash
   MEMORY_PLATFORM_REPO=/path/to/new/quanta-memory-platform \
     tools/ci/gates/gate-memory-platform-doctor.sh
   MEMORY_PLATFORM_REPO=/path/to/new/quanta-memory-platform \
     tools/ci/gates/gate-memory-platform-lint.sh
   ```

6. Run the Semantica static guards (the boundary fences):

   ```bash
   cargo test -p quanta-runtime    --test runtime_structure_static_guard
   cargo test -p quanta-memory-host --features full --test host_boundary_static_guard
   ```

## Done criteria

- Extracted repo: `scripts/check-workspace.sh`, `scripts/memory-platform-doctor`,
  and `scripts/scaffold-external-repo.sh` all pass standalone.
- Host adapter compiles against the extracted repo.
- Both Semantica static guards pass.
- `memory-platform-doctor` reports clean using repo-local lint/build/test rails only.
