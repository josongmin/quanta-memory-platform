# QMP-010 — Total validated policy and fail-closed class authority

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-000  
Blocks: QMP-020, QMP-030, QMP-080, QMP-140

## Objective

mandatory memory class가 빠진 configuration을 build/reconfigure 단계에서 거부하고, ledger가 validation을 우회한 partial policy를 받을 수 없게 한다.

## Confirmed evidence

- mandatory taxonomy: `crates/quanta-memory-platform-core/src/domain/policy.rs:147-166`
- `validate_residency`는 map에 존재하는 entry만 검사: `domain/config.rs:191-282`
- ledger reserve는 `if let Some(budget)`일 때만 class cap 적용: `domain/ledger.rs:346-370`
- custom builder test는 class 하나짜리 policy를 valid로 취급: `api/builder.rs:158-180`

## Failure model

```text
manifest.classes omits ArtifactCache
  -> validate_residency succeeds
  -> LiveProcessMemoryBroker builds
  -> reserve(ArtifactCache, bytes) sees no class budget
  -> global/owner cap only; class isolation absent
```

이것은 default 선택이 아니라 fail-open configuration이다.

## Canonical write set

- `crates/quanta-memory-platform-core/src/domain/policy.rs`
- `crates/quanta-memory-platform-core/src/domain/config.rs`
- `crates/quanta-memory-platform-core/src/domain/ledger.rs`
- `crates/quanta-memory-platform-core/src/api/builder.rs`
- `crates/quanta-memory-platform-core/src/api/control.rs`
- `crates/quanta-memory-platform-core/tests/wave1_policy_contracts.rs`

Owner symbols:

- `MemoryClass::mandatory`
- `validate_residency`
- `ProcessMemoryBrokerBuilder::{build, build_live}`
- `LiveProcessMemoryBroker::{from_report, try_live_reconfigure}`
- `AccountingLedger::{new, swap_policy, reserve}`

## Required design

1. `ValidatedResidencyPolicy` 같은 opaque validated type을 canonical construction result로 도입한다.
2. validation은 다음을 보장한다.
   - `MemoryClass::mandatory()` 전부 존재
   - 각 class `reserved <= soft <= hard <= global`
   - owner/group mapping 완전성
   - reserved/headroom floor overflow를 saturating success로 숨기지 않음
3. future classes의 semantics를 결정한다.
   - 미설정 future class를 사용할 때 deny, 또는
   - explicit zero budget으로 materialize
   - global-only implicit admission은 금지
4. `AccountingLedger::new`는 validated type만 받고 외부 공개하지 않는다.
5. `swap_policy` direct trust 경로를 제거한다.
6. live reconfigure도 동일 validator와 total map을 통과한다.
7. unknown/missing class reserve에는 typed config/capacity error를 반환하고 ledger는 byte-for-byte 불변이어야 한다.

## Producer → contract → consumer wiring

```text
Manifest / preset / override
  -> validate + materialize total class map
  -> ValidatedResidencyPolicy
  -> builder / live reconfigure
  -> ledger admission
```

validator와 ledger가 서로 다른 completeness 규칙을 가지면 안 된다.

## RED tests to add

Core config/builder:

- `validate_rejects_missing_each_mandatory_class`
- `builder_rejects_manifest_missing_mandatory_class`
- `validated_policy_materializes_no_implicit_class_holes`
- `reserved_floor_checked_add_overflow_is_config_error`

Ledger/control:

- `reserve_for_unconfigured_class_is_denied_without_mutation`
- `reconfigure_rejects_policy_missing_drained_mandatory_class`
- `reconfigure_missing_class_denial_preserves_policy_scheduler_and_usage`

Cross-cutting:

- `every_preset_and_custom_validated_policy_is_total_over_mandatory_classes`

## Acceptance

- [x] unvalidated `ResidencyPolicy`로 ledger/live broker를 public construction할 수 없다.
- [x] mandatory class 하나씩 제거한 table test가 모두 typed error다.
- [x] absent class reserve가 global-only로 진행되지 않는다.
- [x] build와 reconfigure가 같은 validation owner를 사용한다.
- [x] denial 전후 ledger/report/scheduler snapshot이 동일하다.
- [x] preset output은 기존 semantics를 보존한다.
- [x] public docs가 future class behavior를 명시한다.

## Verification

```bash
cargo test -p quanta-memory-platform-core --all-features validate_rejects_missing_each_mandatory_class
cargo test -p quanta-memory-platform-core --all-features reserve_for_unconfigured_class_is_denied_without_mutation
cargo test -p quanta-memory-platform-core --all-features --test wave1_policy_contracts
cargo clippy -p quanta-memory-platform-core --all-targets --all-features -- -D warnings
cargo test -p quanta-memory-platform-core --all-features
```

Cargo/public use path가 바뀌면 QMP-120 actual extraction receipt도 요구한다.

## Non-goals

- soft/reserved/burst의 runtime semantics 구현: QMP-080
- public builder default 변경: QMP-030
- host manifest migration: QMP-130

## Receipt

- Product implementation commit: `10af630512aa809ca5fe8b69d9e15d6e636d7462`
- Receipt: [`receipts/2026-09-20T183204Z-10af630.md`](receipts/2026-09-20T183204Z-10af630.md)
- Lifecycle remains `IMPLEMENTED`, not `QUALIFIED`: actual standalone extraction exposed the existing QMP-120 missing-semgrep-rule defect.
