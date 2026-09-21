# QMP-070 — Accounting-safe reclaim/spill transfer and real backing capability

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-020, QMP-040, QMP-050  
Blocks: QMP-090, QMP-100, QMP-130, QMP-140

## Objective

reclaim된 value의 accounting ownership을 destination tier로 원자 이전하고, discard/in-memory/durable spill을 서로 다른 의미로 분리하며, backing feature가 실제 canonical store에서 도달 가능하게 한다.

## Confirmed evidence

- revocable `reclaim_one`은 refund 후 `(Key, Value)`만 반환: `revocable_store/service.rs:144-154`
- routed spill은 accounting-neutral: `spill/service.rs:54-59`
- in-memory target은 같은 process RAM에 handle을 유지: `spill/service.rs:114-131,174-185`
- `NoopSpillController`는 handle을 drop하고 `Ok(())`: `spill/service.rs:43-51`
- redb는 실제 `SpillTarget`이지만 reopened state와 controller count가 불일치.
- moka/scc는 standalone size index이고 store/backing port를 구현하지 않음.

## Required tier taxonomy

```text
Evict/Discard:
  destructive, explicit, no restore claim

InProcessTier:
  RSS remains resident; destination must retain semantic charge

DurableOffProcessSpill:
  destination commit/receipt required before source release
```

한 `spill()` success가 위 세 의미를 동시에 나타내면 안 된다.

## Required transfer protocol

```text
1. prepare source entry and immutable cost/receipt reference
2. reserve destination capacity/ownership
3. write destination
4. commit destination and obtain durable/tier receipt
5. remove source entry
6. release or transfer source charge
```

각 실패 지점에서 source 또는 destination 중 정확히 하나가 authoritative owner여야 한다.

## Canonical write set

- `facade/src/revocable_store/{mod,service,backings/**}.rs`
- `facade/src/spill/{mod,service,backings/**}.rs`
- relevant facade builders/features
- facade/testkit spill conformance

Owner symbols:

- `RevocableStore::{insert_checked, reclaim_one}`
- `SpillController::{spill, restore}`
- `SpillTarget::{is_available, accept, restore}`
- `NoopSpillController`
- `RoutedSpillController::{new, spilled_count}`
- `InMemorySpillTarget`, `RetainingSpillTarget`, redb target
- `MokaBacking`, `SccBacking`

## Required implementation

1. `ReclaimedEntry`가 value만이 아니라 cost와 transfer authority를 소유한다.
2. in-process target은 destination charge가 성공하기 전 source refund를 하지 않는다.
3. durable target은 fs/db commit 성공 후에만 source release가 가능하다.
4. restore는 reverse transfer를 수행하고 target receipt를 감소시킨다.
5. `NoopSpillController`를 typed unavailable로 바꾸거나 explicit `DiscardController`로 rename한다.
6. controller telemetry는 durable backing reopen 시 persisted inventory에서 reconcile한다.
7. backing port를 canonical revocable store/builder가 실제 사용한다.
8. moka autonomous eviction에는 accounting callback/receipt 반환이 필요하다. 제공할 수 없으면 feature를 제거/experimental rename한다.
9. scc도 동일 semantic port와 conformance를 통과해야 한다.
10. redb corruption/offline/key exhaustion은 source ownership을 보존한다.

## RED tests to add

- `reclaim_to_in_memory_tier_keeps_bytes_charged`
- `durable_spill_releases_source_only_after_commit`
- `failed_destination_write_preserves_source_entry_and_charge`
- `failed_source_finalize_reconciles_destination_without_double_owner`
- `restore_transfers_charge_back_exactly_once`
- `noop_spill_is_not_successful_persistence`
- `reopened_redb_reconciles_controller_count`
- `moka_and_scc_backings_pass_revocable_semantic_suite` 또는 feature removal compile assertions
- concurrency/model: spill vs reclaim/restore/offline transition

## Acceptance

- [ ] process-resident bytes가 uncharged 상태로 tier 이동하지 않는다.
- [ ] spill success는 destination ownership level을 typed result로 표현한다.
- [ ] destructive discard는 explicit API다.
- [ ] restart 후 retained inventory와 controller telemetry가 일치한다.
- [ ] advertised backing feature가 canonical builder에서 reachable하다.
- [ ] 모든 backing이 동일 semantic conformance suite를 통과한다.

## Verification

```bash
cargo test -p quanta-memory-platform-facade --all-features reclaim_to_in_memory_tier_keeps_bytes_charged
cargo test -p quanta-memory-platform-facade --all-features reopened_redb_reconciles_controller_count
cargo test -p quanta-memory-platform-testkit --all-features spill
tools/ci/feature-matrix.sh
cargo test -p quanta-memory-platform-facade --all-features
```

## Non-goals

- general broker pressure trigger: QMP-080
- performance threshold: QMP-100
- external host storage choice: QMP-130

## Current-source delta: failure consumes the only handle (2026-09-21)

At `47021d3`, `revocable_store/service.rs:258-284` removes the source and refunds its lease before returning `(Key, Value)`. `spill/service.rs:81-99` then consumes `Handle` by value: if no destination is available or `accept` fails, the handle drops on `Err`, so the caller cannot roll back the source or retry another destination. This is in addition to the uncharged in-process tier and `NoopSpillController` successful discard already recorded above. The QMP-070 protocol must return owned source recovery authority on every pre-commit error, or keep the source intact until destination receipt commits; an `Err` with no handle is not fail-closed. Add a fault-injected unavailable/accept-error test asserting source value+charge survive and destination has no receipt; then inverse restore and reopen tests. This is static reachability evidence, not an executed negative test.

## Implementation checkpoint (2026-09-21; partial, not acceptance)

The isolated QMP-070 owner branch introduces a `SpillFailure<Handle>` that returns the exact handle on every ordinary pre-commit `Err`. `NoopSpillController` now rejects instead of silently discarding. `SimpleRevocableStore<_, _, Budgeted>::spill_one_into` moves a non-clone `RevocableSpillHandle` with its existing `ManagedCharge` to an in-process target; an ordinary destination rejection restores the source entry at its original FIFO position without a refund. `accept_restored` validates the original binding, broker identity and charge axes before reattaching. Closing the source does not refund a charge still held by the tier; retiring the tier handle refunds it. The redb byte target returns the original bytes on failed commit.

The next checkpoint removes the controller's duplicate atomic count. `SpillInventory` makes the target's retained inventory authoritative, so a controller created over a reopened redb database immediately reports the persisted record count. Successful writes return a typed `SpillReceipt`: in-process targets return `InProcess { inventory_after }`, while redb returns `Durable { sequence, inventory_after }` only after commit. `InProcessSpillController` / `InProcessSpillTarget` marker contracts prevent the charge-carrying revocable handle path from accepting a durable byte tier by accident.

These tests prove only that scoped in-process path, ordinary returned errors, typed tier receipt, and restart-safe inventory observation. They **do not** prove the ticket's revocable-to-durable transfer, canonical moka/scc integration or removal, concurrent spill/reclaim/restore, or panic/abort fault model. In particular, a `SpillController` implementation that panics after consuming a handle can still lose source recovery authority. A redb commit error also needs post-error inventory reconciliation before the returned byte handle can safely be retried: a failed/ambiguous commit cannot be treated as proof that no destination record exists. The protocol must not be advertised as transactional across untrusted callbacks or ambiguous durable commits. Keep QMP-070 `IN_PROGRESS` until the remaining acceptance items have source-bound receipts.
