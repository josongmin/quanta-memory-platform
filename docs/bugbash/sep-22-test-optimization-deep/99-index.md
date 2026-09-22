# Deep Test-Optimization Audit — Synthesis

Status: audit complete. This is a source-audit and prioritization record, not
an implementation receipt. It does not authorize weakening an oracle, deleting
a test before its keeper is strengthened, or claiming a runtime qualification.

## Audit closure

| Scope | Census | Critic | Result |
| --- | ---: | --- | --- |
| T01 ledger | 28 | critic-1 | pass |
| T02 domain + policy contracts | 49 | critic-1 | pass |
| T03 control | 76 | critic-1 | pass; attribute/function line convention documented |
| T04 API rest + public surface | 13 | critic-1 | pass |
| T05 application/adapters/ports | 54 | critic-1 | pass |
| T06 payload + retained stores | 81 | critic-1 | pass |
| T07 revocable + transcript stores | 93 | critic-2 | pass; two attribute anchors normalized |
| T08 pipeline/shared/presets | 59 | critic-2 | pass; census/findings anchor convention documented |
| T09 spill/redb/bench/surface | 55 | critic-2 | pass |
| T10 conformance | 45 | critic-2 | pass |
| T11 testkit rest | 31 | critic-2 | pass |
| T12 e2e/public surface | 23 | critic-2 | pass |
| **Total** | **607** | both critics | **pass** |

Timing evidence is retained in `timing/top40.md` and `timing/full.txt`. The
original v1 audit is retained in the sibling directory as its shorter,
source-citation-first companion.

## Prioritized implementation backlog

### P0 — strengthen semantic oracles before removing anything

1. Replace source-text/TOML/README grep assertions with API behavior or
   compile-time boundary proofs. Primary evidence: T06 G04, T08 D11, T09
   T09-11, T12 T12-07.
2. Replace pass-only, tautological, or error-variant-only assertions with exact
   postimages and typed payload checks. Primary evidence: T02 D2/D4/D5/D7,
   T05 D1–D3, T09 T09-09/T09-13, T11 G07–G09, T12 T12-06.
3. Preserve independent production-cost oracles: the prepared-publication
   literal cost oracle is deliberately duplicated and must not be factored into
   production helpers (T08).

### P1 — remove duplicate setup only when a stronger keeper remains

1. Table-drive same-shape denial and missing-binding cases: T01 F4–F6, T03
   D01/D06/D08, T04 A-1, T05 E1–E9, T08 D01/D03, T09 T09-01–03, T11 G01–G06.
2. Retain layer-distinct tests. A unit and public/integration test are both
   keepers when one proves local behavior and the other proves public wiring;
   do not collapse them merely because constants look similar (T02 E4, T04
   E1/E2, T12 T12-08).
3. Keep a complete fail-closed postimage at every deletion target: returned
   value/handle, store state, ledger state, and retry behavior where applicable.

### P1 — eliminate slow fixture mechanics without reducing failure signal

1. Add an RAII temporary-redb guard and generic spill helper pair; retain a
   round-trip, recovery, denial-retains-record, and commit-failure rollback
   keeper (T07 G3/G7; T09 T09-06–08).
2. Split multi-operation unwind tests into named tests sharing one helper;
   shrink default concurrency only after retaining a deterministic stress path
   and exact count/ownership oracle (T08 D06–D09; T12 T12-01/T12-02).
3. Replace busy-yield retry loops with bounded, diagnostic backoff and retain
   a one-winner assertion (T08 D07).

### P2 — reduce high-cardinality trace costs with a measurement receipt

1. The conformance proptest trace is the measured dominant in-scope test
   (T10 G01). Sample expensive full views while retaining exact end/drain
   checks, and quantify the before/after seed/case matrix.
2. Reduce fixture sizes/iteration counts only with a fixed deterministic seed,
   a retained boundary case, and a report of old/new wall time (T01 F2/F3,
   T03 D03–D05, T06 G02/G07, T10 G02, T12 T12-01).
3. Keep Loom and child-process isolation where they protect failure modes; move
   cost only after the replacement execution path is explicit and reproducible.

## Execution rules

- One owner slice per change; do not combine test cleanup with product behavior
  changes.
- Every proposed deletion names its keeper and preserves the stronger oracle.
- Run the owning crate tests first, then `scripts/check-workspace.sh` for an
  integrated change.
- Record the exact source SHA and measured wall time for changes claiming a
  runtime reduction.
- Reopen this audit if a production defect is found: its test recommendation is
  no longer merely an optimization item.
