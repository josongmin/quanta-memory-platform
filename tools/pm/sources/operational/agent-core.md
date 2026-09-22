## Working Rules

Keep a **thin, inward-only boundary** across the hexagonal kernel and the
feature-sliced facade:

| Layer | Location | Responsibility |
| --- | --- | --- |
| **Domain** | `crates/quanta-memory-platform-core/src/domain/` | Pure budget/accounting logic: ledger, policy, config, context, stats. No I/O, no clocks. |
| **Ports** | `crates/quanta-memory-platform-core/src/ports/` | Trait definitions: pin, scratch, store, telemetry. No adapters. |
| **Adapters** | `crates/quanta-memory-platform-core/src/adapters/` | Port impls (hdrhistogram, metrics). Telemetry is advisory. |
| **Application** | `crates/quanta-memory-platform-core/src/application/` | Use-case services: pressure controller, scheduler. |
| **API** | `crates/quanta-memory-platform-core/src/api/` | Builder-first public broker surface. |
| **Facade** | `crates/quanta-memory-platform-facade/src/<feature>/` | Portable stores + pipelines; backings under `backings/`, feature-gated. |
| **Testkit** | `crates/quanta-memory-platform-testkit/src/` | Conformance / chaos / fixtures / doctor. Never a prod dep. |

On any change:

- **Ledger / accounting** → add `#[cfg(test)]` tests asserting the invariants
  defined in `crates/quanta-memory-platform-core/src/domain/ledger.rs` and that
  mutations are fail-closed.
- **Policy / manifest / preset / residency** → add validation tests; update
  `crates/quanta-memory-platform-core/tests/policy_contracts.rs` if a
  cross-cutting contract changed.
- **New port** → trait in `ports/`, impl in `adapters/` (or facade backing); keep
  `domain/` free of the impl.
- **New facade store/pipeline** → service + builder in `shared/`, backings behind
  a feature; never reach back into `core` internals.
- **Public API change** → update the `lib.rs` re-exports and the examples
  (`hello_world`, `effective_budget_report`) so they still compile.
- **Agent guidance** → edit `tools/pm/sources/**`, run `python3 tools/pm/pm.py sync`.

Direction & portability are the two things reviewers check first: never invert
`core <- facade <- testkit`, and never let Semantica/host nouns into the public
API. When in doubt, run `scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp`.
