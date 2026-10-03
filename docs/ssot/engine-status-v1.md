# Memory-platform engine status

Authority: **code**. Audited 2026-10-03. Active product = **in-memory**
ledger/broker. `spill-redb` compiles as opt-in maintenance, not 0.2 product.

## Live broker

`LiveProcessMemoryBroker` (`feature = "live_control"`, default):
`reserve_owned` / `reserve_managed_charge` / `recharge` / `release`.
`ManagedCharge` is non-clone; Drop refunds. Ledger writes go through
`AccountingLedger::reserve` → `check_ledger_postimage`.

Admission axes that actually cap: global, class (+borrow pool), owner,
group hard/fair-burst, protected floors, execution-scratch, live ticket
count, rebuild/unknown-class blocks.

`Context` / `KindSlot` are overflow-guarded buckets, **not** policy caps.

Reclaim: `reserve_with_reclaim` drops the lock, runs `MemoryReclaimer`
(`catch_unwind`, `max_attempts: 1`), relocks, remeasures. `ReclaimRung` is
copied onto the request; **the broker does not walk the ladder**.

Facade stores take `StoreAccounting::Budgeted(LiveProcessMemoryBroker)`.
Not pass-through.

## Class labels

`MemoryClass` includes `MemoResidency`, `PtaResumeState`,
`PtaAuthorityState`, `PtaPortableBundle`, `IdeWorkingSet`. These are
ledger keys + preset weights. No Memo/PTA/IDE engine lives here.
Do not document the public enum as Semantica-noun-free.

## Features

- `scheduler` is an alias of `live_control`. Implementation is
  `application/scheduler.rs` under live control, not `cfg(feature = "scheduler")`.
- `spill-redb` / `full-backings`: real FIFO+recovery code, Cargo-marked
  not production-qualified, off `full`.
- `doctor`: thin report struct.
- `telemetry` off: no-op emitters.

## Host coupling

This repo does not depend on Semantica. The consumer's two Cargo workspace
roots and lockfiles determine whether Semantica resolves QMP by sibling path
or pinned Git revision. Check those files at the consumer commit; QMP source
alone cannot establish the active host binding.
