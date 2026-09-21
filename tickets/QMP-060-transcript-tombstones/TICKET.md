# QMP-060 — Bounded transcript tombstones and explicit idempotency horizon

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-050  
Blocks: QMP-090, QMP-100, QMP-130

Status: `QUALIFIED` locally at `0c126a25aeab11f457e20c4a209fc7bb631ea859`.
See [`receipts/2026-09-21T072928Z-0c126a2.md`](receipts/2026-09-21T072928Z-0c126a2.md).

## Objective

transcript의 replay/idempotency evidence가 process lifetime 동안 무제한 성장하지 않게 하고, 보존 한계에 도달했을 때 silent identity reuse가 아니라 명시적 fail-closed 또는 durable handoff를 수행한다. Evidence/session byte accounting은 QMP-050 소유이며, 여기서는 bound와 horizon을 소유한다.

## Confirmed evidence

- 지원 retention은 process-lifetime tombstone뿐: `transcript_store/mod.rs`의 `TranscriptRetentionPolicyV1`.
- session state는 active rows 외 두 evidence map을 보유: `transcript_store/service.rs:94-105`.
- 현재 QMP-050 구현은 truncate에서 active lease만 환급하고 evidence/session lease와 `retained_bytes`를 유지한다. QMP-050 이전 body-only stats/무과금 evidence는 **historical baseline**, 현행 결함이 아니다.
- builder에 tombstone count/byte cap이 없다.
- `ExpiredIdentity` variant는 선언됐지만 현재 경로에서 생산되지 않는다.

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

## Implemented policy and external design references

`fail-closed saturation`을 선택한다. Identity evidence는 `close` 전까지
evict/expire하지 않고, configured count/byte horizon에 도달하면 기존 exact
retry와 conflict 판정은 계속 수행하되 새로운 identity만 typed error로
거부한다. 이 정책은 restart durability를 주장하지 않는다.

- [AWS Builders' Library — Making retries safe with idempotent APIs](https://aws.amazon.com/builders-library/making-retries-safe-with-idempotent-APIs/):
  caller-provided request identity, same-token semantic equivalence, parameter
  mismatch rejection, token+mutation atomicity가 이 구현의 replay/conflict/
  atomic-publication 계약과 일치한다.
- [AWS Lambda Powertools idempotency](https://docs.aws.amazon.com/powertools/python/develop/utilities/idempotency/):
  idempotency horizon은 explicit policy여야 하고 storage TTL의 eventual
  deletion만으로 expiry를 판정하지 않는다. QMP in-memory backing은 clock/
  durable tier가 없으므로 time expiry를 가장하지 않는다.
- [Redis key eviction policy](https://redis.io/docs/latest/develop/reference/eviction/):
  identity evidence가 cache가 아닌 correctness state인 경우 `noeviction`
  방식처럼 기존 state를 버리지 않고 새 write를 거부한다. QMP는 이를
  typed `RetentionSaturated`와 no-charge/no-mutation postcondition으로 강화한다.

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
- `truncate_refunds_body_but_preserves_charged_evidence` (QMP-050 regression guard; 이 티켓의 신규 RED가 아님)
- `retention_reconfigure_below_live_evidence_is_rejected`
- concurrency: append vs truncate/evict has one linearized result

## Acceptance

- [x] QMP-050의 active rows=0 evidence charge를 회귀시키지 않고 count/byte horizon과 일치시킨다.
- [x] unique append→truncate 반복이 configured bound를 넘지 않는다.
- [x] bound 도달 semantics가 typed result로 공개된다.
- [x] exact retry/conflict semantics가 eviction policy와 모순되지 않는다.
- [x] process restart claim은 durable implementation이 있을 때만 문서화된다.
- [x] transcript conformance suite가 honest/broken retention fake를 구분한다.

## Qualified implementation checkpoint

- Default policy is finite: 65,536 evidence entries and 64 MiB.
- Store-global O(1) component counters own active/evidence/session-key totals;
  checked reads reconcile them against retained maps.
- Saturation is checked after exact replay/conflict lookup and before Clone,
  accounting charge, or publication.
- Runtime policy changes share the mutation lane; limits below live evidence
  return `BelowLiveEvidence` without changing policy.
- `TranscriptRetentionPolicyV1` and unreachable `ExpiredIdentity` were removed.
  QMP-140 owns the pre-1.0 migration/semver note.
- Exact source `f3b5a13` passed 601 workspace tests, the repo-local doctor,
  feature matrix, Semgrep with zero findings, and fresh actual standalone
  extraction. Extracted bootstrap commit: `10c0d5c1a319aa6e5c87c02f48a4a5b9a344e0b5`.

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
