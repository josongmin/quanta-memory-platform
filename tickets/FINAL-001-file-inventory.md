# FINAL-001 File Inventory

Frozen review base: `101781c2ea6122602b4acd9aa69ebfc13fda958f` (`main` and
`origin/main` when reviewed). This inventory classifies every path in that
commit. Later untracked inputs are listed in `FINAL-CLOSEOUT.md` and are not
silently treated as part of this frozen set.

## Current requalification delta

Current reviewed commit: `4aff89156049d06145f295b93d9a4b017210cb1f` (tree
`be72d2a2b4bdf2534dbd9624d43a6a2583daa54d`), relative to the historical
receipt source `014eb65d09c38f1aba949ec438ca9c04c9261381`. The only remaining
working-tree path when this section was written is generated `.cursorrules`;
it is produced from `tools/pm/sources/rules/build.md` and must be committed
with that source before FINAL-002. The table below classifies all 25 committed
paths in this requalification delta; it does not treat the historical receipt
as proof for this newer source.

| Owner / purpose | Paths reviewed | Verdict |
| --- | --- | --- |
| Workspace safety and lint policy | `Cargo.toml` | Root workspace lint levels add `unsafe_code = "deny"` plus Clippy levels. Each library still retains non-overridable `#![forbid(unsafe_code)]`; policy verification rejects a missing attribute or unauthorized exception. No dependency graph change. |
| Generated guidance and source authority | `.cursorrules`; `CLAUDE.md`; `tools/pm/targets.json`; `tools/pm/README.md`; `tools/pm/sources/agent-specific/{claude.md,cursor.md}` (delete); `tools/pm/sources/rules/{architecture.md,build.md,cheatsheet.md}` | The two retired agent-specific source mappings are removed. The three remaining PM targets are authoritative. `.cursorrules` is generated-only and its pending regeneration is tracked explicitly above. |
| Canonical shell rail | `scripts/check-workspace.sh` | ShellCheck replaces a duplicate `bash -n` pass and fails closed when no shell scripts are found. This is a validation deduplication, not a coverage removal. |
| Product and extraction documentation | `CHANGELOG.md`; `README.md`; `EXTRACTION.md` | Documentation now names the actual ShellCheck rail and keeps extraction/host proof boundaries explicit. |
| Current closeout authority | `tickets/FINAL-CLOSEOUT.md` | Historical receipt is preserved; FINAL-001 through FINAL-006 are correctly marked as requalification work for the current source. Ticket files are tracked repository inputs but not extraction-manifest copy roots. |
| Structural remediation queue | `tickets/TEST-OPTIMIZATION-EXECUTION.md`; `tickets/TO-01-redb-concurrency.md` through `tickets/TO-09-transcript-partitions.md` | Nine scoped producer-owned remediation tickets. They are implementation plans, not evidence of completion; TO-03 precedes TO-02 on the shared scaling owner. |

No unexplained deletion, public API expansion, host noun, upward dependency, or
direct generated-file edit was found in the requalification delta. The queue
does not itself close FINAL-002 through FINAL-005.

## Remediation implementation delta

The following candidate delta is reviewed before its exact-HEAD commit. It is
the implementation of the tracked TO-01 through TO-09 queue, plus structural
deduplication discovered while executing it. Its qualification status is
pending the clean committed SHA; no result below reuses the historical receipt.

| Owner / purpose | Paths reviewed | Verdict |
| --- | --- | --- |
| Core behavioral contracts | `core/src/api/{builder.rs,control.rs}`; `core/src/domain/{ledger.rs,policy.rs}`; `core/src/ports/telemetry.rs`; `core/tests/{policy_contracts.rs,public_api_surface.rs}` | TO-05/06/07/08: two-cardinality bounded release probe; public zero-pool fail-closed admission; builder-to-live reclaimer path; exact duplicate/dead assertion removal. No production ledger mutation. |
| Facade storage and public boundary | `facade/src/{lib.rs,payload_store/service.rs,retained_map/service.rs,revocable_store/service.rs,spill/backings/redb.rs,transcript_store/service.rs}`; `facade/tests/public_api_surface.rs` | TO-01/02/09: barrier-backed Redb receipt/payload oracle, compiler-backed visibility contracts replacing source greps, and independent transcript partition-cost oracle. Product production changes are limited to test-only contract/doc changes; no new public export. |
| Facade performance authority | `facade/benches/scaling.rs`; `scripts/verify-scaling-results.py`; `scripts/tests/test_scaling_results.py` | TO-02/03: allocation epoch excludes fixture allocation and disables on panic; revocable elapsed includes fill plus drain; a single 10x cardinality normalized-growth validator has linear, quadratic-negative, and single-cardinality-smoke fixtures. |
| Feature resolution authority | `scripts/verify-feature-matrix.sh` | TO-02: resolved Cargo feature graph, not TOML/README text, rejects `unbudgeted-stores` in default, `full`, and `full-backings`. |
| Fault negative pole | `testkit/src/faults/scenarios.rs`; `tickets/TO-04-fault-driver-negative-pole.md` | TO-04: one private before/result/after classifier is shared by honest, lying, drift, and unreadable-count controllers; focused selector explicitly enables `faults`. |
| Extraction and generated guidance | `scripts/extract-to-standalone-repo.sh`; `.cursorrules`; `AGENTS.md`; `CLAUDE.md`; `CHANGELOG.md`; `tools/pm/sources/{operational/agent-core.md,operational/agent-playbook.md,rules/{architecture.md,build.md,cheatsheet.md,dependencies.md,determinism.md,error-handling.md,tests.md}}` | Extraction runs its canonical rail once; its first fail-closed stage performs locked destination metadata validation, so no outer duplicate metadata call remains. Source guidance is consolidated, and all generated outputs are produced by PM sync. |
| Ticket command correctness | `tickets/TO-03-scaling-epoch.md` | The bench selector names its required `payload_store,revocable_store` features; the previous command could not compile the target. |

The current candidate has no Cargo dependency or public API graph change.
`core <- facade <- testkit` remains the resolved direction; full feature-matrix
and public/doctest rails are required again after the candidate is committed.

## Post-candidate rail deduplication delta

| Owner / purpose | Paths reviewed | Verdict |
| --- | --- | --- |
| Feature matrix | `scripts/verify-feature-matrix.sh` | Identical Cargo feature sets are compiled once: empty default equals bare, and a singleton default equals that same isolated feature. Every distinct logical feature set remains covered, including bare, full, and workspace bookends. |
| Policy test selection | `scripts/tests/test_workspace_policy.py` | Removes the current-worktree duplicate; the canonical rail still invokes the live policy checker directly, while fixture tests retain missing inheritance, missing non-overridable forbid, and unauthorized exception negative cases. |
| Scenario/changelog documentation | `docs/verification-scenarios.md`; `CHANGELOG.md` | Replaces renamed/removed test selectors and records the validated deduplication. No runtime API or extraction input change. |

## Final candidate hardening delta

| Owner / purpose | Paths reviewed | Verdict |
| --- | --- | --- |
| Workspace lint authority | `Cargo.toml`; `CHANGELOG.md`; `core/src/api/{builder.rs,control.rs}`; `core/src/domain/policy.rs` | Adds `clippy::redundant_clone = deny` at the workspace authority and removes three provably moved test-only clones. No accounting semantics, dependency, or public API changes. |
| Exact-source receipt authority | `tickets/FINAL-CLOSEOUT.md`; `tickets/TEST-OPTIMIZATION-RECEIPTS.md`; `tickets/TO-01-*` through `tickets/TO-09-*` | Separates the historical bare tag from a new exact-source tag family. The receipt annotation, not a self-referential tracked document, binds SHA/tree/commands. Implemented tickets cannot be called done before that receipt exists. |
| Extraction validation ownership | `scripts/extract-to-standalone-repo.sh` | The extracted destination runs the canonical rail once. That rail's first resolved-graph stage owns locked metadata validation; the extractor has no duplicate outer invocation. |
| Semgrep fixture authority | `scripts/tests/extraction-contracts.sh`; `CHANGELOG.md`; `tickets/{TEST-OPTIMIZATION-EXECUTION.md,TEST-OPTIMIZATION-RECEIPTS.md}` | One JSON Semgrep fixture corpus proves all six expected rule IDs fire while clean/suppressed cases remain clean. This removes startup duplication without letting one rule mask a broken peer; receipts remain historical unless the final tag binds them. |

No current candidate change introduces a host noun, upward dependency, raw
authority export, generated-file hand edit, test skip, or assertion weakening.

| Owner / purpose | Paths reviewed | Verdict |
| --- | --- | --- |
| Removed local-only build preferences | `.cargo/config.toml` (delete); `clippy.toml` (delete); `deny.toml` | Correct: portable rail owns its settings and deny has no stale exception. |
| Generated guidance outputs | `.codex/CODEX-RULES.md` (delete); `.codex/CODEX-START-PROMPT.md` (delete); `.cursorrules`; `AGENTS.md`; `CLAUDE.md` | Correct: only three currently supported outputs remain and PM regeneration is clean. |
| Product and extraction documentation | `CHANGELOG.md`; `EXTRACTION.md`; `MIGRATION.md`; `README.md`; `docs/verification-scenarios.md` | Correct after removing stale campaign claims and separating repo proof from host proof. |
| Core implementation/comment cleanup | `crates/quanta-memory-platform-core/src/api/control.rs`; `src/application/controller.rs`; `src/ports/pin.rs`; `src/ports/telemetry.rs` | Comment-only/terminology cleanup; no authority or accounting behavior changed. |
| Core cross-cutting contract naming | `crates/quanta-memory-platform-core/tests/wave1_policy_contracts.rs` (delete); `crates/quanta-memory-platform-core/tests/policy_contracts.rs` (add) | Rename retains the same policy contract coverage and removes obsolete wave naming. |
| Facade public example/builder | `crates/quanta-memory-platform-facade/examples/hello_world.rs`; `src/shared/builder.rs` | Comment/type cleanup; builder-first behavior unchanged. |
| Testkit public/conformance surface | `crates/quanta-memory-platform-testkit/src/conformance/suites.rs`; `src/lib.rs`; `tests/facade_conformance_bite.rs` | Transcript row alias removes repeated public signature complexity; bite tests remain authoritative. |
| Bounded v1 bugbash record | `docs/bugbash/sep-22-test-optimization/00-plan.md`; `01-core-domain.md`; `02-core-api-application.md`; `03-facade-stores.md`; `05-testkit-integration.md` | Retained audit evidence, not executable ticket/campaign admission tooling. |
| Canonical workspace rail | `scripts/check-workspace.sh`; `scripts/lib/run_cargo_test.py`; `scripts/run-scaling-bench.sh`; `scripts/verify-feature-capabilities.sh`; `scripts/verify-feature-matrix.sh` (rename/add); `scripts/verify-workspace-policy.py` (rename); `scripts/tests/test_workspace_policy.py` (rename); `scripts/tests/test_run_cargo_test.py` (add) | Correct consolidation into a repo-local, fail-closed rail. |
| Extraction rail | `scripts/extract-to-standalone-repo.sh`; `scripts/lib/self-hosting-manifest.sh`; `scripts/tests/extraction-contracts.sh`; `scripts/tests/test_extraction_manifest.py` | Correct self-hosting/extraction contract ownership. The later untracked-input guard is a follow-up to this group. |
| Removed wrapper scripts | `scripts/memory-platform-doctor` (delete); `scripts/run-checked-cargo-test` (delete); `scripts/scaffold-external-repo.sh` (delete) | Correct: their covered checks moved into the canonical rail or extraction contract. |
| Removed CI-only campaign gates | `tools/ci/check_workspace_lints.py` (delete); `tools/ci/feature-matrix.sh` (delete); `tools/ci/gates/gate-memory-platform-doctor.sh` (delete); `tools/ci/gates/gate-memory-platform-lint.sh` (delete); `tools/ci/tests/test_workspace_lints.py` (delete) | Correct: no workflow consumes them; their portable checks moved under `scripts/`. |
| Prompt-manager runtime/config | `tools/pm/pm.py`; `tools/pm/README.md`; `tools/pm/targets.yaml` (delete); `tools/pm/targets.json` (add); `tools/pm/tests/test_pm.py` | Correct standard-library/JSON PM contract; tests reject output/source drift. |
| Prompt-manager sources | `tools/pm/sources/agent-specific/codex.md` (delete); `agent-specific/cursor.md`; `operational/agent-core.md`; `operational/agent-playbook.md`; `operational/agent-router.md`; `operational/quick-reference.md` (delete); `rules/architecture.md`; `rules/build.md`; `rules/cheatsheet.md`; `rules/cursor-architecture.md` (delete); `rules/dependencies.md`; `rules/determinism.md`; `rules/error-handling.md`; `rules/tests.md` | Correct: sources align with three retained generated targets and current rail names. |
| Removed PM templates | `tools/pm/templates/_banner.j2` (delete); `agents.md.j2` (delete); `claude.md.j2` (delete); `codex-rules.md.j2` (delete); `codex-start.md.j2` (delete); `cursor-rule.md.j2` (delete); `cursorrules.md.j2` (delete) | Correct: no retained PM code reads Jinja templates. |
| Static regression policy | `tools/semgrep/qmp-regressions.yml` | Correct: tracks the consolidated rail paths. |

No source file in the reviewed commit lacks an owner/purpose classification.
The review found no upward dependency, raw-ledger public authority, direct host
dependency, generated-file hand edit, or retained reference to a removed rail.
