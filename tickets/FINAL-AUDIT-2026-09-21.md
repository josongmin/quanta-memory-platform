# quanta-memory-platform final adversarial audit — 2026-09-21

This document is the source-bound final audit and remediation plan for the
current implementation. Ticket lifecycle authority remains [`README.md`](README.md);
each ticket owns its implementation contract. Earlier evidence is preserved in
[`AUDIT-BASELINE.md`](AUDIT-BASELINE.md) and
[`AUDIT-DELTA-2026-09-21.md`](AUDIT-DELTA-2026-09-21.md).

## 1. Verdict

**FAIL — not production-ready and not host-qualified.**

- P0: 0. No demonstrated immediate exploit or unavoidable total-system loss.
- Newly confirmed in this audit: P1 5, P2 4.
- Cumulative campaign inventory: P1 17, P2 20.
- The portable workspace has strong local compile/test hygiene and the exact
  clean product source passed the full local doctor rail.
- Those passes do not close public charge-authority leakage, durable spill
  recovery ambiguity, HEAD-unbound extraction, host revision split-brain, or
  proof commands that select zero tests.

The architecture direction remains sound: `core <- facade <- testkit`, pure
domain/port boundaries, builder-first intent, and advisory allocator telemetry.
The failure is at authority ownership and proof integrity, not the high-level
crate split.

## 2. Frozen source and proof boundary

```text
audited product repository: quanta-memory-platform
audited product branch: codex/qmp070-spill-contract
audited clean product HEAD: dfb58a68ff2a859e829e18e4649a8da80ddd262c
audit-document base: cf527e2e (QMP-090 checkpoint), then fdc3a82 claim
shared main observed HEAD: 2cfccc685275521bc148491eb6cf10e9a6b37ba7
shared main state: dirty; excluded from implementation/audit mutation
external Semantica HEAD observed: 1dc185fb9f3d54a1c4ee85d569adbbd405113969
external Semantica state: heavily dirty; read-only census only
```

Excluded shared-main paths:

```text
M Cargo.toml
M crates/quanta-memory-platform-core/Cargo.toml
M crates/quanta-memory-platform-facade/Cargo.toml
M crates/quanta-memory-platform-testkit/Cargo.toml
M tools/ci/gates/gate-memory-platform-lint.sh
?? tools/ci/check_workspace_lints.py
?? tools/ci/tests/
```

Proof types used:

- `STATIC`: exact reachable source review and dependency/path census.
- `RUNTIME PROBE`: a separately compiled public consumer exercised an exported
  API and printed the postimage.
- `EXECUTED`: command was run and exit/count inspected.
- `WORKSPACE`: exact clean source completed the repository doctor rail.
- `HOST CENSUS`: read-only external Cargo/gate resolution; no host behavior run.

No remote CI, production RSS, host QBC, performance threshold, Loom exploration,
or Miri run is claimed.

## 3. What is actually proven

At clean `dfb58a6`, `scripts/memory-platform-doctor` exited 0 and covered:

- `cargo fmt --all --check`;
- workspace/all-target/all-feature clippy with `-D warnings`;
- Semgrep and feature compilation matrix;
- workspace check, tests, docs and examples;
- `cargo deny --all-features check`.

Observed test totals included core 177 unit tests, facade 248 unit tests plus 18
hellgates and 21 public-surface tests, and testkit 68 unit tests plus integration
and public-surface tests. QMP-090's black-hole target and independent fixed trace
each selected one test and passed. This is meaningful local regression evidence,
but it is not terminal qualification for the gaps below.

## 4. Newly confirmed findings

### P1-F01 — public cloneable raw ticket leaks charge authority

Proof: `RUNTIME PROBE` + `STATIC`.

Reachable path:

- `core/src/domain/ledger.rs:64-81` defines `ReservationTicket: Clone` with no
  ownership `Drop` behavior.
- `core/src/lib.rs:74` root-reexports it.
- `core/src/api/control.rs:384-407` exposes raw `reserve` beside the owned path.
- `core/src/api/control.rs:414-436` exposes raw committed reservation beside the
  owned path.
- `core/src/api/control.rs:772-798` exposes raw commit/release/batch operations.
- `core/tests/public_api_surface.rs` deliberately pins the raw surface.

An external consumer reserved and committed 100 bytes through the public raw
path, dropped its only ticket, then observed `raw_ticket_drop_owner_bytes=100`.
The ledger remains internally consistent but the consumer has lost the only
release capability. This contradicts QMP-030's prior raw-authority closure.

Required repair:

1. Make the owned `PendingReservation`/`CommittedCharge`/`ManagedCharge` surface
   the only public mutation authority.
2. Move raw ledger tickets and raw commit/release behind crate-private or an
   explicitly unsafe/internal compatibility boundary with a removal deadline.
3. Add an external compile-fail bypass probe and drop/refund E2E; a public raw
   ticket must be impossible to construct, clone, abandon, or release twice.
4. Classify the public removal under QMP-140 SemVer/migration policy.

Owners: QMP-020, QMP-030, QMP-140.

### P1-F02 — durable spill has no authoritative unknown-outcome protocol

Proof: `STATIC`; failure injection still required.

Reachable failure sequences:

1. `spill/backings/redb.rs:82-115` inserts and commits, then maps every commit
   error to ordinary `SpillFailure(handle)`. If the commit became durable but its
   acknowledgement failed, `revocable_store/service.rs:372-375` restores the
   source entry. Destination and source can both remain authoritative.
2. `redb.rs:118-130` collapses `DurableCommitFailure::Ambiguous` during generic
   restore to plain `MemoryError` and discards the prepared bytes. If deletion
   committed, the only returned candidate is lost to that API caller.
3. `redb.rs:93-102` derives the next sequence from the current last row and
   reuses `0` after the queue becomes empty. A stale `commit_restore(0)` can then
   address a later entry with the same identity (`:163-205`).
4. `revocable_store/service.rs:282-313` and `:320-377` remove the source entry
   before calling a by-value controller. A controller panic bypasses both success
   and recoverable-error branches and loses the removed in-memory owner.
5. `restore_one_durable` publishes a local entry after an ambiguous delete
   (`:419-438`) without a durable transfer id/fence. A retry can re-admit the same
   logical record or leave an unresolvable local/durable duplicate.

Required repair:

1. Persist a transfer record with globally monotonic transfer id, source id,
   payload digest, phase, and terminal disposition in the same durable protocol.
2. Return `Committed`, `NotCommitted`, or `Unknown` as distinct owned outcomes;
   `Unknown` must retain a recovery handle, never an ordinary retryable payload.
3. Reconcile by transfer id/digest after reopen; retries must be idempotent and
   stale acknowledgements must not target a reused queue id.
4. Put untrusted controller calls behind an ownership-preserving unwind contract
   or explicitly exclude panicking implementations from the public trait and
   enforce that boundary.
5. Add commit-before-error, error-before-commit, crash/reopen, stale-ack, retry,
   duplicate, controller-panic, and inventory/accounting conservation hellgates.
6. Exercise the locked redb `2.6.3` behavior with version-specific fault
   injection; latest upstream docs are architecture references, not proof.

Owners: QMP-070, QMP-090.

### P1-F03 — extraction names HEAD but copies working-tree bytes

Proof: `STATIC`.

`scripts/extract-to-standalone-repo.sh:103-137` obtains a path list from
`git ls-files`, then hashes and copies bytes from the working tree. It does not
require the source tree to be clean, read blobs from the claimed commit, compare
the working-tree digest to `HEAD`, or include executable mode in its canonical
digest. A receipt can therefore name commit X while extracting dirty bytes, and
mode-only drift is outside the digest.

Required repair:

1. Select one authority: extract directly from `git archive`/tree blobs at an
   exact commit, or fail on any tracked byte/mode difference before copying.
2. Manifest path, blob id, mode, size, content digest, source commit and toolchain.
3. Recompute the extracted tree manifest and require exact equality before build.
4. Add negative fixtures for dirty tracked content, executable-bit drift,
   submodule/symlink surprises, omitted tracked files, and claimed-SHA mismatch.

Owners: QMP-120, QMP-140.

### P1-F04 — host gate can verify a different QMP checkout than Cargo uses

Proof: `HOST CENSUS`, not host execution.

The external Semantica tree at `1dc185f` resolves its QMP path dependency to the
adjacent shared-main checkout at `2cfccc6`. `MEMORY_PLATFORM_REPO` only redirects
the gate target; it does not rewrite Cargo's resolved dependency. A gate can pass
against one checkout while the host builds another. Current host source also no
longer contains the two exact integration-test selectors named by QMP docs; old
names survive only in comments/policy migration text.

Required repair:

1. Derive QMP root, package id, canonical path and source SHA from one
   `cargo metadata --locked` result.
2. Make every gate consume that identity; reject an override whose canonical
   path or SHA differs.
3. Record host HEAD, dirty digest, QMP root/SHA, feature set and selected test
   counts in one receipt.
4. Replace stale selector names with current executable owner tests before host
   qualification.

Owner: QMP-130.

### P1-F05 — a documented GREEN selector can run zero tests

Proof: `EXECUTED`.

At `dfb58a6`, all three commands exited 0 while Cargo reported zero selected
tests:

```text
cargo test -p quanta-memory-platform-core --all-features reserved_floor_protects_class_from_peer_consumption
cargo test -p quanta-memory-platform-facade --all-features spill_transfer_preserves_total_charge_until_source_release
cargo test -p quanta-memory-platform-facade --all-features redb_reopen_reconciles_controller_count
```

The ticket validator currently verifies structure and literal receipt fields,
not executed test counts or selector identity. Therefore an exit-0 command is not
by itself evidence that the named contract was exercised.

Required repair:

1. Store commands/results/counts/source identity in a machine-readable receipt.
2. Require `selected > 0` for every named selector and exact expected count for
   single-test hellgates.
3. Add validator fixtures for zero-test, stale selector, ticket/receipt mismatch,
   invalid state, orphan/missing claim, filename/HEAD mismatch, source drift, and
   historical-receipt reuse.
4. Run documented commands in CI rather than parsing their prose.

Owners: QMP-000, QMP-140.

### P2-F01 — poisoned commit can consume rollback authority

Proof: `STATIC`; poison fault test required.

`api/control.rs:76-83` takes the ticket out of `PendingReservation` before the
broker commit. If commit returns an error and `rollback_pending_ticket` also
errors, `?` returns while `self.ticket` is already `None`; `Drop` at `:96-101`
has nothing left to recover. `lock()` maps poison to an error at `:375-377`, so
this exceptional sequence can strand the reservation.

Repair: retain the ticket until commit or rollback has established a terminal
postimage; return an owned recovery error when cleanup cannot complete. Add an
injected-poison test proving exact class/owner/context/group balance and eventual
recovery.

Owners: QMP-020, QMP-110.

### P2-F02 — conformance model is independent but incomplete

Proof: `STATIC` + `EXECUTED` focused tests.

The new model is valuable: black-hole and wrong-axis fakes are rejected, and the
same literal oracle runs against fake and live brokers. It is still one fixed
256-step LCG trace (`testkit/src/conformance/suites.rs:1487-1568`) with no
shrinker, persisted failure case, or generated operation distribution.
`AccountingView` (`target.rs:16-35`) omits group and live-ticket identity, aliases
fake context total to owner total (`:189-193`), and its partition helper uses
saturating addition (`:52-59`). The trace does not cover denial postimages,
overflow, rebuild mutation rejection, reconfigure, concurrent interleavings,
durable spill recovery, or reentrant observation.

Repair: expand the independent state machine first, then generate shrinkable
traces with proptest; add small Loom models using Loom synchronization types and
a separate Miri lane. A repeated ordinary stress test is neither a property
shrinker nor exhaustive interleaving proof.

Owner: QMP-090.

### P2-F03 — verification documentation describes stale semantics

Proof: `STATIC` + the zero-test execution above.

`docs/verification-scenarios.md:56-63,179-181` still describes saturating ledger
totals and names `saturating_add_near_u64_max_keeps_class_owner_balanced`, while
the implementation moved to checked failure and that selector no longer exists.
`EXTRACTION.md` uses present-tense workflow success without observed remote CI.
This is proof drift: the product may be correct in a narrower scope, but the
published evidence points at a different behavior or no test.

Repair: generate the scenario/index from executable test metadata where
practical; otherwise add a CI command audit that verifies every selector exists,
runs at least once, and maps to a current ticket acceptance condition.

Owners: QMP-000, QMP-140.

### P2-F04 — in-process spill copies uncharged binding identity per entry

Proof: `STATIC`.

`revocable_store/service.rs:302` clones the full `StoreBindingHandle` into every
in-process spill handle. Binding owner/runtime strings are caller-controlled
metadata but are outside the per-entry retained-cost formula. Many entries with
long identities can increase retained process memory without corresponding
ledger charge. This is not a claim that semantic byte accounting equals exact
allocator RSS; it is a confirmed omission from this library's declared retained
metadata model.

Repair: use a shared immutable binding identity or include its exact/amortized
retained cost with a documented allocation model. Add long-identity/many-entry
accounting and drop/restore conservation tests.

Owners: QMP-050, QMP-070.

## 5. Revalidated open baseline risks

The final audit also confirmed these existing tickets remain open:

- QMP-060: transcript tombstone/idempotency sets are unbounded.
- QMP-070: advertised moka/scc/redb features are not all canonical
  builder-reachable parity backings; moka/scc act as narrow indexes.
- QMP-080: reserved floor, soft cap, burst percent and reclaim priority are not a
  coherent runtime policy; legal-borrow reconfigure and headroom predicates
  diverge; reclaim is not the broker admission slow path.
- QMP-100: payload uniqueness fill, front-vector removal, transcript/session
  scans and ledger snapshot cloning leave O(N²) candidates with no frozen
  cardinality/latency benchmark.
- QMP-110: observation still has forgeable snapshots, panicking infallible totals,
  poison-to-zero behavior, stale provenance, string denial reasons, arbitrary
  metric names and no demonstrated live emission path.

These are not recounted as new final-audit IDs; their owning tickets already
exist in the inventory.

## 6. Ticket status and required next move

| Ticket | Current | Blocking next move |
| --- | --- | --- |
| QMP-000 | IMPLEMENTED | enforce receipt state/identity/command/count/source schema; zero tests must fail |
| QMP-010 | IMPLEMENTED | preserve totality under QMP-080 policy rewrite; obtain compatible terminal receipt |
| QMP-020 | IMPLEMENTED | delete public raw ticket authority; make exceptional rollback owned/recoverable |
| QMP-030 | IMPLEMENTED | remove root/raw bypass and pin canonical owned builder API externally |
| QMP-040 | IMPLEMENTED | close callback/destructor exceptional ownership and lock-order contracts |
| QMP-050 | IMPLEMENTED | charge/share spill binding identity and preserve exact cost oracle |
| QMP-060 | READY | bounded tombstone/idempotency horizon with accounted metadata |
| QMP-070 | IMPLEMENTED | durable intent/fence/reconcile protocol; ABA/panic/backing parity hellgates |
| QMP-080 | READY | one runtime admission/reconfigure/reclaim predicate for every public policy field |
| QMP-090 | IMPLEMENTED | generative/shrinking model plus group/context/live-ticket, spill and concurrency axes |
| QMP-100 | BLOCKED | remove measured O(N²) owners and set frozen workload thresholds |
| QMP-110 | BLOCKED | fallible typed observation and bounded live telemetry after policy semantics freeze |
| QMP-120 | IMPLEMENTED | bind extraction bytes and modes to exact commit; observe remote workflow |
| QMP-130 | BLOCKED | resolve host Cargo/gate identity, then exact-head host behavior qualification |
| QMP-140 | BLOCKED | SemVer migration, executable docs, governance schema and final evidence reconciliation |

No ticket is `DONE`. `IMPLEMENTED` means partial code/focused proof exists; it is
not production or host qualification.

## 7. Execution plan — how to close it

### Wave 0 — proof rail (QMP-000)

- Own only `tickets/**`, `tools/tickets/**`, and `scripts/verify-tickets.sh`.
- Define a versioned JSON/TOML receipt schema and derive Markdown views from it.
- Make invalid status, claim mismatch, stale receipt, missing commit, dirty source,
  and zero selected tests deterministic validator failures.
- Do not continue terminal qualification while the proof rail can accept a
  no-op command.

Exit hellgate: every negative fixture fails for its causal reason; the live tree
passes; changing one named selector to nonsense fails despite Cargo exit 0.

### Wave 1 — charge authority (QMP-020 then QMP-030)

- Serialize ownership of `ledger.rs` and `api/control.rs`.
- Replace public `ReservationTicket` mutation with non-clone owned leases.
- Preserve rollback authority across poison/error and make retry/retirement
  explicit in types.
- Update root exports, public-surface compile tests and examples in the same
  compatibility change.

Exit hellgate: external raw-bypass consumer fails to compile; owned pending and
committed drops drain all axes; duplicate/cross-broker/stale operations fail
closed; poison recovery reaches a deterministic terminal postimage.

### Wave 2 — durable and retained stores (QMP-050, QMP-060, QMP-070)

- Freeze the retained-cost formula before changing handle identity.
- Bound transcript tombstones independently of spill work.
- Implement durable transfer intent, monotonic identity and reopen reconciliation
  before treating redb errors as retryable.
- Remove unsupported feature claims or route every advertised backing through the
  canonical builder and parity suite.

Exit hellgate: source charge + durable charge is conserved across every fault
point; no unknown outcome returns an ordinary payload; crash/retry/reopen and
stale acknowledgements converge exactly once; tombstones and metadata remain
within explicit count/byte bounds.

### Wave 3 — policy and observation (QMP-080 then QMP-110)

- Define reserved/soft/hard/burst/headroom semantics in one pure prospective
  predicate reused by reserve and reconfigure.
- Wire reclaim to admission with bounded attempts and typed outcomes.
- Expose fallible snapshots/reports and bounded denial/reclaim/spill labels;
  telemetry remains advisory and cannot mutate accounting.

Exit hellgate: every public policy field has a positive and negative runtime
consumer test; reconfigure preserves legal borrow; poison/overflow is a typed
observation error, never zero usage.

### Wave 4 — independent verification (QMP-090)

- Model context, group, ticket lifecycle, rebuild, denial and durable transfer.
- Use proptest strategies and shrinking; persist regression cases.
- Add small Loom models for ticket/reconfigure/spill transitions and a separate
  nightly Miri lane. Keep ordinary stress tests as non-exhaustive evidence.

Exit hellgate: each deliberately broken target is rejected at the causal family;
failure traces shrink and replay; Loom and Miri receipts name exact configuration
and source.

### Wave 5 — scaling (QMP-100)

- Replace front-vector removal, repeated linear scans and full-map clone/validate
  hot paths only after exact affected-slot invariants exist.
- Benchmark fixed cardinalities, payload distributions and contention levels;
  record p50/p95/p99, throughput, peak RSS and toolchain.

Exit hellgate: source-level asymptotic owner removed, correctness suite unchanged,
and thresholds pass on the frozen runner. A microbenchmark without cardinality
growth is insufficient.

### Wave 6 — release identity (QMP-120)

- Extract from exact commit blobs or fail dirty content/mode before copy.
- Verify manifest equality, locked/frozen builds, clean extracted git, examples,
  deny and self-hosting from a fresh destination.
- Run the actual remote workflow and attach its immutable artifact identity.

Exit hellgate: claimed commit, source manifest and extracted manifest are
cryptographically identical including modes; dirty/mode fixtures fail.

### Wave 7 — host adoption (QMP-130)

- Freeze a clean/explicit host source and derive QMP dependency identity from
  Cargo metadata.
- Rewire the host adapter to the exact qualified QMP revision.
- Run current executable behavior selectors, host all-features/explicit-feature
  lanes, and runtime/store integration under one source-bound receipt.

Exit hellgate: host, gate and Cargo all name one canonical QMP path/SHA; test
counts are nonzero; no adjacent checkout can silently substitute.

### Wave 8 — compatibility and truth (QMP-140)

- Publish the raw-ticket removal migration and SemVer decision.
- Reconcile README/EXTRACTION/scenario docs from executable evidence.
- Re-run workspace, extraction, host, conformance and performance receipts at
  the final product source; only then consider `QUALIFIED`/`DONE` transitions.

## 8. SOTA design constraints adopted

The detailed mapping is in [`REFERENCES.md`](REFERENCES.md). The audit applies
these constraints, not the external implementation details:

- Linux cgroup v2 separates protection, pressure and hard limits; QMP policy
  fields need distinct reachable semantics.
- DataFusion reservations and Tokio owned permits model capacity as owned
  lifecycle authority, supporting removal of cloneable raw tickets.
- redb transaction durability requires QMP to represent unknown outcomes and
  reconcile persisted intent; a generic error is not evidence of no commit.
- proptest adds strategy generation, shrinking and persisted failure cases;
  Loom explores instrumented synchronization interleavings deterministically.
- Cargo locked/frozen semantics and SLSA provenance require the released bytes
  to be bound to the claimed source identity, not merely a tracked path list.

These references define acceptance quality. They are not proof that QMP meets it.

## 9. Final closeout checklist

- [ ] QMP-000 rejects zero-test and source/receipt identity drift.
- [ ] No public cloneable/raw reservation mutation authority remains.
- [ ] All owned reservation error/drop/poison paths retain recoverable authority.
- [ ] Every durable spill outcome is `committed`, `not committed`, or recoverable
      `unknown`, backed by persisted identity and idempotent reconciliation.
- [ ] Sequence identity never reuses an acknowledgement namespace after emptying.
- [ ] Transcript and spill metadata are bounded and charged/shared explicitly.
- [ ] Every policy field has a runtime consumer and denial/reconfigure oracle.
- [ ] Conformance covers context/group/live tickets, failures, rebuild and spill;
      generative traces shrink and concurrency has a real Loom model.
- [ ] Hot paths meet frozen cardinality/contention thresholds.
- [ ] Observation is fallible/typed; telemetry is bounded and non-authoritative.
- [ ] Extraction bytes and modes equal the exact committed tree.
- [ ] Remote extraction workflow is observed, not inferred.
- [ ] Host Cargo, gate and receipt resolve the same QMP path and SHA.
- [ ] All documented selectors exist and execute nonzero tests.
- [ ] Public API removal has an explicit SemVer and migration record.
- [ ] Final exact-source workspace, extraction, host and performance receipts all
      exist before any campaign `DONE` claim.

## 10. Commands required at each final source freeze

```bash
git rev-parse HEAD
git status --short
scripts/verify-tickets.sh
python3 tools/pm/pm.py lint
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny --all-features check
scripts/check-workspace.sh
scripts/memory-platform-doctor
# fresh unique destination, exact committed source, then extracted clean-git and
# locked/frozen workspace/examples/deny/self-hosting rails
scripts/extract-to-standalone-repo.sh <fresh-empty-destination>
```

Host qualification and performance remain separate receipts because they have
different repositories, runners and source identities. Passing one cannot be
used as evidence for the other.
