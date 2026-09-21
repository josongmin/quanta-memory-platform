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
- caller-supplied cost lie: arbitrary `ByteSized` 구현의 허위 신고는 generic API로 검증할 수 없다. 지원 범위는 honest-client semantic estimate이며, 외부 erased handle에는 checked `MeasuredValue`를 사용한다. 물리 RSS enforcement는 별도 allocator authority 없이는 주장하지 않는다.
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

- [x] public store generic key/identity에 `ByteSized` 또는 checked measured wrapper가 요구된다 (honest-client estimate; 악의적 구현까지 검증하지 않음).
- [x] tested zero-value large-key/identity 경로는 cap을 우회하지 못한다.
- [x] tested transcript/prepared tombstone/index byte가 stats와 ledger에서 사라지지 않는다.
- [x] QMP-050 semantic component/lease arithmetic은 checked이며 saturation으로 cost drift를 숨기지 않는다. **Core ledger 전축 exactness는 별도 QMP-020**의 통합 proof를 요구한다.
- [ ] formula V1은 명명됐으나 public compatibility/migration policy는 QMP-140에서 확정해야 한다.
- [x] prepared/transcript에 independent literal oracle과 underbilling fake RED가 있다. 전체 facade store state-machine oracle은 QMP-090에서 확장한다.

## Verification

```bash
# Each selector must report at least one executed test, not merely exit 0.
cargo test -p quanta-memory-platform-facade --all-features large_key_with_zero_value_is_charged_and_denied_without_state_change
cargo test -p quanta-memory-platform-facade --all-features exact_retry_charges_once_and_truncate_refunds_only_active_body
cargo test -p quanta-memory-platform-facade --all-features prepared_identity_index_and_tombstone_keep_exact_independent_cost
cargo test -p quanta-memory-platform-testkit --all-features unbilled_identity
cargo test -p quanta-memory-platform-facade --all-features
cargo test -p quanta-memory-platform-testkit --all-features
```

## Non-goals

- actual allocator exact RSS billing
- tombstone eviction policy: QMP-060
- data structure performance: QMP-100

## Current scoped implementation receipt

At `b18e47d178297e6d1e781d96439002be46ad94fe`, QMP-050 code and
clean detached workspace doctor are `IMPLEMENTED`, not terminally qualified.
See [`receipts/2026-09-21T000848Z-b18e47d.md`](receipts/2026-09-21T000848Z-b18e47d.md).
The later QMP-020 core integration, final extraction/remote CI, general
conformance, and external host have independent authorities.

## Current-source design freeze (2026-09-21)

Historical design source: `fac68ead0487ba66bbec2147f746e440d80c3d6e`
(QMP-040 `IMPLEMENTED`; foreign workspace-lint changes excluded). This section
records the pre-implementation decision, not a current-source failure or GREEN
receipt. The current implementation and its scoped evidence are recorded in
the dated receipt below; historical line numbers remain source-snapshot hints.

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
| Transcript | pre-fix `transcript_store/service.rs:24-68,248-377,435-585`: session ID, row/body, retained message-ID copies, two digests, evidence indexes | At append, issue distinct durable evidence/session and active-row leases. Row removal batch-releases only active-row leases; evidence/session authority survives until eviction (QMP-060) or close. The pre-fix full-row refund was undercounting; current implementation has separate leases. |
| Prepared | pre-fix `publish_pipeline/prepared_publication.rs:196-240,430-525,541-701`: payload, retained identity, token/index overhead | At prepare, separate persistent identity-index/tombstone-compatible lease from active-only lease. Abort/GC batch-releases active-only leases and retains the persistent lease. The pre-fix `charged_bytes=0` with live tombstones was undercounting; current implementation retains a persistent lease. |

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

## Final-audit retained-cost delta — in-process spill identity clone

`RevocableSpillHandle` stores a cloned `StoreBindingHandle` per transferred
entry, while the retained-cost formula charges only the original key, value and
entry overhead. Large owner/runtime identifiers can therefore be deep-cloned
into many in-process tier handles without a corresponding delta charge.

QMP-050 owns the cost decision and QMP-070 owns the transfer implementation:

- make binding identity shared/interned (`Arc`-style) so transfer does not add a
  per-entry deep allocation, or
- include the destination identity delta in the transfer receipt and charge it
  before source publication is finalized.

Hellgate: many entries with large owner/runtime identifiers move to the in-
process tier and `ledger == source retained cost + destination-only delta`
after every move, restore, source close and tier drop.

## Current-source refresh (2026-09-21)

The indexed payload store retains a canonical-digest index and a reference-
digest index. `PayloadRef` therefore requires `Eq + Hash`, and the portable
semantic cost charges two retained digest slots. For the existing 32-byte
fixture the independent literal changes from 89 to 121 bytes. This is a real
public bound/cost-model migration and must be recorded by QMP-140 rather than
hidden behind compatibility prose.

The independent transcript/prepared underbilling fakes and all workspace rails
pass at `0c126a25aeab11f457e20c4a209fc7bb631ea859`; see
[`receipts/2026-09-21T072928Z-0c126a2.md`](receipts/2026-09-21T072928Z-0c126a2.md).
QMP-050 remains `IMPLEMENTED`: public version/migration policy and the
in-process spill identity-cost decision remain open.
