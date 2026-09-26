# ADR 0005: Regression tests must observe the claimed authority

Status: Accepted for the implemented 0.2 test and conformance contract.
Consolidates the completed Sep-26 fail-open audit integrated in `4d59d72`
and `c3be922`, with selector ownership reconciled in `87e2eaa`.

## Decision

- Fake-broker reclaim plans from actual ticket provenance: owner, class,
  context, kind, original group and remaining bytes. Preflight every refund
  axis before mutation. Partial reclaim preserves remaining authority; full
  reclaim invalidates it; zero-byte tickets retain their authority. Unrelated
  surviving tickets must still release successfully.
- Denial containment checks the same retained identity and accounting
  postimage. Unchanged total bytes alone cannot prove a pinned entry survived.
  Offline spill refusal preserves the same target's existing inventory and
  permits exact restoration after returning online.
- Complexity and reuse claims measure actual keyed/iteration/precompute work.
  Equal results cannot establish cached reuse or absence of a linear scan.
  Instrumentation must observe nested iteration and snapshot-copy bypasses.
- Arithmetic, policy-floor and pressure fixtures distinguish the named wrong
  implementations: maximum-limit overflow, incorrectly combined reservation
  floors and allocator-driven cached semantic pressure.
- Isolation tests exercise actual store seeds. Public conformance dispatch
  must reject a deliberately broken fake; facade composition must contain
  each required assertion exactly once. Lower-level success is insufficient
  evidence for the public dispatcher.
- Subprocess tests require exactly one selected child test and verify its
  execution result. A successful zero-test child exit is not a pass. Keep
  timeout and exit checks; explicitly selected ignored tests must execute.
- A reclaimer fixture must not strongly own the victim store while that
  store owns the broker/reclaimer. Use weak references and prove actual store
  deallocation; zero ledger bytes alone cannot detect an ownership cycle.
- A valid mutation changes the claimed owning behavior, compiles, executes
  the intended test and fails its behavioral predicate. Compiler errors,
  stale filters and no-op operators are invalid evidence. Restore and rerun
  the baseline after every mutation.

## Consequences and evidence

The completed audit corrected one reproduced testkit accounting defect and
nine oracle/harness defects, and added two dispatcher/composition guards.
It did not establish a production core/facade failure for those audit items.
Generic WEAK labels, protocol-model boundaries and assertion deletion alone
are not additional runtime defects or automatic work tickets.

The historical verification recorded 19 baseline GREEN / mutant RED / restored
GREEN triples and a full workspace gate against its frozen Rust hashes. Its
standalone proof was dry-run only. Those results do not qualify later HEADs.
The complete snapshot receipt and raw evidence are recoverable from
`8df9d54:docs/bugbash/sep-26-fail-open-audit/`; original prompts, findings and
census are recoverable from `37b369a` at the same path.
