# ADR 0001: One portable accounting authority

Status: Accepted for the 0.2 implementation.

## Decision

- Keep the dependency graph `core <- facade <- testkit`. The host adapter lives
  outside this repository. `domain/` and `ports/` remain independent of host
  names, filesystem I/O, and wall-clock reads.
- A `LiveProcessMemoryBroker` owns one semantic ledger for a budget domain.
  Facade stores and attached runtime scratch use that broker; no store or host
  shadow ledger may grant or refund the same residency independently.
- Semantic bytes follow a versioned cost model. Retained payload, retained
  metadata, and live scratch are **lifetime buckets**, not physical-memory
  measurements or a per-field byte breakdown. Class, owner, context, kind,
  group and total views must reconcile to the same live tickets.
- Every charge has one non-clone release authority, including zero-byte
  tickets. Reserve, commit, recharge, batch release, rebuild and reconfigure
  must either move the whole postimage or leave the old state intact. Policy
  changes must use live ticket attribution, not independently reconstructed
  marginal totals.
- Admission checks all applicable hard axes and protected group headroom
  before granting. A reclaim callback's reported bytes are advisory; the
  broker remeasures its ledger before retry. Permanent errors such as ticket
  identity exhaustion do not trigger reclaim. Zero-usage feasibility probes
  decide whether reclaim can help; they do not replace the live attempt's
  first-breach diagnostic.
- Protected group floors track non-Headroom usage independently from total
  usage. Class reservations and group reservations are separate constraints;
  they are not added into one joint reservation floor. Release and live
  reconfiguration preserve each ticket's original group attribution.
- Pressure and its cached classification follow semantic ledger usage.
  Allocator observations are advisory and cannot change admission or the
  semantic pressure cache. Percentage arithmetic uses overflow-safe widening.
- Rebuild exclusion has one move-only epoch owner. Competing owners cannot
  finish the fence or drain deferred cleanup; exhausted epochs fail before
  changing fence state. Fence and cleanup receipts require explicit handling.
- Live-ticket and other management state require explicit count/input bounds.
  Such bounds do not imply a bound on allocator overhead, temporary caller
  copies, RSS, or arbitrary host callbacks.
- Weighted shared-burst shares use largest-remainder (Hamilton)
  apportionment, with canonical group-name tie-breaking. Shares are computed
  per policy snapshot and checked together with live caps and protected
  floors. This is memory admission, not a throughput-fair work scheduler.
- `RetainedCostV1` checks the sum of declared payload, identity, index,
  tombstone and fixed-overhead components. A store captures the charge at
  admission and refunds that amount rather than remeasuring mutable data.
  Caller-defined `ByteSized` values must conservatively cover their retained
  lifetime or be explicitly replaced/recharged when their cost changes.

## Why and consequences

One authority makes refund, denial and reconfiguration independently testable.
An owner or store cannot manufacture capacity by updating only one projection.
It also means callers must share a broker explicitly when they intend to share
a domain; two builders can create two independent domains.

The process-local library is not an OS memory limit, tenant security sandbox,
or guarantee that the host can reclaim a minimum working set. Host-owned
identity, workload validity and process-level memory policy remain outside
this authority. The implementation authorities are
[`ledger.rs`](../../crates/quanta-memory-platform-core/src/domain/ledger.rs),
[`scheduler.rs`](../../crates/quanta-memory-platform-core/src/application/scheduler.rs),
and [`measure.rs`](../../crates/quanta-memory-platform-facade/src/shared/measure.rs).
