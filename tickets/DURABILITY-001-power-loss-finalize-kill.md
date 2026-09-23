# DURABILITY-001 — finalize instruction-point kill and physical power-loss qualification

Status: `L1 implemented — 2/2 GREEN, 4/4 mutants RED, rail GREEN on dirty workdir; frozen-candidate Slice D receipt pending; L2/L3 OPEN`
Contract: `option 1 (existing contract accepted)` per §2 planning default; no host contract presented requiring a durable publication ACK. The L1 after-oracle asserts no resurrection and no replay expectation, and asserts nothing about the killed process's volatile resident copy.
Priority: `P1 only if durable spill is promoted as power-loss-safe; otherwise release-boundary hardening`
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

`RevocableStore::restore_one_durable` publishes a resident entry and then asks
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

1. Run the same public worker against a disposable loop device.
2. Use `dm-flakey`/`dm-error`, or a test-only syscall interposer, to fail or
   block writes and sync operations during a commit.
3. Hard-kill the worker, detach the fault target, remount/reopen, and run the
   same state-machine verifier.
4. Record whether the run interrupted data write, first sync, commit-marker
   write, or final sync when the mechanism can observe it.

Proposed files:

| File | Purpose |
| --- | --- |
| `scripts/run-durability-qualification.sh` | Fail-closed dispatcher: `--local`, `--emulated`, or `--physical`; never silently substitutes one level for another. |
| `scripts/durability/run_emulated_linux.sh` | Creates and destroys only an explicit disposable image/device, validates the target, then runs L2. Requires root and Linux; unsupported hosts exit non-zero with `UNAVAILABLE`, not GREEN. |
| `scripts/durability/verify_receipt.py` | Validates source identity, selected scenarios, non-zero attempts, verifier results, and artifact hashes. |

Safety requirements:

- refuse `/`, `$HOME`, the repository filesystem, mounted non-disposable
  devices, unresolved variables, and raw device globs;
- require an explicit created image path or allowlisted device identity;
- trap cleanup, but preserve the failed image and receipt for forensics;
- never describe VM shutdown or device-mapper faults as physical power loss.

L2 is not required on every PR. Run it nightly or before a durability release.
Estimated implementation/setup: 4–8 engineering hours on an available Linux
host.

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

## 8. Completion and reopen conditions

L1 complete only when:

- both named kill points select non-zero tests and pass on the same source;
- child death is proven as forced termination;
- cold reopen satisfies the exact point-specific state oracle;
- all four negative mutants are RED;
- canonical, extraction, benchmark, example, and exact-source host rails pass.

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

- Slice A, local and immediately actionable: section 2 contract record plus L1
  implementation/tests/docs. Estimated 0.5–1 day.
- Slice B, optional Linux infrastructure: L2 script and receipts. Estimated
  0.5–1 day after host access exists.
- Slice C, external qualification: DUT/PDU worker, controller, and L3 cohort.
  Estimated 0.5–1 day setup plus physical reboot runtime.
- Slice D, serial integrator: canonical/extraction/benchmark/QBC/tag receipt.

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

These are integration/regression receipts for the current dirty source, not a
frozen-candidate release receipt. The source remains dirty and shared with
other work; do not promote these results to the clean exact-source gate. The
20-iteration stress is opt-in/nightly evidence and should not be added to every
PR or run concurrently against the canonical Cargo target directory. On source
change, rerun focused L1 first; run the canonical rail once after all related
changes settle. Keep the slower extraction/benchmark/host qualification behind
the required frozen-candidate gate. Physical L3 and Linux L2 remain open.
