# QMP-040 — Store typestate, explicit close, and lock-safe accounting

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-020, QMP-030  
Blocks: QMP-050, QMP-060, QMP-070, QMP-090, QMP-100, QMP-110

## Objective

모든 facade store에서 accounting mode를 construction 전에 고정하고, store 종료 시 live charge를 회수하며, public accounting callback을 container mutex 안에서 실행하지 않게 한다.

## Affected stores

- payload store
- retained map
- revocable store
- transcript store
- prepared publication service

## Confirmed evidence

- charged entry마다 ledger receipt가 있지만 facade에 `Drop`/`close` 구현이 없다.
- payload/revocable/transcript/prepared는 populated object에 `charged_against`를 적용할 수 있다.
- retained map만 builder-only charged construction을 사용한다.
- accounting callback이 store mutex 아래서 실행된다.
- payload/revocable의 saturating counters가 overflow를 숨긴다.
- payload/revocable/transcript 일부 stats는 poison을 zero로 반환한다.

## Canonical write set

- `facade/src/shared/{builder,accounting}.rs`
- `facade/src/payload_store/{mod,service}.rs`
- `facade/src/retained_map/{mod,service}.rs`
- `facade/src/revocable_store/{mod,service}.rs`
- `facade/src/transcript_store/{mod,service}.rs`
- `facade/src/publish_pipeline/prepared_publication.rs`
- facade/testkit store tests

QMP-050은 byte cost 산정만 소유하고, 이 ticket은 lifecycle/choreography를 소유한다.

Owner symbols and mutation call sites:

- `AccountingSink::{charge, refund, refund_many, recharge}`
- `SimplePayloadStore::{new, charged_against}` and `PayloadStore::{put_canonical, delete_if_unpinned, stats}`
- `SimpleRetainedMapStore::{new, new_charged}` and `RetainedMapStore::{insert_checked, remove, sweep, stats_checked}`
- `SimpleRevocableStore::{new, charged_against}` and `RevocableStore::{insert_checked, reclaim_one}`
- `SimpleSessionTranscriptStore::{new, charged_against}` and transcript append/retain/truncate/stats paths
- `PreparedPublicationService::{new, charged_against}` and prepare/abort/GC paths

## Required construction model

1. `Budgeted`와 `Unbudgeted`를 다른 builder state/type으로 표현한다.
2. sink는 materialization 전에 선택한다.
3. constructed store에는 public `charged_against(self, ...)`가 없다.
4. unbudgeted constructor는 `new_unbudgeted`처럼 이름으로 드러나고 production feature에서 금지/격리 가능해야 한다.
5. public `Simple*::new`가 builder-first를 우회하지 못한다.

## Required lifecycle model

1. `close/drain`은 live entries의 charge를 batch로 release하고 store를 closed state로 전환한다.
2. close 실패 시 store entry와 charge ownership이 함께 보존된다.
3. close 성공 후 mutation은 typed `Closed` error다.
4. `Drop` 정책을 명시한다.
   - preferred: already-closed assertion + best-effort emergency cleanup telemetry
   - silent ledger leak 또는 panic 금지
5. multiple logical clones/Arc ownership이 가능하면 마지막 owner만 close하는 구조를 명시한다.

## Lock choreography

외부 sink callback을 mutex 안에서 호출하지 않는다. 권장 protocol:

```text
lock -> read version + prepare immutable mutation plan -> unlock
accounting operation outside store lock
lock -> verify version/state -> apply
conflict -> compensate accounting -> retry or typed conflict
```

다른 허용 설계는 sink를 sealed/trusted non-reentrant authority로 좁히고 global lock order를 type/module boundary에서 강제하는 경우뿐이다.

## Counter/error requirements

- retained byte add/sub는 checked arithmetic을 사용한다.
- overflow/underflow는 mutation 전에 typed error다.
- `stats()`가 poison을 empty로 위장하지 않는다.
- infallible legacy stats가 필요하면 `Unknown/Unavailable`을 표현하는 별도 observation type을 사용한다.

## RED tests to add

Construction:

- 각 store별 `cannot_attach_sink_after_materialization` compile-fail
- `budgeted_builder_requires_sink_before_build`
- `unbudgeted_store_type_cannot_be_passed_as_budgeted_store`

Lifecycle:

- 각 store별 `close_refunds_every_live_entry_exactly_once`
- `close_refund_failure_preserves_store_and_ledger_ownership`
- `drop_without_explicit_close_does_not_strand_capacity`
- `mutation_after_close_is_rejected`
- `double_close_is_idempotent_or_typed_without_double_refund`

Concurrency/failure:

- `reentrant_accounting_sink_does_not_deadlock`
- `concurrent_mutation_and_close_has_one_linearized_outcome`
- `counter_overflow_is_rejected_without_mutation`
- `poisoned_stats_is_error_not_empty`

## Acceptance

- [x] late attach API가 public surface에서 사라진다.
- [x] charged store가 종료된 뒤 ledger/group usage가 정확히 zero다.
- [x] close failure가 ticketless entry나 phantom refund를 만들지 않는다.
- [x] public dyn callback이 store mutex 아래서 호출되지 않는다.
- [x] all stores가 동일 lifecycle contract/conformance suite를 통과한다.
- [x] checked overflow와 poison truth가 trait level에 반영된다.

## Verification

```bash
cargo test -p quanta-memory-platform-facade --all-features close_refunds_every_live_entry_exactly_once
cargo test -p quanta-memory-platform-facade --all-features reentrant_accounting_sink_does_not_deadlock
cargo test -p quanta-memory-platform-facade --all-features
cargo test -p quanta-memory-platform-testkit --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Non-goals

- key/metadata byte formula: QMP-050
- tombstone retention policy: QMP-060
- spill ownership transfer: QMP-070
- performance container rewrite: QMP-100

## Frozen audit receipt (2026-09-21)

Audit base: `f0841e5d9d77114e0a50ca69ddafa1cd47599e73`
Implementation prerequisite checkpoint: `ecbeacb` (`ManagedCharge` + shared lifecycle gate)

### Confirmed blockers at the frozen base

| Severity | Finding | Reachable evidence | Required owner fix |
| --- | --- | --- | --- |
| P0 | accounting mode can change after materialization | public `Simple*::new` plus `charged_against(self, ...)` in payload/revocable/transcript/prepared | construction typestate; no `build` before mode selection; no late attach |
| P0 | store destruction strands committed ledger bytes | every charged entry owns a clonable receipt, but no store `close`/`Drop` owns refund | broker-bound non-Clone RAII lease; explicit batch close; Drop cleanup |
| P1 | arbitrary public callback runs under non-reentrant store mutex | `AccountingSink` is public and all five stores call charge/refund/recharge under their container lock | remove dyn callback seam; concrete sealed broker authority only |
| P1 | naive unlock/callback/relock would lose linearizability | duplicate insert, overwrite/remove, sweep, transcript append and prepare can race between planning and apply | one nonblocking lifecycle mutation lane; accounting outside container mutex; typed busy |
| P1 | close/mutation behavior is unspecified | no `Open/Closing/Closed` authority and ticket selectors execute zero tests | shared lifecycle gate and owner tests on all five stores |
| P1 | stats can fabricate empty truth after poison | payload/revocable/transcript infallible projections default to zero | checked trait surface returning typed error |
| P1 | local counters can silently drift | payload/revocable saturating arithmetic | checked preimage/postimage before any payload or ledger mutation |
| P1 | public conformance does not cover the lifecycle | testkit omits revocable lifecycle and has no common close/drop oracle | five adapters plus broken leak/state-loss/double-refund fakes |

The exact callback-under-lock base paths were:

- payload insert/delete: `payload_store/service.rs:166-191`, `:210-230`
- retained insert/overwrite/remove/sweep: `retained_map/service.rs:151-186`, `:201-234`, `:329-343`, `:352-393`
- revocable batch/insert/reclaim: `revocable_store/service.rs:79-107`, `:124-151`
- transcript retain/append/truncate: `transcript_store/service.rs:192-198`, `:253`, `:288-342`, `:391-396`
- prepared prepare/abort/GC: `publish_pipeline/prepared_publication.rs:288-351`, `:386-397`, `:440-472`

These line references are evidence for the frozen base, not the post-fix tree.

### Selected implementation architecture

```text
LiveMemoryPlatform
  -> budgeted builder state (contains concrete broker authority)
  -> budgeted store type
  -> one ManagedCharge per live entry

explicit close:
  lifecycle Open -> Closing
  -> detach entries under container mutex
  -> release_managed_many outside container mutex
  -> success: Closed
  -> failure: restore exact entries/leases, reopen Open

implicit Drop:
  -> detach entries with Mutex::get_mut
  -> ManagedCharge::drop
  -> normal: exact release
  -> rebuild active: broker-owned deferred queue
  -> finish_rebuild: atomic deferred drain
```

Decisions:

1. Delete `AccountingSink`, `SharedAccountingSink`, and `LiveMemoryPlatform::sink()` from the public surface.
2. A budgeted builder is issued only by `LiveMemoryPlatform`; it cannot be rebound.
3. `AccountingUnset` deliberately has no `build` method.
4. `Unbudgeted` is a different product type, available only through the non-default `unbudgeted-stores` feature. `default` and `full` must not enable it.
5. Store mutations acquire the shared atomic lifecycle permit before touching state. Reentry or close contention returns `MemoryError::OperationInProgress`; post-close mutation returns `MemoryError::Closed`.
6. Every entry and its `ManagedCharge` move together. No ticket clone is an ownership authority.
7. Drop never panics. During rebuild it preserves conservative accounting until `finish_rebuild` drains the broker queue.

### Industry reference alignment

- Apache DataFusion models retained memory with `MemoryReservation`, explicitly releases the reservation on Drop, and makes the consumer responsible for reserving bytes before retaining producer output: <https://docs.rs/datafusion/latest/datafusion/execution/memory_pool/trait.MemoryPool.html>
- Tokio's `OwnedSemaphorePermit` is an owned, non-borrowed capacity authority whose Drop releases permits, and batch ownership is expressed by consuming/merging owned permits: <https://docs.rs/tokio/latest/tokio/sync/struct.OwnedSemaphorePermit.html>
- Rust's `Drop` guidance treats memory/capacity as owned resources and warns destructors not to panic, especially during unwind: <https://doc.rust-lang.org/std/ops/trait.Drop.html>

The selected design follows those ownership properties but adds a repo-specific
fail-closed rebuild rule: Drop during a closed rebuild fence defers the exact
ticket instead of mutating counters or losing authority.

## Execution tickets

### QMP-040A — Core ownership primitive — checkpointed

Write set:

- `core/src/api/control.rs`
- `core/src/lib.rs`

Deliverables:

- non-Clone `ManagedCharge`
- failure-preserving explicit release/recharge
- same-broker atomic batch release
- rebuild-safe deferred Drop queue and finish drain
- poison recovery restricted to exact cleanup

Checkpoint: `ecbeacb`

### QMP-040B — Construction boundary

Write set:

- `facade/Cargo.toml`
- `facade/src/shared/{accounting,builder}.rs`
- facade root/module exports
- public compile-fail surface tests

Hellgates:

- `budgeted_builder_requires_platform_authority`
- `cannot_attach_accounting_after_materialization`
- `unbudgeted_store_type_cannot_be_passed_as_budgeted_store`
- feature graph proves `full` excludes `unbudgeted-stores`

### QMP-040C — Five owner migrations

Each of payload, retained map, revocable, transcript, and prepared publication
must independently prove:

- `close_refunds_every_live_entry_exactly_once`
- `close_refund_failure_preserves_store_and_ledger_ownership`
- `drop_without_explicit_close_does_not_strand_capacity`
- `drop_during_rebuild_defers_until_finish`
- `mutation_after_close_is_rejected`
- `double_close_does_not_double_refund`
- `concurrent_mutation_and_close_has_one_linearized_outcome`
- `counter_overflow_is_rejected_without_mutation`
- `poisoned_stats_is_error_not_empty`

### QMP-040D — Public conformance bite

Write set:

- `testkit/src/conformance/**`
- `testkit/tests/facade_conformance_bite.rs`
- `testkit/Cargo.toml`

Required oracle:

1. populate: broker bytes equal store bytes;
2. close: broker zero and store closed;
3. double close: still zero;
4. post-close mutation: typed `Closed`;
5. rebuild close failure: store and broker postimage unchanged;
6. rebuild finish + retry: zero;
7. last-owner Drop: zero;
8. rebuild-time Drop: conservative until finish, then zero.

Required broken fakes:

- `LeakingCloseFake`
- `StateLosingCloseFake`
- `DoubleRefundFake`

The suite is not accepted merely because honest targets pass; every broken fake
must produce a conformance violation.

## Qualification receipt requirements

QMP-040 may move to `IMPLEMENTED` only with all of the following recorded under
`receipts/`:

- frozen product commit and clean dirty fingerprint;
- non-zero owner selector counts for all five stores;
- full facade and testkit results;
- clippy `-D warnings`, feature matrix, doctor, deny, examples;
- detached-worktree RED mutation that removes one close/Drop release and is caught by the named semantic oracle;
- exact remaining gap to `QUALIFIED` (standalone extraction and external host adoption remain QMP-120/QMP-130).

## Adversarial delta audit — current dirty implementation (2026-09-21)

The five owner tests passing is not terminal proof. During implementation, a
separate read-only adversarial pass found panic/reentrancy paths missed by the
original checklist. Paths below refer to the current implementation; line
numbers must be re-frozen with the final product SHA.

| Severity | Reachable failure mode | Current action / exit gate |
| --- | --- | --- |
| P0, repaired locally, not yet qualified | `retained_map/service.rs` overwrite dropped old `Value` after broker recharge but before the committed-byte postimage; `Value::drop` panic left ledger and store divergent | Publish the full new state before retiring old value; `panic_in_old_value_drop_cannot_split_committed_bytes_from_ledger` must pass. |
| P0, repaired locally, not yet qualified | retained-map `close` released all leases then cleared entries before committing Closed; a panicking `Value::drop` reopened a ticketless partial map | Commit empty terminal state and Closed before retiring old map; `panic_in_close_value_drop_keeps_terminal_image_and_zero_ledger` must pass. |
| P0, repaired locally, not yet qualified | retained-map `sweep` extracted charges and restored/removed entries through `Key::Hash/Eq` after the plan began; callback panic caused partial lease Drop or ticketless entries | Positional selection, validation, extraction and rollback, then callback-free `retain` after broker release; panic-key and panic-value semantic oracles must pass. |
| P1, repaired locally, not yet qualified | transcript retain invoked user `SessionId::Ord` for map lookup after detaching charged rows, including broker-denied rollback | Stable session ordinal under mutation permit; positional lookup after broker call, with panic-Ord negative and full row/ledger postimage tests. This changes retain lookup to O(session count), requiring QMP-100 workload review. |
| P1, repaired locally, not yet qualified | store `Drop` paths invoked arbitrary client destructors; multiple panicking fields during an existing unwind could abort the process | Shared per-field `retire_client_value`, with managed charges dropped first, applied to all five store owners. Require owner tests for multiple panicking fields during outer unwind on every generic store before receipt; a single `catch_unwind` over a whole collection is insufficient. |
| P1, repaired locally, not yet qualified | generic `Hash/Eq/Clone/CanonicalKey/Ord` callbacks still execute under some store mutexes (e.g. retained `get_checked`, payload duplicate scan, transcript snapshot/replay, prepared resolve) | Shared nonblocking state-lock entry makes any same-store callback reentry return typed `OperationInProgress`, including read→read/close; owner-local callback tests are required on all affected paths. Ordinary lock contention now also returns Busy, an explicit API/scaling tradeoff to revisit in QMP-100. Accounting callback elimination alone was insufficient. |
| P1, repaired locally, not yet qualified | `retained_map::sweep` at `e047215` used hashlink's `retain` after refund; hashlink can run multiple client destructors from one filtered collection | Pre-refund raw identity/hash validation, callback-free owned removals after refund, then per-field retirement outside the mutex. Old code failed the multi-panic subprocess (child 101); new outer-unwind/two-key/two-value test, Hash reentry Busy, and no-post-refund-Hash oracles pass. Re-run exact-source qualification. |
| P1, repaired locally, not yet qualified | transcript append at `e047215` published session version and charged row before client `MessageId::clone` and `MessageId::Ord` (`transcript_store/service.rs:520-536`) | Old code failed panic-on-Clone/Ord version-postimage tests; preflight two clones and index vacancies, then publish only infallible owned values; single-callback panic releases the lease outside the mutex and preserves readable preimage. Re-run exact-source qualification. |
| P2, unresolved exceptional abort | `BTreeMap::entry` takes a generic session/message key; if its `Ord` panics while the state mutex is held, a second panic in the owned key's `Drop` can abort before an outer `catch_unwind` regains control | Single-callback panic must preserve the ledger/store and keep the mutex readable. Full double-panic containment requires a different index ownership/layout (for example, an outside `Arc` owner) or a documented key contract; do not claim arbitrary adversarial `Ord`+`Drop` safety from the current catch boundary. |

The P0/P1 implementation blockers above were repaired at `85cbd81` and the
internal acceptance/receipt gates passed on clean `eb58374`; see
`receipts/2026-09-20T231633Z-eb58374.md`. At that historical checkpoint QMP-040
was `IMPLEMENTED`, **not** `QUALIFIED` or production-ready. The P2 double-panic case is an explicit
generic-key contract boundary: stable `Ord`, and no panicking key `Drop` during
unwind. A blanket guarantee for arbitrary adversarial `Ord`+`Drop` requires
index ownership redesign, not another catch shim. QMP-140 must carry this
limitation into public documentation; QMP-100 must measure the indexing and
contention tradeoff. QMP-120 actual extraction and QMP-130 host adoption remain
separate terminal gates.

### Host adapter migration dependency (coordination, not qualification)

The external Semantica `quanta-memory-host` owner reported 37 compile errors
while consuming the in-progress public API cutover, before its owner-local QBC
could execute. That count is an external report, not independently reproduced
in this repository. Do not restore `SharedAccountingSink`/late attachment to
make the old adapter compile. After this ticket has a frozen product SHA,
migrate the host adapter to platform-issued budgeted builders, mandatory mode
types, typed close/stats, and the feature-gated pin boundary. QMP-130 owns the
separate exact-head host qualification; this report is neither QMP-040 product
failure proof nor host-adoption closure.

## Reopened adversarial delta — destructive leaf and callback panic (2026-09-21)

At historical product `47021d3271c3613c861ad4c06c4d3c2b4b1edb17` plus then-in-progress QMP-050 leaf changes, prior QMP-040 receipt `eb58374` remained valid **only for its then-tested source**. The following paths were not covered by that receipt; their old line numbers and failures are historical RED, not a claim that the current product still has raw destructive leaves.

| Finding/proof | Reachable path | Canonical repair and negative gate |
| --- | --- | --- |
| P1-D02, isolated subprocess RED | `payload_store/service.rs:340-347` releases the charge and publishes removal, then raw `drop(removed)` destroys generic `PayloadRef` and `Bytes` together. Two panicking field destructors caused child signal 6. `retire_entries` at `:34-40` already provides per-field containment in close/Drop but this leaf bypasses it. | Retire each generic field through the same canonical helper after exact refund/postimage, outside locks; child process with two panicking fields and outer unwind must exit normally, ledger zero and store readable after catching the caller panic. |
| P1-D02, isolated subprocess RED | `retained_map/service.rs:516-522` publishes refund/removal, then raw `drop(retired_key)` before returning `removed.value`. A key destructor panic unwinds and drops the undelivered value; a second destructor panic caused child signal 6. | Retire key field in per-field guard after publication. Test a panicking key/value pair under outer unwind; require no abort, exact zero ledger, and no hidden value ownership ambiguity. |
| P2-D01, static (no RED yet) | Client `CanonicalKey`/`Clone`/`Hash`/`Eq` callbacks still run under the container mutex: payload dedup/new put `:249-252,295-302`, transcript replay/load `transcript_store/service.rs:555-569,761-772`, retained-map get `retained_map/service.rs:491-494`. A callback panic poisons the mutex and `shared/lifecycle.rs:69-79` then permanently rejects reads/retries. The payload new-put clone occurs **before** `Vec::push`; no postcommit ghost-entry claim is made. | Choose a supported-client panic contract. If arbitrary callback panic must be recoverable, move callback computation outside the lock and use versioned optimistic validation/callback-free publish; do not merely ignore poison or catch around a partially published state. Add each callback panic followed by readable preimage, retry, and ledger equality. If panic is excluded, document and compile/type-bound what can be enforced; keep remaining risk explicit. |

RED receipt: detached clean `eb58374` at `/tmp/qmp-red-yvJZM3/source`, temporary owner subprocess tests `red_delete_with_two_panicking_client_fields_must_not_abort` and `red_remove_with_panicking_key_and_value_must_not_abort`. Each `cargo test -p quanta-memory-platform-facade --all-features <selector> -- --nocapture` returned exit 101; its child process exited on signal 6 (`reference drop trap`→`body drop trap`, or `retired key drop trap`→`unreturned value drop trap`→"panic in a destructor during cleanup; aborting"). The fixture is uncommitted and not a product GREEN receipt.

At current product `b18e47d` (also present in `59f0c4f`), payload delete calls `retire_entry(removed)` and retained-map remove calls `retire_client_value(retired_key)` after exact refund, outside locks. Owner subprocess tests `delete_contains_two_panicking_client_destructors_after_exact_refund` and `remove_contains_panicking_key_and_returns_value_after_exact_refund` passed in the clean b18 doctor. This repairs the tested two-field abort mechanism. Those exact leaves have **not** yet run a child fixture with an outer caller unwind as required above; add it before checking the stronger gate. P2-D01 callback-under-lock remains a static availability finding and needs a supported-client policy decision.

Additional exception-boundary audit (STATIC, not an executed RED): `shared/lifecycle.rs:22-24` discards the `Err` from `catch_unwind` after catching a client destructor. Rust's `catch_unwind` documentation states that dropping the returned panic payload can itself panic; an already-unwinding outer caller or a second panicking owned field can then abort. The existing subprocesses use ordinary string panic payloads, not a payload with a panicking `Drop`. Add a child fixture using `panic_any` with a panicking payload destructor under outer unwind; decide a bounded payload-retirement policy (possibly intentionally forgetting an exceptional panic payload) without claiming that `catch_unwind` can intercept `panic=abort` or all foreign exceptions. Do not count this static candidate as a reproduced P1.

The earlier P2 `Ord`+`Drop` double-panic limitation remains separate. A single callback panic causing poison is not evidence of accounting drift, whereas the old raw multi-field destructor paths aborted an isolated process. Prior tests of reentrant Busy responses do not test callback panic outcomes.

## Current-source qualification (2026-09-21)

QMP-040 is locally `QUALIFIED` at
`0c126a25aeab11f457e20c4a209fc7bb631ea859`; see
[`receipts/2026-09-21T072928Z-0c126a2.md`](receipts/2026-09-21T072928Z-0c126a2.md).
The final hostile-payload subprocess proves that the canonical retirement
helper does not drop a caught panic payload whose destructor can panic during
outer unwind. All five real store adapters pass the shared lifecycle oracle and
the leak/state-loss/double-refund fakes are rejected. Repo-local doctor and an
actual clean standalone extraction both pass at the same source.

This qualification is scoped to the documented supported-client contract. It
does not claim recovery from `panic=abort`, foreign exceptions, or arbitrary
`Ord` panic plus a second panicking key destructor while a standard-library
index owns that key. QMP-140 owns the public compatibility wording for that
boundary; QMP-130 separately owns external host adoption.
