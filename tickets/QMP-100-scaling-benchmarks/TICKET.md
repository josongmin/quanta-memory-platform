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
- `revocable_full_drain_has_linear_total_work`
- `feature_matrix_is_warning_free`
- `ledger_release_work_is_bounded_by_affected_slots`, 10K/100K live owners; core overflow negative fixtures preserve exact denial before/after optimization
- benchmark smoke가 실제 public builder/store path를 사용
- perf gate rejects intentionally regressed fixture

## Acceptance

- [ ] payload unique-fill이 O(N²) scan을 하지 않는다.
- [ ] revocable full drain이 O(N²) shift를 하지 않는다.
- [ ] public semantics/accounting invariants가 기존 conformance를 통과한다.
- [ ] benchmark results가 source SHA/toolchain/hardware와 묶인다.
- [ ] contention 및 memory drift scenario가 존재한다.
- [ ] bare/single-feature matrix가 `-D warnings`로 clean하다.
- [ ] docs의 performance claim이 실제 executable target과 일치한다.

## Verification

```bash
cargo test -p quanta-memory-platform-facade --all-features
cargo bench -p quanta-memory-platform-facade --all-features
tools/ci/feature-matrix.sh
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

PERF receipt가 없으면 코드가 빨라 보여도 ticket은 `QUALIFIED`가 아니다.
