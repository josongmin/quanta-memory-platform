## Working Rules

- Preserve unrelated dirty work; inspect status before editing and verify only
  intended paths changed.
- Make the smallest owner-level change that proves the behavior. Do not add a
  shim around a broken accounting or storage authority.
- Couple behavior changes to the owning test: ledger invariants and fail-closed
  mutations, policy validation, backing round-trips, or public API examples.
- Put new traits in `ports/` and implementations in adapters or facade backings.
  Keep production backings feature-gated and testkit out of production graphs.
- For public API changes, update re-exports and the real public examples.
