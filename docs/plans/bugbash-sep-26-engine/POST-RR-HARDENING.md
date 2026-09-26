# Post-review hardening over QMP `f5e20c5`

Status: implementation candidate in the local working tree. This document is
not a clean-HEAD, hosted-CI, Semantica-host, or performance qualification
receipt. The full workspace rail and actual standalone extraction must be
recorded at the final frozen source before a release claim.

The follow-up audit found no new demonstrated P0/P1 runtime corruption. It
found one release-blocking extraction contract gap and several P2/P3
correctness/diagnostic/test gaps. The owner-level repair boundaries are:

| Boundary | Repair | Required independent oracle |
| --- | --- | --- |
| Core grouped admission | Track per-group total and non-Headroom usage separately; preserve the exact ticket class/owner/group attribution across reserve, recharge, release, batch release, and policy reconfigure. | A Headroom 100 + peer non-Headroom 800 request must not consume another group's reserved 100; peer 700 and protected owner's own 100 are admitted. Reconfigure and release preserve the same floor. |
| Core terminal refusal | A zero-usage probe decides whether reclaim is useful; its diagnostic does not replace the actual live attempt's first breach. Permanent ticket-ID exhaustion blocks before any reclaimable byte breach. | Public reserve, locked direct attempt, and preflight agree on terminal error; exhaustion stays `InvalidState` even for an over-cap request. |
| Rebuild authority | Mark both fence and cleanup receipt `must_use`; a competing fence leaves deferred tickets untouched and reports the fence error. | Warnings-denied compile-fail docs, preserved queue/ledger test, subsequent drain. |
| Facade client-trait unwind | Guard each in-flight owned field, including revocable measurement input and partial retained-map/transcript read clones. | Isolated child tests combine later Clone/ByteSized panic with earlier Drop panic, then verify usable store and exact cleanup. |
| Payload and prepared roots | Reject a non-injective public payload reference before charging; cap unbudgeted prepared admission to the complete GC-root input bound. | Collision keeps the first reference reachable and counters unchanged; 16,384 distinct roots fit, the next is rejected, and GC can free a slot. |
| CI/extraction | Copy both CI configs into the self-hosted repo; call one executable extraction/example script from both providers using the extracted `Cargo.toml`. | Negative CI config fixtures reject schedule/main-filter drift, disabled steps, early shell close, and source-checkout examples; actual extraction remains a separate clean-commit gate. |
| Redb ACK reconciliation | Decide an ambiguous finalize from the exact record's generation proof, disposition, and retained payload state; accept an abort only when its own claim is no longer active. | Inject lost finalize and abort ACKs, allow a new claim before the probe, then assert the prior claim's exact outcome and the new claim's authority. |
| Revocable reclaim | Partition selected FIFO entries and merge rollback entries in linear passes instead of repeated indexed `VecDeque` shifts. | Oldest-half and sparse 16,384-entry round trips preserve order; accounting rejection still restores exact charges and counters. |
| Test fixture lifetime | Use `Weak` for a reclaimer's victim store. | Store deallocates after the test's last owning `Arc` drops; ledger zero alone is insufficient. |

Scope limits: the in-memory 0.2 contract remains separate from RFC-0001's 0.3
taxonomy/SDK migration. The revocable algorithmic fix is not a measured STR-06
throughput qualification. The Redb ACK tests are owner-local fault injections,
not a device-failure or hosted durability qualification. Any independently
changing worktree file must be identified before committing or extracting.
