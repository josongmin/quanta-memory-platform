# Standalone extraction

Export a clean committed QMP source into an independently verified Git repository.
Package direction remains `core <- facade <- testkit`; host adapters remain
outside QMP. No local path dependency may escape the extracted workspace.
Direct SDK use does not require extraction or Semantica adoption.

## Run

Requirements: the committed lockfile and pinned toolchain, Python 3.9+,
ShellCheck, Semgrep and cargo-deny. Missing requirements fail closed.

```bash
# Source-side graph, lockfile and canonical-file preflight only
scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-extracted

# Actual export, destination verification and bootstrap commit
scripts/extract-to-standalone-repo.sh /path/to/qmp-extracted
```

The actual command freezes HEAD/tree, rejects relevant staged, unstaged,
untracked and file-mode drift, and exports committed bytes. The canonical
manifest binds paths to Git mode, blob ID and SHA-256; destination content
must match. The destination runs `scripts/check-workspace.sh` before Git
initialization and the bootstrap commit. Failed verification creates no
success commit; a successful destination has a clean worktree.

## Public examples and CI

```bash
# Shared verifier: actual extraction plus all three public examples against its manifest
scripts/verify-extracted-examples.sh
```

CircleCI's main-push and PR-update workflow is the required hosted verifier.
The `run_heavy=true` pipeline runs scaling receipts only on explicit request.
A CI configuration or dry-run pass does not prove that extraction, examples or a
hosted job executed. Record
the exact source and actual result as specified by
[ADR 0003](docs/adr/0003-source-bound-qualification.md).

## Optional consumer integration

After extraction a consumer may select a local path or pinned Git revision.
That integration is owned by the consumer and does not block SDK completion.
For Semantica, keep consumer dependencies on `workspace = true`; update both
workspace roots, their QMP dependency-contract metadata and contract document
together. Confirm Cargo resolves the same intended QMP source from both roots
before executing applicable host suites. An environment override is not proof
of dependency resolution. See [ADR 0004](docs/adr/0004-host-adoption-boundary.md).
