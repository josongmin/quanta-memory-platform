# Final SSOT/interface audit — 2026-09-24

## 최종 candidate 감사 — 현재 실행 목록의 근거

QMP `a1179a10c5569619210cb1c03ceeae7430109c25`, Semantica main
`f5e8ff63642ce815cafed296f2f5556f6e8e4048` 저장 소스를 다시 확인했다.
별도 candidate `/Users/songmin/Documents/code-new/semantica-memory-adoption-codex`는
detached `80822f963760cfd33628c1085cc0ec2ceea2411f` + dirty 상태다.
후보의 22 tracked 변경 파일과 신규 memo-retention 2개 파일은 이번 감사에서 수정하지 않았다.

| 범위 | main | 기존 candidate | 남은 작업 판정 |
| --- | --- | --- | --- |
| 기본 live report / observe stats | 구현됨 | 유지/확장됨 | 신규 구현 제외; 기존 회귀 재사용 |
| handle overlay/selection | API·저장·report 경로 남음, query selector로는 전달 안 됨 | 제거 diff 있음 | 001: 검토·실패 보완·검증; 재작성 금지 |
| output_mb/scratch_mb | 설정/lowering/report 있음, QMP 제품 집행 literal 미발견 | 제거 diff 있음 | 002: 제거-only cutover 검토·검증 |
| owner-key 중복 | literal 복제 | host constants reexport | 별도 구현 티켓 제외 |
| defaults/readback/public smoke 테스트 | 기존 회귀 있음 | overflow/invalid policy/reconfigure/public smoke 소스 있음 | 기존 테스트 실행; 신규 동목적 suite 금지 |
| memo retention 신규 port/adapter | 이 API 수정과 무관 | 신규 파일·module export 작성됨 | 009 통합 patch에서 분리, 다른 owner 작업 삭제 금지 |

candidate `packages/ crates/`의 Rust census에서 `MemoryBudgetSelectionV1`,
`MemoryBudgetOverlayV1`, `ValidatedMemoryBudgetSelectionV2`, `with_memory_budget(`,
`with_memory_budget_preset(`, `output_mb`, `scratch_mb` hit가 없었다.
선택한 README/docs/packages/crates 문서·Python·JSON·YAML·TOML 검색에서도 해당 공개
호출/축 문자열 hit를 찾지 못했다. compiler-resolved 외부 consumer census는 아니다.
기존 execution budget/rail 파일은 candidate diff에 없었다.

candidate tracked crates diff SHA-256 (`git diff --binary -- packages/analysis/quanta-v2/crates`):
`8f5aeb82003a6a509df3adea3f1c2dc7d0784f37dab278767b091c399c676af7`.
신규 untracked memo 파일은 이 digest 밖이며 활성 통합 범위에서도 제외한다.

candidate 핵심 owner SHA-256 (Semantica 상대 경로):

```text
6b8506b0b437795de47b14ca46dbcc1605546a70ca02bc71ad6bfb9248dacce5  packages/analysis/quanta-v2/crates/quanta-runtime-memory-compose/src/lib.rs
b57dd98041f9ba39bb1b977b6b89a87a7bca9e1c53ab508d22393853a6573dd0  packages/analysis/quanta-v2/crates/quanta-runtime-memory-compose/src/lib_tests.rs
f9d23735f27ca52818edf62276047e1c9596e70ef7cbea4c4b9ae2ff8405fe04  packages/analysis/quanta-v2/crates/quanta-runtime/src/sdk/code/mod.rs
7eb199568de0e5e0566948ac37f122444ee9a620112c17d87a98a353e58583b0  packages/analysis/quanta-v2/crates/quanta-runtime/src/sdk/memory_budget.rs
```

candidate `git diff --check` 통과. Rust/QBC/benchmark 미실행이므로 OWNER_GREEN이나
main 통합 완료로 판정하지 않는다. 현재 티켓은 신규 구현이 아니라 기존 candidate의
검토·검증·통합 잔여다. 003~008은 [archive](archive/README.md)로 이동했다.
아래 과거 감사의 TODO/신규 설계 문구보다 [현재 README](README.md)가 우선한다.

## 이전 main-only 작업 재분류 (candidate 확인 전 기록)

Semantica `f5e8ff63642ce815cafed296f2f5556f6e8e4048`에서 composer를 재확인했다.
기본 report의 live `effective_report()`와 stats의 단일 `observe()`는 이미 구현됐다.
따라서 아래 F01의 host 신규 연결 작업은 **SUPERSEDED / IMPLEMENTED_IN_SOURCE**이며
SMA-001 실행 목록에서 제거했다. 기존 installed-policy 회귀 테스트를 재사용한다.
이 판정은 코드 존재 확인이며 이번 문서 수정에서 Rust 테스트를 실행한 것은 아니다.

현재 활성 범위는 SMA-001의 overlay 제거, SMA-002의 미집행 축 정리, SMA-009의
해당 변경 검증뿐이다. F02 owner-key 중복은 P2 유지보수 후보로 분리했고,
F04~F06 memo/reclaim 설계는 해당 도입을 채택할 때만 적용한다.
003~008은 비활성 backlog이며 필수 출하 gate가 아니다. 상세 현재 목록은 [README](README.md).

## 과거 감사 snapshot — 근거 보존, 활성 TODO 아님

Status: source audit and plan corrections complete; implementation/qualification PLANNED.
QMP HEAD: `a1179a10c5569619210cb1c03ceeae7430109c25`.
Semantica re-audit HEAD: `cd926f66743758c6085e74199d9f07cfc4ff0f49`.

The original [audit](AUDIT.md) snapshot is preserved. Its 12 owner-file hashes were
rechecked and unchanged despite Semantica HEAD movement. Shared dirty files at this
snapshot: executor `src/client/query_api.rs`, `src/client/tests/group_gqlang_public_path.rs`
and `tools/testing/suite-registry-v1.yaml`. No changes to those files were made here.
QMP changes are this documentation packet and its index, not runtime changes.

Closeout hash check: Semantica had advanced again to
`ec11d46a4029594069d4ecc87e09eaba66eafd82`; all 20 recorded owner-file hashes
(original 12 + additional 8 below) still matched. The shared checkout then had 19 dirty
paths. This is a file-scoped audit, not a frozen whole-repository qualification;
dispatch must recapture dependency, dirty ownership and affected caller closure.

## Findings and ticket corrections

| ID | Source evidence at snapshot | Correction / owner |
| --- | --- | --- |
| F01 | QMP core `api/control.rs:1723,1737`: live checked report and one-lock `observe()` already exist; composer `lib.rs:1049` samples separately | Reuse observation, no new snapshot coordinator. SMA-001 |
| F02 | Host `runtime/ports_impl.rs:36`, `runtime/mod.rs:31`: public owner keys; composer `lib.rs:113` copies literals | Import/reexport canonical constants; preserve SDK-owned product defaults. SMA-001 |
| F03 | Execution `budget_handle.rs:64,263`; rail `execution_rail_contract_v1.rs:133`; host SPI `relation.rs:15` already provide shared root/limits/opaque permits | No second execution counter/limit enum. QMP domain versus execution cap distinguished. SMA-002 |
| F04 | Contract `memo_table.rs:67,82` has cloneable previous-row receipt and separate publication reservation; canonical table `:738,859,896,926` publishes/retracts it | Old row survives replacement in receipt. Shared charged rollback custody, clone/retract/GC tests. SMA-003 |
| F05 | `MemoEntry` is CAS references plus metadata; canonical adapter lacks QMP dependency; host Cargo forbids reverse runtime/adapters dependency | Charge metadata without duplicating artifact payload; minimal contract-port retention seam. SMA-003 |
| F06 | Existing managed charge can release via broker; owner lock discipline must cover destructor paths as well as reserve | Detach then drop outside owner lock; weak reclaim graph. Required design/proof, not a reproduced deadlock. SMA-003/006 |
| F07 | Semantica dependency SSOT specifies both workspace manifests and verification-only environment selector; FIN-20 already owns checked stats/paired proof | Reuse dependency and test authority; no second gate/catalog. SMA-001/009 |

Exact interface requirements and migration closure are in
[INTERFACE-SSOT.md](INTERFACE-SSOT.md). Source facts above establish missing plan
constraints, not runtime reproduction of data loss, OOM or deadlock.

## Revised implementation boundary

- No new QMP report resolver, execution counter, owner registry, serialized memo mirror,
  generic retained-value framework, or duplicate checked-stats qualification suite.
- One justified new seam: host-neutral persistent memo retention admission and opaque
  custody, implemented by existing QMP managed charges. Existing publication identity,
  generation, cancellation and invalidation remain canonical.
- Breaking changes update all producers/consumers once. DTO serialization remains
  separate from live resource authority. Unsupported controls are removed, not no-op'd.
- Production defaults and resource metrics keep their distinct authorities. A common
  platform does not turn semantic bytes into physical RSS or a per-process hard limit.

## Extra owner-source SHA-256

Paths are relative to Semantica; original 12 hashes remain in AUDIT.md.

```text
d75565d2697081f2dcb15ddec360e9a50a7c0352ef4f37825969384cb4472996  packages/analysis/quanta-v2/crates/quanta-contract-ports/src/memo_table.rs
bd043679d01693cb1db7d054b1cc385d13c528b23d71f984aea30e08ee33ec05  packages/analysis/quanta-v2/crates/quanta-runtime-execution/src/budget_handle.rs
aa4306e8bf045266aefbcf6bd51fae43391851baea50d7aa85ddcc5dc0983253  packages/analysis/quanta-v2/crates/quanta-contract-execution-rail-shared/src/execution_rail_contract_v1.rs
4ac3262d5fdee7204bedb1caf3f961c11e8a8f6d1c39ebde73175da907739b40  packages/analysis/quanta-v2/crates/quanta-gqlang-v2-exec-host-spi/src/relation.rs
19e807861cb2b589c21f6fca4c8380f5baa0e95abe032094b0fc8f5505e6773a  packages/analysis/quanta-v2/crates/quanta-memory-host/src/runtime/ports_impl.rs
a14915e393a109858eadc48fb35bddcb034255d583ec92a0df69431fdb3580b8  packages/analysis/quanta-v2/crates/quanta-memory-host/src/runtime/mod.rs
7d4cfcf17b76b6f258dc7f3bd15c5d82ccc0f6bb4abee6bb716e9c6c4aae310a  packages/analysis/quanta-v2/crates/quanta-adapters-in-memory-shared/Cargo.toml
8109f8bef43f50f0b8e0c2986e1585b6951f8764904b298ebeab468b84175f84  docs/ssot/quanta-memory-platform-dependency-contract-v1.md
```

## Unclosed implementation evidence

No Rust/QBC runtime tests or performance trials were executed by this final document
audit. The remaining gates are receipt-aware memo retention, actual SDK cutover,
feature-compatible owner tests, product-path integration and source-bound performance.
Conditional reclaim/disk work still requires named demand. Documentation validation
cannot promote any SMA ticket to OWNER_GREEN or PRODUCT_VERIFIED.

Documentation checks: 13 packet files / 9 tickets, 31 local links resolved, 20 source
hashes matched, trailing whitespace/final newline checks and `git diff --check` passed.
