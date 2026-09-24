# QMP adversarial coverage: final optimized implementation plan

Status: **QMP-scoped implementation and qualification complete; publication and
host adoption are separate**. Base HEAD on 2026-09-24:
`b6d162650edba4f2b32e5020c8f0ac707efb0c22`. The shared worktree contains
uncommitted changes, including the bounded codec. The qualification receipt is
the exact-content clean temporary candidate created from this worktree, not a
commit or release of the shared main branch. Evidence: bounded owner selectors,
2/2 cross-store E2E, 19/19 verifier tests, clean-candidate 48-row smoke and
220-row full bench, five Linux tmpfs ENOSPC modes, canonical workspace rail,
and standalone extraction dry-run. The benchmark workload collision discovered
during final audit was fixed before this final receipt. Raw benchmark artifacts
are retained in the candidate's `target/scaling-results` directory.

`spill-redb` is in the requested **QMP optional-capability** release scope,
not the Semantica host release dependency graph. Semantica currently enables
facade `full`, not `spill-redb`/`full-backings`. Product decision: its
reconstructible, revocable in-memory cache entries may disappear on reclaim,
process exit or restart and are rebuilt from their authoritative source; do
not add Redb persistence or a host Redb qualification gate for those entries.
If a future host path retains the only authoritative copy or explicitly
enables `spill-redb`, reopen its durability requirement and qualify that exact
consumer/feature graph. This plan covers QMP's
portable contracts, not host cache identity, root activation/GC, async
cancellation, arbitrary caller codec allocation, allocator abort/OOM or
physical power loss. No finite suite proves every schedule; use representative
cross-owner histories and explicit failure-model limits.

## Final audit: what already proves the boundary

| Boundary | Current evidence | Decision |
| --- | --- | --- |
| Ledger, 0-byte tickets, cap axes, group remap, reconfigure/reclaim, callback reentry | `core/src/domain/ledger.rs`, `core/src/api/control.rs`: independent axis checks, 0-byte scope tests, concurrent reserve/reconfigure, single-flight callback, panic/partial-progress recovery. | **No blanket new core test suite.** Add an owner-local selector only if a new cross-store history exposes a previously untested transition. |
| One charged store with broker | `facade/tests/hellgate_e2e.rs`: 18 tests; charged retained-map/payload, denial, rebuild, reconfigure, one-store contention and drain. | New `cross_store_hellgate.rs` adds only the distinct two-store broker history and store/scratch/0-byte remap history. |
| Redb claim/unknown outcome and real process death | `revocable_store/service.rs`, `spill/backings/redb.rs`; four `spill_redb_kill_reopen.rs` and three `spill_redb_finalize_kill.rs` tests. The latter already kills a store with a live resident charge before/after finalize commit. | Do **not** add an unconditional spill commit→ACK SIGKILL hook: its post-restart durable postimage is not distinct from the existing post-commit cold reopen. Reopen only with a counterexample. |
| Actual filesystem ENOSPC | Docker runner now has backing write/restore/finalize plus bounded charged spill and charged restore-commit modes. | Verify all five on one source digest. The charged commit mode fills the isolated volume after a valid claim and before Redb's queue-to-recovery write; the test-only hook is absent from production feature graphs. |
| Bounded typed durable codec | `durable_codec.rs`, public bounded methods and five focused owner tests; a literal V1 fixture and unsupported-version no-publication oracle are added. The closure codec remains explicitly unbounded. | Resident cost is one revocable-store formula used by both codec preflight and final charge; absent-cap denial is already tested before ID reservation. No allocator-OOM claim. |
| Scaling bench | `scaling.rs` labels real timed calls, adds retained-map point reads and a bounded three-topology pairwise comparison. `verify-scaling-results.py` enforces complete v3 rows. | Old full-mode 1k-only **51-row** fixture must be RED; clean smoke requires 48 rows and full requires 220, with raw artifact and compatible baseline metadata. |

## Minimum new implementation: owner-local first

Only the following packets add coverage. Owner-local oracles test transitions;
integration runs only where a
different process, real filesystem fault or two product owners interact.

| Packet / gate | Cheapest owner-local work | Necessary integration/E2E work | Exact acceptance oracle |
| --- | --- | --- | --- |
| **T0 / P1** bounded durable owner | Finish the pre-existing WIP without another accounting authority. Keep the deprecated legacy method names as forwarding aliases for 0.2 callers; custom codec allocations remain explicitly unbounded. The absent-cap negative oracle proves no ID reservation or ledger mutation. | Run public bounded example, focused codec/store tests, five-mode Linux tmpfs runner, then the canonical rail on fixed source. | Literal V1 bytes decode with exact cost; QMR2 is not published or deleted; resident/codec cost agrees; denial leaves source/pending/ledger exact and successful round-trip drains. |
| **T1 / P1** cross-store accounting | Reuse ledger/control owner tests and an independent numeric cost oracle. | Two feature-gated cases live in **new** `facade/tests/cross_store_hellgate.rs`, preserving the original retained-map-only test slice. One shared global slot is contended by map/revocable stores; the second covers store+scratch+0-byte ticket through denied remap and legal raise. | At each quiescent point, exactly one admitted store, typed capacity denial, owner/class/context/kind totals and ticket count; release→peer handoff and final zero. |
| **T2 / P1** real charged restore fault | Backing file-cap and fake StorageFull tests remain owner-local controls. | Five Docker modes on separate disposable 8 MiB tmpfs volumes, including bounded charged spill and charged restore-commit. A feature-gated one-shot hook fills only the test volume after claim validation, without a production callback. | Real `VOLUME_ENOSPC` marker, charged restore failure or bounded uncertainty, exact fenced claim/charge, whole victim plus unrelated record after cold reopen, no readable duplicate, final inventory and ledger zero. Same-process reopen is not a power-loss claim. |
| **T4 / P1 proof gate** scaling verifier and receipt | Python negative fixtures independently enumerate 48 smoke / 220 full v3 cells, including custom dimensions, duplicate/missing/extra/truncated output and incompatible machine/toolchain/feature graph/dependency lock/schema baselines. Different baseline source SHA is allowed. | Runner retains raw JSONL on success/failure, records HEAD/tree and clean end-state, checks invocation-derived matrix; run smoke and full once on a clean committed candidate. | Old 51-row fixture is RED. Only exact matrices pass; incompatible baseline and incomplete run are RED. Current permissive regression thresholds are not a performance SLO. |
| **T5 / P2 measurement** honest workloads | Existing owner tests retain correctness authority for `get_checked`, dedup, cap denial and drain. | Benchmark v3 relabels dedup put and stats contention, adds actual retained-map hit/miss reads and a 3-topology × 2-size broker-release pairwise comparison; `retained_map` becomes a bench required feature. | Every sample's timed/success/denial counts, retry count, throughput denominator, semantic peak and final drain match the timed calls. Contention writes use collision-free IDs and assert the expected unique retained count; only compatible v3 baselines compare. |
| **T6 / optional** phase-change endurance | Existing bounded fuzz/model/reconfigure/callback controls remain. | Do not add a long random sweep to daily rails without a newly identified invariant or failure. | If later needed, use a seeded opt-in phase driver with bounded time, operations and disk; RSS is separate from semantic authority. |

## What the prior broad packet over-specified

- Extra restore-publication/finalize and unconditional spill commit→ACK process
  hooks repeat existing postimages. Add another hook only after naming a
  distinct reachable state and independent oracle.
- A "pinned store chooses another victim" integration test would test the
  **host reclaimer's selection policy**, not QMP. QMP already proves pin denial
  and broker callback bounds; test actual victim selection with the host.
- A new 100k-operation all-API model is not justified by existing 400-step and
  short property traces without a missing invariant. T6 is bounded and opt-in.
- More corruption/copy/claim tests are not automatic work: Redb has schema,
  lineage, usage-counter and concurrent-claim owner tests. Reopen only for a
  new reachable schema/version transition; T0's V1 fixture is the current one.
- Generic caller codec scratch is not the bounded typed codec's guarantee.
  Physical power loss, hard allocator OOM/abort and host root activation remain
  separate contracts; do not manufacture green QMP tests for them.

## Writer boundaries, rails and stop conditions

Order: **durable owner T0/T2 serially; cross-store T1 and benchmark T4/T5 on
disjoint files in parallel; then fixed-source integration**. The benchmark
producer and verifier coordinate one v3 scenario schema, but the Python
fixture's expected cells are independent of production emission.

Daily/focused: relevant owner-local selector with a **nonzero selected count**,
two facade E2E cases, Python verifier unit/negative tests, then
`scripts/check-workspace.sh` on the candidate. Release/opt-in: real SIGKILL
tests, five Docker ENOSPC modes, clean full bench, optional T6 soak
and the pre-existing Linux L2 syscall rail only when its failure model is
required. Cargo/feature changes additionally require the standalone extraction
dry-run on a committed clean candidate per repository policy.

Final receipt records HEAD/tree and source digest before/after, dirty
fingerprint, feature graph, exact commands and selected counts, test point,
seed, allowed/observed postimage, independent payload/ledger/pending oracle,
machine/toolchain, elapsed time, and raw benchmark data. A source change during
a run invalidates that run; a focused GREEN does not substitute for the
canonical rail, Docker/L1/L2, or host activation. Stop and reopen a packet if
its expected postimage needs a new product contract, an owner file changes
mid-run, or a real failure appears outside the enumerated model.
