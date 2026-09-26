# Reusable test-oracle audit prompt

Revised: 2026-09-26. This is an execution protocol, not a current-source audit
receipt. The completed Sep-26 campaign remains historical; its accepted
decisions are in [ADR 0005](../adr/0005-regression-oracle-acceptance.md).
Source and qualification boundaries follow
[ADR 0003](../adr/0003-source-bound-qualification.md).

Copy the execution prompt below into a task opened on this repository. It
discovers the repository and creates a fresh campaign directory; no absolute
checkout path, historical dirty-file list or advisory test count is embedded.
The nine lanes can run sequentially. Parallel delegation requires an explicit
user request and available worker slots. Census, critique and synthesis are
separate stages even when one agent performs them all.

## Research basis and final audit corrections

The research cutoff is 2026-09-26. These sources support the protocol choices;
they do not establish that this prompt or QMP is the best implementation.

- [cargo-mutants: how it works](https://mutants.rs/how-it-works.html) uses
  scratch source copies and an unmutated baseline, separates build viability
  from execution, and notes that surviving mutations can be no-ops.
- [Mutation Coverage is Not Strongly Correlated with Mutation Coverage
  (AST 2024)](https://ieeexplore.ieee.org/document/10556415/) shows that operator
  choice can substantially change the score for the same suite. Record the
  fault model and denominators instead of treating one score as universal.
- [Where tests fall short (ESEM 2025)](https://gregorykapfhammer.com/research/papers/maton2025/)
  distinguishes execution coverage from oracle gaps. Its Java evaluation is
  evidence for investigating unchecked behavior, not a Rust qualification.
- [Do LLMs Generate Useful Test Oracles? (ASE 2025)](https://www.lucadigrazia.com/papers/ase2025.pdf)
  evaluates oracle correctness and fault detection separately. Its Java/model
  results do not make an LLM's static verdict an independent contract oracle.
- [Auditing and Decomposing Feedback-Driven Evolution (2026-08-20
  preprint)](https://arxiv.org/abs/2608.19626v1) studies spurious fault detections
  caused by invalid/underspecified inputs and a single-reference oracle. Use
  independently justified preconditions and semantic witnesses; the preprint's
  benchmark results are not a general ranking of Rust testing approaches.

Corrections to the historical prompt pack:

| Previous failure mode | Required replacement |
| --- | --- |
| Restore mutations with a checkout command in a shared dirty checkout | Mutate only a private disposable source copy; restore its exact preimage |
| Compare global status to "only my report changed" during parallel reports | Compare frozen input hashes and enforce each worker's output ownership |
| Infer census completeness from grep counts; permit grouped rows | Reconcile source definitions with collected runtime IDs; retain every ID |
| Confirm survival without a passing baseline or an executed-test count | Baseline / mutant / restored runs, each selecting and executing one test |
| Treat heuristic assertion patterns as established defects | Candidate labels until contract-valid mutation evidence is available |
| Reject a finding after any mutant failure | Require the intended behavioral predicate; unrelated failure is inconclusive |
| Trust a critic-proposed mutation as evidence of contract breakage | Establish semantic difference independently of the accused oracle |
| Treat audit lane boundaries as safe parallel fix boundaries | Derive actual write sets and serialize shared owners during remediation |
| Aggregate partial reports or optional backings into product closure | Separate incomplete audit, test weakness, runtime defect and product scope |

## Execution prompt

```text
Perform a complete test-oracle audit of the quanta-memory-platform repository
in the current task. Produce a source-bound census, independently checked
findings and a final remediation decision document. Finish all stages below.
This task authorizes temporary mutation verification in disposable copies.
It does not authorize production fixes, commits, pushes or changes to existing
audit packets. Use normal repository instructions; invoke no named workflow
skill unless the user explicitly requests it in this task.

WHAT THIS REPOSITORY IS
A portable semantic memory accounting/budgeting library. Dependency direction
is core <- facade <- testkit. The ledger is authoritative; allocator telemetry
is advisory. An accounting mutation must succeed completely or preserve its
preimage. Current in-memory product scope and optional durable maintenance
scope must be read from README.md rather than inferred from feature presence.

READ FIRST
AGENTS.md, README.md, tools/pm/sources/rules/tests.md,
tools/pm/sources/rules/cheatsheet.md, relevant sections of
tools/pm/sources/rules/determinism.md, docs/adr/0003-source-bound-qualification.md
and docs/adr/0005-regression-oracle-acceptance.md. Read owning implementations,
public callers, fixtures and selectors as each lane requires them.

STAGE 0 — FREEZE INPUTS AND ASSIGN OWNERS
1. Discover the Git root from the current task, verify the expected workspace
   members, and record HEAD, tree ID, status, Rust/Cargo versions and lockfile
   hash. Do not reset, stash, clean, commit or restore active-checkout files.
2. Create a new docs/bugbash/oracle-audit-<UTC timestamp>-<short HEAD>/ directory.
   Compute its name yourself. If it exists, choose a fresh suffix; overwrite
   no previous campaign. This new directory is the only active-checkout write
   set. Build caches, scripts written for this audit and mutation artifacts
   may be created under a unique temporary directory; final evidence belongs
   in the new campaign directory.
3. Freeze the actual current source inputs into a disposable worktree/copy.
   For a clean source, use the exact committed tree. For a dirty source, also
   preserve the current file postimages, added/deleted paths and modes, and
   relevant untracked input files. Record this as a dirty overlay, never as a
   clean commit. Do not silently replace the requested working-tree source
   with HEAD. Inventory crate source/tests/examples, harness scripts, Cargo
   manifests/lock/config/toolchain, CI selectors and contract/rule documents.
   Exclude only identified build outputs and campaign outputs. Preserve and
   document excluded paths. Include external Cargo path inputs if any.
4. Hash the complete selected input inventory before and after capture,
   including the enumerated path set and file modes. Accept the snapshot only
   when those inventories agree with the captured bytes. Retry on capture
   drift without overwriting user work. If a stable capture is unavailable,
   report BLOCKED_SOURCE_DRIFT; make no source-bound confirmed claims.
5. Write campaign.json with source commit/tree, overlay identity if present,
   input paths/modes/hashes, aggregate input digest, frozen source directory,
   resolved dependency identities, toolchain, output ownership and test
   configurations. Keep frozen inputs unchanged; run builds with a separate
   temporary CARGO_TARGET_DIR. Check that Cargo resolution binds the intended
   frozen sources, including dependencies outside the three crates.
6. Every worker reads the same campaign.json and frozen source. Resolve lane
   scopes to an explicit file inventory before dispatch; inventory every
   crate test-bearing input. Assign unlisted files to exactly one lane or
   mark them explicitly excluded with a reason. Shared callers/helpers may
   be read by all lanes; each census item and output file has one owner.
7. Execute lanes sequentially unless parallel delegation is explicitly
   authorized. If authorized, respect actual worker slots; critic starts
   after auditor outputs are terminal, synthesis after critic is terminal.
   A terminal partial/blocked output permits partial synthesis, not COMPLETE.

LANE MANIFEST — RESOLVE AGAINST THE FROZEN SOURCE
01 core-domain: core/src/domain/** and core/tests/policy_contracts.rs.
   Inspect every accounting axis, checked arithmetic, rollback/preimage,
   group/class/owner/context policy, zero-byte authority and survivor release.
02 core-api: core/src/api/** and core/tests/public_api_surface.rs.
   Inspect reserve/commit/release/rebuild postimages, fences, cached pressure,
   callback/reclaimer lifetime, public signature and construction contracts.
03 app-ports: core/src/{application,adapters,ports}/**.
   Inspect actual reclaim work, scheduling decisions, fault reachability and
   the telemetry/authority boundary, including nested helper implementations.
04 revocable-store: facade/src/revocable_store/**.
   Inspect pin/deny identity retention, content, authority transfer and fences;
   distinguish in-process behavior from optional durable behavior.
05 facade-stores: facade/src/{transcript_store,retained_map,payload_store}/**.
   Inspect exact retained content/identity, replay/eviction, actual store seeds,
   keyed work/iteration, complexity claims and independent expected values.
06 spill-redb: facade/src/spill/**.
   Inspect same-target inventory, offline refusal, exact restore and sequence
   identity. Label optional durable behavior MAINTENANCE_ONLY under current
   product scope; do not invent a consumer or production qualification claim.
07 crash-rails: facade/tests/{spill_redb_kill_reopen,spill_redb_finalize_kill,
   cross_store_hellgate,hellgate_e2e}.rs and child example sources they invoke.
   Inspect actual child selection/execution, fault markers, kill coordinates,
   deadlines, exit/result parsing, restart and content checks. Process kill is
   not physical power-loss proof. Apply current scope labels to durable rails.
08 pipeline-shared: facade/src/{publish_pipeline,shared,presets}/**,
   facade/tests/public_api_surface.rs and residual facade root/API tests.
   Inspect publication/abort state, shared cost helpers, public API shape and
   per-lease accounting. Read cross-lane helpers without duplicate ownership.
09 testkit: testkit/src/**, testkit/tests/** and residual testkit examples.
   Inspect honest/broken-fake poles, actual public conformance dispatch, exact
   suite composition, ticket provenance and independent invariant oracles.
The core/facade/testkit abbreviations refer to their actual crate directories.
Resolve missing/new paths explicitly; historical approximate counts are not
inputs. Enumerate doctests, macro-generated tests, integration targets and
examples with tests, including residual core/facade root tests/examples.

STAGE 1 — CENSUS AND STATIC CANDIDATES
1. Build source-definition and runtime-test inventories separately. Source
   inspection must cover attributes, cfg/cfg_attr, macro-generated/property
   tests, ignored tests and compile_fail doctests. rg is discovery, not the
   completeness oracle. Do not add a dependency merely to parse the census.
2. For applicable targets/configurations, use collected test IDs from Cargo
   or installed nextest listing in the frozen tree. Record exact commands,
   target, features, environment, exit status and collected counts. --list
   does not execute tests. Inspect feature declarations and the registered
   feature matrix; default and all-features alone may miss exclusive branches.
   Identify and enumerate any applicable cfg branch those builds do not reach.
   Doctest enumeration that the available tooling cannot collect is a stated
   gap, not a fabricated runtime ID or count.
3. Give every source test definition a stable source ID and every collected
   test a configuration-qualified runtime ID. Preserve one runtime-census row
   per ID; do not collapse homogeneous groups into a single row. Map generated
   runtime cases back to their definitions. Document every source-only,
   runtime-only, gated, ignored or unavailable item; counts need not be equal
   when one definition generates multiple runtime cases. Explain differences.
4. For each item, follow contract -> exercised owner/caller -> fixture/helper
   -> observable postcondition -> assertion. A test name is a claim to inspect,
   not the expected-behavior authority. Use accepted contracts, independently
   derived invariants or a separately justified reference model. Reusing the
   SUT's own cost/policy calculator as an expected value can be circular.
5. Record static assessment as:
   OBSERVES_CONTRACT: the inspected oracle observes the stated predicate;
     this is not universal proof that the test detects every fault.
   WEAK_CANDIDATE: an asserted predicate is weaker than the claimed contract.
   FAILOPEN_CANDIDATE: name a reachable, contract-breaking behavior that may
     leave this oracle green, with a concrete mutation to check it.
   VACUOUS_CANDIDATE: the claimed path may be unreachable or unconstrained.
   UNKNOWN: reading/build/selection limitations prevent a reliable assessment.
   Do not classify a complete suite as strong from one representative body.
6. Investigate assert-on-input, possibly empty loops, advisory-only counters,
   discarded Results, broad catch_unwind, unreachable faults, shape-only
   round-trips and missing negative poles as heuristics. A negative-pole gap
   can be a documented conformance requirement gap; it is not automatically a
   runtime defect. API-shape, telemetry and panic-contract tests can legitimately
   assert those properties; judge the contract they actually claim.
7. For each candidate, state the contract citation, scenario/preconditions,
   owner-to-oracle path, minimal semantic mutation, independently observable
   wrong postcondition and exact accused-test selector/configuration. Quote
   the relevant assertion only. Distinguish production, fixture, conformance,
   harness, model-only and maintenance-only scope.
   Give each candidate a stable ID, including its scenario and predicate; one
   census row may link multiple candidate IDs without duplicating the test ID.
8. Write findings-<lane>.md and census-<lane>.json in the new campaign directory.
   Include source digest, owned file list, source/runtime/reconciled counts,
   all census rows, candidate details, unknowns and commands actually run.
   Each lane writes only its assigned reports and evidence subdirectory.

STAGE 2 — INDEPENDENT CRITIC AND MUTATION VERIFICATION
1. Reconcile all lane IDs against the campaign census. Detect missing and
   duplicate ownership, unresolved cfg branches and source-digest mismatches.
   Reject mismatched reports as STALE; keep missing scopes as audit gaps.
2. Review every FAILOPEN_CANDIDATE and VACUOUS_CANDIDATE. Also inspect up to
   five WEAK_CANDIDATE and five OBSERVES_CONTRACT items per lane, selected by
   ascending SHA-256(source digest + runtime/source ID). Record selection and
   denominators. Static spot checks do not substitute for mutation executions.
   Unsampled items remain static/unverified. Audit completeness does not imply
   exhaustive mutation adequacy; no sampling-based overall strength claim.
3. Before executing a proposed mutant, independently check that it changes a
   reachable owning behavior in violation of a cited contract. Prove semantic
   difference with code reasoning and, where feasible, an independent witness
   in the disposable copy. A mutant surviving the accused oracle does not by
   itself prove contract breakage. Assertion deletion alone, renamed filters,
   compilation failures, wrong configurations and no-op/equivalent operators
   do not establish oracle weakness. Edit SUT/fixture/harness code only where
   that layer owns the named behavior; never weaken the accused assertion.
   Any added witness harness is a separately identified experiment overlay;
   keep it out of the accused-test baseline/mutant/restored triplet.
4. Create a private disposable critic copy from the frozen snapshot. Check
   input equality and resolved Cargo identities before each experiment.
   Never mutate the active checkout or the auditors' frozen source. Save exact
   preimage bytes and modes for every file affected by a mutant. Run one mutant
   at a time in each private copy; prevent cross-mutant build contamination.
5. Run the exact accused test UNMUTATED first. Use a full ID with --exact and
   explicit target/features; use --ignored only for an intentionally selected
   ignored test. Require exit success, selected=1, executed=1, passed=1,
   failed=0 and ignored=0 from the actual terminal result. Zero-test success is
   invalid. For generated cases, select one collected runtime ID. If the tool
   cannot isolate a doctest/case, record that limitation instead of claiming
   an exact-test experiment. Confirm child execution separately in subprocess
   tests: parent exit/counts alone cannot establish that the child ran.
   If this baseline fails or selects/executes zero tests, apply no mutant and
   classify the candidate UNRESOLVED with the actual failure evidence.
6. Apply and hash the reviewed minimal mutant, build it with the same inputs,
   and run that exact test again. Capture compiler outcome, selection and
   execution counts, exit status, full log and the actual failing predicate.
   A compile failure is UNVIABLE, never a killed behavior. Timeout, an unrelated
   panic/assertion, infrastructure failure or unstable baseline is INCONCLUSIVE.
   For compile_fail/UI tests, distinguish an implementation build failure
   from the compiler outcome intentionally checked by the original negative
   compilation harness. Breaking that intended predicate can legitimately
   kill a buildable mutant; record the actual diagnostic/acceptance mismatch.
7. Restore saved preimages in the private copy, verify full input hashes/modes
   equal the frozen snapshot, and rerun the exact baseline successfully.
   Complete restoration and remeasurement after every mutant, including error
   paths. If restoration fails, stop using that copy and invalidate its receipt.
8. Assign mutant outcome separately from finding disposition:
   SURVIVED: valid non-equivalent mutant executed the exact test and stayed green.
   KILLED: valid mutant failed the intended behavioral predicate.
   UNVIABLE: mutant did not compile.
   EQUIVALENT_OR_INVALID: no proven reachable contract difference / invalid setup.
   INCONCLUSIVE: environment, selection, instability, restore or witness gap.
   Finding disposition is CONFIRMED only for a justified SURVIVED experiment
   with passing baseline and restored runs; REJECTED_FOR_THIS_MUTANT for KILLED;
   otherwise UNRESOLVED. A killed mutant is not proof of universal adequacy.
9. Deduplicate identical owner/contract/scenario/mutant experiments, retaining
   all accused test mappings and executing each mapped test separately. Do not
   limit experiments silently to save time. If blocked, list every unexecuted
   candidate and reason; overall status becomes PARTIAL.
10. Write findings-critic.md and mutations.json with report/input hashes, exact
    patches/preimages, independent semantic justification, all three commands,
    configurations/counts/exit codes/log paths, outcome and disposition. Check
    conformance gap maps independently; preserve requirement gaps separately
    from mutation-confirmed findings. No production fixes in this stage.

STAGE 3 — SYNTHESIS AND FINAL OMISSION AUDIT
1. Produce 99-index.md. Begin with COMPLETE, PARTIAL or BLOCKED, frozen source
   identity, dirty-overlay status, product scope and missing evidence. COMPLETE
   requires all in-scope census items reconciled and all required candidate
   experiments/critic checks terminal without unresolved evidence. Explicitly
   excluded scopes remain outside that claim. Missing inputs, STALE reports,
   unresolved candidates and uncollected in-scope targets prevent COMPLETE.
2. Report unique source definitions, configuration-qualified runtime IDs,
   static candidates, mutation attempts, valid SURVIVED/KILLED outcomes,
   UNVIABLE/invalid/inconclusive outcomes, confirmed findings and unresolved
   gaps separately. Preserve numerator and denominator. Report killed-candidate
   counts as rejection for the tested mutation, not a universal overclaim rate.
3. Rank confirmed findings by reachable consequence, owning contract and
   active consumer exposure. Separate oracle/harness weakness from an observed
   production runtime defect. State what independent evidence supports each.
   A confirmed weak test does not prove unmodified production code is broken.
   Label excluded durable capability MAINTENANCE_ONLY; do not promote it into
   an in-memory release blocker without a current exposed contract/consumer.
4. Record conformance requirement gaps, missing negative poles and model/proof
   limitations separately. Do not turn generic WEAK labels, assertion patterns
   or historical remediated findings into automatic implementation tickets.
5. Deduplicate the actionable union by owning root cause while retaining every
   affected test and contract. A shared fix may cover multiple symptoms only
   when its proposed verification covers those symptoms explicitly.
6. Give actionable repairs exact proposed write sets, prerequisites, semantic
   expected postimages, valid mutant-to-kill checks and required delivery rails.
   Derive real shared-helper/fixture conflicts; audit-lane ownership alone does
   not prove safe parallel fix ownership. Assign one owner to each shared file
   and a serial integration boundary. Mark these repairs PLANNED, not VERIFIED.
7. Reconcile every source/runtime ID and every candidate through the final
   disposition table. List excluded/unavailable configurations and unresolved
   work explicitly. Check report links, raw log presence and claimed counts.
8. Compare the active checkout's non-output input inventory to the initial
   inventory. Your writes must be confined to the new campaign directory and
   temporary workspace metadata/builds. If another writer changed inputs, do
   not revert them: report INPUT_DRIFT and retain the evidence as applicable
   only to the frozen snapshot, not to current HEAD. End with the exact paths,
   commands run, commands not run and remaining audit/qualification gaps.
9. Do not label this audit as canonical workspace, extraction, hosted CI,
   installed host, deployment, SLO or physical power-loss qualification. If
   those rails were independently run, report their exact source and scope.
```
