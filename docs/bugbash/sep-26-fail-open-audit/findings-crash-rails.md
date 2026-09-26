# T7 — Crash/Durability Rails Fail-Open Audit (sep-26)

## Header

Method: pure reading (no rail executed — every question was answerable from the
rail bodies, the two child example sources, and the SUT hook sites they target).
Canonical-gate inclusion verified by reading `scripts/check-workspace.sh`
(`cargo test --locked --workspace --all-features`), which enables every cfg
gate below.

Scope files and per-file grep counts (`#[test]` + `#[tokio::test]`, verified
with `grep -c -E '#\[test\]|#\[tokio::test\]'`; no `#[tokio::test]` variants
with arguments exist in scope):

| file | grep count |
| --- | --- |
| `crates/quanta-memory-platform-facade/tests/spill_redb_kill_reopen.rs` | 4 |
| `crates/quanta-memory-platform-facade/tests/spill_redb_finalize_kill.rs` | 3 |
| `crates/quanta-memory-platform-facade/tests/cross_store_hellgate.rs` | 4 |
| `crates/quanta-memory-platform-facade/tests/hellgate_e2e.rs` | 18 (16 top-level + 2 in `mod payload_e2e`) |
| **total** | **29** |

Census table row count: **29 == grep count** (sampling: none).

Read-scope child example sources (no other examples are spawned by scope):
`crates/quanta-memory-platform-facade/examples/spill_redb_crash_child.rs`,
`crates/quanta-memory-platform-facade/examples/spill_redb_finalize_kill_child.rs`.
`cross_store_hellgate.rs` and `hellgate_e2e.rs` spawn **no** child processes
(despite the "hellgate" name they are same-process ledger/concurrency rails).

Worktree note: the briefing described `spill_redb_kill_reopen.rs` as dirty; at
audit time it is byte-identical to HEAD (`git diff HEAD -- <file>` empty), so
the audited form is both the working-tree and committed form. Pre-existing
unrelated state (`docs/bugbash/sep-26-fail-open-deep/prompts.md` deletion,
untracked `docs/bugbash/sep-26-fail-open-audit/`) left untouched.

## Census table

Verdicts: STRONG / WEAK / FAILOPEN / VACUOUS. "Critic mutation" names the edit
that must turn the row red; child-rail mutations may target the named child
example source. Key: KR = spill_redb_kill_reopen.rs, FK =
spill_redb_finalize_kill.rs, CS = cross_store_hellgate.rs, HE =
hellgate_e2e.rs.

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| `sep23_fup006_sigkill_after_commit_reopens_with_stable_identity_and_exact_drain` (KR:272) | STRONG | Coordinate: child commits 16 records + persists restore claim, prints flushed `BACKEND`+`READY`, sleeps; parent asserts SIGKILL (signal 9); recovery `Some` is `.expect`ed (not Option-blind); backend identity equality; exact 16-record drain with per-record sequence+index+filler equality; `retained_count()==0`; no stranded claim. | none | In `prepare_restore`, skip persisting `ACTIVE_RESTORE_SEQUENCE` → recovery returns `None` → `.expect("interrupted prepare survives SIGKILL")` red. |
| `sep23_fup006_sigkill_during_write_stream_converges_without_duplicates_or_tears` (KR:312) | STRONG | `READY` is emitted only after record 0 is durably committed (child:84–86), so the kill exercises a guaranteed non-empty prefix; `!drained.is_empty()` guards A6; strictly-increasing sequence AND index (payload duplication caught); holes in `first..=last` must resolve `Absent`; exact drain + no claim. | none | Make `resolve_record` return `Finalized` for unknown/Reserved ids → hole assertion red; or double-write a payload under a second sequence in `commit_spill` → index-strictness red. |
| `sep23_fup006_reserved_hole_stays_absent_across_sigkill` (KR:359) | STRONG | `HOLE` id printed after `reserve_record_id` durably commits a `Reserved` row (redb.rs:1112–1151, own txn) — the coordinate is real, not in-memory; hole must be `Absent` not `Finalized`; exact drain `[(0,0),(1,1)]` pins the committed set. | none | Map `RecordDisposition::Reserved` → `Finalized` in `resolve_record` → Absent assertion red. |
| `durable_spill_store_charge_and_record_survive_process_boundary` (KR:383, cfg revocable_store) | STRONG | Child drives the full public spill path and itself asserts `SpillReceipt::Durable{sequence:0}`, ledger drained to 0, `spilled_count()==1` before `CHARGE 0`/`READY` (pre-kill negative poles inside the child); survivor restores via public API, asserts `Restored{0}`, ledger>0, reclaim returns exact payload bytes, second restore `Empty`, count 0, ledger total 0. Ledger-level, fail-closed. | none | Make the store spill path write no durable record (route to memory only) → child `.expect("one committed record")` fails pre-READY → parent spawn fails red. |
| `durability_l1_store_and_ledger_recover_across_finalize_kill` (FK:255, cfg revocable_store) | STRONG | Both store modes in one body; armed point/sequence asserted, `charged>0` at kill time; `store-before` → victim `Present` + public recover + exact payload reclaim; `store-after` → victim `Finalized` + recover yields `Empty` (no replay); then unrelated seq 1 restores + payload equality; `spilled_count()==0`; `active_restore()==None`; ledger total 0. | none | Make `finalize_restore` persist the Finalized row before `txn.commit()` returns (move table writes ahead of BeforeCommit hook) → store-before branch resolves `Finalized` → red. |
| `durability_l1_finalize_before_commit_kill_recovers_present_and_drains_exactly_once` (FK:392) | STRONG | Armed at `FinalizeBeforeCommit` verified by token; victim stays `Present`; recovery `Some`; backend stable across kill AND fresh generation (`assert_ne!`); recovered claim commits+finalizes; drain exactly 2 with full payload equality; second drain empty; retained 0; no claim. Bounded repetition guard fails closed to 1 (never 0). | none | Reuse the killed generation in `recover_active_restore` → `assert_ne!` red; or drop recovery-row persistence → `.expect("interrupted finalize survives SIGKILL")` red. |
| `durability_l1_finalize_after_commit_kill_resolves_finalized_without_resurrection` (FK:444) | STRONG | Armed at `FinalizeAfterCommit` verified; victim exactly `Finalized`; no recovery payload, no claim; unrelated record `Present` and untouched; drain exactly 1 skipping the victim; FIFO prepare empty; resurrection rejected for identical AND different bytes (explicit A8 negative pole); retained 0. | none | Allow `commit_spill` to overwrite a `Finalized` row → both `is_err()` assertions red. |
| `distinct_grouped_stores_compete_for_one_global_slot_and_handoff_after_release` (CS:151) | STRONG | Exactly-one-of-two admitted (count asserted, not ≥0); loser must be a typed `Capacity` error; winner branch (either side) runs the full `assert_only_charge` oracle + store counters + content equality on remove/reclaim + `assert_drained`; then handoff proves the released slot readmits the peer. Ledger-level. Minor: `assert!(map_bytes + revocable_bytes > map_bytes)` is trivially true (cosmetic precondition guard, A1 flavor — harmless since the one-slot arithmetic is structural). | drop the trivially-true guard or assert `sum > cap` explicitly | Make over-cap admission clamp instead of erroring → exactly-one count red. |
| `store_scratch_and_zero_byte_authorities_survive_denied_remap_and_legal_raise` (CS:247) | STRONG | Denied live remap → **full snapshot equality** after the error (fail-closed detector, not just totals); old cap still denies (negative pole); accepted raise leaves snapshot byte-equal; new entry then admitted with per-owner/context attribution; all authorities drained. | none | Install the new policy before validating live usage → post-denial snapshot inequality red. |
| `other_store_reclaim_frees_admission_and_drains_every_ticket` (CS:329) | STRONG | Direct A3 negative pole: the reclaimer **lies** (`ReclaimOutcome{reclaimed_bytes: 0}` while actually evicting) and admission must succeed from the broker's own remeasurement; exactly one callback; inner sweep stats asserted; per-owner ledger totals + store counters; FIFO head-not-tail eviction pinned by reclaiming `v2`; full drain; reclaimer holds only a `Weak` (cannot pin a closed store). | none | Make the broker trust the advisory `reclaimed_bytes` instead of remeasuring → insert denied → `.expect("admitted after cross-store reclaim")` red. |
| `mixed_store_kind_lifetimes_balance_every_axis` (CS:441, cfg publish_pipeline) | STRONG | Independent cost constants (no production helpers); asserts every axis separately: per-kind totals, per-owner, per-class, per-context buckets, ticket count — a correct grand total with wrong attribution fails; abort refunds only the payload lease (tombstone persists); close drains all axes. | none | Attribute prepared payload bytes into the metadata bucket → per-axis tuple assertion red despite correct total. |
| `charged_store_bytes_flow_through_the_ledger_and_drain_to_zero` (HE:71) | STRONG | Independent cost oracle; ledger total == oracle; store counter == ledger; overwrite re-charges (never doubles); drain to exactly 0; balanced axes at every observation. | none | Skip the ledger refund on remove → "drain to zero" red. |
| `close_drains_all_charges_once_and_rejects_later_mutation` (HE:103) | STRONG | Close drains to 0; idempotent second close; post-close insert must be a typed `Closed` error (negative pole). | none | Make close leave charges live → total==0 red; or accept late writes → `Closed` match red. |
| `failed_close_during_rebuild_preserves_store_and_retries_after_fence` (HE:126) | STRONG | Close during rebuild → typed `InvalidState`; **snapshot equality** after denial (fail-closed); data and counter intact; retry after fence drains. | none | Partially tear down on denied close → snapshot inequality red. |
| `store_drop_during_rebuild_defers_refund_until_finish` (HE:148) | STRONG | Drop during rebuild must NOT zero the ledger (charge pinned at oracle value, balanced); finish() drains it. | none | Refund on drop while fenced → fenced-total assertion red. |
| `e2e_spill_tier_round_trips_fifo_and_never_touches_the_ledger` (HE:173, cfg spill) | STRONG | FIFO restore order pinned (`h1` then `h2`, then `None`); `spilled_count` balances 2→0; accounting-neutrality asserted at three observation points. Minor: neutrality check is total-bytes only (full snapshot equality would also catch bucket misattribution that preserves the total). | use full `LedgerSnapshot` equality for the neutrality check | Charge the ledger on spill routing → neutrality assertions red. |
| `class_cap_gates_store_bytes_fail_closed` (HE:209) | STRONG | The model fail-closed test: over-cap insert → typed `Capacity`; **full snapshot equality** (ledger byte-for-byte unchanged); store counter unchanged; survivor content intact; no phantom entry (`get_checked("overflow").is_none()`). | none | Reserve-then-deny (partial charge) → snapshot/counter equality red. |
| `overwrite_recharge_admits_net_fitting_replacement` (HE:246) | STRONG | Discriminating arithmetic: cap 1_050 vs transient old+new 1_982 — a charge-then-refund implementation must deny; only the atomic recharge admits. Asserts exact post-image oracle. | none | Implement overwrite as charge-new-then-refund-old → `.expect("net-fitting overwrite via recharge")` red. |
| `rebuild_fences_charged_store_writes_then_resumes` (HE:265) | STRONG | Insert during rebuild → typed `InvalidState`; pre-rebuild charge intact (exact oracle value); writes resume after finish with exact totals; balanced. | none | Admit writes during rebuild → InvalidState match red. |
| `reconfigure_raise_preserves_charged_usage` (HE:294) | STRONG | Legal raise preserves charged usage exactly (oracle value, balanced); the new cap then admits an entry the old cap would have denied (negative pole on the raise being real). | none | Reset charges on reconfigure → preserved-usage assertion red. |
| `concurrent_charged_inserts_never_exceed_global_and_stay_consistent` (HE:331) | STRONG | 8 threads × 50 inserts vs global 100k: total ≤ global; store counter == ledger exactly (no torn updates); **near-saturation check** `headroom < cheapest entry` proves capacity denials actually fired (anti-vacuity for the concurrency path); busy-retry is bounded and typed. | none | Off-by-lock TOCTOU in admission → total > global or counter≠ledger red. |
| `double_remove_does_not_double_refund_the_ledger` (HE:384) | STRONG | Second remove returns `None`; ledger exactly 0 (cannot go negative / double refund); balanced. | none | Refund on missing-key remove → negative/double refund red. |
| `broker_recharge_denied_preserves_managed_authority` (HE:414) | STRONG | Over-cap recharge → typed `Capacity` AND `ManagedChargeRechargeRecovery::ChargePreserved` (the recovery contract, not just the error); old 800 bytes preserved; release then drains. | none | Drop the old charge on denied recharge → 800-byte assertion red. |
| `release_many_drains_a_batch_atomically` (HE:462) | STRONG | Batch release drains all three; every ticket inactive; total 0; balanced. | none | Partially release the batch → total≠0 red. |
| `dropping_managed_charges_releases_each_exact_authority` (HE:475) | STRONG | Distinct lease ids; each drop releases exactly its own bytes (200 then 0). | none | Drop leaking its authority → 200/0 sequence red. |
| `release_many_with_an_already_released_ticket_is_fail_closed` (HE:487) | STRONG | Batch containing an inactive ticket → typed `InvalidState`; **snapshot equality** proves the still-live ticket was not partially refunded; flags re-checked; single release still works; final drain. | none | Partially refund the batch before rejecting → snapshot equality red. |
| `release_many_during_rebuild_is_fail_closed` (HE:513) | STRONG | Batch during rebuild → typed `InvalidState`; snapshot equality (nothing stranded); all tickets still active; retry after fence drains exactly. | none | Apply the batch during rebuild → InvalidState match red. |
| `charged_payload_dedup_pin_delete_flow_through_the_ledger` (HE:558, mod payload_e2e) | STRONG | Independent constant 121 with derivation; dedup returns the same ref and does not double-charge; pinned delete → typed `InvalidState` with ledger still charged (negative pole); unpin+delete refunds to exactly 0. | none | Charge on dedup hit → 121 assertion red; allow pinned delete → InvalidState match red. |
| `charged_payload_over_cap_put_is_fail_closed` (HE:590, mod payload_e2e) | STRONG | Over-cap put → typed `Capacity`; ledger total byte-unchanged. Any phantom retention necessarily charges bytes, so total equality still bites. Minor: detector is total-only — narrower than its sibling `class_cap_gates_store_bytes_fail_closed` (no full snapshot equality, no phantom-ref probe). | mirror the sibling: full snapshot equality + `payload_ref` absence probe | Create the entry then deny → total-unchanged assertion red. |

**Verdict totals: STRONG 29, WEAK 0, FAILOPEN 0, VACUOUS 0.**

### Archetype sweep (A1–A8) — findings

- A1 assert-on-input: one cosmetic instance, CS:155 `assert!(map_bytes +
  revocable_bytes > map_bytes)` (trivially true; the real one-slot arithmetic
  is structural). No detection power claimed or needed. Not a verdict changer.
- A2 vacuous iteration: none. `drain_all` loops are unbounded-input but
  bounded-output (DB contents); every iteration asserts content. FK's
  `iterations()` fails closed to 1 (env var cannot make it 0).
- A3 advisory-vs-authority: directly negative-poled by CS:329 (reclaimer lies;
  broker remeasures) and KR:383/FK:255 (telemetry-free survivor checks).
- A4 `let _ =` on the op under test: one A4-flavored site examined and
  CLEARED — `spill_redb_finalize_kill_child.rs:139`
  `let _ = store.restore_one_durable_bounded(&controller);`: the armed hook
  parks forever inside `finalize_restore`, so the call cannot return; if it
  ever did, the child exits `3` and the parent's `spawn_until_armed` fails.
  The discard is unreachable, not a swallowed result.
- A5 catch_unwind: none used anywhere in scope.
- A6 fault never lands: the signature risk, and it is covered at every layer —
  READY/ARMED handshakes are printed only *after* the durable pre-kill writes
  (child sources verified line by line), SIGKILL is verified via
  `status.signal() == Some(9)`, pre-kill self-death fails the rail with stderr
  attached, missed hook points exit `3` (fail-closed), and every rail asserts
  non-empty post-state (`recovery.expect(...)`, `!drained.is_empty()`,
  `Restored{sequence:0}`). Suite level: `check-workspace.sh` runs
  `--all-features`, so `spill-redb`, `durability-test-hooks`, and all cfg
  gates in scope are enabled in the canonical gate — no rail is compiled out.
- A7 shape-only round-trip: none — every restore path asserts payload bytes
  (index prefix + 0xA5 filler + length 32), not just presence.
- A8 no negative pole: none — denial/rejection poles exist in every relevant
  row (typed errors, `Empty`, `is_err()` on resurrection, `InvalidState`
  matches, near-saturation proof in HE:331).

## Crash-coordinate table

Intended crash points verified against the child sources and the SUT hook
sites (`redb.rs` `finalize_kill_hooks::fire` at :1833 BeforeCommit — after all
table writes are staged, immediately before `txn.commit()` — and :1841
AfterCommit — immediately after `txn.commit()` returns `Ok(())`, before any
ACK handling).

| rail (test) | intended crash point | proof child reached it BEFORE the kill | partial-state detector strength |
| --- | --- | --- | --- |
| KR:272 (fill 16) | after 16 committed records + a persisted active restore claim, child quiescent | flushed `BACKEND <u128>` + `READY` printed only after `prepare_restore` returned (child:72–81); BACKEND value cross-checked against the recovered claim; SIGKILL signal 9 asserted | **full**: recovery `Some` expected; backend identity equality; 16/16 records with per-record sequence+index+filler equality; `retained_count()==0`; `active_restore()==None` |
| KR:312 (stream) | arbitrary point in an endless reserve/commit loop, with record 0 guaranteed durable | `READY` only after `commit_one(0)` returned (child:83–86) — guaranteed non-empty committed prefix, no timing bet; non-emptiness re-asserted post-kill | **full for any prefix**: strict sequence+index monotonicity (no dup/reorder/tear), holes→`Absent`, exact drain, no stranded claim |
| KR:359 (hole) | after 2 commits + one durably reserved-never-committed id | `HOLE <id>` printed after `reserve_record_id` committed its `Reserved` row (child:93–99; SUT redb.rs:1112–1151, own txn) | **full**: hole must be `Absent` (not `Finalized`); exact committed set `[(0,0),(1,1)]`; retained 0 |
| KR:383 (store-spilled) | after a public-path charged spill completed and the child ledger drained to 0 | child asserts `SpillReceipt::Durable{sequence:0}`, ledger total 0, `spilled_count()==1`, then prints `CHARGE 0`+`READY` (child:103–133); survivor asserts `Restored{0}` | **full**: exact payload bytes on reclaim; second restore `Empty`; `spilled_count()==0`; survivor ledger total 0 (ledger-level, not process-level) |
| FK:255 store-before | inside `finalize_restore`, staged-but-uncommitted (hook parks before `txn.commit()`) | `ARMED FinalizeBeforeCommit 0 <gen>` emitted from *inside* the finalize transaction after `BACKEND` + positive `CHARGE`; parent asserts the token, sequence 0, and charged>0; missed hook → child exit 3 | **full**: victim `Present`; public-store recover `Restored{0}`; exact payload; unrelated record drains with payload equality; ledger total 0; no claim |
| FK:255 store-after | inside `finalize_restore`, immediately after `txn.commit()` returned Ok | `ARMED FinalizeAfterCommit 0 <gen>` from the post-commit hook site; same handshake assertions | **full**: victim exactly `Finalized`; recover yields `Empty` (no replay of killed resident); unrelated record intact + drained; ledger total 0 |
| FK:392 (before) | same BeforeCommit point, backing-only path | same ARMED token + `charged_bytes == None` asserted (mode purity) | **full**: `Present`; recovery `Some` with stable backend + **fresh generation** (`assert_ne!`); recovered claim commits+finalizes; exact drain 2 with payload equality; second drain empty; retained 0; no claim |
| FK:444 (after) | same AfterCommit point, backing-only path | same ARMED token + mode purity | **full**: `Finalized`; recovery `None`; claim `None`; unrelated `Present` + drains alone; prepare empty; resurrection rejected for identical AND different bytes; retained 0 |

No rail in scope can be killed "before any write" and stay green: the
handshake ordering (durable write → flushed marker → parent returns → kill)
plus the post-kill non-empty expectations close that door in every row.

### Boundary honesty (docs vs body)

- KR header explicitly disclaims power loss and disclaims proving interrupts
  inside a specific redb transaction or at ACK/source-release/restore-delete/
  finalize boundaries — and the body matches (kill points are exactly the
  three documented ones). The disclaimed finalize boundary is covered by the
  separate FK rail, which exists and arms precisely there.
- FK header explicitly scopes to L1 (inside finalize, outside dependency-owned
  pwrite/fsync = L2, power loss = L3 stays UNVERIFIED). The hooks verified in
  SUT source sit exactly at the two claimed sides of the linearization point.
- CS/HE make no crash claims; their invariants are ledger-level
  (snapshot equality, per-axis attribution, exact drains), not "it didn't
  hang". No docs/body mismatch found anywhere in scope.

## Footer — risk summary

1. All 29 rows are STRONG; no FAILOPEN or VACUOUS rows — the crash rails have
   airtight coordinate proofs (post-durable-write handshakes, verified
   SIGKILL, fail-closed missed-hook exits) and full partial-state detectors
   (per-record content equality, ledger snapshot equality, no-resurrection
   poles).
2. Residual risk is breadth, not soundness: KR:312's stream kill point is
   genuinely nondeterministic per run (deliberate, monotone oracles mitigate),
   and FK's multi-iteration stress (`QMP_FINALIZE_KILL_ITERATIONS`) defaults
   to 1 iteration in the canonical gate, so the canonical rail kills each
   armed point exactly once per run.
3. Two cosmetic hardening opportunities (no verdict impact): CS:155's
   trivially-true precondition guard, and widening the two total-only
   detectors (HE:590, HE:173) to full `LedgerSnapshot` equality like their
   siblings.
