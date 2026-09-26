# Sep-26 remediation — historical verification receipt

Implementation integrated in `4d59d72` through `c3be922`; selector ownership
reconciled in `87e2eaa`. The decisions are consolidated in
[ADR 0005](../../adr/0005-regression-oracle-acceptance.md), with production
accounting/store contracts in ADR 0001 and ADR 0002.

This receipt records the original frozen verification source, not a fresh
qualification of subsequent HEADs. The original full narrative and completed
audit packets remain in Git at `37b369a`.

| Evidence | Recorded result and source boundary |
| --- | --- |
| Canonical gate | PASS; [workspace log](remediation-proof/workspace-gate.log) ends with `check-workspace: clean (full repository gate)` |
| Mutations | 19 baseline GREEN / mutant RED / restored GREEN triples, exactly one test per run; [matrix and raw logs](remediation-proof/99-mutation-summary.md) |
| Source custody | 85 Rust files, aggregate SHA-256 `b25da136cd839eb7e07e0165f5e6b9fbaada1ea6dcc6dda719598893f1cb29c6`; [hash map](remediation-proof/source-sha256.json), [attestation](remediation-proof/shared-source-attestation.json) |
| Original source base | `61e9c7c47fa7294f9a926fd867e4af0b116c429b` plus the frozen dirty Rust source identified by those hashes |
| Extraction | Dry-run PASS on clean candidate `df881232c3a7f4c4014b4468a84e2078a17c5635`, tree `f5f5158e71244c1c8c7d667cf47d559a019dd59c`; [log](remediation-proof/extraction-dry-run.log). This did not run a destination build |

No hosted CI, installed host, release activation, performance SLO or physical
power-loss result is claimed. Re-run affected delivery gates at the selected
source revision according to [ADR 0003](../../adr/0003-source-bound-qualification.md).
