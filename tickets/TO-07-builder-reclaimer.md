# TO-07 — Exercise Reclaimer Wiring through the Builder-First API

Status: `IMPLEMENTED; owner proof recorded; included in historical qmp-final-qualified-sep23-followup-v5 source; current HEAD qualification separate`
The problem and edit plan below are a pre-implementation snapshot; the owner proof and v5 receipt govern the historical result.
Priority: `P1`
Depends on: none

## Problem

`ProcessMemoryBrokerBuilder::with_reclaimer`와 `build_live` alias가 public builder 경로에서
행동 검증되지 않는다. control tests는 `LiveProcessMemoryBroker`의 lower-level constructor로
reclaimer semantics를 검사하므로 builder가 callback을 버리거나 alias가 다른 경로를
타도 잡지 못한다.

## Write set

- `crates/quanta-memory-platform-core/src/api/builder.rs` test module
- public API, control implementation, existing lower-level tests는 변경하지 않는다.

## File-by-file implementation plan

### `crates/quanta-memory-platform-core/src/api/builder.rs`

- Add one `#[cfg(feature = "live_control")]` test named
  `builder_reclaimer_reaches_build_live_broker` in the existing `tests` module.
- Add test-local `RecordingReclaimer { requests: Mutex<Vec<ReclaimRequest>> }`; implement
  `MemoryReclaimer::reclaim` by cloning the request and returning
  `ReclaimOutcome { reclaimed_bytes: 0 }`.
- Reuse existing `class_budget` helper and construct a manifest with global `1_000`,
  ArtifactCache `soft=50`, `hard=100`, borrow false.
- Build only through `ProcessMemoryBrokerBuilder::new().with_manifest(...).with_reclaimer(...).build_live()`.
- Reserve a 60-byte managed ArtifactCache payload for owner `builder-reclaimer`; require success.
- Assert exactly one request and exact trigger/class/owner/bytes/max-attempts fields.
- Assert runtime reclaim attempts `1`, ledger total `60`, then drop/release charge and assert zero.
- Keep production `with_reclaimer`, `build`, `build_live`, `resolve_report` unchanged. Do not reuse
  or move `api/control.rs`'s private test fake.
- File-local verification:
  `cargo test -p quanta-memory-platform-core --all-features builder_reclaimer_reaches_build_live_broker -- --nocapture`.

## Test design

- test-local `RecordingReclaimer`가 `Mutex<Vec<ReclaimRequest>>`를 소유하고
  `ReclaimOutcome { reclaimed_bytes: 0 }`을 반환한다.
- valid manifest를 사용하되 ArtifactCache를 `soft=50`, `hard=100`, borrow false로 설정한다.
- 정확히 다음 public chain만 사용한다.

```text
ProcessMemoryBrokerBuilder::new
  -> with_manifest
  -> with_reclaimer
  -> build_live
  -> reserve_managed_charge
```

- 60-byte charge가 hard cap 아래에서 성공하고 callback이 정확히 한 번 호출되는지 확인한다.
- request의 `trigger == SoftThreshold`, requested class/owner/bytes, `max_attempts == 1`을
  확인한다.
- broker reclaim runtime attempts가 1이고 ledger total이 60인지 확인한다.
- charge drop/release 뒤 ledger total이 0인지 확인한다.

기존 control test의 private `RecordingReclaimer`를 production module로 끌어올리지 않는다.
작은 test fake 중복이 test boundary를 오염시키는 shared helper보다 낫다.

## Negative proof

- temporary mutant로 builder의 `reclaimer` 전달을 `None`으로 바꾸면 request-count assertion이
  RED여야 한다.
- temporary mutant로 `build_live`가 `build_report_only` 또는 별도 broker construction을
  사용하게 만들면 compile 또는 behavior rail이 RED여야 한다.

```bash
cargo test -p quanta-memory-platform-core --all-features builder_reclaimer -- --nocapture
cargo test -p quanta-memory-platform-core --all-features api::builder
```

실제 test 이름은 `builder_reclaimer_reaches_build_live_broker`처럼 selector가 고유하도록
정한다.

## Acceptance / reopen

- lower-level constructor가 아닌 builder-first chain 전체가 실행된다.
- callback request 내용, admitted ledger state, cleanup state를 모두 검증한다.
- reclaimer를 설치했다는 field-level assertion만 남으면 reopen한다.

## Historical owner proof (subsequently covered by the v5 source receipt)

- Mutant: `build` passes `reclaimer = None` instead of the configured
  callback.
- RED: `builder_reclaimer_reaches_build_live_broker` FAILED at
  `builder.rs:297` (request-count assertion).
- Post-revert: 1 passed; production byte-identical to HEAD.
- Reroute mutant: `build_live` rebuilt to bypass reclaimer wiring
  (`from_report_with_reclaimer_and_events(report, None, …)`) → test
  FAILED at `builder.rs:299` (`left: 0, right: 1`, request-count).
  Proves the test exercises `build_live` itself, not `build`.
  Post-revert: 1 passed.
