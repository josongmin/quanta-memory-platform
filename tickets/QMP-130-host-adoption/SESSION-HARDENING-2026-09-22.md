# QMP-130 session hardening — proof boundary and corrective evidence

## Scope and attribution

- The previously qualified, pushed host pair remains Semantica
  `986d59b6455028bbf423797ba8b745e46e0cad83` / QMP Git
  `4d0857923b20df75520b28d8700198f51c4159e4`.
- The calling session's newer host commits are `da8eac1e1635754c6247045e5b58b2c5956f25be`
  (production-composition execution-cap test and suite selection) and
  `8e174d9889c1053e0cb6849bc31ed888dcc5bed1` (top/nested lock refresh
  required by changed external GQLang manifests). They are local and were not
  pushed as a replacement qualification.
- The GQLang and Quanta Index source changes belong to external checkouts, not
  this QMP-130 write set. The lock refresh changed only GQLang package-edge
  entries in the two host lockfiles; no external checkout was edited here.

## Confirmed proof defects

1. The historical `2026-09-21T180157Z-b6ca81a.md` receipt labels its first
   `HOST` row as the identity-verifier unittest command, but its
   `e368e406...` digest is the **whole composite gate log**, not a standalone
   unittest log. The gate log contains the observed 9/9 identity result, so
   this is attribution drift, not evidence that the gate failed. A standalone
   rerun of the nine tests passed; its captured output SHA-256 was
   `2d3f4396960bce7c0487ab87ed44abd9f257a2ff395660dc94d546853f5e4d5a`.
   The corrective receipt uses the gate itself as the command for the gate
   digest and does not relabel suite JSON hashes as command-output hashes.
2. The selected execution-budget QBC test at the prior host HEAD exercised
   subquery preflight/mutation/final enforcement. The receipt's statement that
   *every* non-memory axis had a selected host test was too broad. New
   `production_cutover_exposes_all_qmp_execution_ceilings_v1` constructs real
   production cutover dependencies and checks the QMP-derived wall-time,
   subquery, trace-edge and output-byte caps at the boundary and boundary+1.
   Direct owner-local Cargo selection passed 3/3 at the new host source, but
   raw Cargo is not the host's QBC qualification authority. The scratch axis
   remains owned by QMP live-broker tests, not this non-memory adapter test.

## New host proof attempt — not terminal

```text
new local host HEAD: 8e174d9889c1053e0cb6849bc31ed888dcc5bed1
new local host tree: 558da9631fa3ad48c0ff2fd15e3763238ca1f1e8
QMP Git revision: 4d0857923b20df75520b28d8700198f51c4159e4
external GQLang HEAD at lock refresh: 9a23f937c8d83dfc83cb6ae59e531037d83c5072
```

- Owner-local `cargo test --manifest-path
  packages/analysis/quanta-v2/Cargo.toml -p quanta-runtime-memory-compose
  --no-default-features --test production_memory_budget_contract -- --nocapture`:
  exit 0, selected/executed/passed 3/3/3. This is focused comparison only.
- QBC composite attempt 1: dependency identity 9/9; production contract QBC
  3/3 PASS with unchanged source digest
  `115c9b522edd639515bd6fd01c7aceab65f503a907461b13599d59b5e631ff07`;
  host-full-wiring closeout then rejected because the Cargo path-dependency
  source closure changed during execution. Gate exit 1, log SHA-256
  `fd67512c2f326e31ad00d4514689855a8944aaeb83f7e11241d7cdfae524a16b`.
- QBC composite attempt 2: dependency identity 9/9; production-contract
  closeout rejected the same source-drift condition. Gate exit 1, log SHA-256
  `6f829bd1ae5f9ab8b6ffbcce3f104fb3319e15735041d2566c14adc494d91585`.
- The primary host tree stayed clean at `8e174d9`; the external
  `/Users/songmin/Documents/code-new/quanta-index` worktree changed during
  qualification. QBC's campaign digest includes sibling path-dependency
  repositories, so these partial passes cannot be stitched into one GREEN.

## Disposition

The prior `QMP-130=QUALIFIED` inventory row refers only to its explicitly
recorded pushed `986d59b` host snapshot. The newer local `8e174d9` is
`IMPLEMENTED` with focused proof, not `QUALIFIED`; it must receive a fresh
clean-source composite QBC gate after the external source closure stops
changing. Neither attempt closes QMP-140 host-main merge/activation.
