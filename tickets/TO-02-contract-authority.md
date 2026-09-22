# TO-02 — Replace Source/Text Greps with Executable Contract Authorities

Status: `OPEN`
Priority: `P0`
Depends on: `TO-03` for `scaling.rs`

## Problem

현재 다음 테스트가 실제 Rust visibility/feature resolution이 아니라 파일 문자열과 문서
형식을 검사한다.

- payload, retained-map, revocable store의 `include_str!` source greps
- facade `tests/public_api_surface.rs`의 source/TOML/README greps

이 detector들은 주석/format 변경에 RED가 되고, re-export나 feature activation 같은 실제
위반을 놓칠 수 있다. 해결은 grep 문구를 갱신하는 것이 아니라 compiler와 Cargo graph를
권한으로 승격하는 것이다.

## Write set

- `crates/quanta-memory-platform-facade/src/payload_store/service.rs`
- `crates/quanta-memory-platform-facade/src/retained_map/service.rs`
- `crates/quanta-memory-platform-facade/src/revocable_store/service.rs`
- `crates/quanta-memory-platform-facade/src/lib.rs`
- `crates/quanta-memory-platform-facade/tests/public_api_surface.rs`
- `crates/quanta-memory-platform-facade/benches/scaling.rs`
- `scripts/verify-feature-matrix.sh`
- `scripts/verify-scaling-results.py`
- `scripts/tests/test_scaling_results.py`
- README/Cargo.toml은 product 계약 자체가 바뀌는 경우가 아니면 수정하지 않는다.

## File-by-file implementation plan

### `crates/quanta-memory-platform-facade/src/payload_store/service.rs`

- Delete only `source_has_no_public_raw_constructor_or_late_attach`.
- Keep the two `SimplePayloadStore` rustdoc `compile_fail` examples for
  `charged_against` and raw `new` construction unchanged.
- Do not change `new_budgeted`, `new_unbudgeted`, `with_accounting`, visibility, or store tests.
- File proof: facade doctest must select both snippets; temporary public constructor/late attach
  mutant must make the relevant compile-fail snippet compile and therefore fail rustdoc.

### `crates/quanta-memory-platform-facade/src/retained_map/service.rs`

- Extend `SimpleRetainedMapStore` rustdoc with two separate `compile_fail` examples:
  external call to private `new_budgeted`, and nonexistent `charged_against` late attach. Keep them
  separate so exposing one path cannot be masked by the other snippet still failing.
- Delete `retained_map_owner_has_one_keyed_container_and_no_linear_point_path`; do not replace it
  with another container-name grep.
- Keep `first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1` and its 4,096
  entry first/last get/overwrite/remove probe limits unchanged.
- In the poison test, delete `include_str!("mod.rs")` and the `unwrap_or_default` string assertion.
  Preserve the four typed `MemoryError::Poisoned` behavior assertions. Use UFCS through
  `RetainedMapStore` only if method-resolution ambiguity exists.
- Do not edit `MapState`, `LinkedHashMap`, entry choreography, or production point operations.

### `crates/quanta-memory-platform-facade/src/revocable_store/service.rs`

- Delete only `revocable_full_drain_uses_fifo_deque_without_front_shifts`.
- Preserve `reclaim_one_is_fifo_and_releases_its_bytes` plus spill/recovery ordering tests.
- Do not change `RevocableState`, `VecDeque`, `reclaim_one`, or production drain code in this
  ticket. The temporary quadratic mutant is verification-only and must be reverted.

### `crates/quanta-memory-platform-facade/src/lib.rs`

- Current conflict: 파일별 계획 보강 시점에 다른 작업자의 workspace-policy overlay가
  `#![forbid(unsafe_code)]`를 이동 중이다. 그 owner가 commit/release하기 전에는 edit 금지;
  이후 새 HEAD blob을 재동결하고 아래 rustdoc만 추가한다.
- Add two independent crate-level `compile_fail` snippets: one imports `AccountingSink`, the other
  imports `SharedAccountingSink` from the facade root. A combined snippet is forbidden because one
  still-private name could mask accidental exposure of the other.
- Do not add, rename, or re-export accounting types. Existing public re-export list is unchanged.

### `crates/quanta-memory-platform-facade/tests/public_api_surface.rs`

- Delete complete tests `accounting_mode_negative_contracts_remain_in_the_public_source` and
  `full_and_default_features_do_not_enable_unbudgeted_stores`.
- Remove now-unused imports only if the compiler identifies them; do not touch behavioral public
  builder/store tests.
- This file must contain no Cargo.toml/README parsing after the change.

### `scripts/verify-feature-matrix.sh`

- Extract the current `facade_full_tree` check into
  `assert_facade_escape_hatch_absent <label> <cargo-tree args...>`.
- Invoke it for consumer default, `--no-default-features --features full`, and
  `--no-default-features --features full-backings`.
- Match the resolved line `quanta-memory-platform-facade feature "unbudgeted-stores"`; do not parse
  Cargo.toml text. Print the label and full offending tree on failure.
- Keep bare-facade core default/live-control leak check and all clippy matrix combinations intact.
- `--all-features` is an intentional positive activation and must not be passed to this absence
  helper.

### `crates/quanta-memory-platform-facade/benches/scaling.rs`

- Edit only `revocable_fill_and_drain` after TO-03 lands.
- Move `let elapsed = started.elapsed()` from after fill to immediately after the reclaim loop.
  This makes elapsed/throughput cover one full insert+drain lifecycle per entry.
- Keep insert latency samples, operation name, JSON schema, semantic peak/final-zero assertions,
  allocation epoch, and close sequence unchanged.

### `scripts/verify-scaling-results.py`

- Add a validator for `revocable_fill_and_drain` groups with at least two entry cardinalities.
- For each size, use median `elapsed_ns / entries` across repetitions. Compare the largest size to
  the smallest; normalized per-entry cost may grow at most 4x when cardinality grows 10x.
- Cardinality span이 10x 이상일 때 이 law를 적용한다. one-size smoke나 10x 미만 custom
  artifact는 다른 검증에는 유효하지만 complexity contract를 close하지 못한다.
- Keep schema, allocator drift, required operation, matrix, baseline, and latency checks intact.
- Emit an error naming operation, small/large sizes, normalized medians, and allowed ratio.

### `scripts/tests/test_scaling_results.py`

- Extend fixtures so revocable samples exist at 1,000 and 10,000 entries with three repetitions.
- Add positive test: linear total elapsed (10x entries ≈10x elapsed) passes.
- Add negative test: quadratic total elapsed (10x entries ≈100x elapsed) fails with the new
  revocable growth error.
- Add smoke/single-cardinality test proving ordinary smoke validation still passes without claiming
  the growth contract.
- Do not weaken generic baseline regression tests.

## Producer → contract → consumer

1. Rust visibility producer: raw constructors, `charged_against`, accounting sink types.
2. Contract authority: public docs의 `compile_fail` snippets가 외부 crate 관점에서 import,
   raw construction, late attach를 시도한다.
3. Consumer proof: `cargo test --doc`가 forbidden API가 공개되면 snippet compile 성공으로
   전환되어 전체 doctest를 실패시킨다.
4. Feature producer: Cargo feature graph.
5. Contract authority: `cargo metadata`/`cargo tree -e features`의 resolved graph.
6. Consumer proof: default, `full`, `full-backings` 각각에
   `unbudgeted-stores`가 없음을 검증한다. `--all-features` 활성화는 허용한다.

## Implementation A — API visibility

- payload/revocable의 기존 compile-fail contracts를 keeper로 확인한 뒤 raw constructor,
  late attach, container-name source greps를 삭제한다.
- `SimpleRetainedMapStore`에 raw constructor와 late accounting attach가 외부에서
  불가능함을 각각 별도 compile-fail snippet으로 추가한다.
- facade root에서 `AccountingSink`/`SharedAccountingSink` forbidden re-export를 import하려는
  compile-fail snippet을 추가한다. 내부 파일명을 grep하지 않는다.
- `verify-feature-matrix.sh`의 cargo-tree 검사 helper를 default, `full`,
  `full-backings` 세 profile에 적용한다. TOML 문자열 parsing과 README 문구 assertion은
  제거한다.
- source file에 ```compile_fail 문자열이 존재하는지 재차 grep하는 meta-test는 만들지
  않는다.
- rustdoc으로 표현할 수 없는 계약이 확인될 때만 `trybuild`를 검토한다. 기본안에는 새
  dependency를 추가하지 않는다.

## Implementation B — Performance/poison contracts

- retained-map의 `retained_map_owner_has_one_keyed_container_and_no_linear_point_path` source
  grep은 삭제한다. O(1) point path의 keeper authority는 이미 존재하는
  `first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1`이다. first/last
  4,096-entry endpoint에서 get/overwrite/remove의 hash/equality probe bound를 보존한다.
- retained-map poison test의 `include_str!("mod.rs")`/`unwrap_or_default` grep도 삭제한다.
  같은 test가 `RetainedMapStore` trait method인 `get_checked`, `stats_checked`,
  `keys_checked`, `entries_checked`를 poisoned state에서 호출해 typed `Poisoned`를 받는
  behavior oracle를 keeper로 사용한다. 필요하면 UFCS로 trait dispatch를 명시한다.
- revocable의 `VecDeque`/`remove(0)` source grep은 삭제하되, 기존
  `revocable_fill_and_drain` benchmark의 `elapsed` 캡처를 drain loop 뒤로 옮겨 이름대로
  fill+drain end-to-end 비용을 측정하게 한다. FIFO와 exact accounting은 기존 unit tests가,
  복잡도 회귀는 cardinality-normalized growth verifier가 담당한다.
- benchmark operation name과 JSON schema는 바꾸지 않는다. TO-03의 allocation epoch
  phase를 먼저 반영한 뒤 같은 file을 수정한다.

내부 컨테이너 타입 이름 자체는 product contract가 아니다. compiler visibility,
bounded probes, typed failures, baseline-backed lifecycle throughput으로 분해해 각각의
권한에 맡긴다.

## Negative proof

- temporary mutant로 raw constructor 또는 `charged_against`를 public으로 만들면 해당
  compile-fail doctest가 RED여야 한다.
- temporary mutant로 `full`에 `unbudgeted-stores`를 포함하면 feature-matrix rail이 RED여야
  한다.
- temporary mutant로 retained-map point operations를 linear scan으로 바꾸면 probe-bound
  test가 RED여야 한다.
- temporary mutant로 revocable drain을 `Vec::remove(0)` 등 quadratic path로 바꾼 뒤
  full-size `1_000,10_000` normalized-growth rail이 RED여야 한다.
- mutant 제거 후 source grep 테스트가 0개인지 `rg`로 확인한다.

## Verification

```bash
cargo test -p quanta-memory-platform-facade --doc --all-features
cargo test -p quanta-memory-platform-facade --test public_api_surface --all-features
cargo test -p quanta-memory-platform-facade --all-features first_and_last_key_point_operations_share_one_n_independent_probe_bound_v1
scripts/verify-feature-matrix.sh
scripts/run-scaling-bench.sh --sizes=1000,10000
rg -n 'include_str!|read_to_string' crates/quanta-memory-platform-facade/src crates/quanta-memory-platform-facade/tests/public_api_surface.rs
```

마지막 `rg` 결과는 inventory일 뿐 합격 권한은 아니다. 남은 용례가 있으면 목적을 개별
분류한다.

## Acceptance / reopen

- forbidden API 공개와 forbidden feature activation의 temporary mutant가 각각 RED다.
- point-operation probe mutant와 quadratic drain mutant가 각각 행동/benchmark authority에서
  RED다.
- README wording만 바꿔도 rail이 실패하지 않는다.
- public API 계약을 compiler 대신 source text로 다시 검사하면 reopen한다.
