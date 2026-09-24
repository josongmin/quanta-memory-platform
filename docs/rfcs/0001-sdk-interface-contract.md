# RFC 0001: Portable Memory SDK Contract (0.3)

- Status: **accepted design target; not implemented or qualified**
- Target: `0.3` (2026-09-24 design decision)
- Scope: public Rust DSL for in-memory budgeting, accounting, reclaim, stores, and in-process transfer

API freeze still depends on independent-host compile proof, public
deferred-cleanup implementation, and the store-callback lock migration/tests.
The interface decisions below do not claim those changes are implemented.

The snippets specify the intended 0.3 interface, not the current 0.2 API.
Before release they must become complete, compiling public examples.

## 1. Background, purpose, and boundary

QMP is a **portable library** used by independent hosts. It decides whether
declared memory use may be admitted, attributes it, and maintains one
fail-closed accounting authority until release or transfer. The same budget
must cover QMP's built-in stores and host-defined retained components. The
host adapter lives outside this repository; neither public types nor core
policy may encode a particular host's artifacts or execution model.

The current implementation already has a semantic ledger, two-phase tickets,
RAII charges, pressure/reclaim, budgeted store builders, and live policy
replacement. Its public integration surface is less portable than that
engine: `MemoryClass` contains host-specific variants, presets mix deployment
and workload concerns, `ExecutionBudget` carries non-memory limits, and
callers can reach deep broker/building paths. This RFC defines the consumer
contract before reshaping the engine. It does not claim those changes are
already present.

Current-source anchors: [policy and class taxonomy](../../crates/quanta-memory-platform-core/src/domain/policy.rs),
[pending/committed authority](../../crates/quanta-memory-platform-core/src/api/control.rs),
and [facade builders](../../crates/quanta-memory-platform-facade/src/shared/builder.rs).

The SDK must make the common path obvious while preserving explicit
ownership, deterministic policy, and safe extension:

- One typed policy model; profiles provide defaults, runtime knobs edit that
  model, and no parallel string-key registry can disagree with it.
- Host-defined, validated IDs extend taxonomy without a QMP release; they
  never grant special ledger privileges by their spelling.
- One platform owns one ledger. Stores and custom components obtain scoped,
  non-forgeable reservation authority from it.
- Errors, observations, and reconfiguration report machine-readable facts.
  A failed accounting operation does not leave a partial ledger mutation.
- New backings or operational controls remain feature/owner-specific; the
  base SDK does not become a plugin container or host framework.

QMP limits **declared semantic bytes**, including retained payload, retained
metadata, and live scratch charges. It does not allocate application memory
or impose an RSS, mmap, page-cache, or process-wide hard limit. Allocator
telemetry is advisory and never changes semantic counters. Wall time, query
count, graph edges, and output limits belong to the host. Disk-backed Redb
spill and recovery remain an existing opt-in maintenance surface, outside
this 0.3 SDK and host-adoption gate.

## 2. Consumer DSL

The common path is `MemoryConfig → MemoryPlatform → MemoryScope → store or
MemoryConsumer → reservation`. `ResolvedMemoryConfig` is the explicit
inspection/build path, not an extra step every caller must write.
Observation and reconfiguration return to the platform. The facade crate
provides these canonical public paths; raw ledger, ticket, and sink
constructors are not part of the consumer DSL.
Create one platform per independently budgeted domain and share its cheap
scopes across runtimes. Each platform has a separate ledger: creating two
platforms with the same capacity does not impose a combined cap. QMP does not
enforce a process-wide singleton or coordinate budgets across platforms; the
host owns that composition.

### 2.1 Configure once

```rust
let class = MemoryClassId::parse("app.artifact-cache")?;
let owner = OwnerId::parse("retrieval")?;
let group = ResourceGroupId::parse("interactive")?;

let config = MemoryConfig::builder()
    .capacity(MemorySize::gib(8)?)
    .headroom(MemorySize::gib(1)?)
    .class(
        class.clone(),
        ClassBudget::new()
            .reserved(MemorySize::mib(128)?)
            .soft(MemorySize::gib(1)?)
            .hard(MemorySize::gib(2)?),
    )
    .resource_group(
        group.clone(),
        GroupBudget::new()
            .soft(MemorySize::gib(2)?)
            .hard(MemorySize::gib(4)?),
    )
    .owner(owner.clone(), OwnerPolicy::new().group(group))
    .context_kind(ContextKindId::parse("app.request")?)
    .build()?;

let platform = MemoryPlatform::builder(config).build()?;
```

`MemorySize` is a public byte newtype with a private `u64` field:
`bytes(u64)` is explicit, `mib`/`gib` and `try_from_usize` check conversion
and multiplication, and `as_bytes` exposes the canonical value. It does not
implement unchecked arithmetic operators. Public budgets, reservations, and
byte-valued error getters use it; the ledger may
continue to use `u64` internally. No implicit unit conversion or blanket
integer extension trait is required. Every platform, including development,
requires explicit host-supplied capacity.
QMP does not discover RAM, cgroups, files, or environment variables.

`MemoryConfigBuilder::build` checks local shape. The common
`MemoryPlatform::builder(config).build()` path calls the same `resolve`
operation once, expands profile defaults, validates the complete effective
policy, and installs only the validated artifact. For offline inspection or
recorded policy review, the explicit alternative to that construction is:

```rust
let resolved = config.resolve()?; // no broker or accounting authority
let platform = MemoryPlatform::builder_resolved(resolved)
    .reclaimer(reclaimer) // optional host callback
    .build()?;
```

These are two entry points into one resolver and one platform builder
contract, not two policy engines. No invalid, unbudgeted, or
feature-disabled fallback is permitted.

### 2.2 Bind a runtime and use a store

```rust
let scope = platform.scope(RuntimeId::parse("worker-1")?)?;

let payloads = scope.payload_store::<ArtifactId, Vec<u8>>()
    .name("artifacts")?
    .owner(owner.clone())
    .class(class.clone())
    .context(ContextKindId::parse("app.request")?)
    .build()?;
```

`scope(runtime_id)` carries no caller-specified diagnostic kind. If the host
needs that classification, it can call
`scope_with_kind(runtime_id, RuntimeKindId::parse("app.retrieval")?)`.
`RuntimeKindId` is bounded and has no admission privilege. The current
`RuntimeKind` enum is stored/reported, not used to choose admission
behavior; an optional validated ID avoids imposing one host's taxonomy
on every integration.

`MemoryScope` carries runtime attribution; it is not a second ledger or a
control handle. Creating it reserves no bytes. The platform supplies
budgeted accounting to each feature-gated store builder. Store-specific
policy stays on that store's builder. A caller never attaches a broker, raw
ticket, or accounting sink. Store handles have stable feature-specific names,
not a generic store trait without a generic caller.

All budgeted store builders and the custom-consumer builder require an owner
and a declared class. `build()` returns a typed `Build` error if either is
missing, rather than silently choosing an owner or class; context is optional
and defaults as below. A store name is diagnostic and optional, with a stable
feature-specific default; a custom consumer supplies its name to `consumer()`.
The scope itself supplies accounting authority, so callers cannot select an
unset or unbudgeted accounting mode through these builders. This avoids a
typestate parameter for every attribution field while keeping missing policy
inputs fail-closed.

Context kind is an attribution axis, not a quota or privilege. A scope defaults
to the QMP-owned `ContextKindId::runtime()`; every store builder and custom
consumer inherits it unless `.context(kind)` selects a declared host kind.
A store that supports per-item context may override its builder default through
typed item metadata; that override must be declared and is validated before
admission. No store silently substitutes `Request`, `Session`, or another
feature-specific kind. Runtime kind remains a separate optional diagnostic ID.

The in-memory slices are payload, retained map, revocable store, transcript,
and prepared publication.

### 2.3 Account a custom component

```rust
let consumer = scope.consumer("vector-index")?
    .owner(owner)
    .class(class.clone())
    .context(ContextKindId::parse("app.request")?)
    .build()?;

let max_size = MemorySize::mib(16)?;
let pending = consumer.reserve_up_to(max_size, ChargeKind::RetainedPayload)?;
let value = construct_value()?;
let actual_size = MemorySize::try_from_usize(measure_retained_cost(&value)?)?;
let reservation = pending.commit_measured(actual_size)?;
// The component owns value and reservation together until both are released.
```

`MemoryConsumer` is a cheap, immutable attribution descriptor, not a
permanent registry entry. One consumer may own many independent reservations.
Its name is diagnostic. Class, owner, context, charge kind, and the owner's
configured resource group determine accounting. The consumer cannot choose
a different group; `owner → group` in validated policy is the only group
mapping.

`reserve_up_to` admits a conservative upper bound **before** constructing
retained data. `commit_measured(actual_size)` is a single fail-closed
shrink-and-commit transition. It rejects a cost above the bound and rolls
back the pending authority; if cleanup itself fails, the exact ticket
remains owned for bounded deferred retry. The caller discards the
unpublishable value. Dropping an uncommitted reservation also rolls it back.
A zero-byte commit still owns one live ticket until release. If a useful
upper bound cannot be known, the component must budget construction as
scratch/temporary work and reserve measured retained cost before publication.
QMP cannot infer the physical allocation size of arbitrary `T`.

The committed `MemoryReservation` is non-cloneable and broker-bound.
`try_resize(&mut self, new_size: MemorySize)` changes only the charged byte
count; class, owner, context, charge kind, and broker identity remain fixed.
To grow a retained value, first resize its charge to a conservative upper
bound, then mutate or allocate, then resize down to measured cost. If mutation
fails or partially changes the value, keep the upper-bound charge until the
value is measured or destroyed. To shrink, reduce or destroy the value first,
then resize the charge down; a failed refund safely leaves an overcharge.
If no safe retained-growth bound is available, do not mutate in place: build
under a separately bounded scratch reservation and admit the measured retained
charge before swapping or publishing the value. `try_resize` is not a
scratch-to-retained transfer. A mutation error must not return through `?`
while a partially changed value remains live after its charge is dropped;
the component handles that failure with its value and reservation still paired.

For a live reservation, every `try_release` or `try_resize` failure leaves its
exact ticket owned by that same reservation and available for retry. A call on
an already released reservation returns `InvalidState`; it does not claim to
preserve an absent ticket. The 0.2 `ManagedChargeRechargeRecovery::RecoveryRequired`
branch is reached when `recharge` is called after release, while the live
broker's recharge denial preserves the old charge. It does not justify a new
public `RecoveryRequired` state or `try_recover()` method in this 0.3 DSL.
If a future reachable failure cannot preserve a live ticket, design and prove
its ownership recovery separately before exposing it.
`Drop` attempts bounded deferred cleanup but cannot return an error.
Components must destroy retained data before or with its charge. Built-in
stores enforce that lifecycle internally.

Do not add a generic `Retained<T>` wrapper to 0.3: without prohibiting
mutation and value escape it cannot prove that `T` still matches its charge.
A restricted wrapper may be added for a demonstrated caller.

The platform exposes deferred-cleanup state through `MemorySnapshot`:
active-ticket count, pending rollback/release counts, and bounded failure
status. `platform.retry_deferred_cleanup()` attempts at most one published
batch and returns a receipt with attempted/resolved/remaining counts; any
failure retains exact ticket authority and is observable on the next
snapshot. Zero semantic bytes do **not** mean quiescence: shutdown requires
zero live tickets and zero deferred authorities. The current engine queues
and retries internally but does not yet expose this public 0.3 contract.

### 2.4 Observe and change policy

```rust
let before = platform.snapshot()?;
let patch = ConfigPatch::builder()
    .class(
        class,
        ClassBudgetPatch::new()
            .hard(Override::Set(MemorySize::gib(3)?)),
    )
    .build()?;
let receipt = platform.reconfigure(
    ReconfigureRequest::new(before.policy_revision(), patch),
)?;
```

`ConfigPatch` is the typed runtime-knob DSL. Patches are grouped by entity
(`ClassBudgetPatch`, `OwnerPolicyPatch`, `GroupBudgetPatch`) so adding a
field does not add another top-level setter. A missing field means unchanged;
`Override<T>::Set(T)` replaces a leaf in the runtime overlay and `Clear`
removes that override, revealing the validated baseline. Map `Remove` is a
separate runtime tombstone that hides an effective item only if no live
authority depends on it, including zero-byte tickets. Clearing the tombstone
reveals the baseline item; field patches against a tombstoned item are
rejected. Runtime writes always carry an expected revision. Stale revision,
below-usage cap, live owner/group
remap, missing feature, or invalid postimage rejects without changing policy,
revision, fingerprint, or ledger.

`ReconfigureReceipt` reports before/after revision, effective-policy
identity, outcome, and changed fields. An identical effective postimage is
`Unchanged` and does not advance revision. A change advances it exactly once.
`MemorySnapshot` gives one coherent semantic ledger/policy/revision view,
including pressure and reclaim state from that authority.
High-cardinality owner detail is a separate bounded, sorted query.
Allocator/RSS samples are separate advisory observations, never represented
as simultaneous with the semantic snapshot.

## 3. Exact abstraction rules

### Policy, profiles, and knobs

`MemoryConfig` is the single desired-policy type: capacity, headroom,
shared burst pool, pressure thresholds, class budgets, owner quotas,
resource-group budgets, and owner/group bindings. Store options remain with
their store. Validation checks `reserved <= soft <= hard`, global headroom
and floor sums, checked arithmetic, referential integrity, feature
availability, and bounded class/owner/group/mapping/context-kind counts.
Top-level registration methods add classes, owners, resource groups, and
diagnostic context kinds; scalar budget fields live in their typed policies.
`OwnerPolicy` owns an
optional group binding and quota, avoiding separate `owner_group`,
`owner_quota`, and future owner-setting methods. By default an owner with no
policy entry is rejected before reservation, including zero-byte reservation.
`UnregisteredOwnerPolicy::GlobalOnly` is an explicit build-time opt-in for
trusted integrations that intentionally admit arbitrary owners under only
global/class limits; these owners receive no owner/group cap or fair-group
share. It is never inferred from an empty owner map or a selected profile.
Changing this mode requires a new platform, not a runtime patch. Owner IDs are
attribution supplied by trusted host code, not an authentication boundary:
QMP cannot prevent a caller from deliberately selecting another registered
owner. Hosts must bind untrusted identities to owner IDs outside the SDK.

Deployment and workload profiles are separate, explicitly versioned IDs.
They provide portable defaults, never hidden admission exceptions or a
machine-independent capacity. They cannot assign budgets to host classes
unknown to QMP. A changed default gets a new profile ID. Host-specific
profiles are host code that builds ordinary `MemoryConfig` values.
Profiles are optional; the canonical path above uses explicit policy and
does not depend on a built-in preset. The profile *mechanism* is a 0.3 target,
but no specific built-in ID (including `PRODUCTION_V1` or `INTERACTIVE_V1`)
is frozen by this RFC without a normative per-leaf default table, supported
feature set, and golden resolved-policy/fingerprint vectors for that ID and
its supported pairings. Do not publish a named profile or make it a default
merely because the corresponding 0.2 preset exists.
The no-profile path must resolve without hidden class, owner, or capacity
budgets: every omitted field is either optional with a documented neutral
default or a typed configuration error. The explicit example in §2.1 must be
one of the compiling release examples, not merely illustrative syntax.
The two profile axes have no implicit ordering: if both supply the same
canonical policy leaf, `resolve` rejects the collision even when values are
equal, unless the explicit baseline sets that leaf and therefore resolves it.
An explicit baseline leaf overrides either profile default; omitted leaves
inherit nonconflicting defaults. Map entries are compared by typed ID and leaf
path, not insertion order. This composition rule and each profile's leaf set
are versioned with its ID. A profile cannot set capacity, owner-registration
mode, or host-defined class/owner/context registrations.

QMP defines two construction layers—versioned profile defaults and the
explicit typed baseline—and one runtime override overlay.
The host decides whether and how files, environment, CLI, or remote control
produce that baseline. QMP does **not** prescribe source precedence or load
those sources. This avoids making one host's deployment convention a library
contract. `Clear` acts only on the runtime overlay; it cannot erase a
required baseline field. Resolution and every runtime update validate the
complete effective postimage. `ResolvedMemoryConfig` exposes effective
policy, selected profile IDs, and fingerprint; the host owns input-source
provenance. It is an immutable, validated **baseline artifact**, not a live
platform or a mutable runtime policy. Its read-only getters expose the
effective baseline policy, fingerprint, and optional deployment/workload
profile IDs. The platform's active policy is that baseline plus its current
runtime overlay; a snapshot, not the old resolved value, reports active state.

An optional read-only `ConfigSchema` may describe field type, units,
mutability (`BuildOnly`/`Runtime`), and safety level for host tooling. It
does not create a second `Knob<T>` registry. If a document parser is later
provided, it accepts host-supplied bytes, rejects unknown/duplicate fields
and unavailable features, and maps to these same typed fields. JSON/YAML,
file watching, and source-precedence logic are not part of 0.3.

### IDs and privileged roles

`MemoryClassId`, `OwnerId`, `ResourceGroupId`, `ContextKindId`, `RuntimeId`,
`RuntimeKindId`, and `ConsumerName` have private fields, bounded UTF-8
grammar, and fallible `parse`. Class IDs are extensible host taxonomy, but
a class must be declared in validated policy before admission. QMP-owned IDs
are limited to semantics QMP actually implements; the `qmp.` namespace is
reserved.
Host context kinds are declared at build time and capped in number; unknown
kinds are rejected before a ticket is created. The QMP-owned runtime kind is
always available, cannot be removed, and has no admission privilege. Context
usage is aggregated by kind, not arbitrary per-request IDs, so dynamic request
IDs do not become unbounded ledger or metric keys. The closed 0.2
`MemoryContextKind` enum is replaced by this diagnostic taxonomy; migration
must preserve the meaning of existing store charges explicitly.

The existing `MemoryClass::Headroom` is a privileged admission role, not
an ordinary class string. Migration must keep that role non-forgeable and
preserve current headroom/floor/burst mathematics. A host class named
`qmp.headroom` or similar gains no privilege. Current mandatory
host-specific enum variants become host-declared budgets, not mandatory
portable-core classes. Host use of the old privileged role must be inventoried
before its API is removed.

### Authority, errors, and lifecycle

The ledger remains the sole accounting source of truth. Every accounting
mutation validates the complete postimage before committing. Reclaim
callbacks run outside the accounting lock and admission remeasures afterward;
hard denial never becomes silent overcommit. In-process transfer conserves
one live authority and preserves the source on denial. Stores reserve before
retaining, close idempotently, reject mutation after close, and keep cleanup
possible during shutdown. For 0.3, no client-provided callback may execute
under a store or broker lock.

This last rule is a migration requirement, not a property of every 0.2 store
path: current caller `Clone`/`Ord`/`Hash` work can run under a store mutex.
The 0.3 change needs public reentrancy, panic, and slow-callback oracles that
show the lock is released without losing the accounting postimage.

`MemoryError` remains non-exhaustive, with typed capacity axis,
requested/current/limit bytes, relevant IDs, and revision. A live reservation's
ordinary resize/release error never signals lost authority; any recovery state
for a different feature needs its own reachable failure and ownership contract.
Callers do not parse `Display`.
Failed observation does not fabricate zero usage. Events/metrics are bounded;
owner, session, key, path, and arbitrary error text are not metric labels.
Zero-byte tickets and deferred cleanup consume bounded management metadata.
The host owns shutdown ordering: stop work, close stores, release custom
reservations, then inspect final semantic usage.

### Extensibility and compatibility

New host classes and owners need no QMP release; new store/backing semantics
do need a feature-specific typed contract and tests. External ports are
`MemoryReclaimer`, `MemoryEventSink`, and `AllocatorTelemetry`; hosts own
config loading/capacity detection and use `MemoryConsumer` for custom
components. None can write ledger counters directly. Public structs keep
private fields, evolvable enums are non-exhaustive, and each type has one
canonical public path. Existing `0.2` feature names remain unless a rename
has a consumer benefit. Features are additive: default enables enforced
accounting, store slices are independent, `full` covers production in-memory
slices, and Redb remains opt-in/out of this release target.

No promise of zero breaking changes is made for pre-1.0. Instead, changes to
identifier grammar, profile defaults, serialized document shape, or feature
defaults require explicit versioning/migration. `ConfigFingerprint` identifies
the effective policy plus build-only admission mode and declared context kinds,
excluding source/provenance and telemetry. It requires a
versioned canonical encoding and golden vectors before release;
cross-version comparability is not implied.

## 4. Engine obligations and acceptance

The package direction stays `core <- facade <- testkit`. `domain/` and
`ports/` remain free of host I/O/clocks. The facade adapts existing
`EffectiveBudgetResolver`, `PendingReservation`, `ManagedCharge`,
`ProcessMemoryBroker::attach_runtime`, budgeted store builders,
`BrokerObservation`, and `try_live_reconfigure`; it must not add a second
ledger, a global mutable policy, or public raw accounting constructors.

Implement in this order: (1) freeze real 0.2 and host call sites; (2) separate
host taxonomy from privileged headroom and remove non-memory execution
policy; (3) add typed config/profile resolution and structured errors;
(4) add scope/consumer/measurement authority; (5) move stores and
observation/reconfiguration behind the facade; (6) migrate an exact host
revision, then remove deep-path aliases. The public `broker()` escape cannot
be removed before every current host use has a safe replacement.

Acceptance requires compiling examples for both platform construction paths,
scope with/without diagnostic kind, custom retention and growth/shrink, each
enabled store, observation, and reconfiguration. Negative compile tests reject
raw authority construction, caller-selected unbudgeted accounting through
scope builders, disabled features, and forged headroom. Builder tests reject
missing owner or class with
typed `Build` errors for every enabled store and custom consumer; omitted
context and store name use their documented defaults. Behavioral tests prove
pending rollback, measured-commit
surplus/over-bound/zero-byte behavior, authority conservation across
release/resize/transfer failures, default rejection of undeclared owners
(including zero-byte tickets), explicit global-only opt-in, unique owner→group
attribution, profile collision rejection independent of insertion order,
baseline resolution of profile collisions, any published profile's normative
leaf table and golden vectors, context default/override and
unknown-kind rejection, overlay `Clear`, live-remap rejection, stale/no-op
reconfiguration, coherent
semantic snapshots, advisory telemetry separation, and a conformance fake
that is deliberately rejected.
The two construction paths must produce identical effective fingerprints
and failures; checked `MemorySize` constructors reject overflow; omitting
diagnostic runtime kind must not change admission. Scopes derived from one
platform share its ledger, while two separately built platforms do not share
usage or pretend to enforce a combined capacity.

Add fault-injection coverage for deferred cleanup failure: exact ticket
authority remains retained, the backlog/error is publicly observable, bounded
retry can finish, and shutdown cannot report a false zero. Prove that an
independent host can use its own runtime-kind ID without mandatory
host-specific enum variants. For resize/release faults, assert the same live
ticket is unchanged and retryable; post-release calls return `InvalidState`.
Exercise grow-before-mutate, partial mutation failure with conservative charge,
shrink-before-refund, and failed refund retaining an overcharge. No public
`RecoveryRequired` branch is accepted for live resize without a reachable
fault-injection oracle and an exact authority-preserving recovery design.

On a committed clean candidate, run `python3 tools/pm/pm.py sync`,
`scripts/check-workspace.sh`, strict rustdoc, public API baseline, and the
standalone extraction dry-run. The dry-run is preflight, not a real standalone
export qualification. If standalone qualification is claimed, perform real
extraction and its locked rail on the same frozen source. Repo-local checks
qualify only the library;
host adoption requires a separate exact dependency/source receipt. Merge,
deployment, and activation are separate states. This RFC alone closes none
of those gates.

## 5. External design evidence

- [DataFusion's MemoryPool/MemoryReservation](https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/trait.MemoryPool.html)
  supports named consumers and RAII charges; QMP keeps its stricter
  reserve/commit and fail-closed ledger semantics.
- [Velox's memory management](https://facebookincubator.github.io/velox/develop/memory.html)
  separates hierarchy/attribution from aggregate reservations; QMP adopts
  scoped attribution without pretending to own a physical allocator.
- [RocksDB's memory accounting](https://github.com/facebook/rocksdb/wiki/Memory-usage-in-RocksDB)
  shows why a limit must name its covered components and exclusions; QMP
  therefore calls its numbers semantic, not process-wide.
- [AWS SDK for Rust behavior versions](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/behavior-versions.html)
  motivates stable default sets; QMP uses versioned profiles rather than a
  global behavior-version axis.
- [Cargo feature guidance](https://doc.rust-lang.org/cargo/reference/features.html)
  supports additive feature slices; QMP avoids a mandatory backing bundle.
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/type-safety.html)
  supports unit-safe newtypes and builders for genuinely compound values;
  QMP does not introduce a builder for a reservation with only kind and size.

Deferred until a named consumer justifies them: generic `Knob<T>` registry,
plugin/service container, runtime/control handle split, generic durability
tier, async runtime, process-wide shutdown registry, FFI, and package rename.

Do not add a reservation builder merely to move `ChargeKind` out of
`reserve_up_to`; add one only when an independent reservation option has a
real caller. Do not add `scratch.transition(retained)` in 0.3: the current
`ManagedCharge::recharge` can atomically replace a **committed** charge, but
the scratch port owns a separate **pending** authority. A public transition
needs a concrete caller, an ownership bridge, and postimage/failure tests.
If introduced later, releasing scratch before reserving retained bytes is
forbidden while the value remains live. Reclaimability is likewise an
orthogonal store/reclaimer capability, not a privilege inferred from a class
name or an arbitrary per-reservation flag.
