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

The [0.3 SDK RFC](../rfcs/0001-sdk-interface-contract.md) is a proposed
interface, not an accepted 0.2 implementation decision. The
[verification scenarios](../verification-scenarios.md) are a living test
catalogue. Remaining work is in the [plan index](../plans/README.md).

Superseded audit packets, tickets, snapshot scores, and the duplicate detailed
guide were removed from the working documentation tree. The pre-cleanup
commit `25d1b84` retains their original evidence (`git show 25d1b84:<path>`);
these ADRs do not retroactively qualify a later source revision.
