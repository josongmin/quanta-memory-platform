# QMP SDK work scope

Scope: portable 0.2 in-memory SDK. Use the public facade builder API directly;
Semantica adoption is optional. Completed contracts are in
[the ADR index](../adr/README.md).

The audited [remediation plan](audit-remediation.md) first closes confirmed
accounting, payload identity, prepared evidence and guard repairs, then qualifies
that source. The separately selected candidate extends the existing Transcript
port with handle/page/CAS/retire and reduces scheduler/spill/legacy public APIs.
Its contract matrix fixes the
existing IR, attribution, lease, token/root and error boundaries; it introduces
no parallel IR generation. The selected implementation and local qualification
are complete at the plan's recorded code candidate. Optional host adoption and
hosted CI execution remain separate. Add further work only for a reachable
defect, valid surviving mutation or explicit consumer requirement.

The final plan audit keeps local canonical/extraction evidence separate from
hosted CI operations. SLO measurement, broad file reorganization, host adoption
and durable product support remain conditional follow-ups. New parallel IRs,
automatic evidence expiry and store-specific prepared token salts are excluded.

| Trigger | Work to execute |
| --- | --- |
| Deliver or qualify a source revision | Canonical workspace gate and actual standalone extraction/public examples at the chosen clean commit; use [ADR 0003](../adr/0003-source-bound-qualification.md) |
| Claim hosted CI success | Verify that exact SHA's hosted job and extraction/example steps |
| Claim an SLO or performance regression | Relevant consumer workload, preselected threshold and compatible repeated baseline/candidate measurements |
| Start the 0.3 migration | Implement the [accepted RFC target](../rfcs/0001-sdk-interface-contract.md) with compatibility proof; it is not the shipped 0.2 API |
| Adopt a particular host | Apply [ADR 0004](../adr/0004-host-adoption-boundary.md) in that consumer repository |
| Support a durable product | Named consumer and owned storage failure model; current Redb code is outside active in-memory product support |

This index contains no current-HEAD test receipt. Historical passes do not
qualify subsequent changes; generic extra coverage is not an automatic queue.
