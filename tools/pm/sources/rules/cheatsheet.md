## Rule Cheatsheet (self-check before the gate)

The **Enforced by** column is the authority. Semgrep rows and compiler/Clippy
rows are machine-enforced by `scripts/check-workspace.sh`; rows marked `review`
are review contracts, not automated gates. Do not claim a review-only rule is
fail-closed. A Semgrep exception requires a trailing
`// nosemgrep: <rule-id> -- reason: <…>`.

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

### Design Questions Before Writing Code

Apply the rows relevant to the changed owner; do not add unrelated machinery.
These are reasoning obligations, not additional automated enforcement.

| Boundary | Decide before implementation | Distinguishing evidence |
| --- | --- | --- |
| Validation and execution | Can any field, selector or environment setting change after validation? Seal the validated state and consume its authority at use. | Constructor bypass or later configuration fails to compile; invalid runtime input is rejected before side effects; the actual selected child executes. |
| Accounting and arithmetic | Which owner/class/context/kind and lifetime does each byte belong to? Is the value exact usage, an advisory estimate, or a peak? | Independent per-axis values, overflow/underflow rejection, decreasing observations and unchanged rejected postimages. |
| User callbacks and `Drop` | Which lock or pending authority remains held at the callout? Transfer ownership and release guards where reentry is permitted; preserve cleanup authority on failure. | Bounded hostile-caller execution plus surviving identity/content/charge and later usability. |
| Public dispatch and work | Does the real entry point execute and retain each required slice/result? Is cached reuse actually observed? | Broken-fake rejection, independent assertion inventory and metered nested work; equal outputs alone do not prove reuse. |
| Static guard scope and exceptions | Does the rule follow the semantic operation rather than a local binding name or convenient module label? Scope exceptions to the specific justified operation. | Real engine findings at repository owner paths for wrong forms, and no findings for permitted forms. |

Review the applicable patterns below before editing, then inspect the actual
owner and its tests. Do not rename a binding, move work to a helper, suppress a
finding or weaken a test merely to obtain a green gate. Repair the behavior, or
prove the permitted behavior and update the guard contract without losing the
original regression case.

| Don't write this | Write this instead | Enforced by |
| --- | --- | --- |
| `x.unwrap()` in non-test code | `x.ok_or_else(|| MemoryError::config("…"))?` | review |
| `panic!` / `unreachable!` / `todo!` / `unimplemented!` in shipping paths | return a `MemoryError` variant | review |
| `.expect("")` / `.expect("TODO")` | only in `#[cfg(test)]`, with `.expect("<why this cannot fail>")` | review |
| `Err(_) => {}` / `let _ = fallible();` / `.ok();` | propagate with `?` or map to a typed `MemoryError`; production `let _ =` on a `#[must_use]` result needs an explicit reasoned exception | review; production `let _ =` also Clippy `let_underscore_must_use` |
| `map_err(|_| …)` when the source carries actionable context | preserve it: `map_err(|e| MemoryError::config(format!("…: {e}")))`; discarding poison or range-conversion details is allowed when the replacement error is complete | review |
| bare `#[allow(<lint>)]` (no reason) | use `#[allow(<lint>, reason = "…")]` in production; explain test-only exceptions inline | production Clippy `allow_attributes_without_reason`; test-only review |
| `arr[i]` / `&s[a..b]` on untrusted indices | `arr.get(i)` / checked slicing | review |
| `n as u64` (lossy / truncating) | `u64::try_from(n)?` or a checked/saturating op | review |
| `x as i64` / `as i32` / `as i16` / `as i8` (sign-flips a magnitude — the `bytes as i64` ledger-corruption bug) | keep `u64` magnitudes with saturating/checked ops; `i64::try_from(..)?`; widen a signed *diff* to `i128` | regression-guards (`qmp-magnitude-as-signed`) |
| `a.saturating_mul(b) / c` (saturates BEFORE the divide → under-reports the ratio: the pressure-percent / weighted-cap bug) | `u64::try_from(u128::from(a) * u128::from(b) / u128::from(c))` | regression-guards (`qmp-saturating-mul-then-divide`) |
| `Atomic::fetch_sub(..)` on a counter (can underflow → wraps to MAX: the spilled-count drift) | saturating CAS: `c.fetch_update(SeqCst, SeqCst, \|v\| Some(v.saturating_sub(n)))` | regression-guards (`qmp-atomic-fetch-sub`) |
| wrapping multiply followed by division | widen the product to `u128` before division | regression-guards (`qmp-wrapping-mul-then-divide`) |
| saturation in exact `UsageBreakdown` / `LedgerSnapshot` arithmetic | checked arithmetic after full postimage validation | regression-guards (`qmp-exact-ledger-no-saturation`) |
| whole-owner iteration or snapshot copy in release preflight | keyed owner reads; preserve metered work tests | regression-guards (`qmp-release-preflight-no-owner-scan`); indirect work remains runtime-tested |
| fair-share precomputation inside scheduler admission | borrow the construction-time table | regression-guards (`qmp-admission-no-share-recompute`) |
| semantic pressure update inside allocator observation | compare advisory telemetry to semantic usage | regression-guards (`qmp-allocator-observation-no-pressure-write`) |
| scratch-peak replacement or compound assignment | `max(previous, observed)` | regression-guards (`qmp-scratch-peak-monotonic`) |
| public pressure-threshold fields | private fields and validated construction | regression-guards (`qmp-threshold-fields-private`); compile-fail contract |
| persistent prepared-publication lease charged as payload | `RetainedMetadata` for identity/tombstone lifetime | regression-guards (`qmp-persistent-publication-metadata-kind`); independent axis oracle |
| shared/fixed payload-store seed initialization | fresh `RandomState::new()` for each store | regression-guards (`qmp-store-digest-seed-construction`); real-store seed oracle |
| unused `.ok()` on release/rollback/restore cleanup | handle the error and retain exact recovery authority | regression-guards (`qmp-cleanup-result-not-erased`); production `let _ =` also Clippy |
| legacy finalized watermark read/write | exact record disposition and generation proof | regression-guards (`qmp-durable-no-watermark-resolution`); obsolete fixture writes require a reasoned suppression |
| public kit dispatcher manufacturing Passed | return the real conformance fold | regression-guards (`qmp-kit-no-constant-pass`); public negative pole |
| facade suite dropping a required store slice | compose all four slices | regression-guards (`qmp-facade-suite-required-slices`); runtime assertion-name inventory |
| test-binary subprocess launch without selection guard | launch the sealed handle returned by `require_one_test()` | regression-guards (`qmp-isolated-child-requires-selection-guard`); actual selection verified at runtime and later configuration rejected by the compiler |
| other wrapping arithmetic on byte/budget counters | checked arithmetic per the documented invariant | review (ledger correctness) |
| live reconfigure / policy swap that re-checks only future reserves, not the CURRENT resident usage (e.g. raising `headroom_bytes` over live non-Headroom bytes) | re-validate the live snapshot against the NEW policy and fail-closed before installing it | review (ledger invariant 5) |
| byte-target sweep/reclaim loop that breaks only on `reclaimed >= target` | also handle zero-contribution entries / exhaustion so a positive target can't over-evict (skip zero-byte entries) | review |
| durable-queue FIFO key taken from an atomic counter OUTSIDE the write txn | derive the next key from the table INSIDE the write transaction (atomic, restart-safe, no skipped/duplicate keys) | review (spill backing) |
| `a == b` on floats | epsilon / ordered comparison | clippy `float_cmp` |
| `dbg!(...)` | remove it | clippy `dbg_macro` |
| `std::fs` / `std::env` / `SystemTime::now` / `Instant::now` in `domain/` or `ports/` | route through a port + adapter; keep domain pure | regression-guards (`qmp-pure-layer-no-fs-env`, `qmp-pure-layer-no-wall-clock`) |
| `crate::adapters` imported from `domain/` or `ports/` | depend on the port trait only | regression-guards (`qmp-pure-layer-no-adapter-import`) |
| resolved dependency from `core` to facade/testkit (or facade to testkit), including a renamed dependency | keep `core <- facade <- testkit` | Cargo-metadata graph contract in `check-workspace`, extraction pre-flight |
| Semantica nouns (`Semantica*`, `quanta_runtime`, `codegraph`) in public API | keep the library host-independent | review |
| raw broker constructor backdoor in public code | go through the builder-first surface | public API compile-fail tests |
| allocator telemetry mutating ledger counters | telemetry is **advisory** — never writes counters | review (ledger invariant) |
| mutating the ledger partway then erroring | validate first; fail-closed BEFORE any mutation | review (all-or-nothing) |
| `assert!(true)` / `assert_eq!(x, x)` placeholder test | assert the real behavior / invariant | review |
| `#[ignore]`, even with a reason | feature-gate an optional test and execute it on its owned rail; repair a failing test instead of disabling it | full/in-memory gate rejects ignored Rust tests |
