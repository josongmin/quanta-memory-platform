## Error Handling

A memory-accounting platform that **drops failures quietly** corrupts the ledger.
Fail loudly and fail-closed.

The single error type is `MemoryError`
(`crates/quanta-memory-platform-core/src/domain/errors.rs`):

```text
MemoryError = Build | Config | Capacity | Closed | OperationInProgress |
              Poisoned | InvalidState | Report | Unsupported
MemoryResult<T> = Result<T, MemoryError>
```

Domain-intent aliases all resolve to `MemoryError`: `CapacityError`, `PinError`,
`StoreWriteError`, `StoreDeleteError`. Construct via the helpers
(`MemoryError::config("…")`, `::capacity("…")`, `::build("…")`, `::report("…")`).

Principles:

- **No silent recovery.** Production code must not coerce a capacity / config /
  invalid-state failure into an empty default or a swallowed `Ok`.
- **Preserve actionable source context.** When wrapping a backing error (redb),
  include it: `format!("spill write failed: {e}")`. A poison or range-conversion
  error may be discarded when the replacement typed error already contains the
  complete actionable context.
- **Pick the right variant.** Over-budget → `Capacity`; bad manifest/policy →
  `Config`; builder misuse → `Build`; terminal service → `Closed`; occupied
  mutation lane → `OperationInProgress`; poisoned authority → `Poisoned`;
  other illegal state → `InvalidState`; snapshot/report failure → `Report`;
  not-wired capability → `Unsupported`.

`MemoryError` derives `Clone, Debug, PartialEq, Eq` and implements
`std::error::Error` + `Display` — assert on the variant in tests, not on string
substrings where avoidable. Concrete Rust patterns live in the cheatsheet so
the policy has one wording authority.
