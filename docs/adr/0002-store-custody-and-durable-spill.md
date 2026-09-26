# ADR 0002: Store custody and optional durable spill

Status: Accepted for the 0.2 implementation; durable backing is opt-in.

## Decision

- Budgeted stores are built from a platform-bound builder. A retained value
  and its accounting authority move together across publish, replace,
  rollback, reclaim, close and Drop. Caller-owned return clones are outside
  the store's retained charge.
- A store mutation validates inputs and capacity before publication. Failure
  preserves both visible state and accounting. Read and mutation lifecycle
  boundaries contain supported unwind/reentry cases; a same-store callback
  that enters its active mutation lane returns `OperationInProgress`. The
  broker invokes reclaimers outside its own mutex and verifies actual refunds.
- Each caller-owned field remains individually guarded while caller Clone,
  comparison or measurement code can panic. Partial read clones use the same
  discipline. Transcript resolves owned-key map descents before publishing
  rows/indexes/counters; key destructors are contained during unwind.
- A payload reference must identify one canonical value; non-injective caller
  reference conversion is rejected before charging. Prepared admission is
  bounded by the complete GC-root input capacity, including unbudgeted mode.
- FIFO reclaim partitions selected entries and merges rollback in linear
  passes while preserving order and exact charge custody.
- Prepared-publication roots and other unbudgeted management inputs are
  bounded. The host supplies a complete, stable root set; QMP does not infer
  host activation or garbage-collection truth.
- Transcript and prepared-publication replay identities use domain-separated,
  length-framed canonical inputs and BLAKE3 tokens. Payload-store hash values
  are only lookup buckets: a per-store randomized seed reduces chosen-key
  collision attacks, and full canonical/reference equality decides identity.
  Neither token form is an authorization system.
- Durable spill is a custody transfer, not general persistence. Source memory
  may retire only after a known durable commit. Restore keeps durable custody
  until resident charge exists. An ambiguous write/finalize/delete outcome is
  fenced and reconciled by record identity, claim generation and disposition;
  two recoverers must not own one record concurrently.
- `spill-redb` remains an opt-in backing outside the active in-memory product
  scope. Its configured record, logical payload, file and restore-staging
  limits are not filesystem reservation or power-loss guarantees. A future
  durable consumer needs an explicit failure model and its own qualification.
- Physical power-loss qualification is not an open 0.2 defect or release
  prerequisite. It becomes required only when a named consumer enables this
  backing and owns a production guarantee for a fixed device/filesystem/mount
  topology. Until then, maintenance receipts must retain an explicit
  `power_loss_proven=false` boundary rather than creating an L3 backlog item.

## Durable maintenance qualification boundary

The optional backing has three deliberately separate evidence levels:

- Owner tests and fault injection prove record disposition, claim generation,
  single-owner recovery, exact custody transfer, bounded staging and fail-closed
  ambiguous outcomes.
- The L1 process-kill rail brackets the finalize commit with named pre-commit
  and post-commit points, kills without destructors, cold-reopens the same
  database and checks the public recovery state machine. It proves only the
  exercised process-termination boundaries.
- The opt-in Linux L2 rails inject `pwrite64`/`fdatasync` errors and a
  disposable ext4 device-mapper error-table swap. They check whole-state
  reopen outcomes and resource cleanup, but do not model loss of device power,
  volatile controller caches or torn physical writes.

L2 orchestration is fail-closed: the finalize-marker wait and complete run have
positive bounded timeouts; `strace` and its worker share a process group so a
timeout terminates the trace tree; invalid timeout input is rejected before
worker spawn; cleanup continues after evidence-write failures; and receipts
bind the result to the executable source digest. Historical or dirty-source
receipts do not qualify a later candidate; ADR 0003 owns that rule.

Physical L3 is activated only by a consumer decision that names all of the
following: production device and firmware class, filesystem and mount/barrier
configuration, external power-cut controller, recovery/fsck policy, cut-point
matrix, iteration policy, acceptance criteria and evidence owner. Until that
decision exists, L3 is a non-goal rather than residual implementation work.

## Why and consequences

Lifecycle ownership belongs in stores rather than scattered caller cleanup
paths. An I/O acknowledgement can be lost after the operation takes effect;
interpreting that uncertainty as success or failure without a durable proof
can lose or duplicate custody.

Containment covers the tested unwind contract, not `panic=abort`, malicious
nonterminating callbacks, arbitrary interior side effects, or physical device
failure. A same-store reclaimer may therefore receive a bounded Busy result;
the host must choose another victim or reject the request. No 0.3 callback
or pin/lease redesign is implied by this ADR. The owning implementations are
the facade [`lifecycle`](../../crates/quanta-memory-platform-facade/src/shared/lifecycle.rs),
[`payload_store`](../../crates/quanta-memory-platform-facade/src/payload_store/service.rs),
[`prepared_publication`](../../crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs),
and [`spill`](../../crates/quanta-memory-platform-facade/src/spill/mod.rs).
