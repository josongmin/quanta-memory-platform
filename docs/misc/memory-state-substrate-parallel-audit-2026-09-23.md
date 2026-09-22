# QMP memory/state substrate parallel audit

**Latest result:** The [100-ID scored review](memory-state-substrate-scored-review-2026-09-23.md) is the current per-ID verdict for this HEAD. This first-pass status matrix is retained only as source-path evidence. The [non-implemented-items re-audit](memory-state-substrate-gap-reaudit-2026-09-23.md) records the corrected findings and executed probes.

Audit date: 2026-09-23. Source: `main@028448946caa95676df034481a830c1d3e3b1f33`. Checklist: `memory-state-substrate-review-checklist.md`, SHA-256 `a92f16aa840ac31bfee3ce35235af625832ca7e2ae480f6a5d1dda66fb1c43d1`.

This is a **read-only source audit** of all 100 checklist IDs, followed by focused verification of disputed paths. Two defects were reproduced against this HEAD in isolated temporary checkouts/projects; three existing durable-redb tests passed against this HEAD. No broad workspace rail, benchmark, extraction, host suite, deployment, or activation was run. `구현 관찰` means that a relevant code path was inspected, not that the full contract was proved. Existing test names are evidence of coverage intent unless execution is recorded below. `결함` means a reachable source failure; each finding states whether it was executed or remains source-level.

The initial checkout had untracked `docs/misc/` and `tickets/REVIEW-PROMPT.md`; `docs/plans/` appeared during the parallel read. Those paths were not edited by the audit lanes. The latest local annotated qualification tag, `qmp-final-qualified-d370529`, names QMP source `d3705297a6541a37d74dc43bd04620ccbdb465bf`, not this audit's `HEAD`. Its host-pair receipt is historical for this audit source.

Path abbreviations in the matrix: `C/` = `crates/quanta-memory-platform-core/src/`, `F/` = `crates/quanta-memory-platform-facade/src/`, `T/` = `crates/quanta-memory-platform-testkit/src/`. A path and one-based line identify the source inspected. All statuses should be rechecked if these files change.

## Parallel plan and ownership

| Lane | IDs | Primary owners | Output | Dependency |
| --- | --- | --- | --- | --- |
| 1: authority and policy | A01–A06, B01–B10, C01–C08, J01–J06 (30) | `C/domain`, `C/api/control.rs`, scheduler, cost model | Per-ID applicability, source/test pointers, concrete failure sequence | None; read-only |
| 2: facade lifecycle and SDK | D01–D08, E01–E08, K01–K07 (23) | `F/*/service.rs`, `F/shared`, public traits | Per-ID ownership/callback/close path and error contract | None; read-only |
| 3: reclaim and operations | F01–F08, G01–G06, H01–H07, L01–L06 (27) | broker reclaim, pressure, store eviction, telemetry | Per-ID progress/observability evidence | None; read-only |
| 4: durable and proof | I01–I10, M01–M10 (20) | `F/spill`, `F/revocable_store`, testkit, scripts, receipts | Per-ID failure model and qualification boundary | None; read-only |

All lanes may read the same checkout concurrently. They must not write to shared owner files. After the read-only reports, one integrator verifies disputed paths, runs only focused tests needed for findings, checks the source hash again, and updates the verdict. A source or checklist hash change reopens affected IDs before any qualification claim.

## Highest-impact findings

1. **Grouped 0-byte ticket can lose its release path after live reconfiguration (B07/C05/J03/J05; reproduced).** A public 0-byte `ManagedCharge` creates a live ticket and records its issuing group while no owner byte entry is added (`C/api/control.rs:509-523,1150-1162`; `C/domain/ledger.rs:678-702`). Reconfiguration checks owner mapping changes using only positive `snapshot.owner_usage` (`C/api/control.rs:1341-1355`) and can install a scheduler without that group (`:1372-1393`). `release_on` still asks the new scheduler to release the issuing group (`:955-972`), which rejects an unknown group (`C/application/scheduler.rs:81-90`). The isolated test confirmed: valid reconfiguration succeeded; explicit release returned `InvalidState("cannot record release for unknown resource group g")`; Drop left `live_ticket_count == 1`; `finish_rebuild()` failed with the same error. Existing reconfigure tests inspect positive charges (`C/api/control.rs:3315-3401`). This is a release/metadata-authority defect, not evidence of nonzero byte leakage.
2. **Reclaimer can recurse into itself through a same-broker reserve (F06; bounded reproduction).** `MemoryPlatformBuilder::with_reclaimer` accepts a callback (`C/api/builder.rs:74-80`). `reserve_with_reclaim` calls it before the original reservation is committed (`C/api/control.rs:882-915`). A callback that reserves the same class/owner across the same soft threshold sees unchanged ledger state and invokes itself again. `max_attempts: 1` is per request (`:826-837`), not a cross-call reentry fence. The hard-denial branch also calls reclaim during nested reserve (`:920-949`). An isolated public-API probe stopped itself after four callback entries and observed four in both soft and hard paths. Unbounded stack exhaustion remains an inference; it was not executed.
3. **Caller code executes under facade state mutex (E02/E05; source-level).** The revocable encoder receives caller `Key`/`Value` while the state guard remains live (`F/revocable_store/service.rs:385-405`); payload duplicate lookup calls the stored value's `canonical_key()` under its state mutex (`F/payload_store/service.rs:297-304`). Reentry returns typed busy, so deadlock is not established. A slow callback can delay progress. Unlike the encoder's caught unwind, a panic from the stored `canonical_key()` unwinds through `MutexGuard`, poisoning the mutex; subsequent operations return `Poisoned` (`F/shared/lifecycle.rs:76-85`). This is a conditional store-availability defect, not an observed ledger inconsistency. No focused panic execution was run.
4. **Durable uncertainty is queryable; generic error remains an SDK ergonomics gap (E08/K06).** `MemoryError::OperationInProgress(String)` represents both lifecycle contention (`F/shared/lifecycle.rs:104-125`) and reconciliation-required durable operations (`F/revocable_store/service.rs:434-447,722-732`). Public pending/reconcile APIs exist (`F/revocable_store/service.rs:452-505,756-835`), so E08's “structured value or query API” criterion is met. The generic error itself lacks structured scope, bytes, revision, and retryability (`C/domain/errors.rs:12-25`); that is the narrower K06 gap, not proof of lost authority.
5. **0-byte ticket count has no observed independent admission limit (B07).** The public reserve can repeatedly add ticket metadata while charging zero semantic bytes (`C/api/control.rs:509-523`; `C/domain/ledger.rs:678-702`). Treat this as P1 hardening for a trusted in-process API; elevate only if a host path lets untrusted work create such tickets repeatedly. No allocation-failure measurement was run.

## Focused execution and proof boundary

| Check | Exact scope and observed result |
| --- | --- |
| Grouped 0-byte reconfigure/release | Isolated checkout at this HEAD: `cargo test -p quanta-memory-platform-core --test zero_group_reconfigure_audit grouped_zero_byte_charge_releases_after_group_removal -- --exact --nocapture`. The intentionally failing contract test exited 101 after observing the unknown-group release error, retained live ticket, and failed deferred drain. Temporary test: `/tmp/qmp-audit-90OZpH/repo/crates/quanta-memory-platform-core/tests/zero_group_reconfigure_audit.rs`; no shared-repo code edit. |
| Same-broker reclaim reentry | Isolated public-API project: `CARGO_TARGET_DIR=/tmp/qmp-reclaim-reentry-BwfYMw/target cargo run --offline --quiet --manifest-path /tmp/qmp-reclaim-reentry-BwfYMw/Cargo.toml`; exit 0, `soft_admitted=true callback_calls=4` and `hard_denied=true callback_calls=4`. The probe bounded its own recursion; it did not test stack exhaustion. |
| Existing durable-redb tests | `cargo test --locked -p quanta-memory-platform-facade --all-features durable_redb_` on this HEAD: 3 passed, 0 failed. These tests use drop/reopen restart simulation and do not establish forced process-kill recovery or host activation. |

Severity is conditional on host exposure: the zero-ticket release failure and reclaimer reentry are concrete P1 candidates; neither is labeled P0 without a demonstrated externally reachable trigger and production impact. The panic-under-lock issue is a P2 conditional availability defect. F03/F05, G03, L03/L05, and durable disk/codec bounds remain capability or operations gaps, not proven data-loss defects.

## Per-ID static review

Legend: **결함** = concrete reachable source failure; **공백** = contract or proof still needed; **구현** = relevant implementation observed, tests not run; **문서** = scope stated, code proof not claimed; **N/A** = conditional capability not offered by this library surface. Mixed entries use the weaker status when the checklist includes an unproved clause.

### A. Scope (6)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| A01 | 문서 | `README-detailed.md:261-281`: semantic cost is separate from allocator/RSS/transient copies. |
| A02 | 문서 | `README-detailed.md:19-28`: process RSS and host identity are non-goals. |
| A03 | 구현 | `F/shared/builder.rs:172-229`, `C/api/control.rs:257-258`: budgeted stores and runtime use broker-bound authority. |
| A04 | 문서 | `README-detailed.md:159-176,204-210`: feature/capability scope; no async waiting claim found. |
| A05 | 공백 | `F/shared/measure.rs:84-91`, `README-detailed.md:277-281`: caller reports size; post-charge interior growth contract needs explicit host boundary. |
| A06 | 문서 | `README-detailed.md:114-116,196-202`: execution and activation meaning belongs to host. |

### B. Accounting (10)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| B01 | 공백 | `C/domain/ledger.rs:52-60,176-186,1022-1029`: charge kinds separate; pending bytes share the charged kind counter and are not separately exposed. |
| B02 | 구현 | `C/domain/ledger.rs:191-249`: axes and kinds are validated against one ledger. |
| B03 | 구현 | `F/payload_store/service.rs:320-367`, `F/retained_map/service.rs:405-472`: charge before store publication; caller preallocation excluded. |
| B04 | 구현 | `C/domain/ledger.rs:645-702,778-804`, `F/retained_map/service.rs:428-452`: denial/rollback paths; no process-abort rollback claim. |
| B05 | 구현 | `C/domain/ledger.rs:439-515`, `F/shared/measure.rs:47-79`: checked postimages and cost sums. |
| B06 | 공백 | `README-detailed.md:263-281`: V1 formula and original-charge refund; interior mutable payload rule remains open. |
| B07 | 결함 | `C/domain/ledger.rs:678-702`: 0-byte tickets add metadata without a count or byte charge; also part of reconfigure/release defect above. |
| B08 | 구현 | 조건부 dedup: `F/payload_store/service.rs:294-309,320-375` reuses canonical entry rather than adding a charge. |
| B09 | 공백 | `F/retained_map/service.rs:428-482`, `README-detailed.md:277-281`: net postimage charge; old-value destruction and caller clones can overlap outside semantic cap. |
| B10 | 구현 | 제공되는 recharge/batch release: `C/api/control.rs:313-347,1189-1238`; split/merge API는 관찰되지 않음. |

### C. Admission and quotas (8)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| C01 | 구현 | `C/domain/ledger.rs:439-510`, `C/api/control.rs:882-949`: typed capacity denial, one retry; scope is in error text. |
| C02 | 구현 | `C/api/control.rs:1090-1162`, `C/domain/ledger.rs:424-515`: group/scratch/global/class/owner postimages under broker lock. |
| C03 | 문서 | `README-detailed.md:223-259`: static weighting, burst, and fairness limits. |
| C04 | 구현 | 조건부 protection: `C/domain/config.rs:234-338`, `C/application/scheduler.rs:132-195` validate floors/bands. |
| C05 | 결함 | `C/api/control.rs:1341-1355,955-972`: 0-byte live ticket can outlive removed issuing group. |
| C06 | N/A | `README-detailed.md:255-259`: no request queue or async waiting capability. |
| C07 | 구현 | 조건부 borrowing: `C/application/scheduler.rs:200-270,95-123`: static burst cap and release path. |
| C08 | 문서 | `README-detailed.md:223-259`: memory allocation is not throughput/latency fairness. |

### D. Handles and lifetime (8)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| D01 | 구현 | `F/retained_map/service.rs:506-539`: `get_checked` returns caller-owned clone and remove refunds store charge; `README-detailed.md:277-281` excludes transient caller copies/RSS. Not a managed-reference charge defect. |
| D02 | N/A | For retained map reference guards: `F/retained_map/mod.rs:76-82` returns owned values; payload pin counter is not borrowed read guard (`F/payload_store/mod.rs:11-19`). Copy-cost boundary still applies. |
| D03 | 공백 | `F/retained_map/service.rs:523-539`, `F/payload_store/service.rs:187-209`: semantic refund can precede caller destructor/allocator return; physical peak is outside cap. |
| D04 | 구현 | `C/api/control.rs:235-245,298-314,1189-1233`: non-Clone lease, failed release restores authority, batch validates. |
| D05 | 구현 | Conditional ID/slot reuse: `F/payload_store/service.rs:55-65,306-310,419-430`: monotonic refs and removed indexes. |
| D06 | 공백 | `C/ports/store.rs:3-13`, `F/retained_map/service.rs:208-265`: store drains own charges; owner-wide shutdown/child transfer belongs to an unstated host contract. |
| D07 | 구현 | `C/api/control.rs:1013-1043,1255-1265,1803-1837`: exact deferred Drop release and tests exist; tests not run here. |
| D08 | 공백 | `F/shared/lifecycle.rs:104-126`, `F/payload_store/service.rs:165-210,395-400`: close is terminal, but unlike delete it does not inspect outstanding pin count. Host shutdown ordering or pin invalidation must be explicit; no borrowed-guard memory-safety claim. |

### E. Concurrency and failure (8)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| E01 | 구현 | `F/shared/lifecycle.rs:104-126,184-195`, `F/retained_map/service.rs:61-89`: mutation/close gate and preimage restoration. |
| E02 | 결함 | `F/revocable_store/service.rs:385-405`, `F/payload_store/service.rs:297-304`: caller callbacks under state guard; typed busy limits reentry but duration limits progress. Stored canonical-key panic can poison mutex (source-level). |
| E03 | 구현 | `F/retained_map/service.rs:61-89,414-465`: RAII state plan preserves preimage through unwind. |
| E04 | 구현 | `C/api/control.rs:882-949`, `F/transcript_store/service.rs:630-670`: final remeasurement/version comparison before publication. |
| E05 | 공백 | Conditional unwind: `F/revocable_store/service.rs:385-414` catches encoder panic, but stored canonical-key panic under payload mutex (`F/payload_store/service.rs:297-304`) poisons it. No general abort/side-effect rollback promise. |
| E06 | N/A | Store/spill interfaces are synchronous (`F/retained_map/mod.rs:53-105`, `F/spill/mod.rs:111-120`). Host async wrappers need their own contract. |
| E07 | 구현 | Conditional retries: `F/transcript_store/mod.rs:64-88`, `F/spill/mod.rs:64-120`, `F/publish_pipeline/prepared_publication.rs:99-129`. |
| E08 | 구현 | `F/revocable_store/service.rs:452-505,756-835`: public pending/reconcile queries distinguish durable unknown from simple busy as checklist permits; `C/domain/errors.rs:12-25` remains a typed-error ergonomics gap (K06). |

### F. Reclaim and progress (8)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| F01 | 공백 | `README-detailed.md:159-165`, `C/api/control.rs:789-807`: revocable and transcript semantics differ; generic host reclaimer must classify unique vs discardable state. |
| F02 | 구현 | `C/api/control.rs:854-859,942-949`: callback report is advisory; final ledger is remeasured. |
| F03 | 공백 | `C/api/control.rs:789-842`, `F/payload_store/service.rs:377-400`: broker suggests classes/groups; object generation/pin eligibility is enforced by concrete store or host callback. |
| F04 | N/A | `README-detailed.md:255-259`: no separate unused-capacity grant/relinquish abstraction. |
| F05 | 공백 | `F/revocable_store/service.rs:243-253,380-410`, `README-detailed.md:277-280`: release needs no new admission, but codec Vec/scratch peak is not reserved. |
| F06 | 결함 | `C/api/control.rs:882-949`: isolated bounded public-API probe confirmed four nested callback entries on soft and hard paths; per-call attempt bound does not fence reentry. |
| F07 | 공백 | `C/api/control.rs:920-949`, `C/ports/reclaim.rs:50-51`: broker retries once, but arbitrary callback work/time and store scan batch are not generally bounded. |
| F08 | 문서 | `C/domain/policy.rs:6-13,80-87`: execution scratch cap and host backpressure boundary; batch-shrink proof is host-specific. |

### G. Physical pressure (6)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| G01 | 구현 | `C/api/control.rs:1270-1305`: allocator observation is advisory; semantic pressure comes from ledger. |
| G02 | N/A | `C/domain/stats.rs:97-103`, `README-detailed.md:19-28`: no OS/cgroup/PSI adapter; optional allocator sample lacks time/scope/stale fields. |
| G03 | 공백 | `C/application/controller.rs:390-413`, `C/api/control.rs:739-764`: no hysteresis/cooldown; repeat soft crossing can cause churn. No oscillation test run. |
| G04 | 구현 | `C/api/control.rs:901-949,296-309`: admission denial does not block release; physical OOM escalation is host-owned. |
| G05 | 문서 | `README-detailed.md:261-281`: semantic cap is not RSS hard bound. |
| G06 | N/A | `C/application/controller.rs:115-149`: semantic classification is deterministic, no adaptive physical sampler in hot reserve. |

### H. Cache and retention (7)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| H01 | 공백 | `README-detailed.md:159-165`: revocable FIFO vs transcript evidence are distinct; host must classify recomputable vs unique before generic reclaim. |
| H02 | N/A | `F/retained_map/service.rs:666-687`: no engine result-cache identity constructor; host supplies keys if used as a result cache. |
| H03 | N/A | No TTL/read-guard feature claimed for retained map; `F/retained_map/mod.rs:76-82` returns owned clone. |
| H04 | 구현 | `C/ports/store.rs:54-63`, `F/revocable_store/service.rs:209-253`: byte target uses charged size; payload pin blocks delete. |
| H05 | N/A | `README-detailed.md:255-259`: no learned eviction sketch/ghost policy. |
| H06 | N/A | `README-detailed.md:155-165`: no single-flight compute-on-miss capability. |
| H07 | 구현 | `F/transcript_store/service.rs:610,714-770,1008-1015`: replay evidence survives payload truncate within finite horizon. |

### I. Spill and recovery (10)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| I01 | 구현 | `F/spill/mod.rs:123-190`, `F/spill/backings/redb.rs:525-703`: persisted claim/recovery slot protects restore before resident acknowledgement. |
| I02 | 공백 | `F/spill/mod.rs:64-120`, `F/spill/backings/redb.rs:249-280,338-352`: stable monotonic record ID and same-ID retry; no separate attempt ID or explicit logical transfer ID. Equivalent semantics need a declared boundary. |
| I03 | 공백 | `F/spill/backings/redb.rs:283-410`: receipt follows transaction commit and probes ambiguous ACK; backend durability setting and crash/power-loss model are not bound to a receipt. |
| I04 | 공백 | `F/revocable_store/service.rs:358-449,595-709`: resident authority is fenced until commit/reconcile; RAM, encoded Vec, and disk bytes are not all under one resource budget. |
| I05 | 공백 | `F/spill/backings/redb.rs:283-410,474-500`: no explicit disk-byte, record-count, codec scratch, or in-flight I/O quota observed; backend errors propagate. |
| I06 | 공백 | `F/revocable_store/service.rs:608-652`: resident charge precedes durable delete, but decode and copied candidate bytes allocate before charge; peak/decode bound is host codec responsibility. |
| I07 | 공백 | `F/spill/backings/redb.rs:413-469`: reconciliation checks ID presence/finalized watermark, not stored content/digest; same-ID commit retry compares bytes (`:338-352`). |
| I08 | 공백 | `F/spill/backings/redb.rs:824-865`: restore claim is persisted and recoverable; unresolved write's source pending map is process-local (`F/revocable_store/service.rs:105-116,430-449`). Restart discovery/host custody scope needs explicit proof. |
| I09 | 공백 | `F/spill/backings/redb.rs:24-33,283-410`: opaque bytes stored without QMP envelope/version/checksum/decode length bound; host codec/security ownership must be declared. |
| I10 | 문서 | `F/publish_pipeline/prepared_publication.rs:745-770,925-939`: QMP validates supplied root references, while complete stable snapshot and activation barrier are host-owned. |

### J. Reconfiguration (6)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| J01 | 공백 | `C/domain/stats.rs:26-29`, `C/api/control.rs:1470-1510`: report/observation revision exists; individual charge receipt has no policy revision. |
| J02 | 구현 | `C/api/control.rs:1325-1334`, `C/domain/ledger.rs:520-635`: limit lowering below usage is rejected. |
| J03 | 결함 | `C/api/control.rs:955-972,1255-1265`: removed issuing group makes 0-byte charge unreleasable. |
| J04 | 구현 | `C/api/control.rs:1325-1393,1462-1510`: policy/scheduler/report installed under broker lock; zero-ticket exception above. |
| J05 | 결함 | `C/api/control.rs:1341-1355`: zero-byte live owner/group mapping can be removed before ticket cleanup. |
| J06 | 공백 | `C/api/control.rs:1020-1040,1249-1265`: rebuild fence/deferred drain exists, but deferred queue count and work bounds were not established. |

### K. SDK boundaries (7)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| K01 | 구현 | `F/shared/builder.rs:155-211,349-418`, `C/api/control.rs:235-245`: budgeted builder and non-Clone charge. |
| K02 | 공백 | `F/retained_map/mod.rs:53-105`: owned get/remove and close exist; chunk-before-materialization depends on host use path. |
| K03 | 공백 | `F/spill/service.rs:13-21`, `F/spill/mod.rs:59-61`: generic SPI open, charge-bearing authority sealed; custom conformance not executed here. |
| K04 | 구현 | `C/api/broker.rs:11-24`, `crates/quanta-memory-platform-facade/Cargo.toml:61-65`: synchronous core and no required executor; store close carries shutdown semantics. |
| K05 | 구현 | `F/lib.rs:34-53`, `crates/quanta-memory-platform-facade/Cargo.toml:12-50`: feature-gated exports and separate unbudgeted opt-in; matrix not rerun. |
| K06 | 공백 | `C/domain/errors.rs:12-25`: capacity/busy errors lack structured scope/bytes/revision/retryability; durable lower layer and query API compensate partly. |
| K07 | N/A | `F/shared/measure.rs:135-179`: opaque in-process value support is not an FFI deallocator/lifetime API. |

### L. Observability (6)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| L01 | 공백 | `C/api/control.rs:1417-1445`, `C/domain/stats.rs:26-30`: one-lock observation and policy revision, but no per-mutation snapshot sequence. |
| L02 | 구현 | `C/domain/ledger.rs:115-121,240-250`, `F/revocable_store/service.rs:105-117`: disjoint ledger kinds; pending spill/restore stay charged, disk inventory separate. |
| L03 | 공백 | `C/adapters/metrics.rs:8-23`, `F/benches/scaling.rs:259-300`: counters and benchmark tail samples exist; no current run or per-outcome operational tail proof. |
| L04 | 공백 | `C/ports/telemetry.rs:10-100,123-127`, `C/api/control.rs:417-424`: bounded label vocabulary, but synchronous host sink work/queue and snapshot clone cost are not capped. |
| L05 | 공백 | `F/revocable_store/service.rs:453-454,758-759`, `C/domain/stats.rs:74-80`: pending IDs and reclaim counts, no pending age/long-pin/cleanup backlog diagnostic. |
| L06 | 공백 | `C/domain/ledger.rs:180-187`: owner totals/live-ticket count, no live ticket/handle owner inventory for quiescent diagnosis. |

### M. Verification and adoption (10)

| ID | Status | Evidence and limit |
| --- | --- | --- |
| M01 | 구현 | `T/conformance/suites.rs:1829-1840,1880-1960`: independent property trace and broken-fake rejection tests exist; not run here. |
| M02 | 구현 | `C/api/control.rs:235-245`, `F/spill/mod.rs:31-61`, `F/shared/builder.rs:251-270`: compile-fail contracts exist; no current execution. |
| M03 | 문서 | `README-detailed.md:380-397`, `T/loom_models.rs:18-165`: small models only, not all production locks/storage schedules. |
| M04 | 공백 | `F/revocable_store/service.rs:1548-1600`, `C/api/control.rs:3795-3864`: panic/poison tests exist; general allocator OOM/abort recovery is not a supported guarantee. Fallible allocation injection coverage needs a scoped oracle. |
| M05 | 공백 | Three `durable_redb_` tests passed at current HEAD; `F/revocable_store/service.rs:1655-1753`, `F/spill/backings/redb.rs:1105-1160` use drop/reopen and injected ACK loss, not an actual child-process kill/reopen matrix. |
| M06 | 구현 | `F/benches/scaling.rs:145-165,315-325`, `scripts/verify-scaling-results.py:113-165`: workload/thread/cardinality matrix; no current run. |
| M07 | 공백 | No long-duration pin/reconfigure/reclaim soak selector found in `docs/verification-scenarios.md` or scaling rail; existing tests are focused. |
| M08 | N/A | No learned cache policy or new algorithm selection claim; `README-detailed.md:255-259`. |
| M09 | 공백 | `tickets/FINAL-CLOSEOUT.md:19-28`; latest tag `qmp-final-qualified-d370529` covers source `d370529`, not audited `0284489`; deployment/activation remain separate. |
| M10 | 공백 | Annotated tag has a reproducible prior-source receipt (`tickets/FINAL-CLOSEOUT.md:58-74`); no current-source receipt or raw host workload evidence was generated in this read-only audit. |

## Focused verification queue for integrator

1. Repair the grouped 0-byte ticket release failure at the policy/ledger authority boundary, then add an owner-local regression: reserve → valid reconfigure removing original group → explicit release and Drop/finish path. Assert `live_ticket_count` and all axes return to zero or reject the reconfiguration before mutation.
2. Fence same-broker reclaimer reentry at the owning broker boundary, then add a bounded public-API regression for soft and hard paths. Do not create an uncontrolled stack-overflow test.
3. Move stored `CanonicalKey` work outside the payload state lock with generation/preimage revalidation, or make the callback contract explicitly non-panicking/non-blocking and prove containment. Add a panic-under-lock regression and test slow/reentrant Encode separately.
4. Decide whether K06 warrants structured `Busy` versus `ReconcileRequired(id)` errors. E08 already has a query path; do not treat this API refinement as a current authority failure.
5. For `spill-redb`, state disk/codec/restore peak limits and exact crash model before treating I03/I05/I06/I08/I09 or M05 as qualified. Use process restart tests if the advertised contract includes that failure mode.

Do not run `scripts/run-scaling-bench.sh` or extraction preflight on this dirty checkout and treat their prior receipts as current; those rails require a clean, committed candidate. The audit does not authorize product fixes, release tagging, host activation, or changing another owner's untracked files.

## Copy-paste lane prompts

Use each prompt in an independent read-only agent task against the same frozen source. Each agent should report findings to a single integrator; no parallel writer should edit shared source.

### Lane 1 — authority and policy

```text
Audit QMP checklist A01-A06, B01-B10, C01-C08, J01-J06 on the exact checkout provided by the integrator. Read docs/misc/memory-state-substrate-review-checklist.md and record HEAD, dirty paths, and checklist hash first. Trace core/domain/ledger.rs, core/api/control.rs, scheduler, config, facade cost model, and owner-local tests. For every ID report applicability, one status (N/A/documented/implementation observed/proof gap/concrete defect), path:line evidence, and missing oracle. Trace zero-byte tickets through group reconfiguration and final release. Do not edit or run broad tests. Stop and report if owner source changes; distinguish existing tests from tests executed on this source. Return a compact 30-ID matrix and prioritized reachable findings.
```

### Lane 2 — facade lifecycle and SDK

```text
Audit QMP checklist D01-D08, E01-E08, K01-K07 on the exact checkout provided by the integrator. Record HEAD, dirty paths, and checklist hash. Trace public builder/trait -> facade mutation -> broker charge -> close/Drop/panic path -> owner-local tests. Separate QMP-owned guards from caller-owned clones. Inspect callback execution under locks, pin/close behavior, stale plans, and generic error dispositions. For every ID report applicability, status, path:line evidence, and remaining oracle. Do not edit or run broad tests; stop if owner source changes. Return a 23-ID matrix and only proportionate concrete findings.
```

### Lane 3 — reclaim and operations

```text
Audit QMP checklist F01-F08, G01-G06, H01-H07, L01-L06 on the exact checkout provided by the integrator. Record HEAD, dirty paths, and checklist hash. Trace broker reclaim slow paths, ports, store eligibility/pins, semantic versus physical pressure, cache and transcript retention, and telemetry cost. Test the reasoning for reclaimer -> reserve -> reclaimer recursion with an exact call sequence; do not run an unbounded recursion test. For every ID report applicability, status, path:line evidence, and missing oracle. No edits or broad tests; stop if source changes. Return a 27-ID matrix with defect/proof-gap separation.
```

### Lane 4 — durable and proof

```text
Audit QMP checklist I01-I10 and M01-M10 on the exact checkout provided by the integrator. Record HEAD, dirty paths, checklist hash, enabled spill features, and latest annotated receipt source. Trace revocable_store durable transfer -> spill trait -> redb backing, including ACK loss, restore claim, finalization, restart discovery, disk/codec bounds, and host GC roots. Then map testkit, Loom, scaling, crash/reopen tests, extraction, host pair, and receipt scope. For every ID report applicability, status, path:line evidence, and what exact focused execution would prove. Do not edit or run broad tests; do not upgrade drop/reopen simulation or an old tag to current-source host qualification. Return a 20-ID matrix and prioritized capability-specific gates.
```
