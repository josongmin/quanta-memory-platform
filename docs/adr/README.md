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

The [0.3 SDK RFC](../rfcs/0001-sdk-interface-contract.md) is an accepted design
target that is not implemented or qualified; it is not a shipped 0.2 decision. The
[verification scenarios](../verification-scenarios.md) are a living test
catalogue. Remaining work is in the [plan index](../plans/README.md).

Superseded audit packets, tickets, snapshot scores, and the duplicate detailed
guide were removed from the working documentation tree. The pre-cleanup
commit `25d1b84` retains their original evidence (`git show 25d1b84:<path>`);
these ADRs do not retroactively qualify a later source revision.

The completed Sep-26 fail-open prompts, finding tables and source census were
consolidated into ADR 0001/0002/0005 and removed from the active documentation.
Their original files are available at `37b369a`; the compact
[historical verification receipt](../bugbash/sep-26-fail-open-audit/remediation-verification.md)
and raw evidence remain. Completed work is not repeated in the plan index.

The recovered P08 hardening plan is also consolidated: grouped floors,
terminal diagnostics and rebuild ownership in ADR 0001; guarded caller
custody, root bounds, reclaim and ACK reconciliation in ADR 0002; shared
CI/extraction proof in ADR 0003; fixture lifetime in ADR 0005. Its last
working-tree copy is available at
`627b24c:docs/plans/bugbash-sep-26-engine/POST-RR-HARDENING.md`.
