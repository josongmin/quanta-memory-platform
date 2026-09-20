# QMP-110 — Typed observation, poison truth, and bounded telemetry

Priority: P2  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-030, QMP-040, QMP-080  
Blocks: QMP-130, QMP-140

## Objective

corrupted/unavailable state를 empty/zero로 위장하지 않고, memory admission/reclaim/spill/reconfigure 결과를 bounded-cardinality telemetry로 관측 가능하게 한다.

## Confirmed evidence

- payload stats는 poisoned lock에서 default zero: `payload_store/service.rs:240-247`
- revocable stats도 default zero: `revocable_store/service.rs:157-164`
- transcript trait stats는 `unwrap_or_default`: `transcript_store/service.rs:403-405`
- spill retained count도 poison을 zero로 바꾸는 경로가 있다.
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

- `poisoned_payload_stats_returns_error_not_zero`
- `poisoned_revocable_stats_returns_error_not_zero`
- `poisoned_transcript_stats_returns_error_not_zero`
- `poisoned_spill_inventory_returns_error_not_zero`
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
