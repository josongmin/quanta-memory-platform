# RFC 0001: Public SDK Contract for Quanta Memory Platform

- Status: **ACCEPTED — final 0.3 interface contract; implementation and qualification pending**
- Target: `0.3.0`
- Date: 2026-09-24
- Scope: public Rust SDK, configuration, profiles, accounting DSL, observation,
  and live policy updates
- Decision owner: repository maintainers and consuming-host owner

## 1. Decision

QMP remains a small, synchronous, host-independent semantic-memory authority.
Its public SDK is organized around six concepts:

1. `MemoryPlatform` — the configured aggregate and factory.
2. `MemoryConfig` — complete typed desired policy.
3. `ConfigPatch` — typed build-time or runtime overrides.
4. `MemoryConsumer` — reusable accounting identity for custom components.
5. `MemoryReservation` — non-forgeable RAII accounting authority.
6. `MemorySnapshot` — one revision-consistent operational observation.

Built-in stores and spill/recovery remain feature-specific APIs constructed from
the same `MemoryPlatform`. They are not forced through a generic plugin or
service-container abstraction.

This revision deliberately does **not** introduce:

- a Kubernetes-style `spec/status/conditions` control plane;
- a separate `MemoryRuntime` / `MemoryControl` handle hierarchy;
- an AWS-style global `BehaviorVersion`;
- a generic `Knob<T>` registry parallel to typed configuration;
- a platform-wide store registry or drain state machine;
- a package rename as a prerequisite for the API work;
- an embedded config loader, daemon, exporter, or async runtime.

Those mechanisms add state and migration cost without solving a demonstrated
QMP requirement. They may be reconsidered only with a concrete consumer and a
failing use case.

## 2. Source Audit

This contract was derived from the current source at:

```text
HEAD b6d162650edba4f2b32e5020c8f0ac707efb0c22
branch main
```

The worktree contained unrelated active durability and documentation changes.
This RFC does not claim those changes are qualified and does not depend on
their completion.

### 2.1 Existing behavior to preserve

The current implementation already provides valuable primitives:

- one fail-closed semantic ledger;
- two-phase reserve/commit through `PendingReservation`;
- RAII committed authority through `ManagedCharge`;
- deterministic preset/manifest/override resolution;
- ordered override provenance;
- monotonic live-policy revision;
- live reconfiguration that rejects caps below current usage;
- one lock-consistent `BrokerObservation`;
- reclaim callbacks outside the broker lock followed by remeasurement;
- typed, accounting-backed built-in store builders;
- explicit durable uncertainty and reconciliation;
- bounded management metadata and deferred cleanup.

The 0.3 SDK should wrap, rename, and narrow these authorities. It should not
replace them with a second accounting model.

### 2.2 Confirmed public-contract defects

#### A. Host-specific memory classes leak into the portable core

The public `MemoryClass` enum contains names such as:

```text
MemoResidency
TraceIndex
ManifestResidency
ReasoningGraphResidency
PtaResumeState
PtaAuthorityState
PtaPortableBundle
ReopenStore
GenerationMetadata
```

These are consuming-host policy names, not portable memory-library concepts.
They contradict the repository rule that public APIs remain host-independent.

**Decision:** replace the closed enum with a validated, bounded
`MemoryClassId`. QMP may publish a minimal set of genuinely portable constants;
the host defines its own namespaced class IDs in its adapter/configuration.

#### B. Non-memory execution policy is carried by a memory library

`ExecutionBudget` includes wall time, subquery count, trace-edge count, and
output-byte limits. QMP neither owns the execution engine nor observes the
required events. Carrying those fields creates nominal enforcement and expands
the public contract beyond memory authority.

**Decision:** remove non-memory execution limits from the 0.3 QMP contract.
Scratch and retained byte limits remain memory classes or consumer limits.
Wall time, query count, graph edges, and output policy belong to the host.

#### C. Presets conflate deployment and workload

`Development`, `Ci`, and `Production` describe deployment contexts;
`Interactive` describes workload behavior. They currently occupy one enum.

**Decision:** split these axes into `DeploymentProfile` and
`WorkloadProfile`. Profiles expand into ordinary typed configuration and never
bypass validation.

#### D. Production has a machine-independent capacity

The current production preset materializes a fixed `4 GiB` global limit. The
safe limit depends on the process/container boundary, co-resident components,
and headroom.

**Decision:** production construction requires explicit host-supplied capacity.
QMP does not discover cgroups, environment variables, or host RAM.

#### E. Errors require message parsing

`MemoryError` variants currently carry strings. Callers cannot reliably inspect
capacity axis, requested bytes, active revision, closed resource, or recovery
requirement.

**Decision:** retain one non-exhaustive top-level error enum, but give actionable
variants structured payloads. Human-readable text remains `Display` output.

#### F. The root exposes implementation topology and raw broker control

`api`, `application`, `domain`, `adapters`, `ports`, `shared`, concrete
`Simple*` implementations, and direct `broker()` access are visible parts of
the current surface.

**Decision:** converge on canonical root exports and documented extension
traits. Deep implementation paths are not 0.3 contracts. Direct broker access
is deprecated after equivalent platform methods exist.

## 3. Purpose

QMP answers and enforces four questions:

1. May a component retain another `N` semantic bytes?
2. Which class, owner, context, and resource group owns those bytes?
3. How is that authority released, moved, spilled, or recovered without drift?
4. Which exact resolved policy and revision produced the decision?

The ledger tracks semantic retained authority. It does not claim exact RSS,
allocator, mmap, page-cache, or backing-database memory.

### 3.1 Provided capabilities

- global, class, owner, context, and resource-group memory budgets;
- reserved, soft, hard, headroom, and bounded burst policy;
- two-phase fail-closed reservation and commit;
- RAII release and explicit release reporting;
- typed custom-consumer integration;
- accounting-backed payload, retained-map, revocable, transcript, and
  publication stores;
- pressure classification and bounded reclaim requests;
- in-process and durable spill/restore with uncertain-outcome fencing;
- deterministic config layering, validation, provenance, and fingerprinting;
- typed live-policy updates with optimistic revision checks;
- one revision-consistent operational snapshot;
- deterministic conformance and fault-injection support in testkit.

### 3.2 Non-goals

QMP is not:

- a general-purpose allocator;
- an RSS or cgroup enforcer;
- an execution engine or query governor;
- a storage database;
- a service container or plugin runtime;
- an HTTP/gRPC administration service;
- a Prometheus/OpenTelemetry exporter;
- a host configuration loader;
- the Semantica adapter;
- an async scheduling or wait-for-capacity system.

## 4. External Reference Disposition

External designs are used selectively, not copied wholesale.

| Reference | Adopt | Do not adopt |
| --- | --- | --- |
| [DataFusion memory pool](https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/trait.MemoryPool.html) | named consumers, multiple RAII reservations, reserve before retention, spill-or-fail on denial | public pluggable raw pool mutation; unbounded pool as production default |
| [DataFusion reservation](https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/struct.MemoryReservation.html) | grow/shrink/split semantics and drop release | QMP keeps two-phase reserve/commit because retained authority may fail between admission and ownership transfer |
| [Envoy overload manager](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/operations/overload_manager.html) | separate measurement, threshold, and mitigation; external callbacks outside critical locks | allocator/cgroup pressure silently changing the semantic ledger |
| [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html) | additive capabilities and deliberate default features | mutually exclusive policy modes or test hooks in production defaults |
| [AWS behavior versions](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/behavior-versions.html) | explicit versioning is useful when defaults evolve independently of package versions | global `BehaviorVersion` now; QMP has one local config domain and is pre-1.0 |
| [Kubernetes API conventions](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md) | desired configuration must be distinguishable from observed runtime state | resource generations, conditions, controllers, timestamps, and API-server semantics in an in-process library |
| [Tower](https://docs.rs/tower/latest/tower/) | small stable traits and composition outside the trait | an async `Service`/`Layer` abstraction for synchronous accounting |

Revisit `BehaviorVersion` only if one released SDK must intentionally support
multiple incompatible default-behavior families. Revisit conditions only if an
actual external control plane needs them.

## 5. Design Principles

### 5.1 One authority

Every retained semantic byte has exactly one accounting authority. The SDK
never exposes a second counter, raw ledger writer, or independently mutable
accounting sink.

### 5.2 Structured config is the source of truth

Profiles and operator knobs expand into `MemoryConfig` or `ConfigPatch`. They
are not parallel policy systems.

### 5.3 Fail closed

Invalid config, unknown class IDs, unavailable capabilities, stale revisions,
capacity denial, and uncertain durable commits return typed errors. They never
fall back to an unbudgeted or assumed-success path.

### 5.4 Sync core, host-owned orchestration

QMP operations are synchronous and non-waiting. The host owns retries,
timeouts, cancellation, async scheduling, authentication, and coordinated
shutdown.

### 5.5 Portable names, host-defined taxonomy

Core understands bounded identifiers and accounting semantics, not host domain
nouns. A host adapter defines domain-specific class constants and profiles.

### 5.6 Observable effective state

Every accepted configuration resolves to an inspectable policy, source
provenance, fingerprint, and revision. Defaults are never invisible.

## 6. Package and Module Boundary

### 6.1 Packages

Keep the existing three-package graph for 0.3:

```text
quanta-memory-platform-core
        ^
        |
quanta-memory-platform-facade
        ^
        |
quanta-memory-platform-testkit
```

The facade remains the primary consumer entry point. Renaming it to
`quanta-memory-platform` is a release/distribution decision, not a prerequisite
for correcting the interface. No fourth runtime crate is introduced.

### 6.2 Canonical public paths

The facade root re-exports the normal integration surface:

```rust
use quanta_memory_platform_facade::{
    MemoryPlatform,
    MemoryPlatformBuilder,
    MemoryConfig,
    MemoryConfigBuilder,
    MemoryError,
    MemoryResult,
};
```

Optional feature modules expose only their own store/spill contracts. The core
crate remains available for advanced integrations, but raw ledger types and
construction paths stay private.

The following implementation directories are not stable public namespaces:

```text
application
domain
adapters
shared
service
backings
accounting
lifecycle
```

### 6.3 Prelude

A prelude is optional. If added, it contains only high-frequency traits and
types. It must not wildcard-export every feature or hide required authority
types.

## 7. Identifier and Taxonomy Contract

### 7.1 Validated identifiers

```rust
pub struct MemoryClassId(/* private */);
pub struct OwnerId(/* private */);
pub struct ResourceGroupId(/* private */);
pub struct ConsumerName(/* private */);
```

Each identifier:

- validates UTF-8 byte length before entering policy or accounting state;
- uses a stable lowercase dotted or kebab-case grammar;
- is orderable and hashable;
- has private fields;
- provides `parse`, `as_str`, `Display`, and fallible conversion;
- cannot contain empty segments, control characters, or surrounding space.

Recommended namespaces:

```text
qmp.scratch
qmp.transcript
qmp.spill-cache
host.artifact-cache
host.pta.resume-state
```

QMP-owned constants are limited to semantics implemented by QMP itself. Host
constants live in the host adapter.

### 7.2 Why not an extensible enum

A non-exhaustive enum still requires QMP releases for host classes and keeps
host taxonomy in the portable crate. An opaque validated ID permits extension
without granting authority: the ID must still be declared in validated config
before any reservation succeeds.

## 8. Configuration Contract

### 8.1 Complete desired policy

```rust
pub struct MemoryConfig {
    // private; immutable after validation
}

pub struct MemoryConfigBuilder {
    // private
}
```

Logical shape:

```text
MemoryConfig
├── capacity
├── headroom
├── shared_burst_pool
├── pressure
├── classes
├── owners
├── resource_groups
└── owner_group_bindings
```

Non-memory execution limits are absent.

Store, codec, and backing configuration remains with the owning feature API
(`TranscriptConfig`, `RedbSpillConfig`, and similar types). The platform config
does not become a monolithic service configuration document.

### 8.2 Construction

```rust
let config = MemoryConfig::builder()
    .capacity(MemoryCapacity::bytes(8 * GiB))
    .headroom(1 * GiB)
    .shared_burst_pool(512 * MiB)
    .class(
        MemoryClassConfig::new(MemoryClassId::parse("host.artifact-cache")?)
            .reserved(128 * MiB)
            .soft_limit(1 * GiB)
            .hard_limit(2 * GiB)
            .reclaim_priority(ReclaimPriority::First)
            .borrow_from_shared_pool(true),
    )
    .build()?;
```

All byte arithmetic is checked. `build()` performs complete validation and
returns an immutable `MemoryConfig`.

### 8.3 Capacity

```rust
pub struct MemoryCapacity(/* private */);

impl MemoryCapacity {
    pub fn bytes(bytes: u64) -> Self;
}
```

The host may record how it discovered the value in provenance, but discovery is
outside QMP. Production profiles cannot manufacture a machine-independent
capacity.

### 8.4 Profiles

Profiles are versioned, typed functions that produce ordinary patches:

```rust
pub struct DeploymentProfile(/* private */);
pub struct WorkloadProfile(/* private */);

pub mod profiles {
    pub const DEVELOPMENT_V1: DeploymentProfile;
    pub const CI_V1: DeploymentProfile;
    pub const PRODUCTION_V1: DeploymentProfile;
    pub const INTERACTIVE_V1: WorkloadProfile;
    pub const BATCH_V1: WorkloadProfile;
    pub const SERVICE_V1: WorkloadProfile;
}
```

Rules:

- profile IDs are explicitly versioned;
- deployment and workload are separate axes;
- profiles provide defaults, never hidden exceptions;
- `PRODUCTION_V1` does not provide capacity;
- resolved output contains the expanded values and profile IDs;
- custom host profiles live in the host adapter and return `ConfigPatch`;
- applying conflicting profiles to the same axis is an error.

This provides default stability without a global behavior-version mechanism.

### 8.5 Layers and precedence

```rust
#[non_exhaustive]
pub enum ConfigSource {
    Profile,
    Manifest,
    Environment,
    Cli,
    Runtime,
    TestFixture,
}
```

Canonical precedence:

```text
profile < manifest < environment < CLI < runtime < test fixture
```

QMP does not read those sources. The host parses external state and submits
typed layers with source identity.

Within one source tier, order must be explicit and unique. Call order cannot
silently invert source precedence.

### 8.6 Patch semantics

```rust
pub enum PatchValue<T> {
    Unchanged,
    Set(T),
    Clear,
}
```

Map changes use explicit upsert/remove operations. Removing a class, owner, or
resource group with live authority is rejected.

### 8.7 Resolution result

```rust
pub struct ResolvedMemoryConfig {
    // private
}

impl ResolvedMemoryConfig {
    pub fn config(&self) -> &MemoryConfig;
    pub fn fingerprint(&self) -> ConfigFingerprint;
    pub fn provenance(&self) -> &ConfigProvenance;
    pub fn profiles(&self) -> &ResolvedProfiles;
}
```

Resolution creates no broker, store, ticket, or reservation.

### 8.8 Validation

Validation checks at least:

- `reserved <= soft <= hard` for every class and resource group;
- every hard limit is within global capacity;
- aggregate reserved floors plus headroom fit capacity;
- owner and group mappings reference declared IDs;
- all identifiers meet length and grammar bounds;
- class, owner-quota, group, and mapping counts stay within published
  management-metadata bounds;
- checked arithmetic does not overflow;
- production has explicit capacity;
- unknown serialized fields are rejected.

Builder construction may fail fast at the first structurally impossible call.
Document parsing should aggregate independent validation issues so operators can
fix one file in one pass.

## 9. Knob Contract

### 9.1 Knob is an operational role, not a second type system

A knob is a reloadable leaf of `MemoryConfig`. The SDK does not introduce a
parallel generic `Knob<T>` registry.

Typed code uses typed patch methods:

```rust
let patch = ConfigPatch::builder()
    .capacity(10 * GiB)
    .class("host.artifact-cache", |class| {
        class.soft_limit(2 * GiB).hard_limit(3 * GiB)
    })?
    .pressure(|pressure| pressure.soft_percent(75).hard_percent(90))?
    .build()?;
```

This avoids two sources of truth such as both
`config.pressure.soft_limit` and `knobs::PRESSURE_SOFT_LIMIT`.

### 9.2 Field metadata

For documentation and external admin tooling, the optional serialization
feature exposes a read-only schema:

```rust
pub struct ConfigFieldDescriptor {
    // private fields; stable getters
}
```

This schema describes the typed config; it does not mutate policy by itself.

```rust
pub enum ConfigMutability {
    BuildOnly,
    Runtime,
}

pub enum ConfigSafety {
    Standard,
    Expert,
    TestOnly,
}
```

String paths exist only at parsing/admin boundaries. Unknown path, wrong type,
wrong unit, unavailable feature, or test-only field in production is an error.

### 9.3 Forbidden knobs

No configuration may mean:

```text
skip-validation
allow-counter-drift
ignore-release-failure
disable-hard-limit
assume-durable-commit
drop-recovery-records
unbounded-management-metadata
```

## 10. Platform Construction DSL

```rust
use quanta_memory_platform_facade::{
    MemoryPlatform,
    MemoryResult,
    profiles,
};

fn build_platform(
    reclaimer: Arc<dyn MemoryReclaimer>,
    events: Arc<dyn MemoryEventSink>,
) -> MemoryResult<MemoryPlatform> {
    MemoryPlatform::builder()
        .deployment_profile(profiles::PRODUCTION_V1)
        .workload_profile(profiles::INTERACTIVE_V1)
        .capacity(8 * GiB)
        .manifest(manifest_patch)
        .reclaimer(reclaimer)
        .event_sink(events)
        .build()
}
```

Rules:

- `build()` is the normal accounting-backed path;
- `resolve()` returns `ResolvedMemoryConfig` without runtime authority;
- there is no unbudgeted fallback;
- required feature configuration fails if the feature is absent;
- builder fields remain private;
- missing required state returns typed `BuildError`, never a panic;
- config resolution and platform construction are separately testable.

`build_live()` becomes a deprecated alias once `build()` is the only enforced
production path.

## 11. Consumer and Reservation DSL

### 11.1 Consumer

```rust
let consumer = platform
    .consumer("vector-index")?
    .owner("retrieval")?
    .class("host.artifact-cache")?
    .context(MemoryContextKind::Request)
    .resource_group("interactive-queries")?
    .build()?;
```

`MemoryConsumer` is a cheap immutable accounting descriptor tied to one
platform. Creating it does not reserve bytes and does not require a permanent
registry entry.

### 11.2 Two-phase authority

```rust
let pending = consumer.reserve(8 * MiB, ChargeKind::RetainedPayload)?;
let buffer = allocate_and_initialize()?;
let reservation = pending.commit()?;
```

If allocation or initialization fails, dropping `pending` rolls back admission.
After commit, the retained object owns a `MemoryReservation` or transfers it to
a built-in store.

```rust
pub struct PendingReservation(/* private, non-forgeable */);
pub struct MemoryReservation(/* private, non-forgeable */);

impl MemoryReservation {
    pub fn bytes(&self) -> u64;
    pub fn class(&self) -> &MemoryClassId;
    pub fn owner(&self) -> &OwnerId;
    pub fn kind(&self) -> ChargeKind;
    pub fn try_resize(&mut self, new_bytes: u64) -> MemoryResult<()>;
    pub fn try_release(&mut self) -> MemoryResult<ReleaseReceipt>;
    pub fn is_active(&self) -> bool;
}
```

Normative behavior:

- denial leaves all counters unchanged;
- commit is single-use;
- resize is atomic or preserves the prior reservation;
- cross-platform or cross-consumer transfer requires a validated recharge;
- drop attempts release and uses only bounded deferred cleanup;
- failed explicit release leaves the reservation active in the caller-owned
  object;
- no public `forget`, raw ticket, counter setter, or ledger writer exists.

Reservation splitting is deferred. A consumer may hold multiple reservations;
`split` is added only when a real caller needs ownership subdivision and its
failure/recovery semantics are specified.

Built-in stores encapsulate this protocol; ordinary store users should not need
to hold reservations manually.

## 12. Built-in Store DSL

```rust
let payloads = platform
    .payload_store::<ArtifactId, Vec<u8>>()
    .name("artifacts")?
    .owner("retrieval")?
    .class("host.artifact-cache")?
    .context(request_context)
    .build()?;
```

The platform supplies accounting automatically. Public standalone builders may
exist for compile-time typestate, but their unset state has no `build()` method.

Stable public handles use domain names such as:

```text
PayloadStoreHandle
RetainedMapHandle
RevocableStoreHandle
TranscriptStoreHandle
PreparedPublicationHandle
```

The existing `Simple*` implementation types are migrated behind these handles
or deprecated aliases. Do not introduce a trait hierarchy merely to hide a
name; add a trait only when an external implementation or generic caller exists.

Every built-in store must reserve before retaining, keep exactly one authority
per value, fail closed, expose bounded stats, close idempotently, reject mutation
after close, preserve release/recovery during close, and avoid external
callbacks under its lock.

Custom stores compose `MemoryConsumer`; they never implement a raw accounting
sink.

## 13. Pressure and Reclaim

QMP retains its existing semantic pressure model:

```rust
pub enum PressureState {
    Normal,
    Soft,
    Hard,
    Fatal,
}
```

Reclaim callbacks execute outside the broker lock, receive a bounded typed
request, cannot mutate ledger counters directly, are followed by remeasurement,
and cannot convert a hard denial into silent overcommit.

Allocator, RSS, and cgroup observations are advisory. The host may use them for
load shedding or to propose a config change; they never directly mutate QMP
semantic accounting.

Do not add a generic ordered mitigation engine until QMP owns at least two
independently configurable mitigation mechanisms. Reclaim and spill remain
explicit.

## 14. Observation and Operations

### 14.1 One consistent snapshot

```rust
pub struct MemorySnapshot(/* private */);

impl MemoryPlatform {
    pub fn snapshot(&self) -> MemoryResult<MemorySnapshot>;
}
```

The snapshot contains one lock-consistent view of policy revision, config
fingerprint, semantic totals, charge-kind breakdown, class/owner/group usage,
pressure, reclaim counters, bounded deferred cleanup, and optional allocator
advisory data.

High-cardinality owner detail is returned through bounded queries, not metric
labels.

### 14.2 Live reconfiguration

Retain the current atomic reconfigure operation and add expected revision plus
a receipt. A separate plan object is unnecessary.

```rust
let before = platform.snapshot()?;
let receipt = platform.reconfigure(
    ReconfigureRequest::new(patch)
        .expected_revision(before.policy_revision()),
)?;
```

```rust
pub struct ReconfigureReceipt {
    // private fields; stable getters
}
```

Reject stale revisions, build-only changes, caps below live usage, removal of
live IDs, unavailable features, and invalid invariants. Failure leaves config,
revision, and ledger unchanged.

### 14.3 No global control handle yet

`MemoryPlatform` is already the authority-bearing aggregate. Splitting it into
runtime and control handles would not create a security boundary inside one
process and would double forwarding APIs. Introduce separate handles only after
a concrete component must receive data-plane access while being statically
unable to reconfigure.

## 15. Lifecycle Ownership

Stores keep explicit, idempotent `close()` operations. QMP does not yet add a
platform-wide registry or drain state machine: stores are independently owned,
custom consumers cannot be forcibly closed safely, and the host owns shutdown,
deadlines, and cancellation.

The host coordinates shutdown:

1. stop new work at the host boundary;
2. close owned stores/services;
3. complete required durable reconciliation;
4. release custom reservations;
5. inspect a final QMP snapshot;
6. fail qualification if authority is unexpectedly non-zero.

QMP may add a read-only `quiescence_report()` for step 5; it must not pretend to
own steps 1–4.

## 16. Durable Spill and Recovery

Durability remains an explicit feature-specific API rather than a generic
platform tier. QMP currently has concrete Redb constraints; backing paths, file
ownership, and capacity are deployment concerns; and durable ambiguity requires
operation-specific receipts.

The stable contract includes bounded codec APIs, explicit record/resident-byte
limits, spill/restore receipts, uncertain outcomes retaining recovery authority,
inspectable pending recovery, idempotent reconciliation, and no direct
backing-to-ledger mutation.

Raw backing transactions, durable claim constructors, schema tables, and kill
hooks remain private or test-only.

## 17. Error Contract

```rust
#[non_exhaustive]
pub enum MemoryError {
    Build(BuildError),
    Config(ConfigError),
    Capacity(CapacityDenied),
    Closed(ResourceClosed),
    Busy(OperationBusy),
    Poisoned(AuthorityPoisoned),
    InvalidState(StateViolation),
    Unsupported(UnsupportedCapability),
    StaleRevision(StaleRevision),
    RecoveryRequired(RecoveryRequired),
}
```

```rust
pub struct CapacityDenied {
    // private fields; stable getters for every item below
}
```

Its getters expose capacity axis, requested/current/limit bytes, optional class,
owner and resource group, policy revision, and retryability.

Errors may expose backing-independent recovery identifiers, but never concrete
Redb errors or text that callers must parse.

## 18. Serialization Boundary

Serialization is optional and feature-gated. QMP parses bytes supplied by the
host; it does not open files or inspect environment variables.

```toml
schema-version = 1
deployment-profile = "production-v1"
workload-profile = "interactive-v1"

[capacity]
limit = "8GiB"
headroom = "1GiB"
shared-burst-pool = "512MiB"

[pressure]
soft = "75%"
hard = "90%"
fatal = "97%"

[classes."host.artifact-cache"]
reserved = "128MiB"
soft-limit = "1GiB"
hard-limit = "2GiB"
borrow-from-shared-pool = true
reclaim-priority = "first"
```

Unknown and duplicate fields fail; units are mandatory where ambiguous;
overflow and invalid identifiers fail; serialized config cannot enable an
absent feature. Schema version covers document compatibility, profile IDs cover
versioned defaults, and fingerprinting occurs after expansion and validation.

TOML support is optional. JSON/YAML are not part of this RFC.

## 19. Features

Keep current feature names during migration unless renaming has a concrete
payoff. Renaming every underscore feature to a hyphen is churn, not architecture.

Normative rules:

- features are additive;
- default enables enforced live accounting only;
- store slices remain independently selectable;
- Redb remains opt-in;
- `full` means production in-memory capabilities, not every feature;
- `full-backings` adds production backings;
- unbudgeted stores and fault hooks are absent from default, `full`, and
  `full-backings`;
- test-only features cannot change production success semantics;
- feature-off configuration fails rather than becoming a successful no-op.

Moving hooks into testkit is useful only if real backing-path instrumentation is
preserved.

## 20. Extension Contract

Supported external seams:

- `MemoryReclaimer`;
- `MemoryEventSink`;
- `AllocatorTelemetry`;
- documented bounded codecs;
- host-owned config loading and capacity discovery;
- custom retained components built from `MemoryConsumer`.

Sealed/private authority:

- ledger construction and mutation;
- raw reservation tickets and accounting sinks;
- policy revision mutation;
- durable claim minting and backing transactions;
- ambiguous-commit resolution authority.

No `Any` extension bag, generic plugin registry, or arbitrary callback under the
broker lock is introduced.

## 21. Observability Contract

QMP emits bounded semantic events and typed snapshots, not an exporter.

- semantic and process/RSS bytes use distinct names;
- owner, session, key, path, and arbitrary text are not metric labels;
- high-cardinality detail uses bounded diagnostic queries;
- sink failure cannot corrupt accounting;
- explicit `NoopMemoryEventSink` is allowed because the choice is visible;
- feature-off metrics cannot report successful installation;
- snapshots include policy revision.

## 22. Compatibility Rules

- QMP is pre-1.0; 0.3 may intentionally break the current internal surface.
- Every public type has one canonical documented path.
- Public evolvable enums use `#[non_exhaustive]` where appropriate.
- Identifier grammar and maximum lengths are stable contracts.
- Incompatible documents require a schema-version change.
- Profile default changes require a new profile ID.
- Config removal requires deprecation or schema migration.
- Feature removal/default changes require an explicit migration note.
- Public APIs do not expose backing-library errors or concrete lock types.
- Accepted live changes increment revision exactly once.
- Failed mutations change neither policy, revision, fingerprint, nor ledger.

## 23. Migration Plan

### Phase 0 — Freeze and characterize

- record a public API baseline;
- add compile tests for happy paths and forbidden raw paths;
- record exact host call sites and feature selections;
- freeze current semantics with conformance tests.

### Phase 1 — Correct portability boundaries

- introduce `MemoryClassId` and bounded identifier types;
- move host-specific class constants/default weights to the host adapter;
- remove non-memory `ExecutionBudget` fields;
- replace fixed production capacity with required input;
- split deployment and workload profiles.

This phase is first because later DSL work on the closed host-specific enum
would preserve the wrong abstraction.

### Phase 2 — Configuration and errors

- introduce `MemoryConfig`, `ConfigPatch`, fingerprint, and provenance;
- add versioned profile IDs;
- add structured errors;
- wrap the existing resolver rather than creating a second one.

### Phase 3 — Consumer DSL

- introduce `MemoryConsumer`;
- rename/wrap `ManagedCharge` as `MemoryReservation`;
- preserve pending/commit and deferred cleanup;
- add resize/split only with invariant and negative conformance tests.

### Phase 4 — Facade DX

- add canonical platform store, snapshot, and reconfigure methods;
- introduce stable store handle names;
- deprecate `Simple*`, direct `broker()`, deep modules, and `build_live` only
  after replacements cover real host use cases.

### Phase 5 — Optional serialization

- add strict TOML parsing behind a feature;
- generate field docs from parser descriptors;
- do not add JSON/YAML or a dynamic knob registry without a consumer.

### Phase 6 — Host migration and removal

- migrate one exact consuming-host revision;
- verify config fingerprint and reservation lifecycle;
- remove deprecated surfaces only after the host no longer uses them;
- qualify repository and host separately.

## 24. Acceptance Tests

### 24.1 Compile contracts

- minimal platform build and config-only resolution;
- custom consumer reserve/commit/release;
- every enabled built-in store happy path;
- snapshot and revision-checked reconfigure;
- feature-disabled APIs are unreachable;
- raw ledger/ticket/sink construction is unreachable;
- unset store builder cannot build;
- host class constants compile outside QMP core.

### 24.2 Behavioral contracts

- reservation drop restores exact capacity;
- pending drop rolls back every accounting axis;
- resize/recharge conserve authority;
- denial leaves ledger and revision unchanged;
- precedence is independent of builder call order;
- profile expansion is deterministic by ID;
- config fingerprint is canonical;
- unknown config/class/capability fails;
- stale revision and cap shrink fail without mutation;
- observation is revision-consistent;
- feature-off telemetry cannot report successful wiring;
- conformance rejects broken accounting and backing fakes;
- ambiguous durable outcomes retain recovery authority.

### 24.3 Repository and host gates

On one clean committed candidate:

```bash
python3 tools/pm/pm.py sync
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo public-api --simplified
scripts/check-workspace.sh
scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-check
```

Repository GREEN proves only the portable library candidate. Host adoption
requires separate exact-source evidence for dependency identity, host class
taxonomy, resolved config/fingerprint, custom reservations, built-in stores,
snapshot, reconfiguration, and enabled durable recovery. Merge, deployment,
and activation remain separate states.

## 25. Explicitly Deferred

- global `BehaviorVersion`;
- `MemoryRuntime` / `MemoryControl` split;
- Kubernetes-like conditions;
- platform-wide drain ownership;
- generic mitigation engine;
- generic durable tier;
- dynamic `Knob<T>` registry;
- arbitrary plugin registry;
- JSON/YAML;
- async runtime integration;
- cross-language FFI;
- package rename or fourth runtime crate.

## 26. Final Contract Summary

The 0.3 interface is accepted only if:

1. The portable crate contains no consuming-host class taxonomy.
2. QMP owns memory authority only, not generic execution governance.
3. Production capacity is explicit and host-supplied.
4. Profiles are separate, typed, versioned default bundles.
5. Typed config is the source of truth; knobs are typed patches over it.
6. Custom components use consumers and non-forgeable reservations.
7. Built-in stores use the same ledger and hide accounting plumbing.
8. Reconfiguration is atomic, revision-checked, and receipt-returning.
9. Observation is bounded and revision-consistent.
10. Durable uncertainty remains explicit and recoverable.
11. Raw accounting and durable authority remain sealed.
12. New abstractions require demonstrated consumers, not analogy alone.

This is the accepted 0.3 interface contract. Changes to its twelve acceptance
statements require an RFC amendment with source-backed rationale. Acceptance is
not an implementation or qualification receipt.
