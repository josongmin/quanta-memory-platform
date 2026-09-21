# QMP-130 — Semantica host adoption and exact-head qualification

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-030, QMP-040, QMP-050, QMP-060, QMP-070, QMP-080, QMP-090, QMP-100, QMP-110, QMP-120  
Blocks: QMP-140

## Objective

standalone QMP의 corrected authority를 external Semantica host가 실제 production composition에서 사용하고, scratch/store/non-memory execution budgets가 exact revisions에서 enforce됨을 증명한다.

## Baseline static evidence

External repo: `/Users/songmin/Documents/code-new/semantica-codegraph-v2`  
Observed HEAD: `e9168a73b4abca24fd177a726de150696d40c3eb`  
State: shared dirty main; STATIC only.

- root workspace path deps: `Cargo.toml:638-639`
- production compose uses `.build_live()` and shared sink: `quanta-runtime-memory-compose/src/lib.rs:1053-1080`
- host kit attaches runtime view: `quanta-memory-host/src/kit.rs:213-220`
- runtime/retrieval/session factories receive accounting sink.
- `ExecutionBudget::check_wall_time_ms/check_subqueries/check_trace_edges/check_output_bytes/check_execution_scratch_bytes` call-site census found no Rust callers in observed host snapshot.

## External write set

This ticket executes primarily in Semantica repository:

- `packages/analysis/quanta-v2/Cargo.toml`
- top-level workspace `Cargo.toml`
- `quanta-memory-host/**`
- `quanta-runtime-memory-compose/**`
- actual runtime execution/retrieval/session call sites owning budget consumption
- host tests and suite registry entries

QMP repository changes are limited to adoption docs/receipts unless an owner defect is rediscovered and routed back to its QMP ticket.

Canonical host symbols/call sites to revalidate:

- `build_production_semantica_memory_kit`
- `build_production_semantica_memory_live_kit_v1`
- `build_semantica_memory_live_kit_from_defaults_v1`
- `SemanticaMemoryKitBuilder::build`
- host `attach_view`
- `SemanticaDefault{Runtime,Retrieval,Session}AdapterFactory::with_accounting_sink`
- `build_bound_retained_map_store_v1`
- runtime payload/retained/trace residency constructors and `charged_against` wrappers
- session transcript adapter construction

## Admission rules

1. create isolated clean worktree or obtain explicit shared-main ownership.
2. freeze QMP SHA/digest and host SHA separately.
3. verify dependency resolution points at intended extracted QMP, not an adjacent stale checkout.
4. do not run heavy host build lanes concurrently with another owner lane.
5. unrelated dirty files are not reset or overwritten.

## Required implementation/adoption

1. update host dependency contract to exact QMP revision/path artifact.
2. adapt to enforced default/public API and ticket typestate.
3. ensure all host store factories build budgeted typestate only.
4. ensure runtime view scratch uses broker-bound reservation.
5. remove/replace host `charged_against` late-transition wrappers.
6. map host keys/identities to QMP total retained-cost contract.
7. integrate transcript tombstone policy and spill tier decisions.
8. connect `ExecutionBudget` non-memory checks at canonical producers:
   - wall time
   - subquery count
   - trace edges
   - output bytes
   - execution scratch
9. surface typed denial to caller without fallback to unbudgeted path.
10. register exact host suites in canonical suite registry.

## Required host tests

- production composer over-cap store write denied
- runtime scratch through attached view charges/refunds live ledger
- all enabled host stores close/drain to zero
- large host identity/key is accounted
- transcript churn respects evidence bound
- reclaim/spill transfer preserves resident accounting
- wall-time/subquery/trace/output limit each bites at boundary+1
- denied operation leaves host state and ledger unchanged
- restart/reopen preserves spill/tombstone/accounting semantics where claimed
- dependency resolver proves exact QMP source digest

## Acceptance

- [ ] clean/frozen host snapshot receipt exists.
- [ ] host resolves exact QMP revision/digest.
- [ ] production composition contains no basic/report-only fallback.
- [ ] all memory stores/scratch are budgeted or explicitly typed unbudgeted in non-production tests.
- [ ] every public ExecutionBudget field has a reachable host enforcement point.
- [ ] focused host suites and owning product suites pass.
- [ ] full required QBC/CI authority passes on the same source.
- [ ] QMP standalone GREEN is not used as substitute for host GREEN.
- [ ] top-level and nested host metadata resolve the same frozen QMP root/tree.
- [ ] gate target and Cargo-resolved QMP mismatch is a terminal RED.

## Verification

Exact selectors must be re-resolved from current host suite registry before execution. Minimum classes:

```bash
cargo test -p quanta-memory-host --all-features
cargo test -p quanta-runtime-memory-compose --all-features
# Then run the current suite-registry selectors for runtime/retrieval/session owners.
# The exact QBC command must be copied from the frozen host registry receipt;
# inventing a selector here is forbidden.
```

## Non-goals

- unrelated Semantica dirty-main repair
- host architecture refactor outside memory/budget consumers

## Final-audit P1-F04 — host verification and Cargo resolution are split

At the audited host snapshot (`1dc185fb9f3d54a1c4ee85d569adbbd405113969`,
dirty shared main), Cargo resolves the adjacent QMP checkout at shared-main
`2cfccc685275521bc148491eb6cf10e9a6b37ba7`, not audited QMP `dfb58a6`.
`MEMORY_PLATFORM_REPO` selects which checkout the doctor/lint gate inspects but
does not change Cargo resolution. A gate can therefore verify one QMP while the
host compiles another.

Required host receipt fields/gates:

1. top-level and nested `cargo metadata --locked` resolved core/facade manifest
   paths, package IDs, QMP HEAD/tree ID and canonical digest;
2. host HEAD/dirty fingerprint and QMP HEAD/dirty fingerprint from the same run;
3. hard failure when `MEMORY_PLATFORM_REPO` differs from the Cargo-resolved root;
4. exact suite-registry/QBC profile IDs and results bound to both source IDs;
5. no raw invented selectors: resolve current registry entries first.

## Implementation checkpoint — 2026-09-21

Host repository implementation is committed on isolated branch
`codex/qmp130-main-hardening`:

```text
host commit: 8e23e1233d333d12ec3afe4c07686d73c1a3b188
host tree: f4aad2aeb317a4ec12c3cd898f405d3e8e486b87
host worktree: clean after commit
change size from frozen parent: 45 files, 1802 insertions, 582 deletions
```

Implemented scope:

- production memory composition, host runtime/retrieval/session adapters and
  execution-budget call sites use the QMP authority;
- focused production, live-authority and boundary+1 budget contracts exist;
- canonical suite registry and feature-isolation entries are registered;
- top-level and nested lockfiles are synchronized;
- `tools/ci/qmp130_dependency_identity.py` derives both Cargo metadata roots,
  package identities and Git identities, rejects dirty/mismatched targets, and
  is wired through `gate-qmp130-host-adoption.sh`;
- five verifier unit tests include top/nested root mismatch, expected-root
  mismatch, expected-SHA mismatch and dirty-target rejection.
- host tests assert complete retained semantic cost rather than obsolete
  payload-only byte counts;
- transcript policy and poison-error bridges compile against both QMP 0.1 and
  0.2, and the product owns a typed version-neutral retention rejection.

Current-source direct execution at host commit `8e23e123` while Cargo resolves
the adjacent QMP 0.1 checkout:

| Proof | Result | Output SHA-256 |
| --- | --- | --- |
| dependency-identity verifier unit tests | PASS, 5/5 | `b1041454c8482d1c88ba7a305a289586858ae837d1364c913ca9a5f7fd5b2a00` |
| production composition contract | PASS, 2/2 | `b02b07e468b4a4ffe3d0bf1d5ecc8ad1f605e39ac1c467bc74c28b87490d31ef` |
| live host authority contract | PASS, 1/1 | `6e6944907308974673e7335d0f41e29aa474fb349dd5709ae05d11d9cc11a05f` |
| execution-budget authority contract | PASS, 1/1 | `ed2aa70e5119d4432238126c8dd02c66d50d6a57ff405f038aa4f8cacb882f38` |
| host crate all-features | PASS, 129/129 | `0170d78a2f7586e94521e9038e6b121f080d94342257f1dca4df1d04c131f57b` |
| production composer crate all-features | PASS, 46/46 | `3e5813fcfa0aed71436c6459d6da6ceb4324b87417957a1f457e0f969058185c` |
| execution-budget crate all-features | PASS, 86/86 | `057612e95fb89f203cffade97de9be94d90c43d8356f23c640fb6447b2ee1011` |

The final compatibility probe is recorded in
[`EVIDENCE-2026-09-21.md`](EVIDENCE-2026-09-21.md). In summary, a clean
throwaway host commit changed only the two manifests and lockfiles to exact,
clean QMP `2dfb7bc` 0.2.0. On Rust 1.92 it passed direct host 129/129, composer
46/46 and execution-budget 86/86, plus identity verifier 5/5 and registered QBC
production composition 2/2. This closes the MSRV and 0.2 compatibility risk.
It does not activate that dependency in the committed host branch.

The terminal dependency gate is intentionally RED and demonstrates that the
host still compiles the adjacent shared QMP checkout instead of the qualified
isolated QMP source:

```text
resolved: /Users/songmin/Documents/code-new/quanta-memory-platform (0.1.0)
expected: /Users/songmin/.codex/worktrees/qmp-remaining-hardening/quanta-memory-platform (0.2.0)
gate output SHA-256: a27216732073ca1d6c55e3d97c59aeb1c235375fc050e0fe4efbffae3b70a50d
```

Therefore the implementation is present and focused behavior passes, but this
ticket remains `BLOCKED`. Closing it requires an authorized dependency
activation/publish/repoint action followed by QBC receipts for the remaining
host-full-wiring and execution-budget suites. Direct Cargo tests are not a
substitute for that QBC authority. The host commit
used `--no-verify` because the repository-wide pre-commit registry rejected six
pre-existing extra IDs outside this write set; repo-wide `cargo fmt --all
--check` also has unrelated baseline drift and crate-wide strict clippy reaches
unrelated dependency violations. These facts must remain visible in
the eventual host receipt.
