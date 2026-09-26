## Working Rules

- Resolve the requested target before editing. If it remains ambiguous, ask
  for the missing scope; do not substitute an adjacent task or deliverable.
- Invoke named workflow skills only when explicitly requested in the current
  user turn. Words such as review, audit, fix or SOTA do not invoke a skill;
  an invocation from an earlier turn does not carry forward.
- Preserve unrelated dirty work; inspect status before editing and verify only
  intended paths changed.
- Recheck affected files before writing in a shared checkout. Do not restore
  pre-existing changes from HEAD; run mutation experiments in disposable copies.
- Make the smallest owner-level change that proves the behavior. Do not add a
  shim around a broken accounting or storage authority.
- Couple behavior changes to the owning test: ledger invariants and fail-closed
  mutations, policy validation, backing round-trips, or public API examples.
- Put new traits in `ports/` and implementations in adapters or facade backings.
  Keep production backings feature-gated and testkit out of production graphs.
- For public API changes, update re-exports and the real public examples.

### Before Behavioral Rust Changes

1. Trace the real public entry point to the owning state and side effects.
   Identify the invariant, charge axes/lifetimes, and unchanged state required
   on rejection. Include callbacks, destructors and cleanup in failure paths.
2. Choose a representation that preserves the invariant until use: private
   validated fields, single-use authority, or an owned sealed execution handle.
   A validation call on a builder that can still change is insufficient.
3. Name a reachable wrong implementation and the independent observation that
   distinguishes it. Reuse an adequate owner test; extend it when identity,
   content, accounting axes, actual work or public dispatch is unobserved.
4. Review the completed diff for alternate assignment forms, renamed bindings,
   helper indirection and exceptions that bypass the intended boundary. A lint
   pass proves only matched syntax; use compiler/runtime contracts for the rest.
