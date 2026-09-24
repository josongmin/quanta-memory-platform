# QMP adversarial coverage: in-memory scope

Status: **T1 and T4/T5 implemented; current-source qualification is determined by the exact-target local receipt tag**.
At the clean `db0b03c` audit base, the two T1 cross-store tests passed 2/2,
the verifier's Python contracts passed 20/20, and the prior 48-row smoke
and 220-row full artifacts passed the current verifier. Those raw runs were
produced at `7f1c2f8`; `db0b03c` changes verifier outcome-count validation.
A later local clean `db0b03c` smoke artifact at
`target/scaling-results/qmp-scaling.KorxZf/results.jsonl` passed the revised
verifier 48/48 (SHA-256 `502f6e70a2009f6e1c725a72a9167db8895a25117bb4facfc738fd7532b4f8a4`).
It is ignored local evidence, not a durable full-run receipt. Later committed
product behavior changes after `db0b03c` are outside this plan; the later
`98dce1f`/`267cdc9` commits fixed the nested Cargo invocation in crash-test
harnesses. Current source-wide
status is defined by the exact-target annotated `qmp-local-qualified-*` tag;
it must bind the fresh canonical rail, full benchmark, and extraction dry-run
to one clean candidate. See the
[plan status index](../README.md) and [FUP-006 receipt](../sep-23-followup-misc/RECEIPT.md).

### Current-source full-run attempts (2026-09-24)

Both attempts used clean source `fdf34cf057087226c64cea59966e152c8ed2a369`
(tree `7cc3209ff9d7c12b27af8b2d609b87b993f81a85`) and emitted all 220 rows.
The verifier rejected the `revocable_fill_and_drain` 1k→10k growth ratio:

| Raw artifact | SHA-256 | Result |
| --- | --- | --- |
| `target/scaling-results/qmp-scaling.pCLPBs/results.jsonl` | `e16b7531020099db4fa24460fe487b5a977ff6f36f8568832ad2642afdefb0b3` | 9.80x; allowed 4.0x |
| `target/scaling-results/qmp-scaling.tNisZk/results.jsonl` | `5ef094c22a29c82b67f020653236821e20d3386a41348fecb5bf6c448c0a39b3` | 12.25x; allowed 4.0x |

Both run-end records report `source_clean=true` and unchanged HEAD/tree. During
the attempts the shared host had concurrent RustRover analysis and Cargo/rustc
builds; load averages were observed around 90–143. This is a plausible
measurement confounder, not proof that host contention caused either failure
or proof of a product regression. Preserve both artifacts; do not loosen the
verifier threshold on this evidence. Rerun the full bench on the final clean
candidate after competing builds stop. Until a fresh run passes alongside the
canonical and extraction rails, no `qmp-local-qualified-*` tag is issued.

The later tagged candidate `267cdc9` passed 220/220 samples (artifact SHA-256
`ce5be25d5766961f522aa4ca4fa1b50f3a991bd32baaa84e6f6dff2dbe69b558`). After
the closeout-status documentation commit, source `2dc071a48445e21e51bfa42f8e1dd7b049d98322`
(tree `8a911b2eefffee1d81f27239aacda4e7dfa29ec5`) emitted all 220 rows but
failed the same growth verifier at 4.61x. Its run-end record reports
`source_clean=true`; raw artifact
`target/scaling-results/qmp-scaling.8HdqxG/results.jsonl` has SHA-256
`335bedd9c787407ae96b0f09e8cb354618677007d945dad79abcbf0964c6a920`.
Other Cargo/rustc work remained active. As with the earlier misses, contention
is a plausible confounder, not an established cause. The 267 receipt is valid
for that commit only; this newer source needs its own successful exact-head
benchmark and receipt tag.

The active plan excludes Redb-backed durable spill, its codec, and disk
fault/recovery qualification. The 2026-09-24 candidate based on
`b6d162650edba4f2b32e5020c8f0ac707efb0c22` produced historical QMP-only
evidence for those optional paths; that evidence is not a current release or
host gate. In-memory cross-store E2E, benchmark verifier, 48-row smoke,
220-row full bench, and the canonical workspace rail remain relevant to this
plan. Publication and exact-source host adoption are separate.

Semantica's reconstructible revocable entries may disappear on reclaim or
restart and are rebuilt from an independent authoritative source. Do not add
Redb persistence, a durable codec/SDK surface, or a Redb host release gate for
them. The existing opt-in code and historical receipts remain in the repository
but are not action items. A future disk-backed proposal needs a named consumer,
measured benefit, cache identity/invalidation, and an owned failure model before
it can enter a new plan. This plan covers portable in-memory contracts, not
host cache identity, root activation/GC, async cancellation, allocator abort/OOM,
or physical power loss.

## Final audit: what already proves the boundary

| Boundary | Current evidence | Decision |
| --- | --- | --- |
| Ledger, 0-byte tickets, cap axes, group remap, reconfigure/reclaim, callback reentry | `core/src/domain/ledger.rs`, `core/src/api/control.rs`: independent axis checks, 0-byte scope tests, concurrent reserve/reconfigure, single-flight callback, panic/partial-progress recovery. | **No blanket new core test suite.** Add an owner-local selector only if a new cross-store history exposes a previously untested transition. |
| One charged store with broker | `facade/tests/hellgate_e2e.rs`: 18 tests; charged retained-map/payload, denial, rebuild, reconfigure, one-store contention and drain. | New `cross_store_hellgate.rs` adds only the distinct two-store broker history and store/scratch/0-byte remap history. |
| Scaling bench | `scaling.rs` labels real timed calls, adds retained-map point reads and a bounded three-topology pairwise comparison. `verify-scaling-results.py` enforces complete v3 rows. | Old full-mode 1k-only **51-row** fixture must be RED; clean smoke requires 48 rows and full requires 220, with raw artifact and compatible baseline metadata. |

## Implemented packets and remaining proof gates

Only the following packets add coverage. Owner-local oracles test transitions;
integration runs only where two product owners interact or a benchmark needs a
clean source-bound receipt.

| Packet / gate | Cheapest owner-local work | Necessary integration/E2E work | Exact acceptance oracle |
| --- | --- | --- | --- |
| **T1 / P1** cross-store accounting | Reuse ledger/control owner tests and an independent numeric cost oracle. | Two feature-gated cases live in `facade/tests/cross_store_hellgate.rs`, preserving the original retained-map-only test slice. One shared global slot is contended by map/revocable stores; the second covers store+scratch+0-byte ticket through denied remap and legal raise. | At each quiescent point, exactly one admitted store, typed capacity denial, owner/class/context/kind totals and ticket count; release→peer handoff and final zero. |
| **T4 / P1 proof gate** scaling verifier and receipt | Python negative fixtures independently enumerate 48 smoke / 220 full v3 cells, including custom dimensions, duplicate/missing/extra/truncated output and incompatible machine/toolchain/feature graph/dependency lock/schema baselines. Different baseline source SHA is allowed. | Runner retains raw JSONL on success/failure, records HEAD/tree and clean end-state, checks invocation-derived matrix; prior clean smoke/full passed at `7f1c2f8`. Repeat on a clean current candidate after the verifier change. | Old 51-row fixture is RED. Only exact matrices pass; incompatible baseline and incomplete run are RED. Current permissive regression thresholds are not a performance SLO. |
| **T5 / P2 measurement** honest workloads | Existing owner tests retain correctness authority for `get_checked`, dedup, cap denial and drain. | Benchmark v3 relabels dedup put and stats contention, adds actual retained-map hit/miss reads and a 3-topology × 2-size broker-release pairwise comparison; `retained_map` becomes a bench required feature. | Every sample's timed/success/denial counts, retry count, throughput denominator, semantic peak and final drain match the timed calls. Contention writes use collision-free IDs and assert the expected unique retained count; only compatible v3 baselines compare. |
| **T6 / optional** phase-change endurance | Existing bounded fuzz/model/reconfigure/callback controls remain. | Do not add a long random sweep to daily rails without a newly identified invariant or failure. | If later needed, use a seeded opt-in phase driver with bounded time, operations and disk; RSS is separate from semantic authority. |

## What the prior broad packet over-specified

- A "pinned store chooses another victim" integration test would test the
  **host reclaimer's selection policy**, not QMP. QMP already proves pin denial
  and broker callback bounds; test actual victim selection with the host.
- A new 100k-operation all-API model is not justified by existing 400-step and
  short property traces without a missing invariant. T6 is bounded and opt-in.
- Physical power loss, hard allocator OOM/abort and host root activation remain
  separate contracts; do not manufacture green QMP tests for them.

## Writer boundaries, rails and stop conditions

Implementation order was **cross-store T1 and benchmark T4/T5 on disjoint
files, then fixed-source integration**. For requalification, freeze one clean
source candidate before the canonical and benchmark rails. The benchmark
producer and verifier coordinate one v3 scenario schema, but the Python
fixture's expected cells are independent of production emission.

Daily/focused: relevant owner-local selector with a **nonzero selected count**,
two facade E2E cases, Python verifier unit/negative tests, then
`scripts/check-workspace.sh` on the candidate. Release/opt-in: clean full bench
and optional T6 soak. Cargo/feature changes additionally require the standalone
extraction dry-run on a committed clean candidate per repository policy.

Final receipt records HEAD/tree and source digest before/after, dirty
fingerprint, feature graph, exact commands and selected counts, test point,
seed, allowed/observed postimage, independent payload/ledger/pending oracle,
machine/toolchain, elapsed time, and raw benchmark data. A source change during
a run invalidates that run; a focused GREEN does not substitute for the
canonical rail or host activation. Stop and reopen a packet if its expected
postimage needs a new product contract, an owner file changes
mid-run, or a real failure appears outside the enumerated model.
