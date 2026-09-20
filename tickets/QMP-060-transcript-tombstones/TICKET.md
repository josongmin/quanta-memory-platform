# QMP-060 — Bounded transcript tombstones and explicit idempotency horizon

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-050  
Blocks: QMP-090, QMP-100, QMP-130

## Objective

transcript의 replay/idempotency evidence가 process lifetime 동안 무제한·무과금 성장하지 않게 하고, 보존 한계에 도달했을 때 silent identity reuse가 아니라 명시적 fail-closed 또는 durable handoff를 수행한다.

## Confirmed evidence

- 지원 retention은 process-lifetime tombstone뿐: `transcript_store/mod.rs:170-193`
- session state는 active rows 외 두 evidence map을 보유: `service.rs:40-46`
- truncate는 body ticket을 refund하지만 evidence를 유지: `service.rs:166-212`
- `retained_bytes`/stats는 active body만 집계: `service.rs:111-133`
- builder에 tombstone count/byte cap이 없다.
- `ExpiredIdentity` variant는 선언됐지만 실제로 생산되지 않는다.

## Required decision

다음 중 하나를 명시적으로 선택하고 API에 반영한다.

1. **Bounded in-memory horizon**
   - count/byte cap
   - deterministic eviction
   - evicted identity 재사용은 `ExpiredIdentity`
2. **Fail-closed saturation**
   - cap 도달 시 새로운 identity 거부
   - existing exact retry는 계속 idempotent
3. **Durable identity tier**
   - durable target commit 후 memory evidence를 이동
   - restart semantics와 corruption behavior 포함

무제한 process-lifetime in-memory 보존은 허용하지 않는다.

## Canonical write set

- `facade/src/transcript_store/mod.rs`
- `facade/src/transcript_store/service.rs`
- transcript builder section in `facade/src/shared/builder.rs`
- transcript conformance tests

Owner symbols:

- `TranscriptRetentionPolicyV1` and successor policy type
- `SessionState::{rows, sequence_evidence, message_evidence}`
- `SimpleSessionTranscriptStore::{new, retained_bytes, checked_stats}`
- `SessionTranscriptStore::{append, load, retain_sequences, truncate_to, stats}`
- `TranscriptAppendDispositionV1` / `ExpiredIdentity`

## Required implementation

1. `TranscriptRetentionPolicyV2`에 count와 byte budget을 둔다.
2. active body cost와 evidence/tombstone cost를 분리 관측한다.
3. append 전에 prospective body+evidence cost를 검사한다.
4. saturation/eviction/durable handoff는 row/index/ticket을 원자적으로 변경한다.
5. exact retry는 cap 상태에서도 double-charge하지 않는다.
6. conflict identity는 eviction 전후 semantics가 결정적이어야 한다.
7. session close/delete가 허용되면 replay horizon이 종료된다는 명시적 capability를 요구한다.
8. retention policy upgrade/downgrade validation을 추가한다.

## RED tests to add

- `append_truncate_cycles_cannot_grow_unbounded_unaccounted_evidence`
- `tombstone_count_limit_rejects_or_evicts_deterministically`
- `tombstone_byte_limit_counts_large_message_ids`
- `exact_retry_at_saturation_remains_idempotent`
- `new_identity_at_fail_closed_saturation_is_rejected_without_mutation`
- `expired_identity_is_observable_after_horizon_eviction`
- `truncate_refunds_body_but_preserves_charged_evidence`
- `retention_reconfigure_below_live_evidence_is_rejected`
- concurrency: append vs truncate/evict has one linearized result

## Acceptance

- [ ] active rows가 0이어도 evidence usage가 ledger/stats에 보인다.
- [ ] unique append→truncate 반복이 configured bound를 넘지 않는다.
- [ ] bound 도달 semantics가 typed result로 공개된다.
- [ ] exact retry/conflict semantics가 eviction policy와 모순되지 않는다.
- [ ] process restart claim은 durable implementation이 있을 때만 문서화된다.
- [ ] transcript conformance suite가 honest/broken retention fake를 구분한다.

## Verification

```bash
cargo test -p quanta-memory-platform-facade --all-features append_truncate_cycles_cannot_grow_unbounded_unaccounted_evidence
cargo test -p quanta-memory-platform-facade --all-features exact_retry_at_saturation_remains_idempotent
cargo test -p quanta-memory-platform-testkit --all-features transcript
cargo test -p quanta-memory-platform-facade --all-features
```

## Non-goals

- general spill transaction: QMP-070
- host-specific transcript persistence format: QMP-130
