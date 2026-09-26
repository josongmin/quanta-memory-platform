# ADR 0004: Host adoption without duplicate authority

Status: Accepted integration direction; Semantica product qualification is
tracked separately and is not asserted here.

## Decision

- Semantica owns product defaults, owner identities, input lowering and host
  lifecycle. QMP validates and accounts the installed residency policy.
  A live combined report uses one broker `observe()` projection; independent
  samples must not be presented as an atomic snapshot.
- Execution-materialization permits and QMP residency charges are separate
  scopes. Reuse the existing execution root/permit authority; do not add a
  second execution counter to QMP or count one allocation as two disjoint
  physical allocations. If an operation requires both permissions, acquire
  both before publication and roll back the first if the second denies.
- Canonical memo rows retain only memo-owned semantic metadata and indexes;
  the artifact owner retains artifact payload charge. A host-neutral,
  move-only lease ties each row and rollback receipt to the installed QMP
  broker. Replacement may keep old and new rows charged concurrently until
  rollback custody ends. Admission and release callbacks do not run under a
  memo-table lock.
- The Semantica dependency revision must agree across both workspace roots
  and the resolved Cargo graph. A validation environment variable cannot
  substitute for that resolution. SDK and SQL worker processes/domains are
  qualified separately; a process-local cap is not an aggregate RSS cap.

## Why and consequences

The host already owns product and execution semantics. Duplicating their
authorities in QMP would create conflicting limits and refund rules. A row's
receipt or prepared token can outlive the visible table row, so its lease
must follow actual custody, not merely lookup visibility.

The 0.3 builder/DSL in [RFC 0001](../rfcs/0001-sdk-interface-contract.md)
remains proposed. Current Semantica verification obligations are kept in the
[active product gate](../plans/sep-24-semantica-memory-adoption/SMA-009-product-qualification.md).
