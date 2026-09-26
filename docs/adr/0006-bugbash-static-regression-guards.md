# ADR 0006: Guard bugbash regressions at their owning boundary

Status: Accepted.

## Decision

The canonical workspace gate runs the real Semgrep engine with owner-scoped
regression rules. `scripts/tests/test_regression_rules.py` requires rejecting
fixtures for every configured rule, checks that all fixture files were scanned,
and rejects unexpected findings on permitted shapes. Fixtures keep repository
paths so path-filtered rules cannot silently disappear. Fixtures are syntax
inputs, not compiled Rust or runtime correctness evidence.

The guards cover wrapping ratio multiplication, exact ledger saturation,
whole-owner release scans/snapshot copies, fair-share recomputation in admission,
semantic pressure writes during allocator observation, scratch peak replacement,
public threshold fields, prepared persistent-lease bucket selection, payload-store
seed construction, erased cleanup results, production legacy-watermark access,
constant kit verdicts, omitted facade slices, and unguarded test-binary launches.
Existing signed-magnitude, atomic underflow and pure-layer rules remain active.

Every test-binary launch in facade source uses `require_one_test`, including the
retained-map sweep launch that previously checked only the child's summary.
The helper checks actual selection and includes ignored children; timeout and
exit assertions remain the responsibility of each scenario.

## Evidence boundaries

| Bugbash family | Prevention authority |
| --- | --- |
| R05/R07 work complexity and reuse | Narrow AST guards plus actual work/precompute instrumentation; indirect helper work still requires runtime tests |
| R06/R09 arithmetic and semantic pressure | AST guards plus maximum-limit and allocator/cache postimage tests |
| R03/R04 dispatcher and composition | Constant-verdict/call-inventory guards plus broken-fake rejection and independent assertion-name inventory |
| R10/R12 seed and child selection | Constructor/launch guards plus actual store-seed and child-selection tests |
| ENG-11/12/13 observation, validation, bucket | AST guards plus decreasing-peak, compile-fail and independent kind-axis tests |
| R01/R02/R08/R11 provenance, survival, independent floors, inventory | Behavioral and mutation oracles; syntax cannot prove these relationships |
| ENG-01 read panic, compound hostile Drop | Owned lifecycle/InFlight boundaries and hostile-caller subprocess oracles; no global ban on callbacks under locks |
| ENG-03/durable restore authority | Non-cloneable private permit types, compile-fail tests and stale/foreign claimant runtime tests |
| ENG-08/10 bounds and preallocation order | Existing input checks and rejection/postimage tests; syntactic presence does not prove dominance |
| Publication/fence failure atomicity | Failure injection and unchanged/reusable postimages; a general mutation-before-`?` ban would reject valid rollback designs |
| Durable schema, exact disposition, recovery | Watermark guard plus schema/layout/claim/reopen tests; call presence does not prove transaction atomicity |
| L2 worker deadlines, group termination, device cleanup | Existing bounded runner/process-group code; Linux execution is separate evidence |
| CI overrides, incomplete suites, selector ownership, receipt identity | Existing configuration, runner, metadata and receipt contracts |

The exact-ledger rule deliberately permits saturating advisory differences and
unused reserved floors. The cleanup rule permits observed or propagated errors;
production Clippy separately rejects `let _ =` on must-use results unless a
reasoned exception preserves recovery authority. The documented panic-time
abort exception retains its persisted claim and remains valid.

Semgrep OSS is not a Rust type checker or whole-program verifier. Refactoring
through helpers, field aliases, renamed owners or different construction shapes
can move beyond a narrow rule. Runtime and compile-fail oracles remain required.
No static pass qualifies crash recovery, physical power loss or a host deployment.
