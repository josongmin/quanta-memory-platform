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
