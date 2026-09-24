# Current-source audit supporting SMA tickets

이 문서는 아래 baseline의 역사적 근거다. 현재 실행 목록은 [README](README.md)가 소유한다.
E02의 기본 report/별도 stats sampling은 이후 소스에서 수정되어 신규 작업에서 제외됐다.
남아 있는 overlay 문제와 이미 구현된 live 경로를 구분한다.

감사일: 2026-09-24. 방법: 저장된 소스 읽기와 명시된 범위의 exact-text census.
이번 티켓 작성에서 Rust/QBC/부하 테스트는 실행하지 않았다.
QMP baseline: `a1179a10c5569619210cb1c03ceeae7430109c25`.
Semantica baseline: `dfc607d9b714da163b0b6564abf67fec05457a7e`.
Semantica에는 unrelated shared dirty changes가 있다. 아래 SHA-256은 읽은 파일의 내용에 대한
snapshot이며 repo 전체 clean receipt가 아니다.

경로 표기: `S/`는 Semantica repo root,
`C/`는 `S/packages/analysis/quanta-v2/crates/`,
`Q/`는 QMP repo root. line은 이 감사 시점 anchor다.

| Evidence | 현재 소스 사실 | 판정 / 티켓 |
| --- | --- | --- |
| E01 | `C/quanta-runtime/src/sdk/code/mod.rs:1151,1170,1238`: selection 저장/보고, query builder에는 selection 없음. `session_selector.rs`와 `analysis_session.rs`도 전달/보고 용도. | source-confirmed 계약 문제. 실제 초과 query 재현은 미실행. SMA-001 |
| E02 | `C/quanta-runtime-memory-compose/src/lib.rs:1011,1049,1091`: 보고서 projection과 live ledger snapshot/build 경로가 별도. `C/quanta-sdk-runtime-executor/src/runtime_root.rs:392`: builder마다 bootstrap. | 기본 budget 집행을 부정하지 않음. report/domain 확인은 SMA-001 |
| E03 | `rg -n 'reserve_scratch_scope\|check_output_bytes\|check_execution_scratch_bytes' crates packages -g '*.rs'`에서 hit 없음. 기존 gateway execution rail의 `max_output_bytes`는 별도 경로. | 제품 QMP 실행 축 집행 미확인. SMA-002 |
| E04 | `C/quanta-runtime-ingress-core/src/memory_budget_v1.rs:1`: output/scratch None을 disabled/truncated로 설명. `Q/crates/quanta-memory-platform-core/src/domain/policy.rs:46`: None cap은 unlimited. | 연결 전 해결할 semantic mismatch. 현재 런타임 오류 재현은 없음. SMA-002 |
| E05 | `C/quanta-memory-host/src/runtime/memo_residency.rs`는 별도 FIFO byte store. `memo_residency(` 검색은 port 선언/구현/자체 test만. 실제 `memo_table.rs:99`는 행과 여러 locator/index 소유. | 독립 mirror를 채우는 방식은 채택하지 않음. SMA-003 |
| E06 | `C/quanta-runtime-search/src/production_read_query_host/ide_result_cache.rs:10`: 4개 slot, slot별 64 MiB/256 ready entries, 16 flights/256 waiters. `workspace_symbol_index.rs:16`: aggregate 128 MiB, build 8; `:703` per-file symbols copy. | 이미 bounded. 합산 peak와 동시 build 측정 필요. SMA-005 |
| E07 | `C/quanta-bench/src/process_metrics.rs:37`: Linux VmHWM, macOS ps RSS. 후자는 high-water mark가 아님. `isolated_runner.rs:160`는 blocking output; 해당 child call에 timeout 전달 없음. | 이 helper/runner를 채택할 경우 성능 proof 수정 필수. 제품 장애로 분류하지 않음. SMA-004 |
| E08 | QMP `api/control.rs`에 pending/managed charge, `ports/scratch.rs`에 RAII scratch가 있음. `ports/reclaim.rs:33`에는 internal `domain/admission.rs`의 breach axis/deficit이 없음. | 기존 원시 연산 우선. 실제 caller가 필요할 때 SMA-006 |
| E09 | QMP `docs/rfcs/0001-sdk-interface-contract.md:3,59`는 0.3 not implemented, scope attribution only. `README.md:18`는 Redb current product excluded. | 0.3 전체와 Redb는 이 adoption의 선행 조건 아님. SMA-008 |
| E10 | `C/quanta-runtime-memory-compose/src/lib.rs:1152` checked conversion + candidate manifest commit, `lib_tests.rs:1426` overflow regression이 HEAD에 있음. | 재구현 금지. 필요한 owner QBC receipt는 SMA-009에서 확인 |

E03/E05는 literal census이며 compiler-resolved call graph가 아니다.
alias/외부 host가 있다면 dispatch에서 재검증한다. E06 owner-local test 소스가 있다고 해서
current-source runtime GREEN이라고 주장하지 않는다.
Salsa 의존성 부재 및 parser/PTA/trace 제품 후보는 직전 감사 결과를 바탕으로 한 후보 목록이며,
007에서 현재 feature/consumer를 다시 확인한 뒤 구현을 선정한다.

## 실측 전 결론을 내리지 않을 항목

- 여러 runtime의 실제 제품 동시 co-location 및 process-wide cap 필요성;
- memo metadata, parser, IDE duplicate symbols가 차지하는 실제 peak 비율;
- QMP 연결에 따른 p95/lock contention 변화;
- disk reuse의 순이득과 필요한 durability;
- generic isolated runner의 실제 benchmark dispatch 여부.
  “helper가 있음”은 채택/제품 성능 proof가 아니다.

## Semantica source SHA-256

```text
f9dd367677a9238c2d75d1d34bf6aaee475390244144c0f2f153315996db67b4  packages/analysis/quanta-v2/crates/quanta-runtime/src/sdk/code/mod.rs
74f77e989d5d906d0c3060b6b803fff44168384af53a282c7a826105531cbe74  packages/analysis/quanta-v2/crates/quanta-runtime/src/sdk/memory_budget.rs
2b0f79e5cd01c6683640039351205b084804bdbe1139999842c55f2704b93421  packages/analysis/quanta-v2/crates/quanta-runtime-ingress-core/src/memory_budget_v1.rs
106d694b0478f0cee9933e3b2584b6521dd00f57730e339ea5bd7299a441896c  packages/analysis/quanta-v2/crates/quanta-runtime-memory-compose/src/lib.rs
7dd7592810fbcbde24fe2c0f4160577a34910925dd2e15a398a3dff88a6ce0dc  packages/analysis/quanta-v2/crates/quanta-runtime-memory-compose/src/lib_tests.rs
2eb01b683cc4cbce95b75857554e7856b62f76d85373e14a2ea2f673a40dac37  packages/analysis/quanta-v2/crates/quanta-sdk-runtime-executor/src/runtime_root.rs
f17ead2292fd3bbe040cc65bf541663602ee728e909a6aa2311f46648be5b7b7  packages/analysis/quanta-v2/crates/quanta-adapters-in-memory-shared/src/in_memory/core/memo_table.rs
1a591614a392b4f90fd958e4d4a1bcb4c2e672ae4ef6544671a8ea8e30db2c4b  packages/analysis/quanta-v2/crates/quanta-memory-host/src/runtime/memo_residency.rs
15a80315e7b0761e0cb8a87d46f945836f14792a3b5d5bf7a712d8ec20ba3c67  packages/analysis/quanta-v2/crates/quanta-runtime-search/src/production_read_query_host/ide_result_cache.rs
da8a45ff475d58d9f78ea6d30e421dec083caeff2a00743d64b80d30798bfdf4  packages/analysis/quanta-v2/crates/quanta-runtime-search/src/production_read_query_host/workspace_symbol_index.rs
423ad475d7234c708f8b6f87832272ab02f3f88df809ccf433c88fce1ed5de91  packages/analysis/quanta-v2/crates/quanta-bench/src/process_metrics.rs
f23df4d61a613b4c73adf5d524fd206accf45e4db66d4172caebac15adde2667  packages/analysis/quanta-v2/crates/quanta-bench/src/isolated_runner.rs
```

## 설계 참고 (현 소스의 검증 증거와 별개)

- [rustc incremental cache](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html):
  cross-process stable identity.
- [Gradle build cache](https://docs.gradle.org/current/userguide/build_cache.html):
  완전한 input/output 선언과 잘못된 hit 방지.
- [SQLite WAL](https://www.sqlite.org/wal.html):
  설정된 sync 수준과 power-loss durability를 분리.
- [Salsa tuning](https://salsa-rs.github.io/salsa/tuning.html):
  entry-count LRU 자체가 byte budget/RSS cap을 의미하지 않음.

이 자료로 특정 backend가 현재 workload에 가장 빠르다고 판정하지 않는다.
