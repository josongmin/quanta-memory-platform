# Test Optimization Bugbash — Verified Index

Snapshot audited: `main@101781c2ea6122602b4acd9aa69ebfc13fda958f`.

## Remaining work only

| Priority | ID | Problem | Evidence | Required fix |
|---|---|---|---|---|
| P0 | TO-01 | redb concurrency test costs 10.946–20.10 s | `spill/backings/redb.rs:1213-1251` | Barrier + small concurrent sample; preserve unique-key/multiset/zero-inventory oracle. |
| P0 | TO-02 | Architecture/feature tests grep source, TOML, and README formatting | facade store tests; `facade/tests/public_api_surface.rs:84-133` | Use rustdoc compile-fail and cargo metadata/tree authorities; delete text greps. |
| P0 | TO-03 | Scaling allocation epoch includes scheduler setup | `benches/scaling.rs:511-546` | Start epoch immediately before admission loop. |
| P0 | TO-04 | Spill negative pole tests a local classifier, not the real driver | `testkit/src/faults/scenarios.rs:213-248` | Inject honest and lying controllers through the production driver seam. |
| P1 | TO-05 | 10k-owner setup consumes 0.82 s to prove a 3-slot bound | `core/src/domain/ledger.rs:1498-1527` | Smaller non-trivial cardinality; retain exact affected-slot assertion. |
| P1 | TO-06 | Zero-pool test claims denial without executing a reserve | `core/src/domain/policy.rs:662-689` | Add hard-cap + one-byte denial and unchanged-state oracle. |
| P1 | TO-07 | Builder-first reclaimer path and `build_live` alias are unexercised | `core/src/api/builder.rs:74-113` | One behavioral builder test with a recording reclaimer. |
| P1 | TO-09 | Transcript byte partitions use only `> 0` oracles | `transcript_store/service.rs:1471-1493` | Assert independent exact partition costs. |
| P1 | TO-10 | redb test paths collide across processes and leak on panic | `spill/backings/redb.rs:886-950,959-1280` | Process-unique RAII temp-db owner. |
| P2 | TO-08 | Exact duplicate/dead assertions and stale “eight” comment | core telemetry/control/public surface | Delete duplicate/dead checks; correct comment. |

## Measured baseline

Measurements used already-built all-feature test binaries from the audited
snapshot and `--test-threads=1` where measuring a complete library binary.

| Scope | Warm time |
|---|---:|
| core library, 199 tests | 2.37 s |
| facade library, 266 tests | 49.34 s |
| testkit library, 76 tests including `loom` | 14.86 s |
| hellgate integration, 18 tests | 0.15 s harness / 0.49 s process |
| redb concurrent accept exact test | 20.10 s |
| ledger 10k-owner exact test | 0.82 s |
| property trace exact test | 1.53 s harness / 1.86 s process |

## Removed from the queue

The prior 107-item index mixed actual defects with style preferences and
unmeasured estimates. The rewritten track files intentionally remove:

- cross-layer tests that reach different public/charged/unbudgeted/fail-closed
  paths even when their final totals look alike;
- independent accounting formulas (they are required mutation oracles);
- crash-isolation subprocess tests;
- default-suite tests measured at 0.01–0.09 s;
- proposals to use `OnceLock` for mutable platform fixtures;
- proposals to move safety tests to `#[ignore]` without an enrolled CI rail.

## Verification status

- Static source audit: complete for the retained items.
- Warm exact binaries: all measurements above passed.
- Fresh `cargo test --locked --workspace --all-features`: superseded by the
  final source-freeze qualification; see `tickets/FINAL-CLOSEOUT.md` for the
  current receipt boundary.
