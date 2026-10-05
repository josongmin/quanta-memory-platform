# Migration to 0.2.0

QMP 0.2.0 is a deliberate pre-1.0 breaking release. The workspace has not been
published to crates.io (`publish = false`, `UNLICENSED`), so there is no binary
or source compatibility window for the internal 0.1.x snapshots. Consumers
must migrate atomically and qualify their own host composition against the exact
QMP Git tree. No deprecated alias is retained when it would reopen an
unbudgeted or unbounded path.

## Post-0.2 authority and durable API changes

### Transcript sessions use issued handles (breaking)

Call `open_session(caller_session_id)` before appending. Keep its opaque
`SessionHandle` and use it in the existing `TranscriptAppendIdentityV1` session
axis and every load/page/retain/CAS/retire call. Caller SessionId is retained,
charged metadata; two opens with the same value create independent sessions.
Do not route a late request by looking up the newest handle for its string ID.
Handles are process/store-local lookup identities, not durable IDs or
authorization tokens. Constructors remain builder-first and can fail.

`retire_session` releases that handle's session key, active rows and identity
evidence together. On failure they remain intact. After success, every old
handle operation, including an exact append retry, rejects with `InvalidState`.
Retiring it twice also rejects. `close` remains terminal and idempotent. A
caller may reopen the same metadata with a fresh handle without accepting old
requests into the new session.

The existing `TranscriptRetentionPolicyV2` adds `with_session_limits(count,
key_bytes)` for store-wide open-session bounds. Defaults are 65,536 sessions
and 64 MiB of semantic key cost. Zero-row sessions consume these limits and a
metadata lease. Existing evidence limits apply across all open handles;
truncation preserves replay evidence until that handle is retired or the
store closes. Stats report session count/key bytes as well as active/evidence
components. This changes the replay horizon without introducing a new IR.

### Transcript pages and conditional retention

The existing `SessionTranscriptStore` trait now exposes `read_page` and
`retain_sequences_if_version`. `TranscriptSnapshotV1` carries the handle,
version and optional opaque `TranscriptCursor`. Page limits count rows and
their captured active semantic retained costs; they do not bound caller-owned
clones, arbitrary callback latency or RSS. Zero limits and an oversized first
row return `Capacity`. A cursor from another handle/version returns
`InvalidState`; sequence exhaustion ends without wrapping.

A page is only part of a session. Retention selectors describe the entire
session, so passing just one page's sequences evicts other pages' active rows.
CAS detects concurrent mutation, not incomplete selectors. Gather all pages
of one version when a complete selector is required, and restart after a
stale cursor. Stale CAS returns `StaleSnapshot` without changing state.
Versions advance only on a fresh append or actual row eviction, not on replay,
no-op retention or rejected operations. The full snapshot remains available
as a concrete-store convenience.

Reads clone internal pinned rows after releasing the mutex. Evicting pinned
rows, retiring their session or closing the store returns
`OperationInProgress` before releasing any charge. Retry after the read
finishes. Using shared `Arc` row data means cross-thread store use also
requires message IDs/bodies to satisfy the corresponding `Send`/`Sync`
auto-trait bounds; single-thread use remains available.

### Scheduler is internal (breaking)

The scheduler root exports and nested `application::scheduler` path are no
longer public. Use the broker builder and its admission/release APIs; the
scheduler is a derived policy cache, not an independent accounting authority.
The scaling operation is now `broker_group_admission_cardinality` and records
its actual bounded group cardinality. Old standalone scheduler receipts are
not compatible baselines for this broker workload.

### Durable spill protocol is opt-in (breaking)

`spill` and `full` expose process-local transfer contracts. Enable
`spill-durable` explicitly for durable protocol/restore types and methods;
`spill-redb` enables it automatically. `full` includes neither durable API nor
Redb. This feature boundary does not qualify optional durable code for product
use or require host activation.

### Legacy non-retaining publish transitions removed (breaking)

`PublishPipeline`, `SimplePublishPipeline`, `PublishPipelineBuilder`,
`CommitTransition`, `DefaultCommitTransition`, `StagedHandle` and `PublishError`
are removed. The checked QMP examples and Semantica memory integration source
contain no product caller of this transition wrapper. External consumers must
migrate explicitly: a non-retaining value transformation calls its own fallible
transition and owns its result and counter. Do not replace that operation with
`PreparedPublicationService` unless retaining payload and replay evidence is
actually required; its accounting and lifetime contract is different.

Canonical-root activation remains caller-owned. A prepare receipt proves
process-resident prepared state, not publication visibility. `PublishedPayload`
remains the payload store's independent result type.

### Prepared identity evidence limits (breaking)

Replace `with_tombstone_limits(count, bytes)` with
`with_identity_evidence_limits(count, bytes)`. The limit covers every admitted
logical identity, both live prepared entries and aborted tombstones. Fresh
prepare reserves its eventual retirement evidence before charging the payload;
abort and orphan GC require no additional capacity admission. Default limits
remain 4,096 identities and 4 MiB of semantic evidence. Evidence survives until
store close; abort does not free an identity slot.

Match `PreparedPublicationErrorV1::IdentityEvidenceCapacity` in place of
`TombstoneCapacity`. `PreparedPublicationStatsV1` now exposes
`identity_evidence_count` and `identity_evidence_bytes` alongside the existing
live prepared and aborted-only counts. Update struct literals and exhaustive
error matches in the same consumer change. Identity bytes, canonical payload
key encoding and the deterministic 32-byte token remain unchanged.

### Payload references are store-issued Strings (breaking for custom references)

Production payload builders accept only `String` references. Custom
`From<String>` implementations can collapse distinct references, so they can no
longer implement the sealed `PayloadReference` contract. References include a
store nonce and checked entry sequence; retain the returned String verbatim.
They are process-local lookup identities, not durable IDs or authorization
tokens. Pin, unpin and deletion of stale or foreign references fail without
changing another entry. `pin_count` of a missing reference remains zero.

### Durable restore calls now take a claim (BREAKING for external implementers)

`commit_restore`, `finalize_restore`, `abort_restore`, and
`reconcile_restore` previously took a bare `sequence: u64`; they now
take `claim: &DurableRestoreClaim`. The claim binds backend identity,
claim generation, and sequence, and only backends can mint it (no
public constructor), so external crates cannot forge lease authority
and stale generations are rejected instead of double-restoring.
The claim, candidate, and recovery envelope are also deliberately
non-`Clone`; the authority-bearing value must move through one restore flow.
Keep a candidate or recovery envelope as the owner and borrow its claim for
same-claimant retries. Code that previously cloned one of these values must
restructure its flow rather than duplicate the lease.

Caller migration (same flow, one identifier change):

```rust
// Before: the sequence alone authorized the permission change.
backing.commit_restore(sequence)?;

// After: authorize with the candidate's claim.
let candidate = backing.prepare_restore()?.expect("candidate");
backing.commit_restore(&candidate.claim)?;
backing.finalize_restore(&candidate.claim)?;
```

`DurableRestoreCandidate` carries the claim; the active-restore
recovery entry point mints a fresh generation after reseating
ownership. Same-claimant ACK-loss retries of commit/finalize stay
idempotent `Ok`. A second claimant can no longer finalize with a bare
sequence.

This change applies only to consumers implementing or using durable restore.
Each consumer chooses its own feature set; no host checkout is qualified here.

### Redb storage schema v2 (automatic, fail-closed)

New tables/rows: a per-record disposition table, the single-owner
finalize-proof table, persisted backend identity and claim generations.
Opening a v1 file migrates it in the
same transaction: queued/recovery bytes are preserved and the
watermark record is kept. History that cannot be classified stays an
explicit error — the opener never deletes the file, reinterprets
opaque bytes, or guesses a commit outcome. There is no downgrade path
back to v1 layout; keep a file copy before first open if you need one.

### Redb resource limits and restore bytes (existing optional capability)

This migration applies only to callers enabling the opt-in Redb backing; it is
not a requirement of the current in-memory host release scope.

`RedbSpillBacking::create` now uses bounded defaults; production callers can
select explicit `RedbSpillLimits` with the create_with_limits constructor. Opening a file
whose existing payloads exceed the selected limits fails without deleting
records. The limits cover encoded record length, queued-plus-recovery count,
their logical payload sum, Redb database file length, Redb's configured cache,
and backing-owned restore staging bytes/buffer count (including 0-byte
candidates). The file-length cap includes Redb page/copy-on-write overhead,
but does **not** reserve free space on the volume or bound filesystem metadata,
snapshots, caller-owned copies, or private encoder/decoder scratch. After a
file-cap or ENOSPC I/O error, close and reopen the backing before probing its
persisted state; QMP reports the storage-full cause as `Capacity`, while a
poisoned handle reports a reopen-required `OperationInProgress`. Restore may
need a larger file cap to make progress. A
production host still needs volume quota/ENOSPC qualification and a bounded
codec contract.

`DurableRestoreCandidate::bytes` is now read-only via `candidate.bytes()` so
its backing-owned staging lease remains live until the candidate is dropped.
Code that moved the public `Vec` out must explicitly copy into caller-owned
memory and account for that copy under its own budget.

### Reconfigure provenance is bounded (non-breaking)

`override_provenance` keeps the initial resolver records plus at most
one current `residency.active` record; entries from accepted live
reconfigures upsert the single `residency.active` record. It is not a
chronological history (see the field rustdoc). Consumers requiring an audit
history must keep it separately.

### Group/owner removal now consults live tickets (behavior)

Removing a resource group or remapping an owner while ANY live ticket
— including a 0-byte ticket — draws authority from it is denied with
`InvalidState`. Previously a mapping with only 0-byte tickets could be
removed. No signature changed.

### Live management metadata is bounded (behavior)

Live reservation admission now rejects the ticket after
MAX_LIVE_TICKETS_PER_BROKER simultaneous authorities, including 0-byte
reservations. Owner and resource-group identifiers longer than their exported
256 UTF-8-byte limits are rejected (`Capacity` for runtime owner input,
`Config` for policy input). Deferred cleanup remains fail-closed and drains in
batches of at most MAX_DEFERRED_TICKET_DRAIN_BATCH tickets per lock hold; a
stuck pending cleanup is surfaced as `InvalidState` with its authority retained.
Consumers that create more than the limit of overlapping permits must release
or commit-and-release older permits before admitting more. These are logical
metadata limits, not allocator/RSS limits.

Synchronous reentrant reservation from a broker's own reclaim callback now
returns `OperationInProgress`; release and legal reconfiguration remain
available. Concurrent reclaim plans on the same broker do not start a second
callback while the first is active; the skipped plan re-evaluates against live
usage. Telemetry sink calls are single-flight per broker across threads. A
recursive or concurrent advisory event observed while the callback is active
is dropped rather than queued or replayed; this bounds callback reentry but is
not a lossless event stream.

Read-side facade operations may invoke caller-supplied `Clone`, `Ord`, or
canonical-identity code while observing a stable container image. If that code
panics, QMP still propagates the original panic, but first releases the store
mutex normally so later operations are not rejected solely because that read
poisoned the lock. This does not bound callback duration or roll back
caller-owned interior side effects.

## Later 0.2 API and behavior changes

### Rebuild fence requires a permit (source-breaking)

`begin_rebuild` now returns a non-`Clone`, epoch-bound `RebuildFence` and the
tokenless `finish_rebuild()` method is gone: closing the fence consumes its
permit via `RebuildFence::finish()`, which returns a `RebuildReceipt` that
separates the fence-open transition from the deferred-authority drain
(remaining queue depths plus an optional `drain_error`). A second
`begin_rebuild` while a fence is live is rejected; a permit dropped without
finishing leaves the fence observably abandoned
(`rebuild_fence_status() -> RebuildFenceStatus`) and writes stay rejected
until an explicit new `begin_rebuild` re-adopts it. Migration:

```rust
// Before
broker.begin_rebuild()?;
broker.finish_rebuild()?;

// After
let fence = broker.begin_rebuild()?;
let receipt = fence.finish()?;   // receipt.fence_open == true
```

The testkit conformance traits grew an associated `RebuildFence` type: both
`do_begin_rebuild` (returns the permit) and
`do_finish_rebuild(permit)` (consumes it) now pair the window with its
authority. Fakes use `type RebuildFence = ();`.

### `PressureThresholds` fields are private (source-breaking)

`PressureThresholds` no longer accepts direct struct literals: the three
rung fields are private and the only construction paths are the validating
`PressureThresholds::new` (fail-closed `MemoryError::Config` on inverted or
out-of-range ladders) and the valid `Default` (75/90/98). Read the rungs
through the new `soft_percent()`, `hard_percent()`, and `fatal_percent()`
getters. This closes the path where a caller-forged literal bypassed
validation and reached `LadderPressureController::new` unvalidated. Treat
this as a source API change; existing external literals must migrate to
`PressureThresholds::new`.

### Scratch peak observation is monotonic

`ScratchReservation::grow_to_observed` now records
`peak = max(peak, observed)`: a smaller valid observation is accepted
without lowering the recorded high-water mark. Over-ceiling and
post-release observations keep their previous typed errors and leave the
peak unchanged.

### Prepared-publication kind buckets and bounded orphan roots

The prepared-publication service now charges its two owned lifetime
components in their semantic charge buckets: the active payload component is
`RetainedPayload` and the persistent identity/index/tombstone evidence is
`RetainedMetadata`. Previously both leases were charged as payload, so an
aborted empty-payload entry left a metadata-only tombstone in the payload
axis. Admission totals, refunds, and ticket counts are unchanged; only the
public payload/metadata kind split moved. Consumers projecting
`total_retained_payload_bytes`/`total_retained_metadata_bytes` from ledger
snapshots see the persistent evidence move from the payload bucket to the
metadata bucket after this change.

`orphan_tokens` and `gc_orphans` now bound the raw caller-supplied
reference-root slice at 16,384 tokens (duplicates count toward the bound).
A larger slice is rejected with `Storage(MemoryError::Capacity(_))` before
any state lock or mutation; empty, duplicate, unknown, and aborted
references keep their previous typed semantics under the cap.

## Required consumer changes

- Construct live authority through `MemoryPlatformBuilder`,
  `PortableMemoryPresets`, and platform-issued store builders. Raw ledger,
  broker, scratch allocator, ticket, and late `charged_against` constructors are
  not public.
- Treat `build_report_only()` as configuration data only. It cannot create a
  runtime view, scratch reservation, or budgeted store.
- Use `String` payload references through the sealed store-issued codec.
  Arbitrary `From<String>` reference types can no longer construct a payload
  store: a non-injective conversion could alias an earlier deleted entry.
  Refs include an issuing-store namespace and monotonic sequence; do not parse
  their wire spelling or reuse them across processes. Collision-safe lookup
  buckets still compare complete canonical/reference identity.
- Rebaseline semantic retained cost using the actual issued reference length.
  Payload namespace bytes are retained reference identity and are charged.
  This is a portable semantic-cost model, not allocator RSS.
- Replace `TranscriptRetentionPolicyV1` with bounded
  `TranscriptRetentionPolicyV2`. Saturation is a typed denial; no unbounded
  compatibility alias is provided.
- Use `LedgerSnapshot::checked_total_*` for externally assembled snapshots.
  Legacy infallible totals require an exact representable snapshot.
- Treat in-process revocable spill as an ownership transfer. Spill handles share
  one `Arc<StoreBindingHandle>` and carry the original managed charge; they do
  not deep-clone owner/runtime identity or create an uncharged destination copy.

## Public lifecycle examples

The facade's public consumer examples now include the
[managed charge lifecycle](crates/quanta-memory-platform-facade/examples/managed_charge_lifecycle.rs),
which demonstrates `reserve_owned` → `commit` →
successful `recharge` → denied recharge with the old `ManagedCharge` preserved
→ explicit `release` and Drop. Its final broker observation asserts both zero
semantic bytes and zero live tickets. Run it with:

```bash
cargo run -p quanta-memory-platform-facade --features core-live-accounting --example managed_charge_lifecycle
```

[shared budget reclaim](crates/quanta-memory-platform-facade/examples/shared_budget_reclaim.rs)
demonstrates a retained-map overwrite admitted only after a different revocable
store releases its lease on the same broker. Its
callback follows the typed reclaim unit/scope, bounds work when the requested
reduction is unrepresentable, and deliberately under-reports bytes so the
broker's post-callback remeasurement is observable. The example closes both
stores and asserts zero final bytes and live tickets. Run it with:

```bash
cargo run -p quanta-memory-platform-facade --features core-live-accounting,retained_map,revocable_store --example shared_budget_reclaim
```

## Feature policy

- Default: live accounting bridge only.
- Production aggregate: `full`; it excludes every unbudgeted escape hatch and
  third-party durable backing.
- Durable backing aggregate: `full-backings`; currently this adds only
  `spill-redb`.
- Explicit non-production escape hatches: `unbudgeted-stores` and core
  `unbudgeted_pin`. Neither is enabled by default, `full`, or `full-backings`.
- Removed/quarantined names: moka, scc, and bumpalo are not advertised features
  because no canonical public builder path implements those capabilities.
- There is no `advanced-unchecked` feature. Unsafe raw accounting authority is
  private rather than feature-gated.

## Toolchain and distribution policy

- MSRV is Rust 1.92, equal to `rust-toolchain.toml`; every crate declares the
  inherited `rust-version`.
- The repository is internal-only: `UNLICENSED` and `publish = false` are
  intentional. A distributable release requires an explicit license decision,
  crate metadata review, and a deliberately reviewed release change; changing
  only `publish` is invalid.

## Host qualification boundary

Direct SDK use does not require a particular host integration. When a host
is adopted, qualify its actual dependency resolution and product path according
to [ADR 0004](docs/adr/0004-host-adoption-boundary.md).

## Audit behavior repairs

- Optional class removal now rejects any live ticket in that class, including
  zero-byte pending and committed tickets. Release the authorities before
  removing the class; legal cap changes remain available.
- A borrowing class with a zero hard cap remains a reclaim victim when it has
  positive resident usage. Recovery observes actual ticket release.
- Standalone scheduler Headroom release rejects a delta greater than its
  recorded Headroom usage before changing any counters.
- Prepared publication reserves count and metadata capacity for its eventual
  tombstone at fresh prepare. Defaults still bound 4,096 identities / 4 MiB,
  now including live prepared identities. Increase explicit limits if your
  previous live workload exceeded these defaults. Existing replay/conflict
  checks remain available at capacity; abort/GC no longer strand an entry
  merely because other admitted identities consume retirement capacity.
- The generic SpillTarget default restore rejects unavailable targets with
  Capacity; available discard targets still return None. Retaining targets
  must implement their own read availability and FIFO restoration.
