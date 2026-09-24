# Host adoption interface and SSOT contract

2026-09-24. Status: DESIGN REQUIREMENTS; candidate implementation exists, final product proof pending.
Applies to SMA-001~009. Source evidence: [final audit](FINAL-AUDIT.md).
This is a constraints/reference document, not an implementation backlog. The active
scope is owned by [README](README.md). Live default reporting and single-observation
stats are already present at Semantica `f5e8ff63642`; preserve, do not reimplement them.
Memo retention was adopted as [active SMA-003](SMA-003-canonical-memo-accounting.md);
reclaim remains optional. The original memo design is [archived](archive/README.md). Active SMA-002 is a
removal-only cutover, so the dual-authority extension below is not an implementation
step or verification prerequisite. Existing candidate changes are reviewed, not rebuilt.

## Authority and precedence

This packet specifies host adoption, not a competing QMP SDK or storage framework.
Existing repository architecture, Semantica dependency SSOT and
[QMP RFC-0001](../../rfcs/0001-sdk-interface-contract.md) retain their authority.
The RFC's unimplemented 0.3 surface is not assumed available. Use existing primitives
unless a named consumer proves a missing capability.

| Concern | Existing authority to reuse | Forbidden duplicate |
| --- | --- | --- |
| Product defaults | Semantica SDK/composition policy | Replacing intentional product shares with generic QMP preset values |
| Input lowering | Composer `manifest_from_defaults_v1` | Report-only resolver with independently inferred effective caps |
| Installed residency policy/accounting | QMP platform resolver, ledger and live report | Host admission ledger or parallel mutable usage counters |
| Coherent observation | QMP `observe()` / `BrokerObservation` | Independent ledger/pressure/report samples packaged as one atomic snapshot |
| Owner identity | Host public `runtime::owner_keys` | Composer copy or a second owner registry |
| Execution materialization | `ExecutionMaterializationBudgetV1` and existing permits | New root counter from a cloned policy in each leaf |
| Memo publication | Canonical table, reservation, receipt and generation protocol | Serialized FIFO mirror, second publication coordinator |
| Semantic sizing | Existing QMP cost components plus one memo-specific checked formula | Serialized payload copy solely for sizing; allocator/RSS called semantic cost |
| Build dependency | Semantica dependency contract and both workspace manifests | One-root-only pin, source shadow, environment variable claimed as Cargo override |
| Qualification | Existing owner tests, FIN-20 and QBC catalog | Duplicate suite/catalog or historical receipt promoted to current proof |

## A. Construction and observation

1. SDK/ingress validates input syntax and units; composition lowers product policy once;
   QMP validates/resolves the installed policy. Preserve checked MiB conversion, class
   shares, headroom and overflow tests. In particular, the SDK artifact-share default is
   product policy, not a competing ledger.
2. A checked report uses the live inherent `effective_report()` or projects `observe()`.
   Combined report/stats/pressure DTOs project **one existing `observe()`**. Do not
   combine `ledger_snapshot()` and a separately sampled pressure into a coherent claim.
   Independent API calls may legitimately see different revisions; no cross-call
   transaction or retry coordinator is required.
3. Preserve checked error propagation. The trait's infallible report fallback is not a
   substitute for the checked SDK path. Offline config previews are explicitly previews.
4. Reuse/reexport public host owner constants after caller census. Keep one host
   class-label mapping; do not add Semantica classes to the portable QMP public API.
5. A platform defines one budget domain. Builders may create independent domains;
   sharing is explicit at the composition root, not a global singleton. Reports must
   state the domain and metric rather than imply process-wide physical memory coverage.

## B. Execution versus retained-memory authority

The execution rail already accounts materialization. Absence of QMP execution calls
does **not** mean Semantica has no output/scratch enforcement.

- Reuse `ExecutionLimitV1::{Bounded, UnboundedByPolicy}`. Raw optional input can mean
  inheritance before resolution; the resolved limit is explicit. `Bounded(0)` rejects
  positive demand. Do not introduce a competing generic `Unset/Disabled/Limit` enum.
- Reuse one root `ExecutionMaterializationBudgetV1`; its clones share the counter.
  `ExecutionBudgetHandleV1` clones policy, not accounting authority. Leaf adapters receive
  the existing object-safe admission port and opaque permit, never mint another root.
- Execution-local output/materialization limits and QMP domain-wide residency limits
  are distinct policy scopes. Both may legitimately constrain one allocation; do not
  sum their charges as if they were two disjoint allocations or accidentally widen one.
- If both are wired, acquire both authorities before allocation/publication. If the
  second fails, release the first. Recheck publication generation before commit; no
  partially published output. This is an owner rollback protocol, not a new atomic
  transaction spanning two independent ledgers.
- Canonical codec logical bytes, QMP semantic retained cost and RSS are distinct units.
  Preserve existing typed unknown-demand/overflow/exhaustion failures. Zero canonical
  byte permits are not proof of QMP zero-byte ticket lifetime.
- An execution-root permit cannot own a memo row surviving that execution. Remove
  unused public output/scratch fields by default; retain only named, tested product paths.

## C. Minimal canonical memo retention seam

The canonical in-memory adapter currently has no QMP dependency. Keep its port
host-neutral; the host adapter must not depend back on the in-memory/runtime adapter.
No existing execution-scoped port is silently repurposed for persistent retention.

Proposed interface shape (names/signatures finalized with the owning contract crate):

- Composition binds one retention account to domain/owner/class/kind once.
- An object-safe admission method takes a checked semantic-byte demand and returns
  an opaque non-Clone lease or typed failure. No per-operation raw broker/group/owner
  arguments, no public generic `Retained<T>` wrapper with mutation escape paths.
- The host implementation holds the existing QMP managed charge behind the lease.
  Do not implement a second pending/deferred/refund queue. Zero bytes still obtain a
  live authority; a fake admission provider is a trusted SPI, tested for conformance,
  not claimed to be impossible to forge.
- Known cost uses existing reserve/commit. A new measured-commit primitive requires
  a demonstrated caller need and an explicit QMP owner ticket, not speculative API growth.
- Map errors once at the host boundary. Preserve category/source and mandatory failure
  semantics; do not parse `Capacity(String)` for fabricated axis/deficit details.
- Keep serializable `MemoEntry` as data. Internal immutable charged rows may use shared
  custody; the public DTO does not acquire QMP imports or a serializable lease.

### Cost and custody

`MemoEntry` holds CAS references and metadata, not the CAS payload. Charge memo-owned
metadata, key/index copies and publication state; the artifact owner retains its payload
charge. Store the checked cost/formula version with the admitted row; release that exact
amount, not a later recomputation. Reuse QMP cost components without duplicating generic
overhead constants. State whether the bound is retained-only or includes construction.

`MemoPublicationReservationV1` is a publication/generation authority, **not** a QMP
pending memory reservation. Preserve its cancellation and table identity checks.
The existing cloneable insert receipt carries the previous row: replacement does not
end that row's lifetime. Default design is shared immutable rollback custody across
receipt clones, within the existing table/receipt protocol, not a side registry.
The current candidate now shares variable-size `MemoEntry` data in receipt/prepared
tokens; [SMA-003](SMA-003-canonical-memo-accounting.md) keeps its QBC and product
proof gates open until that behavior is verified on the final source.

| Transition | Required ownership/postimage |
| --- | --- |
| Prepare/admit | Candidate cost admitted before publish; old visible row unchanged on denial |
| Replace | New row and old rollback state are both covered while alive |
| Clone receipt | Share immutable rollback state and charge; no hidden deep unmetered copy |
| Retract current receipt | Restore retained previous state without unplanned admission under an exhausted budget |
| Stale/foreign/repeated retract | Preserve successor; no duplicate refund or cross-table restoration |
| GC with live receipt | Detach visible state only; receipt-held rollback custody stays charged |
| Final custody drop | Destroy retained data before refund; drain existing deferred authority when required |

Reserve needed rollback metadata/capacity before publishing the replacement. If current
receipt/API shape cannot support this, change it once across producer, decorator, mocks
and consumers. Do not maintain live v1/v2 publication paths with different semantics.
Returned deep DTO clones are not automatically covered by row custody: either preserve
shared charged representation at the owning boundary or narrow the claim to table-owned
retention and separately identify caller-owned allocations. A total-retained claim must
prove the final owner lifetime; renaming the metric is not that proof.

### Lock and destruction boundary

Admission/reclaim and **release/Drop** may cross into callbacks. Reserve outside the
memo lock, recheck generation under it, detach removed/failed candidates under lock,
then drop/refund after guards are gone. Cover error and unwind ordering, not just success.
Bound retries on generation drift; do not hold a memo/registry lock while entering QMP.
Rebuild-fenced deferred overcharge is explicit and bounded; eventual drain returns to
the expected baseline, not necessarily immediately on failure.

Reclaim callbacks use weak cache handles where needed to avoid
platform → callback → cache → charge → platform strong-reference cycles. Snapshot
eligible handles and unlock the registry before invoking owners. Event sinks likewise
must not accidentally retain the platform indefinitely. Owner eviction, not a reclaimer
side ledger, remains authoritative; QMP remeasures actual returned charges.

## D. Breaking-change handoff and verification

| Change | Single owner / coordinated consumers | Required proof |
| --- | --- | --- |
| Remove handle budget overlay | SDK/ingress → selector/session/kernel mocks, exports, examples | Caller census + real public compile test; defaults remain checked |
| Live observation + owner constants | Composer → SDK report DTO consumers | Single observation projection, reconfigure, typed failure; FIN-20 regression reuse |
| Output/scratch contract | Existing execution rail → ingress/composer/actual consumers | Existing root identity, zero/boundary/unknown demand; paired permit rollback if adopted |
| Memo retention and receipt custody | Contract ports → canonical table + host implementation + decorators/mocks | Replace/retract/clone/GC/cancel lifecycle and no callback under lock |
| Dependency update, only if needed | Both workspace manifests → workspace-inherited consumer dependencies | Actual resolved source/features, compatible host-pair proof |

Semantica authority: `S/docs/ssot/quanta-memory-platform-dependency-contract-v1.md`.
`MEMORY_PLATFORM_REPO` selects a verification target; it does not change Cargo resolution.
Reuse `S/docs/plans/sep-21-analysis-integrity-closure-v2/tickets/FIN-20-memory-checked-stats-cross-repo-qualification/TICKET.md`
for checked StoreStats/error and paired-source proof. Historical status/receipts must be
verified against current source/features. One integrator owns contract ports, composer,
exports, both manifests and test registry handoffs. No simultaneous incompatible schemas.

Owner-local tests carry the exhaustive matrix; integration tests prove actual composition
and only the cross-owner transitions. Feature-gated memo tests must execute nonzero cases
with their required feature (including `canonical-cbor` where applicable). All-features
builds do not prove supported minimal/production feature combinations.

## Design references, not implementation evidence

- [Rust API type safety](https://rust-lang.github.io/api-guidelines/type-safety.html):
  explicit units and states; reuse existing types before adding new ones.
- [Rust destructor semantics](https://doc.rust-lang.org/reference/destructors.html):
  verify data/guard/lease destruction order instead of assuming it from variable names.
- [rustc incremental persistence](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html):
  reuse semantic identity authority; process-local identifiers are not disk keys.

These are design constraints, not a measured superiority or “SOTA” certification.
