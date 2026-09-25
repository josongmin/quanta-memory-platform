# DURABILITY-001 — finalize instruction-point kill and physical power-loss qualification

Status: `HISTORICAL / OUT OF CURRENT PRODUCT SCOPE. L1, Linux syscall-EIO L2 (2/2), and disposable device-mapper L2 (1/1) verified on the recorded source snapshot; physical power-loss L3 remains unqualified. Do not schedule these as current release gates.`
Contract: `option 1 (existing contract accepted)` per §2 planning default; no host contract presented requiring a durable publication ACK. The L1 after-oracle asserts no resurrection and no replay expectation, and asserts nothing about the killed process's volatile resident copy.
Priority: `historical conditional P1; no current product/release priority`
Source freeze: `f637d424724aade94564ca76bb5ca6d9b73b28ae` (`main == origin/main`, clean when planned)
Owner: `quanta-memory-platform facade / durability qualification`
Depends on: closed `FUP-001`, `FUP-002`, and the existing `FUP-006` SIGKILL/reopen rail

## 1. Current fact and target

The existing public-API rail kills a child with real `SIGKILL` and cold-reopens
the same Redb file. It proves three scheduler-selected boundaries:

- committed rows plus a persisted restore claim;
- an arbitrary non-empty prefix of a write stream;
- a reserved-but-never-committed hole.

It does **not** stop at a named instruction inside
`RedbSpillBacking::finalize_restore`, interrupt Redb while its commit is in
flight, or remove power from the storage device. `SIGKILL`, emulated block-I/O
failure, VM power-off, and physical power removal remain distinct evidence.

This ticket adds the cheapest trustworthy evidence first:

| Level | Boundary | Default cadence | Closure claim |
| --- | --- | --- | --- |
| L1 | deterministic kill immediately before and after the finalize transaction commit | PR/canonical rail | QMP finalize linearization and cold-reopen behavior |
| L2 | Linux syscall/block-device disruption during `pwrite`/`fsync` | opt-in/nightly | kernel/filesystem recovery under injected I/O disruption; **not** physical power loss |
| L3 | remotely controlled physical power removal on a production-equivalent storage tuple | release qualification | only the exact tested hardware/firmware/OS/filesystem tuple |

Do not run L3 on a developer laptop, shared workstation, or a disk containing
non-disposable data.

## 2. Contract decision before implementation

`RevocableStore::restore_one_durable_bounded` publishes a resident entry and then asks
the durable controller to finalize. A successful Redb finalize removes the
recovery payload and persists the exact generation's terminal proof in one
transaction.

The current contract therefore treats the Redb commit as the terminal durable
handoff. If the process dies after that commit, the resident entry dies with the
process and the Redb record remains `Finalized`; it is not replayed after
restart. This is valid only when restored entries are revocable/reconstructible
and finalize is the terminal acknowledgement.

Before calling this ticket GREEN, record one of these decisions:

1. **Existing contract accepted:** post-finalize process loss may lose the
   resident copy; the terminal proof prevents duplicate replay. Continue with
   the plan below.
2. **Durable-until-host-ACK required:** stop this ticket. Add a host-owned
   durable publication acknowledgement/state machine; do not make the test
   expect replay from the current terminal state. That is a protocol redesign,
   not a stronger assertion over the existing implementation.

Planning default: accept option 1 because this is a revocable store and the
current API defines finalize as terminal. Reopen the decision only if a host
contract identifies the restored resident entry as non-reconstructible or
requires a separate durable publication ACK.

Forbidden shortcut: classify `Finalized` as replayable after a crash. That
would resurrect an already-consumed record and break the exact claimant proof.

## 3. L1 — deterministic finalize instruction-point kill

### 3.1 Only two persisted-state points are required

Use named hooks at the two sides of the actual linearization point in
`RedbSpillBacking::finalize_restore`:

- `FinalizeBeforeCommit`: recovery removal, active-claim removal, disposition,
  and finalize proof are staged in the Redb write transaction, but
  `txn.commit()` has not been called.
- `FinalizeAfterCommit`: `txn.commit()` returned `Ok(())`, but QMP has not run
  ACK-loss handling, probed state, or returned to the caller.

Do not add redundant hooks before every table mutation. Those writes are part
of the same uncommitted Redb transaction and have the same externally visible
state. Do not use sleeps or probabilistic scheduling to approximate either
point.

### 3.2 File-by-file implementation

| File | Required change | Writer boundary |
| --- | --- | --- |
| `crates/quanta-memory-platform-facade/Cargo.toml` | Add non-default `durability-test-hooks = ["spill-redb"]`. Add a dedicated `spill_redb_finalize_kill_child` example requiring both features. Keep `full` and `full-backings` unchanged; this is test instrumentation, not a production capability. | Manifest owner only. Re-run feature/extraction rails. |
| `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs` | Under `durability-test-hooks`, define the two-point enum and a process-local hook registration used only by the dedicated child. Invoke before `txn.commit()` and immediately after its `Ok(())`. Normal/default builds must compile to the existing path with no environment lookup, I/O, global hook, allocation, or branch. | Do not expose `redb` types. Do not change transaction composition or claim semantics. |
| `crates/quanta-memory-platform-facade/examples/spill_redb_finalize_kill_child.rs` | Create one self-describing record, drive restore through the real public revocable-store/controller path, install exactly one requested hook, emit `ARMED <point> <sequence> <generation>` over the inherited control pipe, then block. The child must never exit normally after `ARMED`. | Test tool only. Do not duplicate finalize logic. |
| `crates/quanta-memory-platform-facade/tests/spill_redb_finalize_kill.rs` | Fresh-build the child with both features, wait for the exact `ARMED` handshake, send real `SIGKILL`, verify signal 9, cold-reopen the same file in a new process context, and assert the point-specific oracle below. | No timing assumptions; timeout is failure, not skip. |
| `docs/verification-scenarios.md` | Add the two selectors only after they pass on the final source. State that they bracket QMP's commit call but do not stop inside Redb's `pwrite`/`fsync`. | Evidence wording must stay narrower than L2/L3. |
| `docs/plans/sep-23-followup-misc/FUP-006-integration-qualification.md` and `RECEIPT.md` | Replace only the `mid-finalize instruction-point kill` residual after exact-source qualification. Keep physical power loss open until L3 passes. | No prospective GREEN text. |

The hook registration may be `#[doc(hidden)] pub` only under the non-default
test feature because an integration-test child is a separate crate. It must be
impossible to enable through `full` or `full-backings`. A generic production
callback, environment-variable kill switch, or always-present global hook is
forbidden.

### 3.3 Point-specific semantic oracles

`FinalizeBeforeCommit` after cold reopen:

- Redb opens without destructive reset.
- The record is still `Present` in recovery, never `Finalized` or `Absent`.
- Recovery issues a fresh exclusive generation bound to the same backend ID.
- Decode/charge/publish/finalize converges exactly once.
- A second drain is empty; retained inventory and active claim are zero.
- Payload sequence and checksum are unchanged.

`FinalizeAfterCommit` after cold reopen:

- Redb opens without destructive reset.
- The exact record resolves `Finalized`.
- Recovery payload and active claim are absent.
- `recover_active_restore` and ordinary FIFO prepare return empty for the record.
- The ID cannot be recreated with identical or different bytes.
- No unrelated record's claim or proof is changed.

The post-commit oracle must not claim that the killed process's volatile
resident value survives. That depends on the contract decision in section 2.

### 3.4 Focused commands and cost

```bash
cargo test --locked -p quanta-memory-platform-facade --features spill-redb,durability-test-hooks --test spill_redb_finalize_kill -- --nocapture

# bounded repetition inside one test process; fresh database per iteration
QMP_FINALIZE_KILL_ITERATIONS=20 cargo test --locked -p quanta-memory-platform-facade --features spill-redb,durability-test-hooks --test spill_redb_finalize_kill -- --nocapture
```

The first command belongs in normal qualification. The 20-run loop is an
opt-in stress receipt; it must not be duplicated inside every canonical run.
Expected implementation plus focused verification: 4–8 engineering hours.

### 3.5 Negative proof

Run temporary mutants in an isolated worktree and remove them afterward:

- omit the `Finalized` disposition insert (M1 as executed; see deviation
  note below): post-commit oracle RED;
- leave `SPILL_RECOVERY` present: post-commit oracle RED (fail-closed
  `resolve_record` rejects payload evidence paired with a `Finalized`
  disposition);
- move the `FinalizeBeforeCommit` hook after `txn.commit()`: pre-commit oracle
  RED;
- downgrade parent kill to clean child exit: signal-9 assertion RED.

`0 selected`, stale child binaries, and a child that exits before the parent
kill are failures, never evidence.

### 3.6 M1 deviation (proof row unobservable via public API)

The specified M1 (omit `SPILL_FINALIZED_BY` insertion) cannot turn the
post-commit oracle RED through any public path: `resolve_record` keys
`Finalized` off the disposition row alone, and the only proof reader is the
same-claim finalize retry — but `DurableRestoreClaim::issued` is `pub(crate)`
by anti-forgery design, so a survivor process cannot reconstruct the victim's
claim after the kill. Executed M1 instead omits the `Finalized` disposition
insert, which the oracle pins (RED confirmed). Proof-row persistence remains
covered in-module by the FUP-002 terminal-proof-first retry tests, which hold
real claims.

## 4. L2 — cheap in-flight I/O disruption without false power-loss claims

L1 brackets the Redb commit call but cannot pause inside dependency-owned
`pwrite`/`fsync`. Add an opt-in Linux-only rail after L1 is stable:

1. Run the real Redb finalize transaction against a disposable database.
2. Use `dm-flakey`/`dm-error`, or a test-only syscall interposer, to fail or
   block writes and sync operations during the commit.
3. After an interrupted/failed transaction, cold-reopen and apply the same
   state-machine verifier. Device-mapper runs must also detach the disposable
   target before reopen.
4. Record whether the run interrupted data write, first sync, commit-marker
   write, or final sync when the mechanism can observe it.

Proposed files:

| File | Purpose |
| --- | --- |
| `scripts/durability/run-emulated-linux.sh` | Fail-closed Linux-only runner for syscall-level EIO injection. Uses disposable Redb files, requires effective UID 0 plus `strace`, rejects artifact paths inside the repo/home, and returns exit 77 `UNAVAILABLE` when OS or privilege prerequisites are absent. |
| `scripts/durability/run-device-mapper-linux.sh` | Explicit-opt-in Linux/UID-0 runner. Creates its own 128 MiB sparse image, loop device, ext4 filesystem and UUID-named DM mapping; never accepts a caller-supplied raw device. Retains the backing image/receipt and cleans up its mapping and loop device. |
| `crates/quanta-memory-platform-facade/examples/spill_redb_durability_l2.rs` | Calibrates/injects syscall EIO or pauses at staged finalize while a disposable DM mapping is changed to `error`. It requires an observed post-marker syscall `EIO`, restores via `--noflush`, unmounts, runs `e2fsck`, detaches the DM target, remounts from the backing loop, cold-reopens and verifies whole-state convergence. Raw traces, DBs, filesystem-check output and hashes are retained outside the image. |

Safety requirements:

- refuse `/`, `$HOME`, repository paths, caller-supplied devices, and absent
  explicit disposable-VM opt-in;
- create and use only the runner-owned sparse image/loop/DM target; do not add a
  raw-device backend without a separate allowlist and safety review;
- preserve failed trace/database artifacts and receipts for forensics;
- never describe VM shutdown or device-mapper faults as physical power loss.

L2 is not required on every PR. Run both subrails nightly or before a
durability release. Syscall injection proves syscall-error convergence; the
DM rail demonstrates one disposable block-I/O EIO, filesystem recovery and
cold reopen for its exact source/environment. Neither rail proves power loss.

## 5. L3 — physical power-loss qualification

### 5.1 Required topology

- disposable device-under-test (DUT), not a developer workstation;
- production-equivalent storage device, controller/transport, firmware,
  filesystem, mount options, kernel, and Redb/QMP build;
- independent controller machine;
- remotely controlled PDU or relay that removes actual DUT/storage power;
- receipts streamed to the controller before power removal. Do not use the DUT
  disk itself as the sole expected-state authority.

Each qualified tuple is independent. Passing USB/APFS does not qualify
NVMe/ext4, and passing one SSD firmware does not qualify another.

### 5.2 Minimal high-value scenario set

Use self-describing payloads containing run ID, sequence, phase, length, and a
checksum. Split workloads so the expected state is unambiguous:

| Scenario | Controller-visible ACK | Allowed state after reboot | Forbidden state |
| --- | --- | --- | --- |
| `spill-commit` | commit returned after Redb sync | every ACKed record is `Present` or was terminally finalized by a later ACKed phase | missing/torn ACKed payload, duplicate ID |
| `restore-commit` | recovery commit returned | exact payload exists in recovery under one recoverable claim | no payload and no exact terminal proof, two owners |
| `finalize` | phase markers before call and after return | unacknowledged finalize is either fully recoverable or exactly finalized; ACKed finalize is exactly finalized | partial table state, wrong generation proof, resurrection |
| `reserved-hole` | reserved ID and later committed/finalized ID reported | hole remains `Absent` | hole inferred `Finalized` from a later ID |

The controller selects a random cut delay and records the last off-DUT phase
marker. After automatic reboot, a verifier opens the database through public
QMP APIs and emits a JSON receipt. Stop on the first invariant violation and
retain the database image/device snapshot when possible.

### 5.3 Efficient run policy

Use escalating cohorts rather than immediately running a multi-day soak:

1. Bring-up: 5 cuts per scenario on one disposable tuple.
2. Candidate: 25 cuts per scenario, zero invariant violations.
3. Release: choose total `N` from the product durability target. With zero
   failures, the approximate one-sided 95% upper failure-rate bound is `3/N`;
   100 cuts only bounds it near 3%, while 1,000 cuts bounds it near 0.3%.

Counts demonstrate sampling depth, not universal safety. A single corrupt,
unopenable, torn, duplicated, resurrected, or unowned state is RED regardless
of pass percentage.

Estimated cost with existing DUT/PDU: 4–8 hours for harness integration plus
the reboot time for the selected cohort. Without a DUT and remotely controlled
power path, L3 remains `UNAVAILABLE`, not failed and not GREEN.

### 5.4 Durability-mode decision

The locked dependency is Redb `2.6.3`. Its write transactions default to
`Immediate` durability; two-phase commit is disabled unless explicitly
enabled. Do not change this blindly.

- First bind baseline L2/L3 receipts to the exact current mode.
- If the production threat model requires the extra two-phase protection, add
  an explicit backing configuration and apply it consistently to every
  lifecycle transaction, not only finalize.
- Benchmark commit latency, throughput, file growth, and recovery time for
  one-phase versus two-phase mode.
- A mode change is a separate API/performance decision and requires a new exact
  source receipt plus host qualification.

## 6. Receipt schema and evidence ownership

Every L1/L2/L3 receipt must contain:

- QMP commit, tree, dirty fingerprint, and canonical source digest;
- Cargo.lock Redb version/checksum;
- test level and exact scenario/kill point;
- selected test count, attempt count, seed, and all exit statuses;
- pre/post record-state summary and payload checksums;
- for L2: kernel, filesystem, fault mechanism, image/device identity;
- for L3: OS/kernel, filesystem/mount options, device model, firmware,
  controller/transport, volatile-write-cache/PLP status, PDU/relay identity,
  power-off/on timestamps, and boot/reopen duration;
- raw artifact hashes and external artifact location.

Raw power-cycle artifacts should live outside Git. Commit only the compact
source-bound summary and artifact hashes. A receipt with a different source
digest, missing hardware tuple, zero selected tests, skipped prerequisite, or
unverifiable external artifact is inadmissible.

## 7. Promotion sequence

After implementation, qualify one frozen clean candidate in this order:

1. L1 focused selectors and negative mutants.
2. `scripts/check-workspace.sh`.
3. `scripts/run-scaling-bench.sh` and both public examples.
4. real standalone extraction with its internal canonical rail.
5. L2 on Linux if the release contract includes injected I/O disruption.
6. L3 for every production storage tuple claimed as power-loss qualified.
7. Semantica dependency-path/source-digest check, compose QBC, and
   `quanta-memory-host` QBC on the exact QMP candidate.
8. Update `RECEIPT.md`, create a new immutable annotated tag, push, then keep
   hosted CI/deploy/activation as separate gates.

Any Rust, manifest, script, durability-mode, or receipt-verifier change after
step 1 invalidates downstream evidence and restarts qualification at the
cheapest affected level.

### Cost-aware test schedule

Use the smallest rail that can falsify the changed behavior, then promote once
the related edits settle. Do not rerun the full suite after each patch and do
not launch concurrent Cargo commands against the shared `target/` directory.

| Change | Cheapest useful check | Promotion / invalidation |
| --- | --- | --- |
| L1 hook, finalize ordering, recovery/disposition oracle, or crash child | Run only `spill_redb_finalize_kill` serially; require non-zero selectors and cold-reopen state assertions. | Run the four L1 negative mutants when L1 behavior/oracles change; run one `scripts/check-workspace.sh` after the whole source batch settles. |
| L2 injector, syscall matching/calibration, L2 example, feature wiring, or receipt generation | Run the two L2 injected-EIO cases on Linux; verify raw trace plus reopen oracle and source digest. | Rerun after any executable-source digest drift. Run opt-in 20-iteration L1 stress only for release/nightly confidence, not each edit. |
| Unrelated Rust production/test changes | Run the narrow owning-crate test(s) or selector(s) first. | One canonical `scripts/check-workspace.sh` for the integrated candidate; its embedded selectors already cover workspace tests, feature capability, and docs selector resolution. Avoid duplicating those suites separately. |
| Docs/ticket wording only | `git diff --check`; run the relevant doc/PM lint if generated guidance sources changed. | No Cargo rerun unless code, selector manifests, or executable scripts changed. |
| Frozen candidate / release admission | Freeze exact source and dirty fingerprint first. | Run focused L1 + mutation battery, canonical check, public examples, extraction dry-run, benchmark, L2 if contract requires it, then exact-source host/QBC rails. L3 remains a separate physical qualification gate. |

Stop early on the first RED or source-digest mismatch. Fix the owner, rerun
that narrow rail, then do the single integrated canonical pass. Keep stress,
L2, extraction, benchmark, host, and L3 out of the edit loop unless the changed
surface invalidates their evidence or the candidate has reached its admission
gate.

## 8. Completion and reopen conditions

L1 complete only when:

- both named kill points select non-zero tests and pass on the same source;
- child death is proven as forced termination;
- cold reopen satisfies the exact point-specific state oracle;
- all four negative mutants are RED when the L1 implementation/oracle changes.

L2 syscall subrail complete only when:

- both injected syscall cases are selected and pass on Linux;
- syscall calibration is tied to the finalize operation, and the raw trace,
  reopened database, environment, and source digest are retained;
- the report explicitly says injected `EIO` is not power-loss qualification.

Frozen-candidate integration complete only when the exact clean candidate has
passed the canonical workspace rail, both public examples, extraction,
benchmark, and exact-source host/QBC rails required by FUP-006. These are
release-admission gates, not prerequisites for saying the isolated L1 or L2
test slice works. They must not be rerun during every narrow edit cycle.

L3 complete only when:

- the contract explicitly names the qualified storage tuple;
- the configured cohort completed with zero invariant violations;
- controller-side receipts and raw artifact hashes are retained;
- no emulation/SIGKILL evidence is substituted for physical power removal.

Reopen immediately on database repair failure, torn or missing ACKed payload,
wrong-generation proof, duplicate owner, finalized-record resurrection,
reserved-hole misclassification, source-digest drift, or a different
production hardware/filesystem tuple.

## 9. Recommended execution split

- Slice A, L1 contract/hooks/oracles/docs: implemented; focused, mutation,
  stress, and current-source canonical evidence recorded in §10+.
- Slice B, L2 syscall-error subrail: implemented and current-digest qualified
  on native ARM64 Linux. Device-mapper/kernel-filesystem failure injection is
  still a separate optional extension, not part of this GREEN subrail.
- Slice C, physical qualification: needs a disposable production-equivalent
  DUT plus an external PDU/relay controller. Setup/runtime cannot be estimated
  responsibly until the hardware tuple and cohort size are supplied.
- Slice D, serial integrator: requires an approved frozen clean candidate,
  then extraction/benchmark/host-QBC and receipt promotion. Do not commit,
  tag, push, or deploy without the separate FUP-006 approval.

Slices A and B must not edit the same facade manifest or crash-child files in
parallel. Slice C can prepare external controller infrastructure in parallel,
but its final receipt must use the frozen candidate produced by Slice D.

## 10. L1 execution record (2026-09-23)

Slice A implemented on workdir `f637d42` + uncommitted L1 files (dirty; see
caveat). L2/L3 not attempted (no Linux+root host, no DUT/PDU: `UNAVAILABLE`
by §4/§5 policy, not GREEN).

Files (this slice only; a concurrent management-bounds batch owns the core
`management_limits` files and is NOT part of this receipt):
`facade/Cargo.toml` (`durability-test-hooks = ["spill-redb"]`, child
`required-features`, `full`/`full-backings` unchanged),
`spill/backings/redb.rs` (cfg-gated two-point hook module + 2 call sites),
`examples/spill_redb_finalize_kill_child.rs` (new),
`tests/spill_redb_finalize_kill.rs` (new),
`scripts/verify-feature-matrix.sh` (absence assertion for the new feature in
default/full/full-backings), `docs/verification-scenarios.md` (2 selectors),
FUP-006/`RECEIPT.md` (dated addenda; v5 history preserved).

Results, all on the same dirty workdir:
- Focused: `cargo test -p facade --features spill-redb,durability-test-hooks
  --test spill_redb_finalize_kill` → 2/2 GREEN (non-zero selected).
- Stress: `QMP_FINALIZE_KILL_ITERATIONS=20` → 2/2 GREEN, 40 kills, 11.64s.
- Negative proof in an isolated removed-afterward copy (`/tmp/qmp-durability-mut`):
  M1 disposition-omission RED, M2 recovery-left-present RED, M3 hook-swap RED,
  M4 clean-exit-assertion RED; every revert shasum-matched the workdir file.
- `scripts/check-workspace.sh` → exit 0, `check-workspace: clean`, 224
  selectors resolve (217 + 2 L1 + 5 management-bounds).

Caveat: exact-source purity is NOT claimed — evidence was collected on a dirty
tree shared with the uncommitted management-bounds batch. Slice D must rerun
§7 steps 1–4 + 7–8 on a frozen clean commit before any tag; the mid-finalize
residual replacement above is conditional on that rerun staying GREEN.

Stability record: 7 GREEN L1 runs (focused, 20-iteration stress, canonical
rail, 3× sequential rerun, 1× single) vs 1 RED run that occurred only while
three top-level cargo commands shared one target dir in parallel (L1 suite +
two example builds); both tests hit the 30s ARMED handshake timeout and
failed closed. No code change made: the timeout matches the FUP-006 rail
precedent and the authoritative sequential rail is GREEN; do not run parallel
cargo top-level commands against one target dir during qualification.

Extraction: workdir `--dry-run` correctly refuses the dirty tree per playbook
(committed-candidate rule). The same content committed in a scratch repo
passes `--dry-run` exit 0, so the new feature/example manifest entries break
no extraction logic.

### Repeatable local test order (2026-09-23)

On the current shared dirty worktree, ran the cheapest decisive rail first,
then one canonical pass (no parallel Cargo invocations and no duplicate suite
inside the canonical run):

1. Focused L1 selectors, serial: 2/2 GREEN, 8.51s.
2. Opt-in stress, 20 fresh databases per selector: 2/2 selectors GREEN, 40
   forced kills, 34.34s.
3. `scripts/check-workspace.sh`: exit 0, about 68s. This covered formatting,
   policy/static rails, all-feature workspace tests, the per-crate feature
   matrix, 10 non-zero feature-capability selectors, all 224 documented test
   selectors, PM lint, and cargo-deny. Do not separately rerun those same
   suites for this candidate absent a source change.
4. Both public facade examples: exit 0.

These were integration/regression receipts before the L2 addition, not a
frozen-candidate release receipt. The working tree was not committed; do not
promote these results to the clean exact-source gate. The
20-iteration stress is opt-in/nightly evidence and should not be added to every
PR or run concurrently against the canonical Cargo target directory. On source
change, rerun focused L1 first; run the canonical rail once after all related
changes settle. Keep the slower extraction/benchmark/host qualification behind
the required frozen-candidate gate. Physical L3 and block-device L2 remain open.

### L2 syscall subrail execution record (2026-09-23)

Ran `scripts/durability/run-emulated-linux.sh` in an isolated Docker Desktop
Linux VM (`6.10.14-linuxkit`, `overlayfs`, `strace 6.1`, Redb `2.6.3`) with
the repository mounted read-only and artifacts on a separate host temp mount.
Source HEAD:
`edfef66c3d9dabbe4d01a98754a44d32ef9c940e`; test-source digest:
`e72a62f491891e7a9284e8652cc9f3f5845ce271c8091d1e4dd83c89d5d3c9f2` over
Cargo/Rust/test/script inputs (planning and receipt documents excluded). The
Redb files were on the Docker VM's `overlayfs` (filesystem device id hex `40`);
they were copied with the syscall traces to the external host temp mount.

- `pwrite64` EIO at the first finalize write (ordinal 9): worker returned
  error; cold reopen found the exact `Present` payload and committed claim;
  fresh recovery finalized it exactly once and drained the unrelated record.
- `fdatasync` EIO at the first finalize sync (ordinal 4): worker returned
  error; cold reopen found the whole `Finalized` state, no recovery claim,
  and no resurrection; the unrelated record drained exactly once.
- Receipt: `/private/tmp/qmp-durability-l2-kRYJoo/run-04/receipt.txt`.
  Raw traces and four Redb files are alongside it; SHA-256 values are in the
  receipt. Earlier `/run-01` through `/run-03` runs are superseded by `/run-04`'s
  final Linux L1 + stress + L2 batch and same test-source digest. The EIO
  injections make no claim about actual device error
  propagation, cache loss, or filesystem crash recovery. Device-mapper L2 and
  physical power-loss L3 remain OPEN.

### Final-source revalidation (2026-09-23)

This section records the earlier `e72a62f4…` snapshot and is superseded by the
current-run addendum below; it is retained as historical evidence only.

Current candidate: HEAD `edfef66c3d9dabbe4d01a98754a44d32ef9c940e` plus the
uncommitted L2 manifest/example/runner and this documentation update. The
executable test-input digest, excluding plans/receipts/docs, is
`e72a62f491891e7a9284e8652cc9f3f5845ce271c8091d1e4dd83c89d5d3c9f2`.

- Linux VM focused L1: 2/2 GREEN, 1.98s; opt-in 20× stress: 2/2 selectors,
  40/40 forced kills GREEN, 11.59s.
- L1 mutation battery on a fresh detached worktree at `edfef66`: all four
  mutants RED, then every source file restored byte-for-byte and the temporary
  worktree removed. M1 omitted the disposition insert and failed cold resolve;
  M2 retained recovery payload and failed the Finalized/payload consistency
  oracle; M3 moved the before-commit hook after commit and failed the Present
  oracle; M4 changed SIGKILL to SIGTERM and failed `signal() == 9`.
- L2 syscall EIO: 2/2 GREEN in the same Linux batch. Final raw receipt and
  hashes: `/private/tmp/qmp-durability-l2-kRYJoo/run-04/receipt.txt`.
- Darwin `scripts/check-workspace.sh`: exit 0; core 242, facade 282, testkit
  82, 10 non-zero feature-capability selectors, 224 documentation selectors,
  feature matrix including `durability-l2`, PM lint, and cargo-deny all GREEN.
- Both public facade examples and `shellcheck scripts/durability/run-emulated-linux.sh`: GREEN.

The working tree still has only the DURABILITY-001 implementation/docs
uncommitted; FUP-006 treats commit/tag/push as a separate approval gate.
Extraction/benchmark/host-pair are not requalified on this source. The
block-device L2 extension and physical L3 still require external hardware and
remain open.

### Current-run addendum (2026-09-23)

After the prior L2 `/run-05` receipt, executable source changed in unrelated
shared-worktree paths (`core/api/control.rs`, `core/application/scheduler.rs`,
`core/ports/{reclaim,telemetry}.rs`, and `facade/src/shared/measure.rs`). The
current executable-input digest, calculated with the L2 runner's exclusion
rules, is `18d3eef24beef31c6a2426cb01a44f357bc73dbe1db08655dad79b056f961561`.
Therefore `/private/tmp/qmp-durability-l2-kRYJoo/run-05/receipt.txt` remains a
valid receipt for its recorded `d75771e8…` snapshot, but is not evidence for
this current source. Rerun L2 only after the source batch is frozen; no claim
is made that unrelated current edits were covered by that Linux run.

On the current shared worktree at HEAD `edfef66c3d9dabbe4d01a98754a44d32ef9c940e`:

- `scripts/check-workspace.sh`: exit 0, `check-workspace: clean`; core 245,
  facade 282, testkit 82, 10 advertised feature paths, and 224 documented test
  selectors. Cargo cache/build lock contention extended elapsed time; no
  duplicate suite was launched by this run.
- Both public facade examples and `git diff --check`: GREEN.
- The working tree remains dirty with multiple shared/user-owned source and
  documentation edits plus the DURABILITY-001 L2 manifest/example/runner.
  These results are useful integrated-worktree checks, not a frozen-candidate
  receipt. Extraction, benchmark, host/QBC, final-source L2, block-device L2,
  and physical L3 remain pending.

### Final current-source run (2026-09-23)

Current executable-input digest is
`add1f9076c03336f0bcc4779616514532025a725539a979b0321920eedfb6d4d`, with
HEAD `edfef66c3d9dabbe4d01a98754a44d32ef9c940e`. The digest was identical
before/after the following final rails:

- Native ARM64 Linux syscall subrail: 2/2 GREEN (`pwrite64` EIO ordinal 9:
  `Present -> recovered -> Finalized`; `fdatasync` EIO ordinal 4: whole
  `Finalized`, no resurrection). Receipt and raw artifacts:
  `/private/tmp/qmp-durability-current.xHkS7e/run-12/receipt.txt` and adjacent
  files. Receipt binds `architecture=aarch64`, Linux 6.10.14-linuxkit,
  `overlayfs`, device id `40`, and `strace 6.1`; it explicitly says
  `power_loss_proven=false`.
- Docker `linux/amd64` attempts produced empty syscall traces despite the worker
  reaching finalize. The harness now rejects an empty trace explicitly and
  retains baseline child stdout/stderr. Do not accept those runs as L2 proof;
  native ARM64 ptrace produced complete traces and the current-source pass.
- `scripts/check-workspace.sh`: exit 0, `check-workspace: clean`; 10 advertised
  feature paths and 225 documented selectors resolved. Both public facade
  examples, `shellcheck scripts/durability/run-emulated-linux.sh`, and
  `git diff --check`: GREEN.

This is exact-digest integrated-worktree evidence, not a clean frozen-candidate
release receipt. The worktree still contains shared/user-owned changes.
Benchmark and extraction scripts correctly require a frozen/clean candidate;
do not bypass that precondition. Exact-source host/QBC, device-mapper L2, and
physical L3 remain unqualified. L3 additionally requires a disposable
production-equivalent DUT and externally controlled power removal; no VM,
SIGKILL, or syscall injection substitutes for it.

### Current read-panic/callback hardening update (2026-09-23)

Executable inputs changed again after the `add1f907…` `/run-12` Linux receipt:
the shared worktree now has L2 source digest
`185dd3329b00d685b866c992c03e50b1bcaa1f8a1097b0310275abb05d4dd2fa` at HEAD
`edfef66c3d9dabbe4d01a98754a44d32ef9c940e`, tree
`4f1a7299af049e7fa1c0eeefc8e683f01239d462`. The changed Rust paths include
`core/api/control.rs`, `core/ports/telemetry.rs`, facade `payload_store`,
`retained_map`, `transcript_store`, and `shared/lifecycle.rs`. The `/run-12`
EIO receipt remains valid only for its recorded digest; final-source L2 must be
rerun after the candidate is frozen. The canonical workspace rail and public
examples pass on this digest, but no Linux syscall injection was run for it.

### Current runner-input capture fix (2026-09-23)

The current worktree removes tracked `.cursorrules` and adds `.cursor/rules`.
Because `git ls-files -co` lists deleted tracked paths, the L2 digest pipeline
would otherwise fail on the missing path before building the example. The
runner now filters out non-existent inputs; current test-input digest is
`51433fd5d80f61348a288bf17d2947b4a3cd8ca0278c757de9c0279215d8b1a6`.
ShellCheck, syntax, formatting, and diff checks pass; running the runner on
Darwin exits 77 as designed. This is a digest-capture regression fix only, not
an L2 execution receipt. Rerun on Linux after freezing the candidate.

### Session-hardening work-directory cleanup (2026-09-23)

The L2 example copied traces and Redb databases to the artifact directory but
left duplicate source artifacts in its separate system-temp work directory.
It now removes that directory after copy, with best-effort RAII cleanup on
early returns. Updated executable-input digest:
`ef0484f9776a4e51da9ba33dacda9a3dbd5a954485ed0adc08c77c34d0813ad7`.
The Linux-only branch was not executed on Darwin; static/format proof does not
replace the frozen-source Linux L2 rerun.

### Linux-branch/root hardening correction (2026-09-23)

Adversarial review found that the cleanup revision attempted to inspect
filesystem metadata after deleting the work directory and passed `WorkRoot`
rather than its `Path` to `Command::arg`; Darwin clippy could not type-check
that `cfg(target_os = "linux")` branch. The candidate now captures filesystem
and device identity from `work_root.path` before cleanup. Both the runner and
the example independently require effective UID 0, and receipts record the
observed UID. Fault-child stdout/stderr are retained, and the runner records
the post-run executable-input digest plus Cargo status even when a scenario
fails. `bash -n`, ShellCheck, rustfmt, recursive feature-exclusion inspection,
Darwin exit 77, and `git diff --check` pass. No Linux compile or runtime claim
is made by those static checks; an exact-current-source root+strace Linux 2/2
receipt remains required.

### Session-hardening review (2026-09-25)

Base at review start: `5ea34c647ddf6027f470dd1e027ae3108d0ffdd6`; the shared
branch advanced during this review to `aec55dac1132709df5253333005638928942deec`
and is now equal to `origin/main`. Unrelated dirty owners at closeout are
`src/publish_pipeline/prepared_publication.rs` and
`scripts/lib/self-hosting-manifest.sh`; they were preserved.

The L2 proof owner had three missing assertions: the uninjected worker was not
required to print successful finalize completion; the fault trace did not
prove the injected syscall was after the finalize marker and at the calibrated
ordinal; and the faulted worker was not required to return an error. These
guards were added to `spill_redb_durability_l2.rs` and landed in `aec55da`.
The current ARM64 Linux run at executable-input digest
`1f76b6ae388ac42a4caef0759a42446204ea4f1e58627c2dd396dd9f80b9cf63` passed
both EIO cases with non-zero worker exits and exact cold-reopen outcomes:
`/private/tmp/qmp-durability-hardening/run-01/receipt.txt`.

The first canonical run then exposed a stale failure-injection fixture in
`scripts/tests/extraction-contracts.sh`: it looked for the removed
`get("default")` parser, so the feature-parser error injection never happened
and the test returned 0. The fixture now targets `json.load(sys.stdin)`, and
the focused contract script plus the full `scripts/check-workspace.sh` both
pass. The full rail reports 89 Python contract tests, 18 PM tests, all feature
combinations warning-free, 10 capability selectors, 293 documented selectors,
and `check-workspace: clean`.

That contract-test correction changes the broad executable-input digest to
`0faa707506a686e8f31d951fc747acad4f21a028526cab8f86197b1cbda212ae`, so the
`1f76b6ae…` L2 receipt is not current-digest evidence. A rerun was attempted,
but the ephemeral Docker container could not resolve the Debian package mirror
and had no installed `strace`; that attempt did not start L2. A network retry
then passed both cases on the exact `0faa7075…` digest; receipt and raw files:
`/private/tmp/qmp-durability-hardening/run-03/receipt.txt`. At that review
point, this did not yet qualify device-mapper L2 or physical power-loss L3.
This ticket remains historical/out of the current release scope.

### Device-mapper L2 completion (2026-09-25)

At HEAD `58a268d92131a52fa8016da94ae2cda244f79174`, executable-input digest
`afd110378590610d47d45a5e8cbc7cf7239ce9f51a86598010fdb9663b33e6c7`, the ARM64
Linux 6.10.14 Docker Desktop VM passed both syscall cases (2/2) and one
device-mapper case (1/1). The DM case paused at `L2_FINALIZE_BEGIN`, swapped the
runner-owned ext4 loop mapping to `error`, observed a post-marker
`fdatasync(3) = -1 EIO`, and propagated failure from the worker. After restoring
the map without flushing failed I/O, `e2fsck` returned 0, the DM mapping was
detached, and the filesystem was reopened through its loop backing device. The
public-API oracle saw `Present -> recovered -> Finalized`; unrelated record
drain and duplicate-ID rejection also passed. `power_loss_proven=false`.

On the same final source tree, the L1 focused selector passed 3/3 and the
canonical `scripts/check-workspace.sh` passed: 89 Python contract tests, 18 PM
tests, warning-free feature matrix, workspace all-features tests, 10 capability
selectors, 293 documented selectors, PM lint and cargo-deny. An earlier
workspace run had transient `NotFound` while spawning a nested Cargo freshness
build; the added failure diagnostic did not reproduce it on the final full
gate. Treat that one initial failure as an unresolved intermittent test-runner
risk, not as durability proof.

Receipts and raw artifacts remain outside Git:

- syscall rail: `/tmp/qmp-durability-artifacts.vUV7eI/syscall-final/receipt.txt`;
- device-mapper rail: `/tmp/qmp-durability-artifacts.vUV7eI/dm-final/receipt.txt`;
- DM backing-image SHA-256: `2849d413ce208bc2cdce658ed1d5836732e63bfecac0be874202aa5efbf4eef6`.

This is one positive DM recovery sample on a disposable ext4 image, not a
statistical durability claim or physical power-loss qualification. Three
earlier failed harness prototypes left inert deferred-remove `error` mappings
in the ephemeral Docker Desktop Linux VM (`qmpdurability162`,
`qmpd4ba621427aa24974b39f63c8dea0b06d`, and
`qmpd4d875cf83be44c04a18691fd2612b199`); their backing loop devices are
detached and they have no dependencies. The final runner passed cleanup and
did not leave its own mapping behind. A Docker VM restart is needed to reclaim
those earlier kernel mappings; no host disk or repository data is involved.
The ticket remains historical/out of the current release scope, and L3 stays
unqualified until tested with a production-equivalent DUT and external PDU.

### Runner RCA and exact-current-source requalification (2026-09-25)

The earlier nested-Cargo `NotFound` was an opaque `Command::spawn` failure:
Rust reports the same error kind when either the executable or explicit
working directory is absent. The original run did not retain enough process
environment to prove which one caused that historical failure. The freshness
builds now set an absolute workspace manifest and explicit workspace working
directory, retry PATH `cargo` only when the `CARGO` executable itself returns
`NotFound`, and report both spawn errors plus path/existence diagnostics. This
was adversarially exercised by launching the integration test from a deleted
inherited cwd with `CARGO=/qmp-test-invalid-cargo`; PATH fallback and the L1
pre-commit kill oracle passed. The normal focused L1/FUP-006 selectors passed
7/7, and the canonical workspace gate passed afterward.

The first exact-source DM rerun found two harness issues rather than a QMP
durability failure: Docker's `/tmp` tmpfs was non-executable, so Cargo could
not execute the Redb build script (`EACCES`); then the wrapper attempted to
append DM metadata before creating its artifact directory, obscuring that
first error with a missing-receipt-path error. The DM wrapper now creates a
failure receipt before appending metadata and records umount/remove/detach
failures without skipping later cleanup. Running Cargo targets under
`/var/tmp` avoids the non-executable `/tmp` mount.

After those fixes, both L2 subrails passed on the same current executable
digest `f818ce9ecc9a190c327486d82df54ed00013fe9d22c2abcdeded1095b0a82f90`
and HEAD `58a268d92131a52fa8016da94ae2cda244f79174`:

- syscall EIO: 2/2 (`pwrite64` ordinal 9 and `fdatasync` ordinal 4), each with
  the expected cold-reopen oracle;
- disposable ext4 device-mapper error-table swap: 1/1, observed post-marker
  `fdatasync = EIO`, filesystem repair status 0, oracle
  `Present -> recovered -> Finalized`, with the runner-owned map detached.

Receipts:

- `/tmp/qmp-durability-artifacts.vUV7eI/syscall-final-rerun-01/receipt.txt`;
- `/tmp/qmp-durability-artifacts.vUV7eI/dm-final-rerun-01/receipt.txt`.

The three older orphan error mappings remain the only environment residue.
Reinspection found zero dependencies, no mounts or process file descriptors,
and `dmsetup remove --force` still returned `Device or resource busy` for all
three. The final runner itself left no mapping. Reclaiming the orphan kernel
references requires restarting the Docker Desktop Linux VM; an active BuildKit
container is currently running there, so this task did not disrupt it. These
maps have no backing devices or data and are not repository/host-disk changes.
Physical L3 remains unqualified: it still requires the separate disposable
DUT and external power controller specified above.
