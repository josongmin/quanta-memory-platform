# ADR 0003: Qualify the exact source and claim only the measured boundary

Status: Accepted.

## Decision

- A test name, historical audit, passing owner subset, or checked-in CI file
  is not a current release receipt. Record the exact source commit/tree,
  features, dependency source, executed test count, exit status and artifacts.
  A dirty overlay is identified separately from a clean committed candidate.
- Owner-local tests prove local invariants and broken-fake rejection; public
  API, cross-store and process-kill tests prove only their exercised
  transitions. `SIGKILL`/cold reopen does not prove physical power loss.
- The portable conformance suite must reject deliberately broken fakes, not
  just pass against an honest implementation. Allocation probes and advisory
  telemetry cannot override semantic ledger truth; Loom models cover their
  modeled authority transitions, not every production schedule.
- Benchmarks measure specified workloads. Structural/absolute verifier passes
  do not establish an approved latency or throughput SLO. A regression claim
  needs compatible harness/build/machine provenance, a fixed threshold chosen
  before comparing, and repeated baseline/candidate receipts. Correctness is
  never inferred from benchmark speed.
- Standalone extraction requires a committed clean source and the resolved
  `core <- facade <- testkit` graph. The actual extracted checkout must run
  the canonical workspace rail and public examples. CI config contracts are
  not hosted-job results. The CircleCI configuration belongs in the extracted
  file set; its normal job calls the executable extraction/example verifier
  against the extracted manifest. Heavy scaling runs only with an explicit
  pipeline parameter. See [EXTRACTION.md](../../EXTRACTION.md).
- Host adoption, deployment and activation are distinct from QMP's local
  qualification. A paired host proof must verify the Cargo-resolved QMP
  revision and the actual product path, not merely an environment override.

## Why and consequences

Qualification belongs to a source and failure model, not to a plan's status.
After source, harness or dependency changes, re-run affected gates and avoid
promoting older receipts. The executable current gates are
[`scripts/check-workspace.sh`](../../scripts/check-workspace.sh),
[`scripts/verify-extracted-examples.sh`](../../scripts/verify-extracted-examples.sh),
and the benchmark verifier documented in
[verification scenarios](../verification-scenarios.md). This ADR contains no
claim that the current HEAD passed those gates.

## Review applicability

Apply only the contracts of a supported capability. Unchecked generic review
items are not defects or implementation backlog. Record source, tested
transition, failure model and owner; keep type/compile proof, runtime tests,
protocol models and consumer deployment evidence distinct. Do not require a
named cache algorithm, async waiting, FFI or durable recovery merely because
a generic substrate checklist mentions it.
