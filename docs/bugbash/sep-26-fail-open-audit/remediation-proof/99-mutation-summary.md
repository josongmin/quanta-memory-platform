# Final mutation verification

19 non-no-op mutations; 19 baseline GREEN, 19 mutant RED, 19 restored GREEN. Every run selected and executed exactly one test. Compiler/selection failures are INVALID and are not counted as rejection. All85 Rust source hashes match the final shared source and the restored isolated candidate.

| Mutation | Baseline | Mutant | Restored |
| --- | --- | --- | --- |
| fake-reclaim-old-axis-subtraction | [GREEN](fake-reclaim-old-axis-subtraction-baseline.log) | [RED](fake-reclaim-old-axis-subtraction-mutant.log) | [GREEN](fake-reclaim-old-axis-subtraction-restored.log) |
| pin-entry-loss-with-debit-retained | [GREEN](pin-entry-loss-with-debit-retained-baseline.log) | [RED](pin-entry-loss-with-debit-retained-mutant.log) | [GREEN](pin-entry-loss-with-debit-retained-restored.log) |
| ledger-nested-owner-scan | [GREEN](ledger-nested-owner-scan-baseline.log) | [RED](ledger-nested-owner-scan-mutant.log) | [GREEN](ledger-nested-owner-scan-restored.log) |
| ledger-outer-owner-scan | [GREEN](ledger-outer-owner-scan-baseline.log) | [RED](ledger-outer-owner-scan-mutant.log) | [GREEN](ledger-outer-owner-scan-restored.log) |
| allocator-driven-pressure | [GREEN](allocator-driven-pressure-baseline.log) | [RED](allocator-driven-pressure-mutant.log) | [GREEN](allocator-driven-pressure-restored.log) |
| pressure-percent-wrapping | [GREEN](pressure-percent-wrapping-baseline.log) | [RED](pressure-percent-wrapping-mutant.log) | [GREEN](pressure-percent-wrapping-restored.log) |
| pressure-percent-saturating | [GREEN](pressure-percent-saturating-baseline.log) | [RED](pressure-percent-saturating-mutant.log) | [GREEN](pressure-percent-saturating-restored.log) |
| scheduler-recomputes-static-shares | [GREEN](scheduler-recomputes-static-shares-baseline.log) | [RED](scheduler-recomputes-static-shares-mutant.log) | [GREEN](scheduler-recomputes-static-shares-restored.log) |
| joint-class-group-floor | [GREEN](joint-class-group-floor-baseline.log) | [RED](joint-class-group-floor-mutant.log) | [GREEN](joint-class-group-floor-restored.log) |
| facade-payload-shared-process-seed | [GREEN](facade-payload-shared-process-seed-baseline.log) | [RED](facade-payload-shared-process-seed-mutant.log) | [GREEN](facade-payload-shared-process-seed-restored.log) |
| facade-InMemorySpillTarget-destructive-offline-accept | [GREEN](facade-InMemorySpillTarget-destructive-offline-accept-baseline.log) | [RED](facade-InMemorySpillTarget-destructive-offline-accept-mutant.log) | [GREEN](facade-InMemorySpillTarget-destructive-offline-accept-restored.log) |
| facade-RetainingSpillTarget-destructive-offline-accept | [GREEN](facade-RetainingSpillTarget-destructive-offline-accept-baseline.log) | [RED](facade-RetainingSpillTarget-destructive-offline-accept-mutant.log) | [GREEN](facade-RetainingSpillTarget-destructive-offline-accept-restored.log) |
| facade-child-stale-filter | [GREEN](facade-child-stale-filter-baseline.log) | [RED](facade-child-stale-filter-mutant.log) | [GREEN](facade-child-stale-filter-restored.log) |
| kit-constant-passed | [GREEN](kit-constant-passed-baseline.log) | [RED](kit-constant-passed-mutant.log) | [GREEN](kit-constant-passed-restored.log) |
| facade-suite-slices-removed | [GREEN](facade-suite-slices-removed-baseline.log) | [RED](facade-suite-slices-removed-mutant.log) | [GREEN](facade-suite-slices-removed-restored.log) |
| ledger-pre-aggregation-scan | [GREEN](ledger-pre-aggregation-scan-baseline.log) | [RED](ledger-pre-aggregation-scan-mutant.log) | [GREEN](ledger-pre-aggregation-scan-restored.log) |
| ledger-preflight-snapshot-copy | [GREEN](ledger-preflight-snapshot-copy-baseline.log) | [RED](ledger-preflight-snapshot-copy-mutant.log) | [GREEN](ledger-preflight-snapshot-copy-restored.log) |
| allocator-mutates-pressure-cache | [GREEN](allocator-mutates-pressure-cache-baseline.log) | [RED](allocator-mutates-pressure-cache-mutant.log) | [GREEN](allocator-mutates-pressure-cache-restored.log) |
| revocable-child-stale-filter | [GREEN](revocable-child-stale-filter-baseline.log) | [RED](revocable-child-stale-filter-mutant.log) | [GREEN](revocable-child-stale-filter-restored.log) |
