## Determinism & Accounting Invariants

The ledger (`crates/quanta-memory-platform-core/src/domain/ledger.rs`) is the
semantic accounting SSOT. It is single-threaded and synchronous; the public
broker wraps it behind a lock. Its behavior must be **deterministic and
reproducible** for the same operation sequence — conformance suites and fault
fixtures depend on it.

Accounting invariants (including live pending authorities; owning definitions
and full invariants are in
`crates/quanta-memory-platform-core/src/domain/ledger.rs`):

1. `sum(class usage) == sum(owner usage)`
2. `sum(owner usage) == retained payload bytes + retained metadata bytes
   + live scratch reservations`  (the `ChargeKind` partition)
3. `reserved <= soft <= hard`  (validated when the policy is installed)
4. No committed/live writes while `rebuild_in_progress`; rollback of an
   incomplete pending reservation remains allowed to prevent stranded bytes.
5. Enforce global/headroom ceilings and the bounded shared-burst pool, including
   existing resident usage when installing a new live policy.
6. Ticket authority is single-use and ledger-bound, including zero-byte tickets.

Rules:

- **Fail-closed before mutation.** Validate the whole operation first; every
  mutating op either fully succeeds or leaves the ledger untouched. Never mutate
  partway and then return `Err`.
- **Telemetry is advisory.** Allocator telemetry (hdrhistogram/metrics adapters)
  never mutates ledger counters — it only observes.
- **Stable ordering.** Anything iterated for a snapshot, report, or assertion uses
  ordered containers (`BTreeMap`) or an explicit sort. Do not depend on `HashMap`
  iteration order for snapshots, `LedgerSnapshot`, `EffectiveBudgetReport`, or
  conformance output.
- **Deterministic fixtures.** `testkit` fault/chaos scenarios are deterministic —
  no timing heuristics, no wall-clock, no RNG driving outcomes. A scenario that
  "sometimes" fails is a bug.
- **Exact accounting arithmetic.** Ledger/`UsageBreakdown`/`LedgerSnapshot`
  mutations use checked arithmetic after full postimage validation. Saturation
  is allowed only for a documented bounded/advisory calculation; it cannot
  replace an overflow/underflow error in exact accounting. No silent wrap or
  lossy `as`.

### No silent heuristics

- **Override resolution:** the effective-budget resolver layers overrides by an
  explicit, documented precedence (`OverrideSource`); do not infer precedence
  from map order.
- **Capacity decisions:** an over-budget reserve returns `MemoryError::Capacity`,
  not a silently clamped or partial reservation.
- **Conformance must bite:** a suite that passes everything is worthless — the
  testkit proves it REJECTS a broken/lying fake as well as accepting an honest
  one. Keep that property when extending it.
