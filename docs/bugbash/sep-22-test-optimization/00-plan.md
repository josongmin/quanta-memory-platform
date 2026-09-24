# Test Optimization Bugbash — Verified Work Plan (sep-22)

Snapshot audited: `main@101781c2ea6122602b4acd9aa69ebfc13fda958f`.

Historical Sep-22 execution plan. TO-01..10 were later implemented or
resolved; this is not a current work queue. See the
[current plan status](../../plans/README.md) and
[TO execution record](../../../tickets/TEST-OPTIMIZATION-EXECUTION.md).

At the audited snapshot, this directory was a remediation queue, not a census
of everything that could be made shorter. An item remained only when that source established an oracle or
isolation defect, or a warm test-binary measurement establishes material cost.

## Execution order

| Order | ID | Owner | Required result |
|---|---|---|---|
| 1 | TO-01 | facade spill/redb | Preserve concurrent durable-write semantics while removing the canonical all-features rail hotspot. |
| 2 | TO-02 | facade API contracts | Replace source/TOML/README string greps with compile-time or metadata-backed authorities. |
| 3 | TO-03 | facade scaling bench | Move benchmark setup outside the allocation epoch. |
| 4 | TO-04 | testkit faults | Make the broken spill fake exercise the real fault driver, not a local classifier. |
| 5 | TO-05 | core ledger | Reduce the 10k-owner setup while retaining the affected-slot bound assertion. |
| 6 | TO-06 | core policy | Make the zero-shared-pool test exercise reserve denial. |
| 7 | TO-07 | core builder | Cover reclaimer wiring and the public `build_live` alias through the builder-first surface. |
| 8 | TO-08 | core cleanup | Remove one exact duplicate telemetry case, one dead assertion, and a stale public comment. |
| 9 | TO-09 | transcript stats | Assert exact independently-derived byte partitions. |
| 10 | TO-10 | redb test isolation | Replace manual temp-file cleanup with process-unique RAII ownership. |

## Closeout rules

- Do not weaken fail-closed, race, crash-recovery, or independent accounting
  oracles to save time.
- Do not move tests to `#[ignore]` unless an enrolled CI selector is added in the
  same change.
- Do not derive expected accounting totals from the production calculator under
  test; retain independent formulas with named components.
- Re-measure warm exact tests after TO-01 and TO-05. The required proof is
  behaviorally identical assertions plus lower wall time, not test-count removal.
- Run `scripts/check-workspace.sh` after the concurrent tracked edits in this
  checkout are reconciled.
