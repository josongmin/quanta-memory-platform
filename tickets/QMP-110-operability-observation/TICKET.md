# QMP-110 — Typed observation, poison truth, and bounded telemetry

Priority: P2  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-030, QMP-040, QMP-080  
Blocks: QMP-130, QMP-140

## Objective

corrupted/unavailable state를 empty/zero로 위장하지 않고, memory admission/reclaim/spill/reconfigure 결과를 bounded-cardinality telemetry로 관측 가능하게 한다.

## Confirmed evidence

- payload/revocable/transcript store `stats`는 QMP-040 이후 `MemoryResult<StoreStats>`로 변경됐다 (`payload_store/mod.rs:18`, `revocable_store/mod.rs:15`, `transcript_store/mod.rs:231`). 이 세 zero-on-poison 결함은 **historical baseline**이며, 아래 테스트는 regression guards다.
- 두 public in-memory/Noop spill target의 retained count는 아직 poison을 zero로 바꾼다: `spill/service.rs:164-165,237-238`. redb observation은 이미 fallible이다.
- core의 `effective_report()` trait는 infallible (`core/src/api/broker.rs:26`); live 구현은 poisoned guard의 실제 report를 복구하지만 poison 상태 자체를 드러내지 않는다 (`api/control.rs:1033-1045`). 이를 fabricated zero로 서술하지 말고 typed observation 계약을 결정한다.
- QMP-020 후 공개 `LedgerSnapshot`의 `total_*` getters는 수동 구성한 unrepresentable snapshot에서 `checked_add(...).expect(...)`로 panic한다 (`core/src/domain/ledger.rs:229-268`). Broker snapshot은 먼저 `validate_exact()`로 typed error를 반환한다. 이 둘의 public contract/compatibility를 QMP-140과 함께 결정한다.
- metrics/hdr adapters는 존재하지만 broker/store production call site가 없다.
- reconfigure report provenance는 active value와 stale해질 수 있다.

## Canonical write set

- core/facade stats traits and error types
- `core/src/adapters/{metrics,hdr}.rs`
- broker/controller/scheduler event emission
- facade store/spill event emission
- observability tests/docs

Owner symbols/call sites:

- `LiveProcessMemoryBroker::{ledger_snapshot, effective_report, classify_pressure, observe_allocator, try_live_reconfigure}`
- store `stats` / `stats_checked` methods
- spill retained/count observation methods
- `AllocatorAdvisory` and metrics/hdr adapters
- proposed bounded outcome event emitter

## Required observation contract

1. authoritative observation은 `Result<Snapshot, ObservationError>`다.
2. poison/unavailable/corrupt는 zero/empty와 다른 typed state다.
3. infallible convenience API가 남으면 `Option/Unknown`을 표현하고 control/admission에 사용하지 않는다.
4. snapshot에는 policy revision/provenance와 ledger epoch를 포함한다.
5. metrics는 advisory이며 semantic counter를 mutate하지 않는다.

## Required events

bounded enum labels로 최소 다음을 집계한다.

- reserve outcome: admitted / global / class / owner / group / scratch / rebuild / poisoned
- release/close outcome
- recharge/transfer outcome
- pressure transition
- reclaim attempts/bytes/outcome
- spill/restore target kind/outcome
- reconfigure accept/reject reason
- observation failure
- tombstone saturation

금지 labels:

- raw owner ID
- runtime ID
- arbitrary key/session/message ID
- full error string

## Error taxonomy

- capacity denial과 invalid state/poison/config/backend unavailable를 구분한다.
- caller-actionable vs retryable vs terminal을 문서화한다.
- formatting string을 programmatic contract로 사용하지 않는다.

## RED tests to add

- `poisoned_payload_stats_returns_error_not_zero` (기존 QMP-040 동작의 regression guard)
- `poisoned_revocable_stats_returns_error_not_zero` (기존 QMP-040 동작의 regression guard)
- `poisoned_transcript_stats_returns_error_not_zero` (기존 QMP-040 동작의 regression guard)
- `poisoned_spill_inventory_returns_error_not_zero`
- `poisoned_broker_report_exposes_status_without_losing_actual_policy_values` (infallible API의 migration/compatibility 판정 포함)
- `forged_unrepresentable_public_snapshot_has_documented_fallible_observation` (panic precondition 또는 새 checked public API를 명시하고 외부 테스트로 고정)
- `denial_reason_emits_one_bounded_metric`
- `allocator_observation_never_mutates_semantic_usage`
- `metrics_do_not_use_owner_or_runtime_as_labels`
- `accepted_reconfigure_snapshot_has_new_revision_and_provenance`
- `observation_error_propagates_to_conformance_verdict`

## Acceptance

- [ ] control/admission code가 infallible zero-on-error stats를 사용하지 않는다.
- [ ] poison과 empty가 모든 public authority에서 구분된다.
- [ ] major operational outcomes에 emission call site가 있다.
- [ ] labels가 bounded set임을 test가 고정한다.
- [ ] semantic counters와 telemetry가 단방향 observation 관계를 유지한다.
- [ ] snapshot values와 provenance revision이 일치한다.

## Verification

```bash
cargo test -p quanta-memory-platform-core --all-features observation
cargo test -p quanta-memory-platform-facade --all-features poisoned
cargo test -p quanta-memory-platform-testkit --all-features observation_error
cargo test --workspace --all-features
```

## Non-goals

- external metrics backend deployment
- unbounded per-tenant debugging labels
- allocator telemetry를 billing SSOT로 사용
