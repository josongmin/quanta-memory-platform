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
- Before behavioral Rust changes, follow the pre-edit procedure in
  `tools/pm/sources/rules/cheatsheet.md` and inspect the owning source/tests.
