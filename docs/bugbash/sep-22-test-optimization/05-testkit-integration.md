# Track 5 — Verified testkit/integration work

Snapshot: `main@101781c2ea6122602b4acd9aa69ebfc13fda958f`.

## TO-04 — Make the spill negative pole test the real driver

- Evidence: `crates/quanta-memory-platform-testkit/src/faults/scenarios.rs:213-248`.
- `spill_fault_classification_reports_escaped_for_a_lying_controller` defines a
  local `classify` function and proves that local function classifies the local
  fake. It never injects the lying controller into
  `drive_spill_failure_after_reservation`, so broken reservation/count plumbing
  in the actual fault driver can still pass.
- Change: parameterize the production fault driver over a `SpillController` (or
  extract its real classification seam), run both the honest unavailable target
  and lying target through that path, and assert `Contained`/`Escaped` plus the
  before/result/after evidence.
- Acceptance: a mutation that makes the real driver ignore `spilled_count` or
  the reservation outcome must fail the negative-pole test.

No other testkit item remains. In particular:

- `ConformanceVerdict::from_assertions` already maps an empty assertion list to
  `Failed`, so the public fixture's `passed()` assertion is not pass-on-zero.
- Sharing one mutable platform through `OnceLock` would violate test isolation.
- The property trace measured 1.86 s warm; reducing 128 cases without a separate
  enrolled stress rail would trade defect-finding strength for a minor saving.
- The complete hellgate binary measured 0.49 s; its cross-layer accounting
  assertions are not a current wall-time problem.
