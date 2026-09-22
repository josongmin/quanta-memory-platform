# FINAL-001 File Inventory

Frozen review base: `101781c2ea6122602b4acd9aa69ebfc13fda958f` (`main` and
`origin/main` when reviewed). This inventory classifies every path in that
commit. Later untracked inputs are listed in `FINAL-CLOSEOUT.md` and are not
silently treated as part of this frozen set.

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
