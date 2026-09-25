# Structural planning audit evidence — 2026-09-26

QMP source: `438b869a36369021ee81560fccdb9692e3f39e35` (`main`), clean before
documentation edits. Diff from `96d0a25` consists only of the two audit/index
documents; the inspected product source is identical. This pass changes plans
and preserves a diagnostic program; no production code or registered test changed.

## Executed public-API observations

An external temporary Cargo consumer used public facade builders and core APIs.
The program is preserved as [contract-observations.rs.txt](evidence/contract-observations.rs.txt).
It is not a registered test or a new proof gate. It prints the current failing
behaviors; its successful process exit means the diagnostic completed, not that
those behaviors satisfy the proposed contracts.

- Working files: `/tmp/qmp-sep26-audit.YIlWb7/{Cargo.toml,Cargo.lock,src/main.rs}`.
- Compiler: `rustc 1.92.0 (ded5c06cf 2025-12-08)`.
- Direct path dependencies: this repo's `quanta-memory-platform-core` default,
  facade default plus `transcript,publish_pipeline`.
- External-consumer dependency lock, not the repository qualification lock:
  SHA-256 `b3fb75f5ceab9becfb1008a46b90c7c9018b48aa1b04dce27b8e564346bdfb5a`.
- Program SHA-256: `f8e8a50597364bb1b50676543d953f1160b36e718cf68818ae8bf5c1dc77d8f4`.
- Initial offline build/run and a subsequent locked/offline run both exited 0.

Executed locked rerun:

```bash
CARGO_TARGET_DIR=/Users/songmin/Documents/code-new/quanta-memory-platform/target/sep26-audit-probes cargo run --locked --offline --manifest-path /tmp/qmp-sep26-audit.YIlWb7/Cargo.toml
```

Observed stdout (stderr panic hook order varies):

```text
ENG-11 peak_after_800_then_200=200 reserved=1000
ENG-11 after_release_total=0
ENG-12 validation=Err(Config("pressure thresholds must be monotonic soft<=hard<=fatal (soft=90, hard=75, fatal=50)")) accepted_controller_at_50=Fatal
ENG-13 aborted_empty_payload payload=129 metadata=0 total=129 tombstone=129
ENG-13 after_close_total=0
ENG-01 lookup_panicked=true stats_after=Err(Poisoned("transcript store lock")) close_after=Err(Poisoned("transcript store lock")) ledger_unchanged=true
ENG-01 after_drop_total=0
```

The deliberate `SessionKey::Ord` panic was caught with `catch_unwind`; the panic
hook also printed `armed SessionKey::Ord`. ENG-01 therefore has an executable
availability reproduction, while its unchanged ledger and successful Drop drain
limit the claim: this probe did not demonstrate accounting loss. ENG-13 has
exact total/refund but wrong persistent-metadata classification. ENG-11 changes
observation, not the reserved charge. ENG-12 is the exported controller path,
not the live broker's valid default controller.

To reproduce after the temporary directory is gone, create a standalone Cargo
consumer with an empty `[workspace]`, edition 2021 and the two path dependencies
above; use the preserved program as `src/main.rs`. First resolve/build in that
consumer, then use its lock for reruns. A newly resolved lock is fresh diagnostic
evidence, not a byte-identical reproduction of this recorded dependency set.
During implementation, migrate each observation into the owning regression with
assertions for the selected contract and a verified failing baseline.

## Source and test cross-checks

| Area | Inspected evidence | Consequence |
| --- | --- | --- |
| Panic ownership | `shared/lifecycle.rs` catch-unwind helper and nonblocking state lock; transcript `session_bytes`, read helpers and panic tests | helper is optional at each call site; existing public panic test iterates retain/truncate/versioned retain only |
| Kind semantics | `ledger.rs` single-kind ticket/rustdoc; `shared/measure.rs` component-total conversion; five store charge sites | use one explicit lifetime-bucket contract; do not claim field-byte-accurate telemetry |
| Independent oracle | prepared `prepared_identity_index_and_tombstone_keep_exact_independent_cost` | tests total/drain, not kind axes |
| Peak | scratch `grow_to_observed_updates_within_reserved_ceiling` | tests only increasing observations |
| Validation | controller fields/constructor/tests and public re-exports | validating one constructor does not seal a publicly forgeable value |
| Bounds | `management_limits.rs`, broker `release_managed_many`, prepared orphan classification | live tickets=16,384, identifier bytes and per-batch deferred limits exist; policy maps/root input remain separate families |
| Rebuild | boolean begin/finish and deferred-drain loop | finish opens admission before cleanup may fail; new authority API must distinguish both outcomes |
| Performance | actual recharge/observe/stat paths, point-read benchmark and release verifier | point reads are single-thread; ordinary verification has different scope from explicit 3+3 release limits |
| Delivery | `.circleci/config.yml`, `.github/workflows/ci.yml`, build/test architecture authorities | automatic main extraction, manual GitHub; current main job does not run the two examples |

## Host reachability recheck

Read-only source inspection used Semantica HEAD
`67c0e9f49f85d5b106ee7970fbf7d669d3fb7c08`. The `quanta-memory-host` paths and
the inspected compose/ingress/SDK usage-projection files had no dirty diff.
No host build or running-process observation was performed.

- No `.with_reclaimer(` or `.with_event_sink(` call appeared in the searched
  `packages/**/*.rs` source. This is a bounded source search, not a claim about
  every possible external/dynamic integration.
- Host rebuild uses found in `runtime/contract_bridge_tests.rs` and the
  `pta_state.rs` test module remain test-only.
- `quanta-runtime-memory-compose/src/lib.rs` still projects both kind totals;
  ingress and SDK expose those fields. Host-facing meaning must be included
  in the D2 migration note.
- The manifest adapter converts root-token bytes and calls QMP orphan GC;
  caller root stability is still an external contract.

## Limits

No full workspace rail, fresh benchmark, standalone extraction, hosted CI,
host E2E, or optional Redb qualification was run for this planning pass.
The prior audit's successful checks remain historical and scoped. The new
program upgrades four findings from source-only analysis to observed public
API behavior; it does not establish four regression tests or a final score.

Documentation checks for this change: `python3 tools/pm/pm.py lint` passed
with all 9 generated targets up to date; `git diff --check` exited 0.
