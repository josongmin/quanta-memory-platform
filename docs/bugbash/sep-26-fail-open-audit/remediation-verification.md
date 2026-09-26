# Sep-26 fail-open remediation — implementation and verification boundary

> Integration update: the Rust fixes and source-bound proof are being consolidated
> in the local commit containing this notice, together with the gate owner branch.
> Earlier dirty-source and isolated-extraction descriptions below are historical
> receipt boundaries. The census verifier now pins the audited base independently
> of HEAD and accepts the newly tracked helper absent from that base.


Base HEAD: `61e9c7c47fa7294f9a926fd867e4af0b116c429b`.
Source state: **dirty working tree** on the base HEAD. Final Rust source is frozen by the 85-file SHA-256 map below. An isolated clean commit was used for the extraction dry-run; the shared checkout was not committed.

The implementation addresses the ten retained items in
[reviewed-actual-findings.md](reviewed-actual-findings.md), plus two specific kit
dispatch/composition guards. One retained item is an actual **testkit runtime
accounting defect**. The other nine are test-oracle or subprocess-harness defects;
they do not establish a production core/facade runtime failure.

The historical auditor reports and [99-index.md](99-index.md) describe the original
audit snapshot. Their broad verdicts are not the final filtered remediation queue.
Current lexical test inventory belongs in
[current-source-census.md](current-source-census.md), with machine-readable rows in
[current-source-census.json](current-source-census.json).

## Implemented corrections

| ID | Owner | Correction and required proof |
| --- | --- | --- |
| R01 | `testkit/src/fake/broker.rs` | Reclaim plans refunds from actual ticket provenance: owner, class, charge kind, original group, and remaining authority. All affected axes are preflighted before publication. Partial reclaim preserves remainder authority; full reclaim invalidates the ticket; zero-byte authority survives. Regressions cover unrelated surviving tickets, mixed classes/kinds, pending commit, group refunds, rebuild, and rejected corrupt-group preflight. This corrects the reproduced runtime defect. |
| R02 | `testkit/src/faults/scenarios.rs` | Pinned-delete containment checks continued lookup of the same reference, pin count, item count, charged bytes, per-kind axis consistency and the full ledger snapshot. The public facade has no payload-byte reader, so this oracle does not claim a byte-content readback. A scoped losing-entry probe must be classified as escaped, rather than contained merely because charged bytes remain. |
| R05 | `core/src/domain/ledger.rs` | Test-only owner-map metering counts keyed reads and keys actually yielded by scans. Single-release work remains bounded at 32/128 owners. Instrumentation has its own keyed/nested-scan/values/borrowed-iteration/read-view checks. |
| R06 | `core/src/application/controller.rs` | The overflow oracle includes limit=MAX, usage=MAX → Fatal and large-limit 80% → Soft. Wrapping or premature saturating multiply/divide must fail, while correct u128 arithmetic passes. |
| R07 | `core/src/domain/admission.rs`, `application/scheduler.rs` | A test-only thread-local counter observes the actual fair-share precompute function. Construction adds one call; repeated admission adds none. Value-identical recomputation cannot satisfy the reuse oracle. |
| R08 | `core/src/domain/config.rs` | Independent-floor fixture includes class reserved=5,000, group reserved=5,000, headroom=1,000, global=10,000. Separate floors fit; incorrect joint floor=11,000 fails the acceptance oracle. |
| R09 | `core/src/api/control.rs` | At semantic usage=800,000/global=1,000,000, allocator observations of both 0 and 999,999 must leave cached pressure immediately Soft, subsequent semantic classification Soft, and the full ledger snapshot unchanged. |
| R10 | `facade/src/payload_store/service.rs` | Seed isolation uses two actual stores and each store's own digest path, with within-store stability. It no longer tests unrelated standalone RandomStates. Random digest collision remains a finite theoretical limitation. |
| R11 | `facade/src/spill/service.rs` | Offline refusal uses the same preseeded target, preserves its inventory, and restores exact retained handles after returning online. Both target implementations are covered. |
| R12 | `facade/src/test_support.rs` and child launch sites | Each guarded command lists its actual test selection and requires exactly one test before launching. Zero/multiple selections are rejected; `--include-ignored` prevents an explicitly selected child becoming a skipped success. Existing child status/timeout checks remain. |

Source paths in this table are relative to `crates/quanta-memory-platform-*`;
the core/facade/testkit owner labels identify the corresponding crate.

## Added kit guard coverage

These two items were explicit guard gaps, not reproduced broken implementations
in the filtered review. They are implemented without promoting them to runtime bugs.

| ID | Guard |
| --- | --- |
| R03 | A scoped test-only fidelity fault enters at `MemoryTestKit::fake_broker` construction. The same public `run_conformance` dispatcher must reject the broken fake with the expected contract family, then pass after guard restoration. Thread-local isolation and a Drop guard restore prior fidelity, including unwind paths. |
| R04 | The facade-suite composition test independently enumerates the required assertion names and requires each exactly once. Removing a facade slice cannot remain green merely because lifecycle assertions still run. The generic verdict fold remains usable for legitimate subsets. |

## Cross-review hardening

- Owner-work metering starts at the beginning of `preflight_release_batch`, before
  refund aggregation, and stays active until function exit through a guard.
- `UsageView.owner_usage` exposes a private-backed keyed read view, preserving
  metering rather than exposing a raw iterable backing map. Test snapshot copying
  traverses the metered iterator, so snapshot-based scans during preflight are observed.
- R09 checks cached pressure **immediately after observation**, before a semantic
  reclassification could repair an incorrectly changed cache.
- The child guard covers **16 launch sites**: transcript 7, retained-map 5,
  payload 2, shared lifecycle 1, revocable 1. The additional revocable launch was
  found during cross-review. A separate retained-map launch already had its own
  one-passed-test guard and is not counted as a newly guarded site.

## Exclusions preserved

- Loom protocol-model tests remain useful model tests; absence of production
  mutex/storage execution is not evidence that their assertions are vacuous.
- Deleting assertions from a metrics test is a test-oracle mutation, not a broken-SUT
  mutation. Changing the unit emitter into a state-bearing struct also does not
  compile unchanged construction tests. Neither establishes the proposed runtime defect.
- Unrelated WEAK rows, generic missing negative poles, additional class samples,
  exact ladder vectors, label uniqueness, and local round-trip assertions remain
  optional coverage unless a reachable defect or valid surviving mutation is established.
- Broad preset-cap, shared reclaim-lane, ticket-leak, and reconfiguration mutations
  already rejected by sibling authority tests are not separate runtime findings.
- Defensive unknown-group paths and type-enforced fence/report-only properties
  are not promoted to reachable public behavior defects.
- This work does not qualify crash recovery, redb durability, or all possible
  concurrency behaviors. It does not expand the fake's minimal algebra contract
  into complete production issuer-identity equivalence.

## Final verification

| Proof | Result |
| --- | --- |
| Canonical rail | **PASS**: `python3 tools/pm/pm.py sync`, then `scripts/check-workspace.sh`; terminal `check-workspace: clean (full repository gate)`. [Full log](remediation-proof/workspace-gate.log). |
| Feature matrix / capabilities | All feature combinations warning-free; 10 advertised capability paths execute non-zero tests. |
| Workspace tests | Core 274, facade 364, testkit 86 unit tests pass; all registered integration and doctest suites pass. Source census and runtime counts remain distinct evidence. |
| Documentation / guidance / dependencies | 298 documented selectors resolve; `pm lint` 9 targets current; cargo deny advisories, bans, licenses and sources pass. |
| Mutation verification | **19 baseline GREEN, 19 mutant RED, 19 restored GREEN**. All57 exact runs execute one test. No compiler failure, no-op mutation or zero-test run is accepted. [Mutation matrix and logs](remediation-proof/99-mutation-summary.md). |
| Source custody | All85 Rust files match the isolated frozen candidate after restoration and the final shared checkout. Aggregate digest: `b25da136cd839eb7e07e0165f5e6b9fbaada1ea6dcc6dda719598893f1cb29c6`. [Hash map](remediation-proof/source-sha256.json), [attestation](remediation-proof/shared-source-attestation.json). |
| Census / report completeness | Nine historical census tables total795 raw rows, matching historical grep claims; one non-test compile pin means794 actual historical tests. Current nine scopes contain801 actual rows =801 anchored rg matches. Nine scope findings, critic and index all exist and are complete. |
| Extraction | **PASS dry-run** from clean isolated commit `df881232c3a7f4c4014b4468a84e2078a17c5635`, tree `f5f5158e71244c1c8c7d667cf47d559a019dd59c`, canonical manifest `4caa4450ff0fb620aae46a247b7f366e1381ee758d7630c3880f72a6a1fb642a`. [Log](remediation-proof/extraction-dry-run.log). |

The clean candidate contains the same final Rust bytes and the documentation
checkpoint before this receipt finalization. Its extraction dry-run validates
preflight/clean manifest and prints the export/build plan; it does **not** run a
destination build. The shared checkout remains uncommitted. No installed host,
release, Linux device-mapper or physical power-loss qualification is claimed.

The initial stopped mutation attempts and pre-fix critic receipts are not part
of the final19-mutant receipt. A scheduler operator initially targeted a trait
declaration and made no change; it was corrected to mutate the actual `admits`
implementation and no-op operators are now rejected before running.
