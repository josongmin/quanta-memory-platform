## Read First — Non-Negotiables

This repo is `quanta-memory-platform`: a **portable, host-independent** memory
budgeting / accounting library. It is consumed by the Semantica monorepo through
a host adapter (`quanta-memory-host`) that lives **outside this repo**, and it is
designed to be extracted into its own git repository with no rewrites.

Hard rules:

- **Dependency direction is strict and acyclic:** `core <- facade <- testkit`.
  No member depends upward. The host adapter depends on this repo, never the
  reverse. Adding a dep that inverts this fails the extraction pre-flight.
- **`domain/` and `ports/` are pure:** no `std::fs`, `std::env`, `SystemTime::now`,
  `Instant::now`, and no `crate::adapters` import. I/O and clocks live behind
  ports/adapters.
- **Host-independent public API:** no Semantica nouns (`Semantica*`,
  `quanta_runtime`, `codegraph`). The host hello-world lives in Semantica.
- **Builder-first surface:** consumers use `MemoryPlatformBuilder`,
  `PortableMemoryPresets`, and the store/pipeline builders — never raw broker
  constructors.
- **Ledger is fail-closed:** every mutating accounting op fully succeeds or
  leaves the ledger untouched; the 4 invariants always hold; allocator telemetry
  is advisory and never mutates counters.
- **Conformance must bite:** the testkit suite must REJECT a broken fake, not just
  accept honest ones.
- **Agent guidance:** edit only `tools/pm/sources/**`, then run
  `python3 tools/pm/pm.py sync`. Never hand-edit `AGENTS.md`, `CLAUDE.md`,
  `.codex/*`, or `.cursorrules`.

## Where Things Live

| Need | Location |
| --- | --- |
| Repo overview + dependency direction | `README.md` |
| Extraction checklist (split into own repo) | `EXTRACTION.md` |
| Pure hexagonal kernel | `crates/quanta-memory-platform-core/src/{api,application,domain,ports,adapters}/` |
| Accounting SSOT (invariants) | `core/src/domain/ledger.rs` |
| Budget policy / presets / manifests | `core/src/domain/{policy,config}.rs` |
| Public broker surface | `core/src/api/{broker,builder,control}.rs` |
| Feature-sliced portable stores + pipelines | `crates/quanta-memory-platform-facade/src/<feature>/` |
| Storage backings (feature-gated) | `facade/src/<feature>/backings/` |
| Conformance / chaos / alloc tooling | `crates/quanta-memory-platform-testkit/src/` |
| Examples (compile vs real public API) | `facade/examples/{hello_world,effective_budget_report}.rs` |
| One-shot lint rail | `scripts/check-workspace.sh` |
| Repo-local doctor rail | `scripts/memory-platform-doctor` |
| Self-hosting scaffold assertion | `scripts/scaffold-external-repo.sh` |
| Extraction bootstrap | `scripts/extract-to-standalone-repo.sh` |
| Agent/operator instruction sources | `tools/pm/sources/**` |
| Prompt manager | `tools/pm/pm.py`, `tools/pm/targets.yaml` |

Generated docs (`AGENTS.md`, `CLAUDE.md`, `.codex/*`, `.cursorrules`) are produced
from `tools/pm/sources/**`. Never hand-edit them.
