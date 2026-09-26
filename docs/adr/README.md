# Architecture decisions

These ADRs record the durable decisions behind the current 0.2 implementation.
They are contracts and scope boundaries, not test receipts or a claim that a
future SDK design has shipped.

| Decision | Scope |
| --- | --- |
| [0001 — One portable accounting authority](0001-portable-accounting-authority.md) | Package direction, semantic cost, admission and ticket lifetime |
| [0002 — Store custody and optional durable spill](0002-store-custody-and-durable-spill.md) | Store mutation, callbacks, reclaim, and ambiguous I/O |
| [0003 — Source-bound qualification](0003-source-bound-qualification.md) | Tests, benchmark claims, extraction, CI, and host proof |
| [0004 — Host adoption without duplicate authority](0004-host-adoption-boundary.md) | Semantica composition, execution limits, and memo retention |
| [0005 — Regression oracle acceptance](0005-regression-oracle-acceptance.md) | Fake provenance, behavioral mutations, dispatcher and subprocess proof |
| [0006 — Bugbash static regression guards](0006-bugbash-static-regression-guards.md) | Executed AST fixtures, owner-scoped prevention and runtime proof boundaries |

The [0.3 SDK RFC](../rfcs/0001-sdk-interface-contract.md) is an accepted design
target that is not implemented or qualified; it is not a shipped 0.2 decision. The
[verification scenarios](../verification-scenarios.md) are a living test
catalogue. Remaining work is in the [plan index](../plans/README.md).

## Historical material

Stale plans, audit snapshots, generic review checklists and raw historical
receipts are removed from the current documentation tree. Retained contracts
are consolidated here. Git preserves the original material:

| Material | Recovery source |
| --- | --- |
| Earlier implementation plans and audit packets | `25d1b84:<original path>` |
| Sep-26 completed findings, prompts and census | `37b369a:docs/bugbash/sep-26-fail-open-audit/<path>` |
| Sep-26 historical receipts, raw logs, hashes and generic checklist | `8df9d54:docs/<original path under docs>` |
| Recovered P08 hardening plan | `627b24c:docs/plans/bugbash-sep-26-engine/POST-RR-HARDENING.md` |

Use `git show <commit>:<path>` to recover a file. These historical receipts
qualify only their recorded source and failure model.
