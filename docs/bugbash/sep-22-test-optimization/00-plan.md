# Test Optimization Bugbash — Plan (sep-22)

Target dir: `docs/bugbash/sep-22-test-optimization/`
Total test surface: ~400 `#[test]`/`#[tokio::test]` sites across core / facade / testkit.

## Parallel tracks (5, no overlap)

| # | Track | Scope | Report |
|---|-------|-------|--------|
| 1 | core-domain | `core/src/domain/` (ledger, policy, config, recharge, stats, context) + `core/tests/policy_contracts.rs` | `01-core-domain.md` |
| 2 | core-api-application | `core/src/api/` (broker, builder, control) + `core/src/application/` (controller, scheduler) + `core/src/adapters/` + `core/src/ports/` + `core/tests/public_api_surface.rs` | `02-core-api-application.md` |
| 3 | facade-stores | `facade/src/payload_store/`, `retained_map/`, `revocable_store/`, `transcript_store/` | `03-facade-stores.md` |
| 4 | facade-pipeline-spill-shared | `facade/src/publish_pipeline/`, `spill/` (+`backings/`), `shared/`, `presets/`, `benches/scaling.rs`, `facade/tests/public_api_surface.rs` | `04-facade-pipeline-spill-shared.md` |
| 5 | testkit-integration | `testkit/src/` (conformance, faults, fixtures, alloc, fake, loom, doctor) + `testkit/tests/`, `facade/tests/hellgate_e2e.rs` | `05-testkit-integration.md` |

Synthesis: `99-index.md` (dedup cross-track duplicates, ranked optimization list).

## Audit lenses (every track, every finding needs file:line evidence)

- A. 짜치는 코딩: copy-paste setups, magic numbers, over-mocking, tests testing mocks, dead helpers.
- B. 짜치는 테스트: no assertion / trivial assertion, `assert!(true)`, only-`is_ok()` where value matters, ignored tests.
- C. 비효율 계산 / 느린 테스트: oversized loops, redundant setup per case, sleep/time-based flakiness (`faults/scenarios.rs`, `faults/mod.rs` are the only sleep sites — check them), serializable-parallel cases, heavy backing (redb) where memory fake suffices.
- D. 검증 애매 (low quality): weak oracles, missing fail-closed checks (ledger invariants), missing negative cases, nondeterministic assertions.
- E. 중복 테스트: same behavior covered in unit + conformance + e2e; near-identical cases differing only by constant. Name the canonical keeper and the deletion candidates, with per-file:line pairs.

## Report contract per file

Each report: summary counts, table of findings (severity S/M/L, lens A–E, file:line, problem, recommended action, est. saving), duplicate clusters with keeper pick, top-5 quick wins. No fix code — audit only.
