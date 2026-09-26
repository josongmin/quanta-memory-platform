# QMP SDK work plan

Updated: 2026-09-26. Source reviewed for this update: `87e2eaa`.
Scope: the portable 0.2 in-memory SDK. Consumers can use the public facade
builder API directly; Semantica adoption is not a prerequisite.

## Remaining SDK work

| Area | Current status | Next action |
| --- | --- | --- |
| Exact-source SDK delivery verification | The latest audit records a full gate and 19 baseline/mutant/restored triples for its frozen source; extraction proof there is dry-run only | At the chosen delivery revision, run the canonical gate and actual standalone extraction/public examples; the older `25d1b84` extraction does not qualify later changes |

Completed implementation and audit contracts are consolidated in
[the ADR index](../adr/README.md); original task packets remain in Git history.
No additional SDK implementation ticket is established by this plan update.
Verification at a new delivery revision is source refresh work, not a new
runtime defect.

## Claim-specific verification

| Claim | When required | Required evidence |
| --- | --- | --- |
| Hosted CI passed | When reporting hosted CI success | Exact SHA, live trigger binding, successful job and extraction/example steps; local gates suffice for a local verification claim |
| Performance regression or SLO | When a consumer sets a workload and threshold, or a performance claim is requested | Relevant workload only, compatible repeated baseline/candidate receipts on a quiet host; no universal SOTA or all-path benchmark requirement |

## Separate work, excluded from the active 0.2 SDK queue

| Area | Disposition | Re-entry condition |
| --- | --- | --- |
| Semantica adoption | Host-owned integration; [SMA-009](sep-24-semantica-memory-adoption/SMA-009-product-qualification.md) is reference only and does not block SDK use or QMP completion | Explicit Semantica integration request; recheck its current source before dispatch |
| 0.3 SDK migration | [RFC 0001](../rfcs/0001-sdk-interface-contract.md) is an accepted design target, not implemented or qualified; current 0.2 API remains usable | Explicit versioned migration scope and compatibility plan |
| Redb durable product / device or power-loss qualification | Outside the active in-memory scope; existing opt-in code and tests do not establish a supported durable product | Named consumer, owned failure model and measured reuse requirement |
| Generic extra audit/coverage | No automatic backlog from WEAK labels, missing hypothetical cases or benchmark checklists | Reachable source-backed defect, valid surviving mutation or an explicit consumer contract |

Use [ADR 0003](../adr/0003-source-bound-qualification.md) for the evidence
boundary. This index records the source and limits of existing receipts; it
does not assert that the latest HEAD has newly passed every delivery gate.

Its original local changes are committed and reconciled into main; the note
is historical evidence, not an additional remaining-work queue.
