## Rule Cheatsheet (self-check before the gate)

Every row is enforced fail-closed by `cargo clippy --workspace --all-targets
--all-features -- -D warnings`, `cargo deny`, `scripts/regression-guards.sh`
(semgrep rules in `tools/semgrep/qmp-regressions.yml`, with a grep fallback), or
`scripts/memory-platform-doctor`. Fix the code, or add a scoped
`#[allow(<lint>)]` with a one-line `-- reason: <…>` on the same item (for a
semgrep row, a trailing `// nosemgrep: <rule-id> -- reason: <…>`).

| Don't write this | Write this instead | Enforced by |
| --- | --- | --- |
| `x.unwrap()` in non-test code | `x.ok_or_else(|| MemoryError::config("…"))?` | clippy `-D warnings`, review |
| `panic!` / `unreachable!` / `todo!` / `unimplemented!` in shipping paths | return a `MemoryError` variant | clippy, review |
| `.expect("")` / `.expect("TODO")` | only in `#[cfg(test)]`, with `.expect("<why this cannot fail>")` | review |
| `Err(_) => {}` / `let _ = fallible();` / `.ok();` | propagate with `?` or map to a typed `MemoryError` | review |
| `map_err(|_| …)` (drops the source) | preserve it: `map_err(|e| MemoryError::config(format!("…: {e}")))` | clippy `map_err_ignore`, review |
| bare `#[allow(<lint>)]` (no reason) | append `-- reason: <…>` on the same line | review |
| `arr[i]` / `&s[a..b]` on untrusted indices | `arr.get(i)` / checked slicing | clippy, review |
| `n as u64` (lossy / truncating) | `u64::try_from(n)?` or a checked/saturating op | clippy cast lints |
| `x as i64` / `as i32` / `as i16` / `as i8` (sign-flips a magnitude — the `bytes as i64` ledger-corruption bug) | keep `u64` magnitudes with saturating/checked ops; `i64::try_from(..)?`; widen a signed *diff* to `i128` | regression-guards (`qmp-magnitude-as-signed`) |
| `a.saturating_mul(b) / c` (saturates BEFORE the divide → under-reports the ratio: the pressure-percent / weighted-cap bug) | `u64::try_from(u128::from(a) * u128::from(b) / u128::from(c))` | regression-guards (`qmp-saturating-mul-then-divide`) |
| `Atomic::fetch_sub(..)` on a counter (can underflow → wraps to MAX: the spilled-count drift) | saturating CAS: `c.fetch_update(SeqCst, SeqCst, \|v\| Some(v.saturating_sub(n)))` | regression-guards (`qmp-atomic-fetch-sub`) |
| wrapping arithmetic on byte/budget counters | `checked_add` / `saturating_sub` per the documented invariant | review (ledger correctness) |
| live reconfigure / policy swap that re-checks only future reserves, not the CURRENT resident usage (e.g. raising `headroom_bytes` over live non-Headroom bytes) | re-validate the live snapshot against the NEW policy and fail-closed before installing it | review (ledger invariant 5) |
| byte-target sweep/reclaim loop that breaks only on `reclaimed >= target` | also handle zero-contribution entries / exhaustion so a positive target can't over-evict (skip zero-byte entries) | review |
| durable-queue FIFO key taken from an atomic counter OUTSIDE the write txn | derive the next key from the table INSIDE the write transaction (atomic, restart-safe, no skipped/duplicate keys) | review (spill backing) |
| `a == b` on floats | epsilon / ordered comparison | clippy `float_cmp` |
| `dbg!(...)` | remove it | clippy `dbg_macro` |
| `std::fs` / `std::env` / `SystemTime::now` / `Instant::now` in `domain/` or `ports/` | route through a port + adapter; keep domain pure | review, architecture rule |
| `crate::adapters` imported from `domain/` or `ports/` | depend on the port trait only | review (inward-only) |
| `facade`/`testkit` named from `core` (or `testkit` from `facade`) | keep `core <- facade <- testkit` | extraction pre-flight, review |
| Semantica nouns (`Semantica*`, `quanta_runtime`, `codegraph`) in public API | keep the library host-independent | doctor, review |
| raw broker constructor backdoor in public code | go through the builder-first surface | doctor (raw-constructor residue) |
| stale `V1`/`V2` / old renamed symbols | use the current names | doctor |
| allocator telemetry mutating ledger counters | telemetry is **advisory** — never writes counters | review (ledger invariant) |
| mutating the ledger partway then erroring | validate first; fail-closed BEFORE any mutation | review (all-or-nothing) |
| `assert!(true)` / `assert_eq!(x, x)` placeholder test | assert the real behavior / invariant | review |
| `#[ignore]` without a reason | `#[ignore = "<reason>"]` | review |

Clippy policy: `clippy.toml`. Run the full rail with `scripts/check-workspace.sh`.
