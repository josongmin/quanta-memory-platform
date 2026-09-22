# Test Optimization Bugbash DEEP round (v2, sep-22)

Why v2: v1 (../sep-22-test-optimization/, 5 tracks, 107 findings) sampled per
track and named slow tests by inspection only. v2 closes both gaps.

## Design

- Phase A: measured timing — full suite with ` -- --report-time`, top-40 slowest
  table committed as `timing/top40.md` (+ raw log `timing/full.txt`).
- Phase B: 12 narrow tracks, each owning a disjoint file list. Every track
  produces a **census table**: one row per `#[test]`/`#[tokio::test]` fn in
  scope with verdict KEEP / SHRINK / MERGE / DELETE + reason. Sampling is a
  defect — critic rejects reports whose row count != grep count.
- Every finding: exact `file:line`, 1–3 line quoted oracle, failure mechanism
  (not vibe), concrete action with keeper `file:line`, est. saving.
- Every track re-verifies its v1 findings: CONFIRM / REFUTE / SUPERSEDE column.
- Phase C: 2 critics verify census counts + spot-check 5 findings each by
  opening bodies. Phase D: synthesis → `99-index.md`.

## Tracks (disjoint)

| # | Scope | Report |
|---|-------|--------|
| T01 | `core/src/domain/ledger.rs` (28) | `t01-ledger.md` |
| T02 | `core/src/domain/` rest (policy, config, context, stats, recharge, errors) + `core/tests/policy_contracts.rs` | `t02-domain-rest.md` |
| T03 | `core/src/api/control.rs` (76) | `t03-control.md` |
| T04 | `core/src/api/` rest (broker, builder) + `core/tests/public_api_surface.rs` | `t04-api-rest.md` |
| T05 | `core/src/application/` + `adapters/` + `ports/` test fns | `t05-app-adapters-ports.md` |
| T06 | `facade/src/payload_store/` + `retained_map/` | `t06-stores-a.md` |
| T07 | `facade/src/revocable_store/` + `transcript_store/` | `t07-stores-b.md` |
| T08 | `facade/src/publish_pipeline/` + `presets/` + `shared/` | `t08-pipeline-shared.md` |
| T09 | `facade/src/spill/` + `benches/scaling.rs` + `facade/tests/public_api_surface.rs` | `t09-spill-bench.md` |
| T10 | `testkit/src/conformance/` | `t10-conformance.md` |
| T11 | `testkit/src/` rest (faults, fixtures, alloc, fake, loom, doctor, lib) | `t11-testkit-rest.md` |
| T12 | `facade/tests/hellgate_e2e.rs` + `testkit/tests/` | `t12-e2e.md` |

No fix code — audit only.
