## Determinism & Accounting Invariants

The ledger (`crates/quanta-memory-platform-core/src/domain/ledger.rs`) is the
semantic accounting SSOT. It is
single-threaded and synchronous; the public broker wraps it behind a lock. Its
behavior must be **deterministic and reproducible** for the same operation
sequence — conformance suites and fault fixtures depend on it.

Global invariants (must hold after every committed operation):

1. `sum(class usage) == sum(owner usage)`
2. `sum(owner usage) == retained payload bytes + retained metadata bytes
   + live scratch reservations`  (the `ChargeKind` partition)
3. `reserved <= soft <= hard`  (validated when the policy is installed)
4. No live writes while `rebuild_in_progress`

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
- **Checked arithmetic on counters.** Byte/budget math uses `checked_*` /
  `saturating_*` per the documented invariant — never silent wrap or lossy `as`.

### No silent heuristics

- **Override resolution:** the effective-budget resolver layers overrides by an
  explicit, documented precedence (`OverrideSource`); do not infer precedence
  from map order.
- **Capacity decisions:** an over-budget reserve returns `MemoryError::Capacity`,
  not a silently clamped or partial reservation.
- **Conformance must bite:** a suite that passes everything is worthless — the
  testkit proves it REJECTS a broken/lying fake as well as accepting an honest
  one. Keep that property when extending it.
