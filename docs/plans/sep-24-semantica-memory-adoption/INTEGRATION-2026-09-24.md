# Worktree cleanup and selected integration — 2026-09-24

Status: CLEANUP_DONE / SMA-001-003_MAIN_INTEGRATED / FINAL_SOURCE_PROOF_BLOCKED.
Semantica main integration is complete; push and product qualification are **not** complete.

## Final main integration

- Semantica main advanced from `1ddf913da9b748fa2fa25d8fab642a89c26ea309`
  to `31a07f43e1f09ab795d7e8832eb82273e334cd98` by fast-forward:
  `17b2702de06f75148ecfa564eea8367056bf47f3` (001/002 API) and
  `31a07f43e1f09ab795d7e8832eb82273e334cd98` (003 memo).
- The integration branch was rebased on then-current main; the original dirty
  memory-adoption checkout was not committed, reset, or deleted. Current-main
  dirty paths did not overlap the 45 merged paths and remained untouched.
- From the source's 46 paths, the unrelated CLI import removal was excluded.
  The two lockfiles gained only the memo adapter dependency; an unrelated
  GQLang dependency deletion was excluded. Existing main manifest-GC behavior
  and its regression test, plus an IFDS capability test, were preserved.
- At integration, `cargo metadata --no-deps --locked`, `git show --check`,
  and the changed Python boundary-guard suite passed on main.
  Targeted `rustfmt --check` passed on the candidate files before merge.
  These are static/Python checks, **not** Rust owner or SDK/product GREEN.
- Internal free space fell below 1 GiB. QBC's earlier owner run refused before
  compilation at 5.3 GiB because it requires at least 50 GiB and 4.75% free;
  its official GC reported zero eligible lanes. No final-main Rust test or
  product E2E executed. A real separate artifact volume or sufficient internal
  capacity is required before SMA-009 can close. No remote push was performed.

## Historical API-only candidate

- Source: `/Users/songmin/Documents/code-new/semantica-memory-adoption-codex`,
  detached `80822f963760cfd33628c1085cc0ec2ceea2411f` + concurrent dirty changes.
- Integration: `/Users/songmin/Documents/code-new/semantica-memory-api-integration-20260924`.
- Branch: `codex/qmp-memory-api-integration-20260924`.
- Parent: Semantica main `e5f0bce3cdc35aab1997d1703d15fe07d14593eb` at worktree creation.
- Later main recheck: `c40bf4c351e4772aff90a14e4540a6f4fa470829`
  with 14 dirty paths. The parent is its ancestor; integration still requires
  current-main conflict and ownership review.
- Commit: `0d04d66410a7a6f74af311d18567a5a5caa7212e`.
- Scope: 20 files; 387 insertions / 1507 deletions. Overlay/public unsupported-axis removal,
  checked installed-policy readback, existing host owner-key reuse and regression sources.
- Excluded: memo-retention ports/implementation/wiring/tests, executor behavior changes,
  unrelated surface and Cargo changes added concurrently to the source worktree.
- Source worktree was not modified or deleted. Its memo work must not be lost.

All selected base files matched the source worktree's committed version before extraction;
the integration did not replace intervening main edits in those files. Mixed composer/test
hunks were separated to exclude memo admission. `git diff --check` and targeted
`rustfmt --check` on all 20 candidate files passed. A workspace-wide
`cargo fmt --all --check` attempt exposed unrelated formatting differences and was
stopped; it is not a successful workspace formatting gate. Integration checkout
is clean after commit. Commit used `--no-verify` per repository convention;
hooks did not provide validation and no runtime GREEN is asserted.

## Verification attempt and blocker

Run from the isolated integration root:

```sh
scripts/quanta-build-cli owner run --lane qmp-api-integration --jobs 2 --wait-seconds 10 --package quanta-runtime-ingress-core --target-kind lib --compile-policy package-all-features
```

The first invocation also consumed a one-time lane token. It exited 1 before compilation:

```text
insufficient free space under /Users/songmin/Library/Caches/semantica-codegraph-v2-target/quanta-build-cli;
free=5.3GiB (0.3%), required=bytes>=50.0GiB and percent_activate>=4.75%
(recover>=5.5%, manual_gc>=5.0%)
```

No test cases executed. Composer/public SDK/product proof remains pending. No capacity
threshold was weakened and no unrelated cache was deleted. `/Volumes/Extreme SSD 1`
resolved to the same internal data filesystem during the probe, not extra mounted capacity.
On the subsequent capacity audit, `scripts/quanta-build-cli gc --dry-run --json`
reported zero eligible lanes, and `scripts/quanta-build-cli gc --json` exited
`blocked` with `insufficient_free_space_after_prune`: zero lanes pruned, zero
delete failures. The GC skipped historical execution-root keys of unknown
provenance. The QBC cache root held only `_state`; its sibling `main` build
cache was about 1.7 GiB. Available space fluctuated around 4–5.5 GiB during checks,
well below the admission gate, so deleting that known cache would not enable
the owner test. No unknown-owner cache or proof state was manually removed.
Resume requires sufficient admitted artifact capacity or an actual mounted, supported
artifact volume. Then use the existing QBC lane if registered and execute 001/002/009 rails.
This was the earlier API-only qualification blocker. The later user-directed
main merge above did not turn it into a release qualification.

## Removed worktrees

Each target had no tracked/untracked changes, was an ancestor of its repository's main,
and had no matching active process command at the pre-removal check. Git removal used no force.

| Removed path | Recoverable source commit | Other removed content |
| --- | --- | --- |
| `/Users/songmin/.codex/worktrees/qmp-redb-qualification-2026-09-24/quanta-memory-platform` | `cb12e7160d4b6a759bbc7e4f6f30ebc24df9d4ae` | None; merged local branch `redb-qualification-2026-09-24` also removed with `branch -d` |
| `/Users/songmin/Documents/code-new/semantica-codegraph-v2-qmp-v5-hostpair` | `0e29a45f9db1a3739b853d0bad75e33eb61fe43f` | Rebuildable ignored Python `__pycache__` directories |
| `/Users/songmin/Documents/code-new/semantica-qmp-proof-20260924-7f1c2f8` | `a71e3e975dcab23aa654ea0c4ea5a66183000412` | Rebuildable ignored Python `__pycache__` directories |

Post-check: all three paths absent and registrations removed. Source commits remain reachable
from main and checkouts can be recreated at the recorded SHAs. No unique source or receipt
was found in the deleted ignored content.

## Preserved intentionally

- QMP `ag5-local-main-merge-20260924`: its sibling Semantica worktree has
  uncommitted work and Cargo depends on this QMP directory by path. No unique
  QMP commits does not make deleting a live dependency safe. The previously
  observed `single-ir-i5-proof` worktree disappeared concurrently; this cleanup
  did not remove it.
- `semantica-codegraph-v2-qmp130-adoption` / `codex/qmp130-main-hardening`: 11 unique
  non-patch-equivalent commits. Do not bulk merge its obsolete dependency pin/version
  compatibility changes. Separate QBC exit/environment fixes still need current-source
  reproduction before selection; do not discard the branch as already merged.
- Original memory-adoption checkout: concurrent memo/executor work remains there.
- New API integration checkout: holds the isolated committed candidate awaiting verification.

QMP had two registered checkouts at the last recheck: main and the retained
dependency-bound copy.
This operation did not clean unrelated Semantica worktrees or publish remote changes.
