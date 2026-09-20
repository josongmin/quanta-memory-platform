# QMP-050 — Total retained-cost accounting

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-040  
Blocks: QMP-060, QMP-070, QMP-100

## Objective

ledger charge가 value/body만이 아니라 store가 실제로 유지하는 key, identity, index, tombstone, fixed semantic overhead까지 포함하도록 하나의 portable cost contract를 만든다.

## Confirmed evidence

- accounting seam은 retained byte 전체를 주장: `facade/src/shared/accounting.rs:3-9`
- retained map은 arbitrary `Key`를 저장하지만 value만 `ByteSized`.
- revocable store도 key를 저장하지만 value만 charge.
- transcript는 `SessionId`, cloned `MessageId`, 두 evidence map을 유지하지만 body만 charge.
- prepared publication은 identity vector와 index를 유지하지만 payload만 charge.
- large key/zero-sized value를 표현하는 public type contract가 존재한다.

## Required cost model

다음과 같은 checked breakdown을 canonical contract로 둔다. 이름은 변경 가능하다.

```rust
pub struct RetainedCost {
    pub payload_bytes: u64,
    pub key_identity_bytes: u64,
    pub index_bytes: u64,
    pub tombstone_bytes: u64,
    pub fixed_overhead_bytes: u64,
}
```

요구 성질:

- `total()`은 checked add이며 overflow를 error로 반환한다.
- 동일 logical entry는 canonical formula 하나만 사용한다.
- charge, stats, reclaim freed bytes가 같은 cost를 사용한다.
- allocator RSS/advisory는 semantic cost를 덮어쓰지 않는다.
- platform-independent estimate와 allocator-specific observation을 구분한다.

## Canonical write set

- `facade/src/shared/measure.rs`
- `facade/src/shared/accounting.rs`
- 각 store entry construction/removal/stats 경로
- `core/src/domain/ledger.rs`는 charge-kind 확장이 필요한 경우만 QMP-020 owner와 serial coordination
- conformance fixtures

Owner symbols:

- `ByteSized::byte_size`
- proposed `RetainedCost::{total, checked_add}`
- payload `put_canonical` / `delete_if_unpinned`
- retained-map `insert_checked` / overwrite / `remove` / `sweep`
- revocable `insert_checked` / `reclaim_one`
- transcript `append` / `retain_sequences` / `truncate_to`
- prepared-publication `prepare` / `abort` / `gc_orphans`

## Required implementation per store

### Payload

- canonical key material 또는 retained digest/reference 비용을 포함한다.
- generated payload ref와 entry metadata overhead를 포함한다.
- dedup hit는 신규 cost를 charge하지 않는다.

### Retained map / revocable

- key cost를 value와 별도 계산한다.
- hash/link/node overhead policy를 문서화한다.
- overwrite는 old total cost에서 new total cost로 atomic recharge한다.

### Transcript

- session key, row message ID, sequence evidence, message evidence 비용을 포함한다.
- active body와 process-lifetime evidence를 다른 component로 관측한다.

### Prepared publication

- identity bytes, token/index, tombstone/root metadata 비용을 포함한다.
- GC/abort가 실제 제거한 component만 refund한다.

## Failure handling

- cost computation overflow: allocation/store mutation 전 typed error
- caller-supplied cost lie: safe bounds 또는 sealed implementation으로 방지
- replacement cost failure: old entry/charge 보존
- zero body + nonzero identity: identity cost만큼 반드시 charge

## RED tests to add

- `large_key_with_zero_value_is_charged`
- `large_identity_with_zero_payload_is_charged`
- `transcript_evidence_bytes_remain_charged_after_body_truncate`
- `prepared_identity_and_index_are_included_in_charge`
- `overwrite_recharges_total_entry_cost_atomically`
- `cost_component_sum_overflow_is_fail_closed`
- `stats_and_ledger_use_the_same_total_cost`
- `allocator_advisory_never_changes_semantic_cost`

Property tests:

- arbitrary key/value sizes에 대해 ledger total == live entry cost sum
- insert/overwrite/remove/truncate/abort sequence 후 exact model equality

## Acceptance

- [ ] public store에 retained but unmeasurable generic field가 없다.
- [ ] zero-value large-key exploit가 cap을 우회하지 못한다.
- [ ] tombstone/index byte가 stats와 ledger에서 사라지지 않는다.
- [ ] all arithmetic is checked; saturation으로 drift를 숨기지 않는다.
- [ ] formula version과 compatibility policy가 문서화된다.
- [ ] conformance가 exact total cost를 independently 계산한다.

## Verification

```bash
cargo test -p quanta-memory-platform-facade --all-features large_key_with_zero_value_is_charged
cargo test -p quanta-memory-platform-facade --all-features stats_and_ledger_use_the_same_total_cost
cargo test -p quanta-memory-platform-facade --all-features
cargo test -p quanta-memory-platform-testkit --all-features
```

## Non-goals

- actual allocator exact RSS billing
- tombstone eviction policy: QMP-060
- data structure performance: QMP-100
