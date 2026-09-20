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
