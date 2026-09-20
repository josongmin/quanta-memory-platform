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

## Current-source design freeze (2026-09-21)

Source: `fac68ead0487ba66bbec2147f746e440d80c3d6e` (QMP-040
`IMPLEMENTED`; foreign workspace-lint changes excluded). This is a STATIC
design decision, not a QMP-050 implementation or GREEN receipt.

### One portable semantic-cost authority

Add `RetainedCostV1` in `facade/src/shared/measure.rs` with checked
`payload_bytes`, `key_identity_bytes`, `index_bytes`, `tombstone_bytes`, and
`fixed_overhead_bytes` components. Named/versioned per-structure constants
represent portable semantic retention, **not** platform `size_of`, allocator
capacity, RSS, or transient cloning. `total()` and component aggregation must
fail before charging or changing an entry. Store the immutable total with each
owned lease; charge, stats, reclaim and release read the same value. A digest
or key generated only to decide identity is not automatically retained cost.

`ByteSized` is a caller-provided estimate (`shared/measure.rs:12-75`), not a
proof about arbitrary `T`'s heap allocations. Adding `Key: ByteSized` closes
the unmeasurable-key API hole for honest implementations, but cannot detect a
malicious underreporting implementation. The accepted contract must state that
limitation and use checked, measured wrappers for erased handles; do not claim
exact physical-memory enforcement without a separate allocator authority.

### Owner mapping and lifetime split

| Store | Retained formula / source path | Required ownership transition |
| --- | --- | --- |
| Payload | `payload_store/service.rs:24-31,238-340`: body, retained reference/digest, pin/entry semantics; temporary canonical-key Vec must **not** be billed twice | Dedup hit has no new lease; deletion releases the entry's full persisted cost. |
| Retained map | `retained_map/service.rs:25-42,254-435,462-612`: key + value + named hash/link node cost | Add `Key: ByteSized`; overwrite recharges complete old/new cost atomically while preserving retained key identity; sweep targets/refunds complete cost. |
| Revocable | `revocable_store/service.rs:16-38,121-280`: key + value + FIFO entry cost | Add `Key: ByteSized`; reclaim_one/until and stats use the same total as the lease. |
| Transcript | `transcript_store/service.rs:24-68,248-377,435-585`: session ID, row/body, three retained message-ID copies, two digests, evidence indexes | At append, issue distinct durable evidence/session and active-row leases. Row removal batch-releases only active-row leases; evidence/session authority survives until eviction (QMP-060) or close. Current full-row refund with surviving evidence is undercounting. |
| Prepared | `publish_pipeline/prepared_publication.rs:196-240,430-525,541-701`: payload, two identity Vec copies, retained canonical key, token/index overhead | At prepare, separate persistent identity-index/tombstone-compatible lease from active-only lease. Abort/GC batch-releases active-only leases and retains the persistent lease. Move the owned token key into the aborted index rather than clone it if both states use the same named semantic cost. Current `charged_bytes=0` can coexist with live tombstones. |

Use `u64::try_from` for lengths and checked sums before every broker call.
Public generic bounds, builders, examples, stats, and the independent testkit
oracle change together; a hardcoded old `7 -> 0` test oracle is not evidence
after a formula change (`testkit/src/conformance/suites.rs:190-215`). QMP-060
owns tombstone eviction policy; QMP-050 owns its accounting while retained.

### RED/qualification order

1. Formula overflow, zero-size payload with large key/identity, and a dedup
   hit with no extra charge.
2. Retained-map overwrite denial and sweep; revocable reclaim; each must show
   exact ledger == stats == persisted entry-cost sum, including failed ops.
3. Transcript truncate leaves evidence/session bytes charged; prepared abort/GC
   leaves exact tombstone bytes charged. Close/Drop still zero all leases.
4. Add an independently calculated cost oracle and underbilling fake to
   testkit. A fixture adjusted merely to match the implementation is invalid.
5. Cross-feature Clippy/test/examples, one-shot rails, then exact-source receipt.

Serial owner barrier: shared `measure.rs` and public formula first; payload and
retained-map, revocable, transcript, and prepared may then use disjoint leaf
write sets. Integrate `shared/builder.rs`, exports, testkit and docs only after
the leaf contract stabilizes. `shared/accounting.rs` and core ledger require
coordination if an atomic split/recharge primitive is truly necessary; do not
add a second accounting authority in a store. In particular, **do not** loop
over per-entry `recharge` during transcript truncate or prepared batch GC: a
later denial after an earlier successful recharge splits the batch. Allocate
separate lifetime leases at ingress and use existing atomic `release_many` for
the active component. Any alternate delta design needs one atomic managed
batch operation with a QMP-020 owner receipt.
