# quanta-memory-platform: detailed design and scenario guide

This document describes the current code structure and behavioral boundaries.
It is a library architecture guide, not a host-adoption or production
qualification receipt. For exact qualification state, use
[tickets/FINAL-CLOSEOUT.md](tickets/FINAL-CLOSEOUT.md); for current plan status
and remaining gates, use [docs/plans/README.md](docs/plans/README.md).

Product-scope note: Redb-backed durable spill and its codec are existing opt-in
code, not part of the active QMP/SDK/Semantica plan. The Redb sections below
describe current implementation and historical tests only; they do not create
a release gate or a host integration requirement.

## 1. Purpose and non-goals

The platform gives a host one portable authority for memory-relevant retained
state. It answers four questions consistently:

1. May this class/owner/context grow by these bytes?
2. Which live authority owns the charged bytes?
3. Can a mutation, transfer, retry, or cleanup leave accounting split from data?
4. What should happen at soft pressure, hard exhaustion, rebuild, poison, or
   uncertain durable I/O?

It deliberately does **not**:

- enforce process RSS or replace an OS allocator/cgroup;
- own host activation state, deployment, tenant identity, or Semantica wiring;
- expose raw ledger tickets or a late accounting callback to application code;
- turn durable spill into a general-purpose persistence API;
- make allocator telemetry or a metrics sink authoritative.

Semantic bytes are the cost model retained by this library. Allocator/RSS data
can be observed and compared, but cannot mutate ledger counters.

## 2. Package and authority topology

```text
consumer / host adapter
        |
        v
MemoryPlatformBuilder
        |
        v
LiveMemoryPlatform ------> feature-sliced facade stores/pipelines
        |                              |
        v                              v
LiveProcessMemoryBroker <------ StoreAccounting / ManagedCharge
        |
        v
AccountingLedger + scheduler + pressure controller
```

| Package | Location | Owns | Must not own |
| --- | --- | --- | --- |
| `core` | `crates/quanta-memory-platform-core` | Pure policy/accounting domain, port traits, broker control, pressure/scheduling adapters. | Host names, facade implementation, storage I/O in `domain`/`ports`. |
| `facade` | `crates/quanta-memory-platform-facade` | Builder-first public platform, stores, pipelines, spill backings. | Raw accounting attachment or a second ledger. |
| `testkit` | `crates/quanta-memory-platform-testkit` | Conformance, broken fakes, deterministic faults, allocation/Loom tooling. | Production runtime dependency. |
| host adapter | outside this repository | Runtime-specific configuration and host lifecycle wiring. | Reverse dependency into this workspace. |

The permitted direction is `core <- facade <- testkit`. The extraction preflight
checks this resolved graph, prevents escaping path dependencies, and rejects a
host back-edge.

## 3. Core: semantic accounting and control

### 3.1 Domain ownership

`core/src/domain/ledger.rs` owns the reservation state machine and accounting
axes. The observable ledger tracks class, owner, context, and charge-kind
usage; live ticket state is private authority used to reject replay/double
release. Policy/configuration live in `domain/policy.rs` and `domain/config.rs`.

The important invariants are:

| Invariant | Consequence |
| --- | --- |
| Class/owner/context totals agree by charge kind. | A store cannot retain bytes without a corresponding owner charge. |
| Retained payload, retained metadata, and live scratch remain partitioned. | A cleanup cannot refund the wrong category. |
| `reserved <= soft <= hard`. | Pressure and admission thresholds cannot describe an impossible policy. |
| Mutating live operations are fenced during rebuild. | Reconfiguration/rebuild cannot race a new authoritative write. |
| Tickets are ledger-affine and single-use. | A foreign, forged, replayed, or double-released authority cannot subtract capacity. |
| Global/headroom/owner/class/group postimages are checked before mutation. | Denial leaves the old snapshot intact. |

The live-ticket index is intentionally membership-only; its iteration order is
not observable. Ordered maps remain where deterministic policy, snapshot, or
batch aggregation behavior requires them.

### 3.2 Admission and lifecycle

```text
reserve request
  -> rebuild/global/headroom/owner/class/group/execution-scratch checks
  -> create pending ticket and charge axes
  -> commit to ManagedCharge, or rollback pending ticket
  -> explicit release, batch release, recharge, store cleanup, or drop
```

`ManagedCharge` owns the non-clone accounting authority. A failed explicit
release returns the same authority to the caller; a drop path attempts exact
cleanup. Batch release validates the complete batch before moving counters, so
a bad ticket cannot partially refund earlier entries.

`LiveProcessMemoryBroker` also owns resource-group admission, pressure
classification, optional reclaimer coordination, bounded advisory events, live
policy reconfiguration, and `observe()`—the fallible one-lock semantic
observation surface.

### 3.3 Policy, pressure, and runtime scratch

Configuration validation checks mandatory class coverage, class/owner/group
caps, group references, reservation floors, and override provenance before
constructing a live broker. Override resolution is deterministic and records
the source of the effective policy.

Soft-cap growth may invoke `MemoryReclaimer` outside the broker mutex. The
callback's reported reclaim amount is advisory; the broker remeasures semantic
usage before one retry. Hard-cap failure remains a typed capacity denial.

Runtime scratch attached through `LiveMemoryPlatform::attach_runtime` uses the
same ledger and execution-scratch cap as facade stores. Execution budget helper
checks are policy helpers; this library is not an execution engine.

### 3.4 Core features

| Feature | Meaning |
| --- | --- |
| `live_control` (default) | Live broker, pressure, and scheduler control surface. |
| `telemetry` | Optional metrics/hdrhistogram adapters. Telemetry remains advisory. |
| `unbudgeted_pin` | Explicit in-memory pin adapter without a live broker. |
| `full` | `live_control` plus `telemetry`. |

`pressure` and `scheduler` are implementation dependencies of `live_control`,
not independent consumer capabilities.

## 4. Facade: public builders and storage scenarios

### 4.1 Builder boundary and typestate

`MemoryPlatformBuilder` delegates policy resolution to the core builder and
materializes `LiveMemoryPlatform`. That platform is the public factory for
budgeted store builders. A store cannot attach an arbitrary accounting sink
after construction.

```text
preset/manifest + override layers
        -> MemoryPlatformBuilder
        -> LiveMemoryPlatform
        -> typed, bound store builder
        -> budgeted concrete store
```

`build_report_only()` validates the same configuration but returns only an
`EffectiveMemoryBudgetReport`. It cannot create a runtime, broker charge, or
budgeted store.

An unset builder cannot build. A budgeted builder is already tied to one broker;
the explicit `Unbudgeted` type exists only when `unbudgeted-stores` is enabled.
It is not a switch that changes a budgeted store after construction.

### 4.2 Feature slices and scenarios

| Slice | Primary scenario | Accounting/lifecycle behavior |
| --- | --- | --- |
| `payload_store` | Canonical payload publication and deduplicated lookup. | Charges retained payload/index cost once; digest buckets still check exact canonical equality. |
| `retained_map` | Typed key/value retention, overwrite, read, remove, targeted sweep. | Charges retained key/index/value cost; replacement is net-postimage accounting. |
| `revocable_store` | FIFO cache or scratch-like entries that may be evicted. | `reclaim_one`/`reclaim_until` refund exact front-entry charges; failure restores FIFO state and authority. |
| `spill` | Move revocable entries to another tier. | In-process handles move the live charge; durable operations fence uncertain writes/deletes until reconciliation. |
| `spill-redb` | Existing opt-in backing; out of active product scope. | Redb implementation remains in the 0.2 source; prior validation is historical evidence. `RedbSpillLimits` bounds encoded record size, retained record count, logical retained payload bytes, Redb database file length, cache setting, and backing-owned restore staging bytes/buffers. The file cap does not reserve filesystem space or bound caller-codec scratch; an I/O error may require cold reopen and additional file capacity before restore can progress. |
| `transcript` | Idempotent session/event transcript. | Exact replay is deterministic; evidence/session-key bytes are separately tracked; retention saturation fails closed. |
| `publish_pipeline` | Stage/commit publication transitions. | Legacy non-retaining pipeline remains supported; retaining prepared publication uses token-owned accounting and root-aware orphan GC. |

All retaining slices have an explicit `close()` lifecycle. A successful close
is terminal and idempotent; later mutation returns `Closed`. Reentrant caller
callbacks or concurrent lifecycle transitions return typed
`OperationInProgress` rather than blocking while an internal store lock is held.

### 4.3 Durable transfer state

Durable spill/restore is a custody protocol, not a best-effort delete:

```text
resident source --commit receipt--> durable record --claim/read/delete--> resident source
       |                                      |
       +-- uncertain acknowledgement ----------+--> fenced pending state -> reconcile
```

The source charge remains authoritative until durable commit is known. During
restore, the destination record is not deleted until a resident charge has been
established. An ambiguous acknowledgement is fenced instead of silently
duplicating or losing ownership. Accounting-bearing durable controller traits
are sealed; the optional Redb backing cannot be bypassed through a generic
accept/restore shortcut.

### 4.4 Transcript and prepared publication details

Transcript identity is session, sequence, and message identity. An exact retry
is replayed; a conflicting identity/payload is rejected. Active rows may be
evicted, but replay evidence remains until close within a finite count/byte
horizon. A new identity at saturation fails before any charge or mutation.

Prepared publication uses a stable token derived from identity and canonical
payload. Same identity/payload replays; divergent payload conflicts. Orphan GC
requires the caller to provide a complete canonical-root token snapshot that is
stable against concurrent activation. QMP validates that snapshot but does not
invent or own host activation state.

### 4.5 Facade features

| Feature | Compile-time boundary |
| --- | --- |
| default `core-live-accounting` | Live broker bridge only. |
| store/pipeline slices | Each module/export exists only when its named feature is enabled. |
| `full` | Every in-memory slice and spill contracts, without a durable backing dependency. |
| `full-backings` | Existing feature adding `spill-redb`; excluded from the current release plan. |
| `unbudgeted-stores` | Explicit non-production escape hatch; absent from default/full/full-backings. |

## 5. Algorithms, data structures, and state machines

This section names techniques that are present in the source. It deliberately
does not attribute an algorithm to QMP when it is merely provided by a
dependency, nor treat an implementation choice as a service-level guarantee.
There is no proprietary or external commercial decision engine in this
repository: admission policy is local, deterministic Rust code. The optional
backing and utility crates are open-source dependencies selected at build time.

### 5.1 Algorithm inventory

| Technique / provenance | Source owner | What QMP uses it for | Explicit limit |
| --- | --- | --- | --- |
| Hamilton (largest-remainder) apportionment, a classical proportional-allocation method | `core/src/application/scheduler.rs` | Split a configured shared burst pool among resource groups by `admission_weight`. | It is an admission allowance, not a work queue, packet scheduler, or throughput-fairness guarantee. |
| Checked integer accounting | `core/src/domain/ledger.rs`, facade `shared/measure.rs` | Compute counter/cost postimages before committing a charge or release. | It meters declared semantic retention, not RSS, allocator capacity, or every transient copy. |
| BLAKE3 cryptographic hash | `facade/src/{transcript_store,publish_pipeline}/` when their features are enabled | Derive fixed 32-byte, domain-separated replay/token identities from canonical input. | A digest is an identity aid; it does not make host publication or activation transactional. |
| Randomized hash table plus linked newest-first order (`hashlink`) | `facade/src/retained_map/service.rs` | Point lookup with randomized hashing and a linked order for newest-first reclamation. | The map's iteration/order is local store behavior, not an inter-process ordering contract. |
| Digest-bucket index with exact-equality collision check | `facade/src/payload_store/service.rs` | Find canonical payload/reference candidates efficiently, then compare the full canonical key/reference before reuse. | Its `DefaultHasher` `u64` digest is not cryptographic and never by itself establishes equality. |
| Ordered maps/sets (`BTreeMap`, `BTreeSet`) | scheduler, transcript, prepared publication | Canonical group/tie order and deterministic identity/token traversal. | Determinism is within one implementation and its canonical inputs; it is not a consensus protocol. |
| Durable record/reconciliation protocol over optional Redb backing | `facade/src/revocable_store/` | Fence source ownership across spill/restore when acknowledgement is uncertain. | It is not distributed two-phase commit: host coordination, retry policy, and backing durability configuration remain outside QMP. |
| HdrHistogram and `metrics` adapters | `core/src/adapters/` | Record advisory telemetry through ports. | Telemetry cannot alter ledger counters, admission, or recovery decisions. |

### 5.2 Weighted burst admission: Hamilton allocation plus live cap checks

`WeightedResourceGroupScheduler` precomputes the allocation once per policy
snapshot. For shared pool `P`, group weight `w_i`, and total weight `W`, it
first assigns `floor(P * w_i / W)`. It then gives the remaining bytes to the
largest fractional remainders; equal remainders break by ascending canonical
group name. The implementation uses `u128` for the product/remainder math and
stores the final `u64` shares in a `BTreeMap`. Therefore all bytes of `P` have a
deterministic owner and runtime admission does not repeat the allocation sort.

For a request that produces proposed group usage `u'`, admission requires all
of the following:

1. The named group exists; unknown names fail closed.
2. The group's protected reserved floor still fits the non-headroom band.
3. `u'` is within the group's hard cap, or its excess over that cap is no more
   than the precomputed weighted burst share and the configured
   `burst_percent` permits.
4. The surrounding broker's global, class, owner, and rebuild-fence checks also
   accept the same prospective mutation.

This is closest to *static weighted resource apportionment*. It is intentionally
not weighted fair queuing (WFQ), deficit round-robin, token-bucket rate
limiting, or an eviction algorithm: QMP has no request queue, clock-based
refill, or execution scheduler. A policy reconfiguration creates a replacement
scheduler/allocation table rather than mutating shares in place.

### 5.3 Semantic-cost accounting, not heap measurement

Every budgeted facade object computes a `RetainedCostV1` before it acquires a
broker charge. The version-1 total is the checked sum:

```text
payload_bytes + key_identity_bytes + index_bytes
  + tombstone_bytes + fixed_overhead_bytes
```

The fixed constants encode logical retained structures: entry state (16 B),
hash-link index (24 B), ordered-index node (24 B), FIFO position (16 B), and a
digest/token slot (32 B). Each store persists the exact charged total with its
non-clone lease and releases that lease rather than remeasuring potentially
mutated caller data. Overflow is a capacity error, never saturation/wrap.

This is a portable admission model analogous to capacity accounting used by
cache/storage systems, but it is not an allocation profiler. `ByteSized` is
caller-supplied; `size_of`, allocator slack, system RSS, and temporary cloning
are explicitly out of scope. Changing a formula is a policy/API change and
requires a new version rather than silently reinterpret old receipts.
The reported semantic cost is captured at admission and is not polled later. A
custom `ByteSized` implementation for shared or interior-mutable data must
remain a conservative upper bound for the retained lifetime, or callers must
perform an explicit recharge/replacement when that retained cost changes.

### 5.4 Identity, replay, and collision handling

Two distinct approaches are used according to the required property:

- Transcript and prepared-publication identity use BLAKE3 only in their enabled
  facade features. Prepared publication hashes a fixed domain tag, little-endian
  lengths, identity bytes, and canonical payload-key bytes, producing a 32-byte
  token. Length framing prevents ambiguous concatenation of variable-size
  fields. Exact identity/payload retries return the existing authority; a
  divergent payload for an existing identity is rejected.
- The payload store uses `DefaultHasher` `u64` values only as index buckets.
  Candidate entries are checked again against the full canonical key or full
  reference. A collision can therefore increase candidate work but cannot
  incorrectly deduplicate, pin, unpin, or delete an unequal object.

Neither mechanism is an authorization system. Callers must still protect token
distribution and supply authoritative host state for activation/root snapshots.

### 5.5 Store lifecycle and panic-safe mutation protocol

All facade services share the finite lifecycle:

```text
Open -> OperationInProgress -> Open
Open -> Closing -> Closed
```

Only one mutating operation may own the transition. Reentry during an active
operation returns `OperationInProgress`; post-close access returns `Closed`.
For operations that invoke user-defined `Hash`, `Clone`, or destructors, the
service removes the canonical state into a local plan, performs the fallible
work outside the mutex, and restores the preimage on unwind/failure. It commits
the terminal image before invoking client destructors, so a destructor panic
cannot reopen a ticketless entry or leak the accounting authority.

The retained map specifically combines `RandomState` hashing (to avoid a
caller-selected deterministic point-lookup degradation) with `LinkedHashMap`
order for newest-first sweeps. This is a data-structure policy, not an LRU
claim: access does not silently redefine reclamation order unless the store API
performs an ordering mutation.

### 5.6 Durable spill/restore custody state machine

When the optional durable spill backing is configured, transfer is a
reconciliation-oriented custody protocol:

```text
resident + source charge
  --reserve durable record / commit spill--> durable + source retired
  --claim restore / read / source charge / finalize delete--> resident + source charge
```

The important transitions are fenced rather than guessed. A successful spill
does not retire the source authority until durable commit acknowledgement. A
restore does not make the resident value visible until it has a source charge
and the durable delete/finalization has a determinate outcome. If a backing
operation may have committed but its acknowledgement is ambiguous, QMP records
pending spill/restore state and exposes reconciliation; it does not duplicate
or silently drop authority. The Redb-specific interface rejects generic
accept/restore shortcuts so a backing cannot bypass the custody protocol.

This resembles write-ahead/recovery design in storage systems, but it is not a
database transaction coordinator. It gives QMP a local ownership invariant;
cross-process locking, backing-file lifecycle, host retries, and publication
activation require host policy.

### 5.7 Determinism boundaries

Canonical ordering is intentionally used where receipt/replay/fair-share output
must be reproducible: resource groups and prepared entries use ordered
collections, and Hamilton ties use group names. Membership-only hot paths may
use randomized hashes. The contract is therefore deterministic for canonical
input and policy within the library, while resistant lookup hashing is allowed
to vary across process instances. Consumers must not use incidental allocation
addresses, randomized map iteration, or telemetry timing as an API contract.

## 6. Error and failure semantics

| Condition | Result | State rule |
| --- | --- | --- |
| Budget/global/class/owner/group cap exceeded | `MemoryError::Capacity` | No accounting or store mutation. |
| Rebuild fences a live write/release | `InvalidState` | Existing authority remains live. |
| Duplicate/foreign/replayed ticket | `InvalidState` | No refund or cross-broker mutation. |
| Same-store reentry or active lifecycle operation | `OperationInProgress` | Caller retries after the active operation. |
| Store already closed | `Closed` | Terminal state remains unchanged. |
| Poisoned lock or invalid snapshot | typed error | Never converted to an empty/zero result. |
| Durable acknowledgement uncertain | `OperationInProgress` plus reconciliation API | Exactly one authority is fenced until proved resident or durable. |
| Transcript retention full for a new identity | `RetentionSaturated` | No new charge or transcript mutation. |

Telemetry and event sinks are explicitly non-authoritative. Sink panics are
contained, operation labels are closed enums, and user-controlled strings are
not metric labels.

## 7. Testkit: prove the contracts can fail

`testkit` is feature-sliced and host-independent.

| Module | Purpose | Boundary |
| --- | --- | --- |
| `fake` | Honest and deliberately broken broker/runtime implementations. | Test substrate, not a host implementation. |
| `conformance` | Generic accounting, facade-store, transcript, and lifecycle suites that localize failures to contract families. | Must reject broken fakes as well as accept honest targets. |
| `faults` | Deterministic reclaim, pin, rebuild, and spill fault scenarios. | No sleep/timing heuristic or external-service injection. |
| `fixtures` | Reusable manifests and a `MemoryTestKit` entry point. | Builds portable live broker/facade targets. |
| `alloc_tracking` | Optional allocation probe. | Allocation signals are not RSS or ledger truth. |
| `loom_models` | Small-state authority-transition models. | Not instrumentation of all production mutex/storage schedules. |
| `doctor` | Small diagnostic report model for host tooling. | Not a host diagnosis service. |

The critical meta-contract is negative: conformance must reject a fake that
under-reports owner usage or diverges class totals, while accepting an honest
fake and the real broker. A green suite that accepts a broken fake is not a
valid conformance rail.

`loom` is separate from testkit's `full` aggregate feature. `--all-features`
activates it for compatibility coverage; enabling `full` alone does not claim
to run Loom models.

## 8. Verification map

| Rail | What it establishes | What it does not establish |
| --- | --- | --- |
| `scripts/check-workspace.sh` | Formatting, shell safety, resolved workspace graph/policy, Python contracts, Semgrep guards, feature combinations, workspace tests, isolated capability tests, documented selector resolution, generated guidance, dependency policy. | Scaling, actual extraction, host adoption, deployment, activation. |
| `scripts/run-scaling-bench.sh --smoke` | Public broker/store smoke with repeated measurements and final semantic/allocator checks. | Portable throughput guarantee. |
| `scripts/run-scaling-bench.sh` | 1k/10k plus 1/2/8/32-thread read/write/mixed matrix; schema, final-zero, drift, latency, and growth validation. | Cross-machine comparison without an explicit same-host baseline. |
| `scripts/extract-to-standalone-repo.sh --dry-run <dest>` | Clean-tree, resolved-graph, manifest, and self-hosting preflight. | A copied repo's full qualification. |
| `scripts/extract-to-standalone-repo.sh <dest>` | Exact-tree export, digest/mode equality, extracted canonical rail, and clean bootstrap commit. | Semantica host compilation/activation. |

Missing ShellCheck, Semgrep, cargo-deny, lockfile, or required source inputs
fails closed. The Semgrep rail guards six known regression classes; it is not a
general security certification. There is intentionally no GitHub Actions
workflow; a nonexistent remote CI job is not part of this repository's
acceptance model.

The full scaling rail records source SHA/tree, toolchain, machine, repeated
latencies and throughput, busy retries, semantic bytes, and allocator signals.
Its verifier requires the expected matrix, enough repetitions, final semantic
and allocator zero, no allocator underflow, and bounded revocable growth. These
are host-local receipts, not portable SLOs.

## 9. Extraction and host adoption

Extraction is mechanical. The script freezes a clean source `HEAD`, verifies
the resolved graph and self-hosting manifest before copying, binds source and
destination bytes/modes with a digest, then runs the extracted repository's
canonical rail.

After extraction, host adoption remains a coordinated external change:

1. Rewire the Semantica workspace dependency to the intended QMP source.
2. Verify both Semantica workspace roots resolve that same source with locked
   Cargo metadata.
3. Run the host's registered adoption suites on the exact host/QMP pair.
4. Treat deployment and activation as separate evidence.

See [EXTRACTION.md](EXTRACTION.md) for the executable procedure.

## 10. Source map

| Need | Source of truth |
| --- | --- |
| Accounting invariants and ticket state machine | `crates/quanta-memory-platform-core/src/domain/ledger.rs` |
| Policy, presets, budgets, resource groups | `crates/quanta-memory-platform-core/src/domain/{policy,config}.rs` |
| Broker, managed charges, rebuild/reconfigure/observation | `crates/quanta-memory-platform-core/src/api/{builder,control}.rs` |
| Ports and adapters | `crates/quanta-memory-platform-core/src/{ports,adapters,application}/` |
| Builder-first platform and store builders | `crates/quanta-memory-platform-facade/src/shared/builder.rs` |
| Store/pipeline implementations | `crates/quanta-memory-platform-facade/src/{payload_store,retained_map,revocable_store,transcript_store,publish_pipeline}/` |
| Spill contracts/backings | `crates/quanta-memory-platform-facade/src/spill/` |
| Conformance/fault/allocation tooling | `crates/quanta-memory-platform-testkit/src/` |
| Repository verification | `scripts/check-workspace.sh`, `scripts/run-scaling-bench.sh` |
| Extraction contract | `EXTRACTION.md`, `scripts/extract-to-standalone-repo.sh` |
