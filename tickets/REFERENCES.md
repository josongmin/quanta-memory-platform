# Design references and required implications

외부 reference는 코드를 복제하기 위한 것이 아니라 ticket acceptance의 최소 설계 기준을 정하기 위해 사용한다.

## Memory control semantics

### Linux cgroup v2 memory controller

Source: https://docs.kernel.org/admin-guide/cgroup-v2.html

Required implications:

- protection, pressure threshold, hard limit, weight는 서로 다른 의미다.
- `reserved/soft/hard/burst`를 단순 validation vocabulary로 두지 않는다.
- finite allocation guarantee는 overcommit 여부와 invalid configuration 거부 규칙이 필요하다.
- weight share는 가능한 한 work-conserving이어야 한다.

Owning ticket: QMP-080.

## Reservation ownership

### Apache DataFusion MemoryPool / MemoryReservation

Source: https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/trait.MemoryPool.html

Required implications:

- allocation 전에 grow/reserve한다.
- failed grow는 allocation/usage를 증가시키지 않는다.
- shrink/release는 reservation owner와 결합한다.
- spillable/unspillable consumer 의미가 분리돼야 한다.

Owning tickets: QMP-020, QMP-050, QMP-070.

### Tokio Semaphore permits

Source: https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html

Required implications:

- capacity 권한을 clone 가능한 숫자 ticket가 아니라 owned guard로 표현한다.
- abandoned pre-commit reservation은 drop 시 rollback한다.
- committed resource는 명시적 ownership transfer/release를 거친다.

Owning ticket: QMP-020.

## Type-system API enforcement

### Rust API Guidelines

Sources:

- https://rust-lang.github.io/api-guidelines/dependability.html
- https://rust-lang.github.io/api-guidelines/type-safety.html
- https://rust-lang.github.io/api-guidelines/documentation.html

Required implications:

- “builder를 사용해야 한다”는 prose가 아니라 visibility/typestate로 강제한다.
- charged와 unbudgeted mode는 혼합 가능한 runtime flag가 아니라 다른 construction state다.
- public examples는 enforcement와 failure behavior를 보여야 한다.

Owning tickets: QMP-030, QMP-040, QMP-140.

## Concurrency verification

### Tokio Loom

Source: https://github.com/tokio-rs/loom

Required implications:

- ticket transition, concurrent reserve/release/reconfigure, spill transfer의 가능한 interleaving을 작은 model로 검사한다.
- ordinary stress test를 exhaustive concurrency proof로 표현하지 않는다.

### Rust Miri

Source: https://github.com/rust-lang/miri/

Required implications:

- aliasing, use-after-free, data race 가능 경로를 별도 execution rail로 검사한다.
- Miri의 특정 execution 통과를 모든 interleaving proof로 승격하지 않는다.

Owning tickets: QMP-090, QMP-100.

## Panic and destructor boundaries

### Rust `Mutex`, `catch_unwind`, and `Drop`

Sources:

- https://doc.rust-lang.org/std/sync/struct.Mutex.html
- https://doc.rust-lang.org/std/panic/fn.catch_unwind.html
- https://doc.rust-lang.org/std/ops/trait.Drop.html

Required implications:

- A client callback panic under a store mutex can poison subsequent access. Poison is advisory, not a transactional rollback mechanism; ignoring it alone does not prove a sound postimage.
- `catch_unwind` catches unwinding panics, not aborting panics. Do not present it as containment of every client destructor or a double panic during cleanup.
- The returned `Err` owns a panic payload whose `Drop` may itself panic. Treat disposal of that payload as another exceptional ownership transition; ordinary string-payload tests do not cover it.
- Retire independently owned generic fields in separate unwind boundaries after publishing the exact charge/state postimage. Test destructive leaves in a child process so an abort is observable instead of killing the whole suite.
- Decide whether arbitrary client callback panics are a supported recoverable case; otherwise make the public contract and residual availability risk explicit.

Owning tickets: QMP-040, QMP-090, QMP-140.

## Public compatibility and feature semantics

Sources:

- Cargo SemVer: https://doc.rust-lang.org/cargo/reference/semver.html
- Cargo features: https://doc.rust-lang.org/cargo/reference/features.html
- Cargo rust-version: https://doc.rust-lang.org/cargo/reference/rust-version.html

Required implications:

- root re-export/constructor 제거는 compatibility change로 관리한다.
- feature 이름은 실제 reachable capability를 제공해야 한다.
- extracted artifact는 explicit MSRV/lockfile/release policy를 가진다.

Owning tickets: QMP-070, QMP-120, QMP-140.

## Metrics cardinality and aggregation

Sources:

- Prometheus naming/cardinality: https://prometheus.io/docs/practices/naming/
- OpenTelemetry Metrics SDK: https://opentelemetry.io/docs/specs/otel/metrics/sdk/

Required implications:

- owner ID, runtime ID, arbitrary key를 unbounded label로 내보내지 않는다.
- denial/reclaim/spill/reconfigure outcome은 bounded enum labels로 집계한다.
- observation loss, overflow aggregation, double-count/drop semantics를 test한다.

Owning ticket: QMP-110.

## Reproducible build/extraction

Sources:

- Cargo build `--locked`/`--frozen`: https://doc.rust-lang.org/cargo/commands/cargo-build.html
- SLSA provenance: https://slsa.dev/spec/v1.2/

Required implications:

- actual extraction은 source manifest, lockfile, tool config를 모두 포함한다.
- verification 실패 전에 invalid bootstrap commit을 만들지 않는다.
- source/extracted digest와 toolchain을 receipt에 기록한다.

Owning tickets: QMP-120, QMP-130.
