# QMP-100 — Data-structure scaling and benchmark authority

Priority: P2  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-040, QMP-050, QMP-070  
Blocks: QMP-130, QMP-140

## Objective

확정된 O(N²) 경로를 canonical container에서 제거하고, throughput/latency/contention/memory drift를 반복 가능한 benchmark와 threshold로 관리한다.

## Confirmed evidence

- payload store는 `Vec`를 사용하고 distinct insert마다 전체 scan: `payload_store/service.rs:40-42,174-176`
- pin/delete도 linear scan.
- revocable full drain은 반복 `Vec::remove(0)`: `revocable_store/service.rs:144-153`
- 각 slice는 global mutex 하나로 직렬화.
- repository에 `benches/`, Criterion, IAI-callgrind, fuzz/loom target이 없다.
- `docs/verification-scenarios.md`는 존재하지 않는 benchmark/allocation gate를 주장한다.
- QMP-020 exactness integration `59f0c4f`에서 ledger single release/rollback/batch는 전체 class/owner/context map clone과 full `validate_exact()` 두 번을 broker mutex 아래 수행한다 (`core/src/domain/ledger.rs:591,663-684`). broker scheduler도 clone한다 (`api/control.rs:624-643`). 기존 reserve에도 snapshot full-map work가 있다. N live owner의 N개 release는 O(N²) work 가능성이 있지만 실제 10K/100K latency는 **미측정**이다. 정확성 guard를 제거하는 대신 affected-slot/aggregate checked preflight를 설계한다.

## Canonical write set

- payload/revocable container implementation
- facade `benches/**`
- workspace dev dependencies / benchmark profiles
- benchmark scripts and result schema
- feature matrix warning policy

Accounting semantics를 바꾸면 QMP-040/QMP-050/QMP-070 owner 검토가 필요하다.
Core ledger/scheduler의 exactness guard를 최적화할 때는 QMP-020 owner 검토와 동일한 overflow/denial/rollback 오라클이 필수다.

Owner structs/hot paths:

- `PayloadState::entries` and `PayloadStore::put_canonical`
- payload pin/delete lookup paths
- revocable entry queue and `RevocableStore::reclaim_one`
- store mutex boundaries measured by public operations
- `tools/ci/feature-matrix.sh::check_combo/check_crate`
- `core/src/domain/ledger.rs::{reserve,preflight_release_batch,release_batch}`와 broker scheduler candidate paths

## Required implementation

### Payload

- canonical key/digest → entry index를 위한 hash index 도입.
- digest collision 시 full key equality로 정확성 보존.
- deterministic payload ref와 enumeration semantics 유지.
- delete 시 index와 entry storage를 원자 갱신.

### Revocable

- FIFO reclaim을 O(1) amortized로 만드는 `VecDeque` 또는 equivalent queue 사용.
- key lookup/revoke 요구가 있으면 canonical index 하나만 둔다.
- accounting receipt와 queue/index ownership이 분리되지 않게 한다.

### Concurrency

- sharding/lock-free 구조는 single-lock correctness baseline 후 도입한다.
- lock sharding 시 cross-shard total cost/close/sweep ordering을 명시한다.
- callback-outside-lock protocol과 충돌하지 않아야 한다.

### Core exactness hot path

- 전체 BTreeMap 복제를 매 reserve/release마다 반복하는 대신 global aggregate와 변경된 class/owner/context/kind/group slot의 checked delta를 한 번에 preflight한다.
- 성공 후 publish하는 postimage가 `validate_exact()`과 같은 invariant를 만족하는지 property/model tests로 고정한다. 임시로 검증을 끄거나 saturating arithmetic으로 복귀하지 않는다.
- 10K/100K live owners와 group cardinality를 바꾼 reserve/release/rollback/recharge throughput 및 p95/p99 lock latency를 수정 전후 동일 source/toolchain/hardware에서 측정한다.

## Benchmark matrix

### Sizes

- entries: 1K / 10K / 100K / 1M 가능한 범위
- payload sizes: 0 / 64 B / 4 KiB / 1 MiB
- key sizes: 16 B / 256 B / 64 KiB
- hit ratios: 0% / 50% / 99%

### Operations

- payload unique insert, dedup hit, get/pin/delete
- retained map put/get/overwrite/remove/sweep
- revocable insert/reclaim full drain
- core broker reserve/release/rollback/recharge, owner/group cardinality sweep
- transcript append/truncate/tombstone saturation
- spill/restore and redb reopen
- store close/drain

### Contention

- 1 / 2 / 8 / 32 threads
- read-heavy, write-heavy, mixed
- p50/p95/p99 latency와 throughput

### Memory

- semantic charged bytes
- allocation probe/RSS delta
- drift ratio와 high-water mark

## Threshold policy

1. frozen toolchain/hardware metadata와 baseline을 먼저 저장한다.
2. correctness change와 optimization 결과를 분리한다.
3. absolute threshold와 baseline regression percentage를 함께 기록한다.
4. flaky wall-clock 하나로 merge gate를 만들지 않는다.
5. IAI/instruction metric 또는 sufficiently repeated distribution을 사용한다.

## RED/perf tests

- source/behavior test: `payload_lookup_probe_count_is_n_independent`
- `revocable_full_drain_uses_fifo_deque_without_front_shifts`
- `feature_matrix_is_warning_free`
- `ledger_release_work_is_bounded_by_affected_slots`, 10K/100K live owners; core overflow negative fixtures preserve exact denial before/after optimization
- benchmark smoke가 실제 public builder/store path를 사용
- perf gate rejects intentionally regressed fixture

## Acceptance

- [x] payload unique-fill이 O(N²) scan을 하지 않는다.
- [x] revocable full drain이 O(N²) shift를 하지 않는다.
- [x] public semantics/accounting invariants가 기존 conformance를 통과한다.
- [x] benchmark results가 source SHA/toolchain/hardware와 묶인다.
- [x] contention 및 memory drift scenario가 존재한다.
- [x] bare/single-feature matrix가 `-D warnings`로 clean하다.
- [x] docs의 performance claim이 실제 executable target과 일치한다.

## Verification

```bash
cargo test -p quanta-memory-platform-facade --all-features
scripts/run-scaling-bench.sh --smoke
tools/ci/feature-matrix.sh
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

PERF receipt가 없으면 코드가 빨라 보여도 ticket은 `QUALIFIED`가 아니다.

## Implementation checkpoint (2026-09-21)

At `0c126a25aeab11f457e20c4a209fc7bb631ea859`:

- payload entries use stable `Vec<Option<Entry>>` slots plus collision-safe
  canonical/reference digest buckets. Deleted slots are reused. Duplicate,
  pin/count and delete operations inspect only the digest bucket and still
  compare exact client values before acting.
- revocable FIFO ownership uses `VecDeque`; normal append, rollback and drain
  are `push_back`, `push_front` and `pop_front` respectively.
- `benches/scaling.rs` exercises public budgeted builders for payload unique/
  dedup and revocable fill/drain. `scripts/run-scaling-bench.sh --smoke` rejects
  a dirty source and emits source SHA/tree, toolchain and machine metadata.
- the executable smoke is part of the canonical standalone extraction file set.

The local 1K/10K single-thread samples are recorded in
[`receipts/2026-09-21T072928Z-0c126a2.md`](receipts/2026-09-21T072928Z-0c126a2.md).
They are observation, not a portable threshold.

At `6d082ee71226b687b2fb500e1cba767d48457f48`, QMP-100 is `QUALIFIED`:

- ledger reserve/release uses exact aggregates and affected-slot preflight;
  `single_release_preflight_is_bounded_by_affected_slots_at_10k_owners` pins
  one class/owner/context probe each regardless of owner cardinality;
- scheduler floor accounting is incremental and reserve/release/rollback/batch
  no longer clone the complete scheduler state;
- the v2 benchmark runs 1/2/8/32-thread read-heavy, write-heavy and mixed
  distributions with five repetitions at 1K/10K, plus the public broker
  owner-cardinality release lane;
- each sample records p50/p95/p99, throughput, busy retries, semantic peak and
  post-close bytes, allocator high-water, exact allocated/deallocated bytes,
  residual live bytes and underflow state;
- `scripts/verify-scaling-results.py` rejects incomplete matrices, fewer than
  three repetitions, semantic/allocator drift, absolute p99 regressions and
  supplied-baseline regressions. Negative fixtures prove every gate bites.

The exact-source qualification receipt is
[`receipts/2026-09-21T095245Z-6d082ee.md`](receipts/2026-09-21T095245Z-6d082ee.md).

## Session-hardening checkpoint (2026-09-21)

At `951ebe403783fd326ba59834ada860ec80e9650b`, a source audit found one
remaining admission-cardinality defect that the earlier qualification did not
measure: every `WeightedResourceGroupScheduler::admits` call recomputed total
weight, allocated a remainder vector and sorted all groups. Hamilton allocation
depends only on installed policy, so the scheduler now computes it once during
construction/reconfiguration and performs cached lookups on admission. Public
`record_usage`/`record_release` also reject unknown groups before they can grow
the internal usage map.

At `f2b74979b9d0aa4d2cd957d4d86f13d18865ae23`, the Rust 1.92 performance
rail was repaired after reproducing two independent proof defects: a false
64-byte residual from the tracking allocator's own process state and a TLS
allocator abort at the first contention lane. The benchmark now uses an
allocation-free atomic `GlobalAlloc` wrapper, creates and retires all measured
objects inside each epoch, and includes a required scheduler group-cardinality
lane. The verifier rejects a missing scheduler lane. The current smoke contains
33 samples; every sample has zero semantic and allocator residual bytes.

Current receipt:
[`receipts/2026-09-21T131415Z-557213b.md`](receipts/2026-09-21T131415Z-557213b.md).

At `557213b01ed93a20c1a45d3c69b2dae20ab90e70`, the scheduler-cardinality
lane also snapshots allocator counters immediately before the admission loop
and fails on any allocation or deallocation inside it. This closes the proof
gap where an implementation could rebuild and free the Hamilton table per
request while still ending with zero residual bytes. Qualification was rerun
from a fresh Cargo target after a stale optimized artifact was detected; the
stale run is not accepted as evidence.
