# Remaining work

This index lists work that is still conditional or unqualified. Completed
implementation tickets and dated audit snapshots are not a dispatch queue;
their source is available in Git history. Architectural decisions are in
[the ADR index](../adr/README.md).

| Area | Remaining gate | Status |
| --- | --- | --- |
| Semantica adoption | [SMA-009 product qualification](sep-24-semantica-memory-adoption/SMA-009-product-qualification.md): exact resolved QMP/Semantica pair, owner/public tests, SDK query and SQL worker product paths | Open in the last recorded host audit; recheck current host source before dispatch |
| Performance claim | Measure the actual group/recharge/reclaim, transcript, retained-map concurrency and prepared-root paths on a quiet host; set consumer SLOs and run compatible repeated baseline/candidate receipts | Conditional; no universal “SOTA” claim |
| 0.3 SDK | [RFC 0001](../rfcs/0001-sdk-interface-contract.md) migration and its own compatibility/acceptance proof | Proposed; not part of the 0.2 completion claim |
| Durable product | Opt-in Redb device/power-loss and host failure-model qualification, only if a durable consumer is adopted | Out of the active in-memory product scope |

For this repository, a current local release claim requires a clean committed
source, `scripts/check-workspace.sh`, real standalone extraction with its
public examples, and the benchmark/profile receipt appropriate to the claim.
Use [ADR 0003](../adr/0003-source-bound-qualification.md) for the evidence
boundary. No historical tag or this index qualifies the present HEAD.

Recovered historical notes: [P08 post-review hardening](bugbash-sep-26-engine/POST-RR-HARDENING.md).
Its original local changes are committed and reconciled into main; the note
is historical evidence, not an additional remaining-work queue.
