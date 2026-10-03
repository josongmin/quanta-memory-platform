## Read First — Non-Negotiables

This repo is `quanta-memory-platform`: a **portable** memory
budgeting/accounting library designed for mechanical extraction. The Semantica
host adapter lives outside this repository. `MemoryClass` labels may still
look host-named; they are ledger keys, not host algorithms.

Current product scope is in-memory accounting and reclaim. Optional durable
spill/Redb code is maintenance scope, not an active release prerequisite;
consult `README.md` before proposing capability or SDK work.

Hard rules:

- **Dependency direction is strict and acyclic:** `core <- facade <- testkit`.
  The host depends on this repo, never the reverse.
- **`domain/` and `ports/` are pure:** no `std::fs`, `std::env`, `SystemTime::now`,
  `Instant::now`, and no `crate::adapters` import. I/O and clocks live behind
  ports/adapters.
- **Public API stays builder-first:** no host-crate imports and no public
  raw-broker construction. Do not claim the `MemoryClass` enum is
  Semantica-noun-free.
- **Ledger is fail-closed:** every mutating accounting op fully succeeds or
  leaves it untouched. Allocator telemetry is advisory. Conformance must reject
  a broken fake, not only accept honest implementations.
- **Generated guidance:** edit `tools/pm/sources/**` or `tools/pm/targets.json`,
  then run `python3 tools/pm/pm.py sync`. Do not hand-edit generated outputs.

## Read Only What the Task Needs

Do not preload the full documentation stack. Open the relevant authority when
the task enters that area:

| Task | Read |
| --- | --- |
| Crate edges, layers, public surface | `tools/pm/sources/rules/architecture.md` |
| Ledger, policy, reclaim, snapshots | `tools/pm/sources/rules/determinism.md` and the owning source |
| Tests or conformance | `tools/pm/sources/rules/tests.md` |
| Source qualification or historical receipts | `docs/adr/0003-source-bound-qualification.md` |
| Fallible Rust paths | `tools/pm/sources/rules/error-handling.md` |
| Cargo deps or features | `tools/pm/sources/rules/dependencies.md` |
| Behavioral Rust changes or known regression review | `tools/pm/sources/rules/cheatsheet.md` |
| Workspace gate or extraction | `tools/pm/sources/rules/build.md` |
| Repository overview | `README.md`; extraction contract: `EXTRACTION.md` |
