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

- [x] process-resident bytes가 uncharged 상태로 tier 이동하지 않는다.
- [x] spill success는 destination ownership level을 typed result로 표현한다.
- [x] destructive discard는 persistence API에서 제거됐다. `NoopSpillController`는 원본 handle을 포함한 typed failure를 반환한다.
- [x] restart 후 retained inventory와 controller telemetry가 일치한다.
- [x] 유일한 advertised production backing인 redb가 canonical revocable durable transfer에서 reachable하다. 계약을 구현하지 않던 moka/scc feature와 dependency는 제거됐다.
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

The durable revocable checkpoint adds a separate encoded-byte protocol. `spill_one_durable` keeps the source entry and charge live through encoding and destination commit, restores the original FIFO entry on encoder error/panic or returned destination error, and releases the source only after `SpillReceipt::Durable`. Reverse transfer uses `prepare_restore` to read without deletion, admits the exact source charge, then deletes the exact durable sequence and publishes locally. Admission or a known-retained delete failure leaves the durable record authoritative and rolls back the provisional source charge. A commit-ambiguous delete publishes a conservatively charged source and returns `DurableRestoreOutcome::RecoveryRequired { sequence, ... }`; it does not claim exactly-once completion. The redb round-trip, denial, retained-error, and ambiguity-classification paths execute this protocol.

These tests prove the scoped in-process path, ordinary returned errors, typed tier receipt, restart-safe inventory observation, and the normal/returned-error durable transfer path. They **do not** prove canonical moka/scc integration or removal, concurrent spill/reclaim/restore, or the complete panic/abort and I/O ambiguity model. In particular, a custom `SpillController` that panics after consuming a handle can still lose source recovery authority. redb's `commit()` may return after entering its commit path, so a storage-level commit error requires persisted intent/reconciliation before the returned byte handle can safely be retried or an exact restore deletion can be classified. The current protocol must not be advertised as transactional across untrusted callbacks or ambiguous durable commits. Keep QMP-070 `IMPLEMENTED`, never `QUALIFIED`/`DONE`, until those remaining acceptance items have source-bound receipts.

## Final-audit P1-F02 — durable commit ambiguity has no reconciliation owner

`spill/backings/redb.rs:82-115` maps every write-transaction commit error to
ordinary `SpillFailure` and returns the encoded bytes. `spill_one_durable`
(`revocable_store/service.rs:360-375`) consequently restores the original source
entry and charge. If redb durably committed but lost the acknowledgement, the
durable record and source are both authoritative. The inverse path classifies
delete ambiguity, but publishes a conservatively charged local entry while the
same durable sequence may remain visible; a second restore can duplicate it.

Required repair:

1. Add a durable-write outcome distinct from precommit `SpillFailure`:
   `RetainedSource`, `Committed(receipt)`, or `RecoveryRequired(intent_id,
   sequence?, error)`.
2. Persist an idempotency/transfer intent before source retirement and reconcile
   it on reopen. Encoded payload identity must be stable across retry.
3. Fence an ambiguous sequence from ordinary FIFO restore until reconciliation;
   repeated restore may not republish the same record.
4. Record redb durability mode explicitly and test crash points before write,
   after table mutation, during commit, after durable commit/before response,
   and during exact delete.
5. Assert after every recovery that exactly one authority is active and that
   inventory, source state, charge total, and intent table agree.
6. Remove the destructive generic redb `SpillTarget::restore` path or make it
   return the prepared bytes together with retained/ambiguous commit outcome.
   It currently discards the only in-memory candidate when delete commit is
   ambiguous; if the record was deleted, that is permanent data loss.
7. Persist a monotonic generation/next-sequence authority. Computing the next
   key from `table.last()` reuses sequence 0 after a full drain, so a stale
   `commit_restore(0)` can delete a newly inserted record (ABA).
8. Make controller panic an explicit trust boundary: seal trusted controllers or
   hold a panic-safe transfer guard. Test panic before accept and panic after
   destination commit separately; the latter still requires reconciliation.

Use redb's transaction durability guarantee only for successful `commit` return;
it does not turn an error return into proof that no durable postimage exists.

## Static hardening checkpoint (2026-09-21; runtime qualification deferred)

Current source adds the ownership protocol required by P1-F02 without claiming
runtime qualification:

- `DurableRecordId` is allocated from persisted monotonic metadata. Emptying the
  queue or reopening the database does not reuse an ID.
- `ReconciledDurableSpillController` borrows encoded bytes. Ordinary failure
  cannot consume the source's only recovery copy; an unknown commit outcome is
  fenced in `pending_spills` until an exact record probe resolves it.
- redb successful/unknown writes are idempotent by stable ID and payload bytes.
  A persisted finalized watermark rejects stale resurrection after restore.
- durable restore preparation persists one exclusive active sequence before
  returning bytes. Decode/admission/known-retained failures abort that claim.
  Ambiguous delete keeps the provisional resident entry charged but unreadable
  in `pending_restores`; persisted reconciliation either retires it while redb
  remains authoritative or publishes it after finalization.
- accounting-bearing controller/target marker traits are sealed to canonical
  implementations. The in-process transfer handle shares binding identity via
  `Arc`, avoiding an uncharged deep clone of caller-controlled strings per row.
- generic redb `SpillTarget::{accept,restore}` now fail closed because that API
  cannot express durable unknown outcomes. The reconciled write and claimed
  two-phase restore paths are the only mutating redb paths.
- `revocable-moka` and `revocable-scc` were removed from the manifest, feature
  matrix, production source and dependency graph. They were standalone size
  indexes, not implementations of the canonical revocable-store contract.

Static proof on the dirty implementation tree: `cargo fmt --all --check`,
`cargo check --workspace --all-targets --all-features`, and
`cargo clippy --workspace --all-targets --all-features -- -D warnings` passed.
The warning-denying per-feature `tools/ci/feature-matrix.sh` initially exposed
and then verified repairs for core no-default and facade revocable-only feature
closure; its final run passed every combination. This is not a runtime receipt
and does not qualify the ticket.

Remaining qualification/architecture gaps:

1. Run the focused durable/spill tests, workspace tests, feature matrix, doctor,
   deny, and actual extraction on one frozen source SHA. The user explicitly
   deferred dynamic verification for this checkpoint.
2. Add deterministic redb fault injection at commit acknowledgement boundaries;
   current source models the outcomes but does not force real redb commit faults.
3. A process crash after durable restore deletion but before the volatile
   resident copy is published can still lose the payload. Closing that requires
   a durable recovery-payload/ack phase, not another in-memory flag.
4. QMP-090 still owns concurrency/model proof across spill, restore, reclaim,
   restart and panic schedules. Static compilation is not that proof.
