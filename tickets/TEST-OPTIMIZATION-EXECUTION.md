# Test Optimization — Structural Remediation Queue

Status: `IMPLEMENTED; exact-source repository/extraction candidate verified; per-ticket mutation receipts and host-pair qualification remain open`

이 문서는 `docs/bugbash/sep-22-test-optimization/`에서 실제 문제로 남긴
9개 항목의 실행 SSOT다. 구현은 현재 `main` 열에 합쳐졌지만, 각 티켓의
temporary-mutant RED 증적과 최종 host-pair 자격은 아직 종결 조건이다. 기존
`tickets/FINAL-CLOSEOUT.md`의
`qmp-final-qualified` historical receipt를 수정하거나 소급 무효화하지 않는다.
아래 구현이 들어가면 별도의 exact-head qualification 영수증을 새로 발급한다.

## Source freeze

- Audit base: `main@d36f29a9030a740e0c872181bc0af5b79f2317e3`
- Original measurement snapshot: `101781c2ea6122602b4acd9aa69ebfc13fda958f`
- Open-ticket owners were unchanged from the measurement snapshot through the committed audit
  base except `redb.rs`: `014eb65` changed only its test DB fixture for resolved TO-10 while the
  TO-01 concurrency test body remained present.
- `014eb65`가 원래 TO-10을 해결했다. test별 PID+nonce directory, RAII cleanup, backing
  drop/reopen 검증이 들어가 수동 cleanup과 cross-process path collision이 제거됐다.
  따라서 TO-10은 open queue에서 제외한다.
- 티켓 파일은 `4aff891`에서 tracked 상태가 됐지만 audit base는 tested head 또는
  clean-tree 증명이 아니다. 구현/qualification은 각 ticket의 postimage exact SHA에
  다시 결속한다.
- `d36f29a9` 파일별 계획 보강 시점의 기존 dirty overlay는 `.cursorrules`,
  `CHANGELOG.md`, `CLAUDE.md`, `Cargo.toml`, `EXTRACTION.md`, `README.md`, 세 crate의
  `src/lib.rs`, `scripts/check-workspace.sh`, `scripts/verify-workspace-policy.py`,
  `scripts/tests/test_workspace_policy.py`, `tools/pm/{README.md,targets.json}`,
  `tools/pm/sources/rules/{architecture.md,build.md,cheatsheet.md}`, 삭제 중인
  `tools/pm/sources/agent-specific/{claude.md,cursor.md}`다. 이 큐의 write lease가 아니며
  구현자가 흡수하거나 되돌리면 안 된다.
  이 overlay는 `4aff891`에 흡수됐으므로 별도 writer lease가 아니다. 특히
  `crates/quanta-memory-platform-facade/src/lib.rs`는 TO-02와 겹치므로 구현 시작 전에
  새 blob digest를 재동결한다.
- 구현 시작 전에 담당 파일의 digest를 재계산한다. 아래 digest가 달라졌으면
  티켓을 그대로 적용하지 말고 해당 소유자와 오라클을 다시 감사한다.

| Owner | SHA-256 at audit base |
|---|---|
| `crates/quanta-memory-platform-core/src/domain/ledger.rs` | `0035f9524ebe0f8d8c8fa4caa9a7f6e1a6843552036f82737801aa000ae8762e` |
| `crates/quanta-memory-platform-core/src/domain/policy.rs` | `fbfcf785793111f22cb86e74cbb56bc24d4dc325f873fc05122f0ac306d1628b` |
| `crates/quanta-memory-platform-core/src/api/builder.rs` | `3d3e03ef369409c406f705fcfbb68b622ff004f2a132bf897ffae1050c951f41` |
| `crates/quanta-memory-platform-core/src/ports/telemetry.rs` | `70b98a1cc75a8b5a155497a966f23770c8daf98df3f2a7d456fc5e8ac2c0f589` |
| `crates/quanta-memory-platform-core/src/api/control.rs` | `2859bc56b7196e8e303dc6a39e69493ef5e9c20330a82a2eb12ecd59113860bb` |
| `crates/quanta-memory-platform-core/tests/public_api_surface.rs` | `536296099fa97b62ec7df26ddb7e64b41451266eaed5b4821036e60159055701` |
| `crates/quanta-memory-platform-core/tests/policy_contracts.rs` | `0abf3fc15febe1630c8cccf42f6ae7405662df2b41d9e1d33ce596506113d6ae` |
| `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs` | `5bf561840ca5754e7e6dda205353781cc05bf4b4b8769b32698d2137964c0ded` |
| `crates/quanta-memory-platform-facade/benches/scaling.rs` | `47e9615253fe6cc2c3e17ab2e8c9bbac6ea3cac589e7a1fdf8112e47d0700faf` |
| `crates/quanta-memory-platform-facade/src/payload_store/service.rs` | `69b9596cbe282b3124b3615708572b6415a3ecdb87d2fc7323f47c02312f7e23` |
| `crates/quanta-memory-platform-facade/src/retained_map/service.rs` | `35317597d2578ac2c5fe7cf2e7ad8fbe8d3c542f6429a4187ec894cbfec55289` |
| `crates/quanta-memory-platform-facade/src/retained_map/mod.rs` | `d8f1f47a097bd5a44745822afb6168f36b783819fd698a3929a856d4832d926b` |
| `crates/quanta-memory-platform-facade/src/revocable_store/service.rs` | `9bd5c2c0daf120c7fa9abb82b2ddc01909b4b2b5d28c5ffa7f2a0d94ac27e76d` |
| `crates/quanta-memory-platform-facade/src/shared/builder.rs` | `f3ab611cedaf9df5a9c71389ba7d8f30a41aebc1d41294911c7fcbdf9c5584d6` |
| `crates/quanta-memory-platform-facade/src/shared/accounting.rs` | `a820ec56f2b0702ea414fad818abdbd84769f20b249305a493a4ea5933232ce8` |
| `crates/quanta-memory-platform-facade/src/lib.rs` | `16fa8f8afb0ca60613937f9f7ad8c77d9777195ccb906700ebb1dcff76824dfd` |
| `crates/quanta-memory-platform-facade/tests/public_api_surface.rs` | `f2272802a3702c56ce312e51eb793af060ae57852c179bfc074e300120bb2487` |
| `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs` | `5da3fb3335435f7214a72f5bde36e1230649510bad7f631e859890c07b47c07f` |
| `crates/quanta-memory-platform-facade/src/transcript_store/service.rs` | `48c9023482ef765d6e64dcab9860be5e3db033075d0ed93fd56f1e89f5496879` |
| `scripts/verify-feature-matrix.sh` | `3a918ca5b42e545397820a3642ddafcfa67f6986df22297e7aad09bef5de2f49` |
| `scripts/verify-scaling-results.py` | `584bb97cc88b66e3deeee476e609a6cc7d19b5c87f6296d886ad63c9d6e6a588` |
| `scripts/tests/test_scaling_results.py` | `c824f7297f27f73d667666f5e080d55ce3d2a0dc054c55941f370460b7b08836` |

## Work graph

| Order | Ticket | Write owner | Dependency / conflict |
|---:|---|---|---|
| 1 | [TO-01](TO-01-redb-concurrency.md) | redb concurrency oracle | Uses landed `TestDb` fixture |
| 2 | [TO-02](TO-02-contract-authority.md) | API/feature/performance authorities | After TO-03; shared scaling file |
| 1 | [TO-03](TO-03-scaling-epoch.md) | scaling benchmark epoch | Blocks TO-02 scaling edit |
| 1 | [TO-04](TO-04-fault-driver-negative-pole.md) | testkit fault driver | Independent |
| 1 | [TO-05](TO-05-ledger-cardinality.md) | ledger bounded-preflight test | Independent |
| 1 | [TO-06](TO-06-zero-pool-admission.md) | policy validation + public contract | Independent |
| 1 | [TO-07](TO-07-builder-reclaimer.md) | public builder test | Independent |
| 1 | [TO-08](TO-08-core-test-cleanup.md) | core test cleanup | Independent by file |
| 1 | [TO-09](TO-09-transcript-partitions.md) | transcript accounting oracle | Independent |

`TO-03 → TO-02`는 직렬 writer다. 같은 파일에 대한 병렬 patch 후 충돌 해소 방식은
허용하지 않는다. 나머지는 각 티켓의 write set을 넘지 않는 경우에만 병렬 가능하다.

## File-by-file writer leases

| File | Ticket | Ordered work |
|---|---|---|
| `crates/quanta-memory-platform-facade/src/spill/backings/redb.rs` | TO-01 | Existing `TestDb` 유지; concurrency test만 축소/강화 |
| `crates/quanta-memory-platform-facade/benches/scaling.rs` | TO-03 then TO-02 | Allocation epoch 먼저 교정; 이후 revocable elapsed를 drain 뒤로 이동 |
| `crates/quanta-memory-platform-facade/src/payload_store/service.rs` | TO-02 | Source grep 삭제; 기존 compile-fail keeper 보존 |
| `crates/quanta-memory-platform-facade/src/retained_map/service.rs` | TO-02 | Concrete negative rustdoc 추가; container/poison greps 삭제; behavior keeper 유지 |
| `crates/quanta-memory-platform-facade/src/revocable_store/service.rs` | TO-02 | Container-name grep 삭제; FIFO/accounting behavior keeper 유지 |
| `crates/quanta-memory-platform-facade/src/lib.rs` | TO-02 | Forbidden accounting authority re-export negative contract owner |
| `crates/quanta-memory-platform-facade/tests/public_api_surface.rs` | TO-02 | Source/TOML/README parsing tests 제거 |
| `scripts/verify-feature-matrix.sh` | TO-02 | Resolved feature graph authority를 default/full/full-backings에 적용 |
| `scripts/verify-scaling-results.py` | TO-02 | Revocable normalized-growth rail 추가 |
| `scripts/tests/test_scaling_results.py` | TO-02 | Linear pass / quadratic fail fixtures 추가 |
| `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs` | TO-04 | Real spill classifier seam 추출; honest/lying pole 공용화 |
| `crates/quanta-memory-platform-core/src/domain/ledger.rs` | TO-05 | 10k fixture를 two-cardinality bounded-probe oracle로 교체 |
| `crates/quanta-memory-platform-core/src/domain/policy.rs` | TO-06 | Validation-only zero-pool test로 축소/정확화 |
| `crates/quanta-memory-platform-core/tests/policy_contracts.rs` | TO-06 | Public broker zero-pool admission/fail-closed contract 추가 |
| `crates/quanta-memory-platform-core/src/api/builder.rs` | TO-07 | `with_reclaimer -> build_live` behavioral test 추가 |
| `crates/quanta-memory-platform-core/src/ports/telemetry.rs` | TO-08 | Exact duplicate row/assertion만 제거 |
| `crates/quanta-memory-platform-core/src/api/control.rs` | TO-08 | 이미 함의된 `assert_ne!` 한 개 제거 |
| `crates/quanta-memory-platform-core/tests/public_api_surface.rs` | TO-08 | “eight”를 “nine”으로 수정 |
| `crates/quanta-memory-platform-facade/src/transcript_store/service.rs` | TO-09 | Exact independent partition oracle 추가 |

표에 없는 파일은 기본적으로 write 금지다. 새 파일/manifest/dependency가 필요해지면 현재
티켓을 확장하지 말고 원인과 필요성을 기록해 integrator가 별도 승인한다.


## Non-negotiable implementation rules

- 테스트 수 감소가 목적이 아니다. 중복 detector는 없애되, 행위 권한과 독립
  음성 오라클은 보존하거나 강화한다.
- `#[ignore]`, wall-clock sleep, mutable global `OnceLock`, production 계산기를
  그대로 호출하는 expected-value 오라클을 추가하지 않는다.
- redb를 in-memory fake로 대체하거나 concurrency/commit 경계를 약화하지 않는다.
- 테스트 편의를 위한 production API 공개, raw constructor, late accounting attach,
  dependency direction 역전은 금지한다.
- 테스트가 실패하면 assertion을 느슨하게 하지 말고 producer/authority를 고친다.
- Cargo dependency 추가는 기본적으로 금지한다. 필요하면 extraction dry-run과
  dependency-direction 검토를 별도 변경으로 분리한다.

## Integration and qualification

각 티켓의 focused command를 먼저 통과시킨 뒤 통합자가 다음을 수행한다.

```bash
cargo fmt --all -- --check
scripts/check-workspace.sh
```

`Cargo.toml`, dependency, cross-crate use path가 바뀐 경우에만 추가한다.

```bash
scripts/extract-to-standalone-repo.sh --dry-run /tmp/qmp-test-optimization-check
```

성능 영수증은 동일 host, 동일 profile, warm cache에서 before/after 각각 최소 3회
측정하고 median을 기록한다. TO-01/TO-05의 host wall-time 개선 목표는 테스트 assertion으로
하드코딩하지 않는다. 알고리즘 차수 계약은 TO-02처럼 cardinality-normalized ratio와
positive/negative verifier fixture로 별도 표현한다.

## Required closeout receipt

각 티켓은 다음을 채우기 전 `DONE`으로 바꾸지 않는다.

- preimage HEAD / postimage HEAD / tested HEAD
- owner file preimage/postimage digest
- exact commands, exit codes, selected test count
- negative-pole 또는 temporary mutant가 실제로 RED를 만든 증거. TO-08처럼
  deletion-only cleanup이면 keeper assertion이 삭제 항목을 완전히 함의한다는 diff proof
- before/after median(성능 티켓만)
- full rail 결과와 미실행 항목
- known risk와 rollback/reopen condition
- clean-tree 여부와 기존 unrelated dirty ownership

`scripts/check-workspace.sh` 통과만으로 배포/host activation을 주장하지 않는다.
