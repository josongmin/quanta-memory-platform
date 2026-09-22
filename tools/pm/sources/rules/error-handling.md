## Error Handling

A memory-accounting platform that **drops failures quietly** corrupts the ledger.
Fail loudly and fail-closed.

The single error type is `MemoryError` (`core/src/domain/errors.rs`):

```text
MemoryError = Build | Config | Capacity | InvalidState | Report | Unsupported
MemoryResult<T> = Result<T, MemoryError>
```

Domain-intent aliases all resolve to `MemoryError`: `CapacityError`, `PinError`,
`PublishError`, `StoreWriteError`, `StoreDeleteError`. Construct via the helpers
(`MemoryError::config("…")`, `::capacity("…")`, `::build("…")`, `::report("…")`).

Principles:

- **No silent recovery.** Production code must not coerce a capacity / config /
  invalid-state failure into an empty default or a swallowed `Ok`.
- **Fail-closed before mutation.** An over-budget reserve, an invalid policy, or a
  write during rebuild returns the appropriate `MemoryError` BEFORE touching the
  ledger — never half-apply then error.
- **Preserve the source.** When wrapping a backing error (redb), include
  it: `format!("spill write failed: {e}")`. Never `map_err(|_| …)`.
- **Pick the right variant.** Over-budget → `Capacity`; bad manifest/policy →
  `Config`; builder misuse → `Build`; operation illegal in current state (e.g.
  write during rebuild) → `InvalidState`; snapshot/report failure → `Report`;
  not-wired capability → `Unsupported`.

Rust patterns:

| Avoid | Prefer |
| --- | --- |
| `.unwrap()` in non-test code | `?` with a `MemoryError` variant, or `.ok_or_else(...)` |
| `.expect("")` / `.expect("TODO")` | tests only, with `.expect("<invariant that cannot fail>")` |
| `Err(_) => {}` / `let _ = fallible();` | propagate with `?` or map to a typed error |
| `panic!()` / `todo!()` in shipping paths | return `MemoryError` with an explicit variant |
| `map_err(|_| MemoryError::…)` | `map_err(|e| MemoryError::…(format!("…: {e}")))` |
| lossy `as` / wrapping math on counters | `try_from` + `checked_*` / `saturating_*` |

`MemoryError` derives `Clone, Debug, PartialEq, Eq` and implements
`std::error::Error` + `Display` — assert on the variant in tests, not on string
substrings where avoidable.
