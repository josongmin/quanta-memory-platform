# Migration to 0.2.0

QMP 0.2.0 is a deliberate pre-1.0 breaking release. The workspace has not been
published to crates.io (`publish = false`, `UNLICENSED`), so there is no binary
or source compatibility window for the internal 0.1.x snapshots. Consumers
must migrate atomically and qualify their own host composition against the exact
QMP Git tree. No deprecated alias is retained when it would reopen an
unbudgeted or unbounded path.

## Sep-23 follow-up (`qmp-final-qualified-sep23-followup-v3`)

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

Host impact: none required — `quanta-memory-host` enables facade
`full` (without `spill-redb`) and calls no durable API.

### Redb storage schema v2 (automatic, fail-closed)

New tables/rows: a per-record disposition table, the single-owner
finalize-proof table, persisted backend identity and claim generations.
Opening a v1 file migrates it in the
same transaction: queued/recovery bytes are preserved and the
watermark record is kept. History that cannot be classified stays an
explicit error — the opener never deletes the file, reinterprets
opaque bytes, or guesses a commit outcome. There is no downgrade path
back to v1 layout; keep a file copy before first open if you need one.

### Redb resource limits and restore bytes (current candidate)

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
chronological history (see the field rustdoc). No reader outside core
exists; no caller migration needed.

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

## Required consumer changes

- Construct live authority through `MemoryPlatformBuilder`,
  `PortableMemoryPresets`, and platform-issued store builders. Raw ledger,
  broker, scratch allocator, ticket, and late `charged_against` constructors are
  not public.
- Treat `build_report_only()` as configuration data only. It cannot create a
  runtime view, scratch reservation, or budgeted store.
- Change payload reference types to implement `Eq + Hash`. Collision-safe
  canonical/reference digest buckets now retain two digest-index slots.
- Rebaseline semantic retained cost. The 32-byte payload conformance fixture is
  121 bytes in `RetainedCostV1`, not 89 bytes. This is a portable semantic-cost
  model, not allocator RSS.
- Replace `TranscriptRetentionPolicyV1` with bounded
  `TranscriptRetentionPolicyV2`. Saturation is a typed denial; no unbounded
  compatibility alias is provided.
- Use `LedgerSnapshot::checked_total_*` for externally assembled snapshots.
  Legacy infallible totals require an exact representable snapshot.
- Treat in-process revocable spill as an ownership transfer. Spill handles share
  one `Arc<StoreBindingHandle>` and carry the original managed charge; they do
  not deep-clone owner/runtime identity or create an uncharged destination copy.

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

Standalone GREEN does not qualify Semantica. The host must prove that both its
top-level and nested Cargo workspaces resolve the same committed QMP root/tree,
then run its registered memory-host, production-composition, execution-budget,
and owning-product suites on that exact source.
