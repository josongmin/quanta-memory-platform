# Fail-Open Test Audit — T2: core `api/` + `tests/public_api_surface.rs`

Auditor: T2 (parallel fail-open audit, 2026-09-26). Read-only pass; the only file
created is this report.

## Header — scope, grep counts, row count

Scope files (all read in full; no sampling — every test body was read, deep-read
budget concentrated on fail/fence/panic/poison/atomic/denial tests):

| file | `#[test]`+`#[tokio::test]` grep count | rows |
| --- | --- | --- |
| `crates/quanta-memory-platform-core/src/api/control.rs` | 129 | 129 |
| `crates/quanta-memory-platform-core/src/api/builder.rs` | 7 | 7 |
| `crates/quanta-memory-platform-core/tests/public_api_surface.rs` | 9 | 9 |
| `crates/quanta-memory-platform-core/src/api/broker.rs` | 0 (no `#[cfg(test)]`) | 0 |
| `crates/quanta-memory-platform-core/src/api/mod.rs` | 0 (no `#[cfg(test)]`) | 0 |

**Grep total = 145. Row count = 145.** Row count equals grep count.

Two grep-accuracy notes (stated so the numbers stay honest):

- **Zero `#[tokio::test]`** anywhere in scope — every test is synchronous.
- The surface file's 9 grep hits include **one false positive**: line 11 is a
  doc-comment sentence containing the literal `` `#[test]` ``. The real test
  count there is **8**; the 9th row (`names_every_public_reexport`, line 96) is
  a `#[allow(dead_code)]` **compile-time pin fn, not a test** — it is rowed so
  the row count still equals the raw grep count, with that explanation as its
  mechanism.

Verdict tally: **134 STRONG, 9 WEAK, 1 FAILOPEN, 0 VACUOUS, 1 non-test compile-pin
row** (134+9+1+1 = 145). One grouping note: no rows were sampled or grouped away
— homogeneous STRONG groups (live-reconfig denials, scratch-cap tests,
sep23_fup007 zero-byte group) are individually rowed with one-line reasons; the
deep-read prose is spent on the ten findings F1–F10 below.

Baseline observed (read-only, `--all-features`): `api::control::tests` 129
passed / 0 failed; `api::builder::tests` 7 passed / 0 failed;
`--test public_api_surface` **8** passed / 0 failed. Cargo's own executed-count
(8, not 9) independently confirms the doc-comment grep false positive above.

## Findings (detail for the non-STRONG rows)

### F1 — FAILOPEN (A3-adjacent): `pressure_classifies_from_semantic_usage` (control.rs:3434)

Oracle (control.rs:3437, 3461):

```rust
assert_eq!(b.classify_pressure().expect("classify"), PressureState::Normal);
...800_000 committed via two owners...
assert_eq!(b.classify_pressure().expect("classify"), PressureState::Soft);
```

Mechanism: the name's load-bearing clause is "**from semantic usage** (not
allocator telemetry)" — `classify_pressure`'s own doc (control.rs:1887) promises
exactly that. But the body never installs an allocator snapshot, so nothing
distinguishes "classifies from the ledger" from "classifies from the last
observed allocator resident value, falling back to semantic when absent". If
`observe_allocator` stashed `resident_bytes` and `classify_pressure` preferred
it when present, this test stays green (no snapshot observed → fallback path).
The A3 exemplar at control.rs:3403 covers the `observe_allocator` side
(advisory must not mutate the ledger); the pressure-classification side is open.
- Minimal fix: after charging 800k, call
  `b.observe_allocator(&AllocatorSnapshot { resident_bytes: Some(999_999), .. })`
  then assert `classify_pressure() == PressureState::Soft` (semantic 80%, not
  the allocator's 100%/Fatal) and ledger totals still 800_000.
- Critic mutation: add `last_allocator_resident: Option<u64>` set by
  `observe_allocator` and used by `classify_pressure` as `used` when present —
  current body stays green (proves fail-open); with the minimal fix applied the
  same mutation turns it red.

### F2 — WEAK: `owner_and_group_soft_caps_each_trigger_the_same_reclaim_lane` (control.rs:6282)

Asserts exactly one request fired on each broker (`len() == 1`) and, on the
group side only, `requests[0].resource_group == Some("soft-group")`. The named
contract is the **lane** — that an owner soft crossing and a group soft crossing
both produce the *same reclaim lane* — but `requests[0].trigger` is asserted on
neither side. A build that fires `ReclaimTrigger::CapacityDenial` (or any other
lane) on soft crossings keeps both reserves succeeding and this test green.
- Minimal fix: assert `requests[0].trigger == ReclaimTrigger::SoftThreshold`
  (and `max_attempts == 1`) on both the owner and the group broker.
- Critic mutation: change the soft-lane plan to emit a non-`SoftThreshold`
  trigger — current body green; fixed body red.

### F3 — WEAK: `reserve_denied_class_hard_cap_no_borrow` (control.rs:3025)

First half asserts only `Err(MemoryError::Capacity(_))` on a fresh broker and
never looks at the ledger afterwards; the borrow positive pair is `is_ok()`-only.
A denied reserve that strands a phantom zero-byte ticket (or nudges totals)
leaves this test green — the sibling at control.rs:2989 does the before/after
check, this one dropped it.
- Minimal fix: after the denial assert `snapshot.total_owner_bytes() == 0 &&
  snapshot.live_ticket_count == 0` (and mirror for the borrow-pair broker after
  its `is_ok()`).
- Critic mutation: make the class-cap denial path leave a rolled-forward
  zero-byte ticket in the ledger — current body green; fixed body red.

### F4 — WEAK: `reserve_borrow_beyond_shared_pool_denied` (control.rs:3068)

Same shape as F3 on a fresh broker: variant-only oracle, no post-denial ledger
or ticket-count check, so a partially-mutating denial is invisible.
- Minimal fix / critic mutation: same as F3.

### F5 — WEAK: `owner_without_group_mapping_is_not_gated_by_admission` (control.rs:4158)

Oracle is `reserve(...5_000_000...).is_ok()` only. The name claims the unmapped
owner is "not gated"; the test proves admission succeeded but never proves the
charge actually landed (a silent no-op `Ok` with an unmutated ledger passes) nor
that it releases.
- Minimal fix: commit and assert `ledger_snapshot().total_owner_bytes() ==
  5_000_000`, then release and assert 0.
- Critic mutation: make `reserve` return `Ok` without mutating the ledger —
  current body green; fixed body red.

### F6 — WEAK: `live_reconfig_class_hard_cap_below_class_usage_denied` (control.rs:3180)

Variant-only (`Err(InvalidState)`) despite 300k of prior live usage in the
broker; unlike its siblings (3149, 3206, 3270, 3315) it never re-checks the
active policy or ledger, so "denied but state mutated" stays green.
- Minimal fix: capture `effective_report()` + `ledger_snapshot()` before and
  assert equality after (copy the 3206 pattern).
- Critic mutation: make the denied class-cap path apply the policy before
  validating — current body green; fixed body red.

### F7 — WEAK: `live_reconfig_owner_quota_below_owner_usage_is_denied` (control.rs:3241)

Identical shape to F6 (variant-only, 300k live usage, no untouched-state check).
- Minimal fix / critic mutation: same as F6.

### F8 — WEAK: `rebuild_fence_rejects_foreign_clone_finish_and_second_live_begin` (control.rs:3803)

The second clause is well tested (foreign `begin_rebuild()` rejected, fence
stays `Active` + `rebuild_in_progress`, owner finish reopens). But the first
clause — "foreign **clone finish**" — cannot be exercised by any body:
`RebuildFence` has no `Clone` derive (control.rs:544-549) and `finish(mut
self)` consumes the permit, so no API exists through which a broker clone could
finish another coordinator's fence. That half of the name is enforced by the
type system and is permanently green at runtime; the test's own comment admits
this ("the tokenless finish path is structurally gone"). Not a defect in
behavior — a defect in name-vs-oracle honesty.
- Minimal fix: rename to drop the unexercisable clause, and/or pin the
  invariant with a compile-fail doc test (`let f2 = fence.clone();` must not
  compile).
- Critic mutation: none possible at runtime for the clone-finish clause (that
  is the finding); for the exercised half, allowing a second `begin_rebuild()`
  over a live fence turns the test red today.

### F9 — WEAK: `sep23_fup003_ticket_exhaustion_blocks_without_callback` (control.rs:7313)

Real oracle on the pure function — `preflight(&inputs)` with
`ticket_id_space_exhausted: true` must return `Blocked{TicketIdSpaceExhausted}`
(byte-equal verdicts, exhaustion wins over a reclaimable byte breach, legacy
error message). But the name's subject is the broker's *reserve* behavior, and
the body never constructs a broker: it fabricates `PreflightInputs` directly
(the comment explains u64 ticket ids can't be exhausted live). The wiring the
name implies — that the broker actually sets `ticket_id_space_exhausted` when
building its preflight inputs — is untested here; "without callback" is
structural (a pure call cannot fire one).
- Minimal fix: add a broker-level leg that pokes the internal ticket-id counter
  to `u64::MAX` (the file already pokes internals elsewhere, e.g. 3778, 7865)
  and asserts `reserve(...)` returns `InvalidState("reservation ticket id space
  exhausted")` with the ledger untouched.
- Critic mutation: stop setting `ticket_id_space_exhausted` in the broker's
  `PreflightInputs` construction — current body green; fixed body red.

### F10 — WEAK (surface): `report_only_builder_returns_data_without_runtime_authority` (surface:331)

Body asserts only `report.preset == Production`. The "without runtime
authority" clause is carried entirely by the return type
(`EffectiveMemoryBudgetReport` holds no broker/handle) — a type-level fact this
test does not and cannot assert at runtime; the behavioral content reduces to
one field.
- Minimal fix: none meaningful at runtime; either accept it as a resolution
  smoke (rename) or assert richer report content (e.g. mandatory class count,
  as surface:298 does) so the row is not preset-field-only.
- Critic mutation: make `build_report_only` return a report with a wrong preset
  for `Production` — test red; but any authority-leak regression is invisible
  by construction.

### A4 verification (`let _ =` on ops under test)

- control.rs:4566 `let _ = b.commit(&t);` — inside the TOCTOU test's worker
  threads. **Benign**: `total_admitted` is incremented only after `reserve`
  succeeds, and the final oracle is
  `snap.total_owner_bytes() == total_admitted` (4581-4585). If `commit` failed,
  the rolled-back ticket would make the ledger smaller than the admitted sum →
  red. The exact-equality oracle subsumes commit success. (Still worth an
  `.expect("commit")` for clarity, not for soundness.)
- control.rs:7210 `let _ = LimitAxis::Global;` — variant touch keeping the
  import live inside an exhaustive match. Benign.
- surface:262/282 `let _ = match k/p { ... }` — exhaustive-match shape pins;
  these *are* the assertion. Benign.

### Track-specific answers

- **State-machine post-state (control.rs):** fence authority, epoch exhaustion,
  and rebuild-ordering tests overwhelmingly assert precise POST-state, not mere
  `Ok`: `rebuild_fence_status()` enum equality (3701, 3775, 3803),
  `rebuild_in_progress` + totals + drain receipts (2388, 2410, 2473, 2523,
  4766, 6783, 7885), exact deferred-queue contents by ticket id (7804, 7860,
  7885), and `sep23_assert_fully_drained` (axes + totals + ticket count +
  attribution + both deferred queues). The only rows where a state-machine
  claim rests on less are F8 (clause structurally unexercisable) and F9 (pure
  preflight, no broker leg).
- **Surface tests:** locking paths/signatures is legitimate (extraction
  contract). `names_every_public_reexport` (surface:96, non-test compile pin)
  locks *resolution* of every re-export in type position but deliberately not
  shape; shape is pinned by the exhaustive-match variant-set tests (182, 256,
  274) and the taxonomy counts (231). One surface test hides a behavior claim
  in light clothing: surface:298 (`builder_first_surface...`) genuinely builds
  a broker and asserts report content — that is fine, but it is behavior, not
  just surface, and would belong in builder.rs by the placement table. No
  surface test merely "uses" an item without locking either resolution or
  shape; the weakest is F10.

## Census table

Verdicts: STRONG / WEAK / FAILOPEN / VACUOUS. "—" = not applicable. For STRONG
rows the critic-mutation column gives a representative behavior break that
turns the test red. File prefix `c/` = `crates/quanta-memory-platform-core/src/api/control.rs`,
`b/` = `.../src/api/builder.rs`, `s/` = `crates/quanta-memory-platform-core/tests/public_api_surface.rs`.

| test fn (file:line) | verdict | mechanism / reason | minimal fix | critic mutation |
| --- | --- | --- | --- | --- |
| c/2274 overlong_owner_is_denied_before_reclaim_or_accounting | STRONG | Err(Capacity) + full snapshot before==after + reclaim attempts 0 | — | accept the overlong owner (or run reclaim first) |
| c/2291 scratch_reserve_cap_overflow_is_denied_without_mutation | STRONG | typed Capacity + snapshot equality + release → total 0 | — | leave a phantom charge on denial |
| c/2323 scratch_recharge_cap_overflow_preserves_original_ticket | STRONG | RechargeError{Capacity} + snapshot equality + both tickets release | — | consume the old ticket on denial |
| c/2367 dropping_pending_reservation_rolls_back_all_axes | STRONG | reserved-state probe (100k) then before==after snapshot | — | drop without rollback |
| c/2388 dropping_pending_reservation_during_rebuild_does_not_strand_capacity | STRONG | rebuild_in_progress + total 0 + finish drains clean | — | strand the pending bytes behind the fence |
| c/2410 failed_pending_commit_during_rebuild_rolls_back_capacity | STRONG | InvalidState + total 0 + finish drain_error none | — | let the commit half-apply |
| c/2429 pending_drop_restores_resource_group_capacity | STRONG | negative→positive `admits_group` probe pair + total 0 | — | fail to restore group capacity on drop |
| c/2473 deferred_committed_cleanup_is_batched_and_drains_large_backlog | STRONG | exact batch size, exact remainder, final tickets/totals/queue all zero/empty (internal poke but real oracle) | — | drain the whole backlog in one pass or drop failures |
| c/2508 managed_charge_drop_on_healthy_broker_releases_exact_capacity | STRONG | 100k charged → drop → 0, field accessors exact | — | drop leaks authority |
| c/2523 managed_charge_drop_during_rebuild_defers_until_finish | STRONG | conservative deferral (100k stays) then drains to 0 | — | release early under fence |
| c/2541 managed_charge_explicit_release_failure_retains_authority | STRONG | typed InvalidState + id/is_active + 100k retained + retry releases | — | retire the lease on failed release |
| c/2560 managed_charge_batch_failure_retains_every_lease | STRONG | all leases active + ids exact + 220k unchanged + atomic retry → 0 | — | release a prefix before failing |
| c/2578 managed_charge_batch_rejects_foreign_broker_before_mutation | STRONG | both brokers' ledgers unchanged + drop cleanup to 0 | — | release domestic leases before hitting the foreign one |
| c/2597 managed_batch_invalid_lease_rejects_before_ticket_collection | STRONG | 5 scenarios: snapshot before==after, exact ids/flags, oversized-slice length reject, mixed-kind drain (0,0,0,0,0) | — | collect tickets before validating |
| c/2767 managed_charge_recharge_denial_restores_old_authority | STRONG | ChargePreserved recovery + id/bytes + 100k + legal swap + drain | — | drop the lease on recharge denial |
| c/2802 managed_recharge_at_live_ticket_cap_keeps_broker_authority_maps_exact | STRONG | validate_exact + ticket count at cap + total 1 + fully drained | — | require a free slot for managed recharge |
| c/2844 replace_denial_returns_original_live_charge | STRONG | ChargePreserved(charge) payload 100k + ledger 100k + release ok | — | consume the charge on denial |
| c/2873 owned_charge_sequence_matches_exact_reference_model | STRONG | 100-step reference model; per-step total equality + 3 balance predicates; final 0 | — | any per-step drift ≥1 byte |
| c/2947 reserve_commit_keeps_class_owner_and_partition_balanced | STRONG | exact per-axis totals (100k payload / 50k metadata) + invariants 1&2 | — | charge a wrong axis |
| c/2989 reserve_denied_owner_hard_cap_leaves_ledger_untouched | STRONG | typed Capacity + before/after totals + 400k pinned | — | mutate on denial |
| c/3025 reserve_denied_class_hard_cap_no_borrow | WEAK | F3: variant-only on fresh broker; no post-denial ledger/ticket check; borrow pair is_ok-only | assert totals 0 + live_ticket_count 0 after denial | strand a phantom zero-byte ticket on denial |
| c/3068 reserve_borrow_beyond_shared_pool_denied | WEAK | F4: variant-only, fresh broker, post-state invisible | same as F3 | same as F3 |
| c/3089 release_returns_bytes_and_preserves_invariants | STRONG | total 0 + class/owner balance after release | — | release subtracts wrong axis |
| c/3112 rebuild_in_progress_rejects_reserve_commit_release | STRONG | all three ops typed InvalidState + positive pair (commit resumes) | — | admit writes under the fence |
| c/3149 live_reconfig_shrink_below_usage_denied_and_noop | STRONG | policy before==after + ledger 300k | — | install the shrunken policy |
| c/3180 live_reconfig_class_hard_cap_below_class_usage_denied | WEAK | F6: variant-only; 300k live yet no untouched policy/ledger check | copy the 3206 before/after pattern | apply policy before validating |
| c/3206 live_reconfig_dropping_a_class_with_live_usage_is_denied | STRONG | typed denial + ledger 300k + global limit still 1M | — | silently uncap the dropped class |
| c/3241 live_reconfig_owner_quota_below_owner_usage_is_denied | WEAK | F7: variant-only; no untouched-state check | same as F6 | same as F6 |
| c/3270 live_reconfig_raising_headroom_over_non_headroom_usage_is_denied | STRONG | denial + headroom stays 0 + legal 100k positive pair accepted | — | skip the live-state headroom check |
| c/3315 live_reconfig_lowering_group_cap_below_live_group_usage_is_denied | STRONG | denial + group usage 90k observed + raise legal + drain to 0 | — | install the contradictory group cap |
| c/3368 live_reconfig_raise_caps_accepted_and_preserves_usage | STRONG | usage 300k preserved + balance + report 2M | — | reset usage on reconfigure |
| c/3403 allocator_advisory_mismatch_does_not_mutate_accounting | STRONG | advisory.mismatch + semantic_bytes=SSOT + drift exact + ledger equality (anti-A3 for observe_allocator) | — | let observe_allocator write counters |
| c/3434 pressure_classifies_from_semantic_usage | FAILOPEN | F1: no allocator snapshot ever installed; allocator-driven classification stays green (A3-adjacent) | observe a bogus 999_999-byte snapshot, then assert classify stays Soft | plumb last observed resident into classify_pressure; current body green |
| c/3472 denied_reconfigure_leaves_scheduler_controller_and_report_unchanged | STRONG | report + ledger totals + class_usage + pressure + admission all before==after | — | rebuild the controller on denial |
| c/3550 accepted_reconfigure_rebuilds_controller_to_new_global_limit | STRONG | Soft→Normal transition proves rebuild + usage 800k + report 4M | — | keep the stale controller |
| c/3608 reconfigure_rejects_policy_missing_drained_mandatory_class | STRONG | typed Config + report equality + total 0 | — | accept an incomplete policy |
| c/3622 reconfigure_missing_class_denial_preserves_policy_scheduler_and_usage | STRONG | report + snapshot + admission before==after with live usage | — | partial policy install |
| c/3655 pressure_thresholds_are_inclusive_at_exact_boundary_fractions | STRONG | 7 exact cases incl. ±1 pairs around 75/90/98% | — | make any boundary exclusive |
| c/3701 rebuild_fence_is_single_owner_finishes_once_and_recovers_after_abandon | STRONG | status enum + write rejection + receipt fields + abandon/recover post-state | — | let a second begin succeed over a live fence |
| c/3775 begin_rebuild_epoch_exhaustion_leaves_fence_state_untouched | STRONG | typed rejection + status stays Abandoned (internal epoch poke to reach u64::MAX) | — | re-adopt the fence on the Err path |
| c/3803 rebuild_fence_rejects_foreign_clone_finish_and_second_live_begin | WEAK | F8: clone-finish clause structurally unexercisable (RebuildFence non-Clone, finish(self)); second-begin half strong | rename / compile-fail pin for `fence.clone()` | (clone half) none possible; (begin half) allow second live begin |
| c/3831 rebuild_fence_gates_reclaim_slow_path_before_capacity_breaches | STRONG | fence wins: no callback, blocked stats, typed InvalidState; post-fence single callback + Capacity | — | run the reclaim callback under the fence |
| c/3917 live_reconfig_denies_removing_a_quota_while_the_owner_holds_authority | STRONG | denial + ledger 800 + retained-quota positive pair + drained | — | silently uncap a live owner |
| c/3979 reentrant_recharge_from_reclaimer_callback_is_bounded_and_typed | STRONG | nested recharge typed OperationInProgress + lease byte-identical + drained | — | allow the reentrant swap |
| c/4074 trait_effective_report_matches_inherent_and_is_not_fabricated | STRONG | inherent==trait byte-equal + 1M limit + no fabricated marker | — | return the fabricated Production fallback |
| c/4099 reserve_gated_by_resource_group_admission_fail_closed | STRONG | typed Capacity + ledger 80k + release→re-admit pair | — | admit over the group cap |
| c/4158 owner_without_group_mapping_is_not_gated_by_admission | WEAK | F5: is_ok()-only; charge landing and release unasserted | assert ledger 5M then release → 0 | return Ok without charging |
| c/4186 execution_scratch_budget_caps_live_scratch_and_spares_other_kinds | STRONG | typed denial + scratch total 80k + payload ungated | — | gate payload by the scratch budget |
| c/4239 execution_scratch_budget_accepts_exact_cap_and_denies_one_more | STRONG | exact-cap admission + 1-byte denial + total 100k | — | off-by-one the cap comparison |
| c/4286 live_runtime_scratch_charge_appears_in_ledger_snapshot | STRONG | all five ledger axes exact + is_enforced + release → 0 | — | charge outside the ledger |
| c/4316 scratch_peak_denial_release_and_drop_keep_authority_exact | STRONG | monotonic peak + typed over-ceiling denial + both explicit-release and Drop drain | — | let an observation lower the peak |
| c/4377 live_runtime_scratch_over_cap_is_denied_without_mutation | STRONG | typed Capacity + snapshot before==after + drop drain | — | partial scratch charge on denial |
| c/4399 runtime_scratch_and_direct_broker_charge_share_one_limit | STRONG | cross-path denial + total_live_scratch 60 | — | give runtime scratch a separate budget |
| c/4424 dropping_runtime_scratch_during_rebuild_returns_capacity | STRONG | rebuild_in_progress + total 0 + finish drains | — | strand scratch behind the fence |
| c/4445 runtime_scratch_rejects_foreign_runtime_context_without_mutation | STRONG | typed InvalidState + snapshot equality | — | accept a forged context handle |
| c/4464 runtime_scratch_release_restores_resource_group_capacity | STRONG | negative→positive admits_group pair + total 0 | — | leak group capacity on scratch release |
| c/4509 recharge_denied_by_scratch_budget_preserves_old_scratch_charge | STRONG | OldChargePreserved + scratch total 80k + axes balanced + drain | — | restore onto the wrong axis |
| c/4550 concurrent_reserves_never_exceed_global_limit_no_toctou | STRONG | cap ≤ 1M AND ledger == exact admitted sum (subsumes the benign `let _ = commit` at 4566) | — | admit over cap or tear an update |
| c/4590 broker_clones_share_one_ledger | STRONG | clone observes 100k; cross-clone release → 0 | — | give clones separate ledgers |
| c/4617 broker_is_send_and_sync | STRONG | compile-time trait-bound oracle (removing Send/Sync breaks it); A1–A8 N/A | — | drop the Send/Sync impl |
| c/4625 accepted_reconfigure_preserves_scheduler_group_usage | STRONG | post-raise 20k denial proves group usage survived | — | reset scheduler usage to zero |
| c/4675 remapping_live_owner_group_is_denied_fail_closed | STRONG | denial + scheduler truth intact + legal after drain | — | remap a live owner |
| c/4766 reserve_committed_during_rebuild_is_fail_closed_no_strand | STRONG | typed InvalidState + totals unchanged | — | strand reserved bytes behind the fence |
| c/4790 reserve_committed_charges_exactly_once | STRONG | 120k then release → 0 (single net charge) | — | double-charge reserve+commit |
| c/4809 recharge_swaps_committed_charge_atomically | STRONG | 50k replaces 100k; old ticket consumed (InvalidState) + drain | — | leave both charges live |
| c/4841 recharge_denied_preserves_original_old_ticket | STRONG | recovery enum + 400k + class/partition balance + same-kind restore | — | restore wrong amount |
| c/4886 denied_recharge_preserves_the_old_axis_not_the_new_one | STRONG | partition-level: payload 400k / metadata 0 | — | restore onto the NEW kind |
| c/4931 recharge_during_rebuild_leaves_old_intact | STRONG | InvalidState + OldChargePreserved + 100k + old ticket valid after finish | — | consume old under the fence |
| c/4967 recharge_does_not_transiently_double_count_group_admission | STRONG | recharge to exact group cap admitted + 1-byte denial | — | charge-new-then-refund admission |
| c/5024 denied_recharge_on_grouped_owner_preserves_group_usage | STRONG | ±1k symmetric admits_group probes (no phantom drift) | — | drift group usage on denial |
| c/5081 recharge_cannot_consume_peer_group_reserved_floor | STRONG | denial + recovery + 500 preserved + 600 boundary legal | — | let peers spend the protected floor |
| c/5155 managed_charge_drop_recovers_poisoned_lock_for_exact_release | STRONG | poison helper asserts it landed; recovered ledger exact + queues empty | — | leak authority on poisoned drop |
| c/5170 poisoned_runtime_scratch_explicit_release_preserves_then_drop_recovers_authority | STRONG | typed Poisoned + is_enforced retained + drop recovers exact axes | — | retire scope on failed release |
| c/5195 poisoned_pending_commit_retains_ticket_until_drop_rollback | STRONG | Poisoned + drop rollback exactness incl. attribution + deferred queues | — | swallow the poisoned rollback |
| c/5225 poisoned_broker_fails_closed_and_trait_report_stays_truthful | STRONG | 10 mutating/observing ops all typed Poisoned + trait report before==after, no fallback marker | — | fabricate a report under poison |
| c/5303 bounded_events_execute_outside_lock_and_cover_major_broker_outcomes | STRONG | sink re-enters effective_report (deadlock canary) + 4 exact events + semantic 0 | — | emit while holding the lock, or drop an outcome |
| c/5366 telemetry_panic_cannot_change_or_abort_accounting | STRONG | reserve Ok + ledger 32 → 0 despite always-panicking sink | — | let sink panic abort or unwind accounting |
| c/5417 sink_panic_payloads_are_retired_once_and_callback_is_quarantined | STRONG | 8 admissions unaffected; calls==1, drops==1 (both panic_on_drop poles); drained | — | re-invoke the quarantined sink |
| c/5468 reclaimer_panic_payloads_are_retired_once_and_admission_remeasures | STRONG | 8 denials all Capacity + calls==1 + attempts 1 + ledger 100 + drained | — | retry the panicked reclaimer |
| c/5533 event_sink_observation_reentry_delivers_outer_event_once | STRONG | nested observe suppressed: calls==1 + ledger 0 | — | deliver the nested event too |
| c/5587 same_broker_reclaimer_reserve_reentry_is_bounded_and_typed | STRONG | nested reserve typed OperationInProgress + ledger 100 → drained | — | allow same-thread reentry |
| c/5661 concurrent_reclaimer_reentry_does_not_start_a_callback_storm | STRONG | calls==1; nested thread denied Capacity; ledger 100 → 0 | — | start a nested callback storm |
| c/5704 independent_outer_reserves_share_one_in_flight_reclaimer | STRONG | channel-synchronized held callback; second reserve denied during hold; calls==1; drained | — | run concurrent callbacks |
| c/5790 soft_and_hard_reclaim_plans_share_one_callback_in_both_arrival_orders | STRONG | both orders: exact trigger slice, semantic bytes, terminal counts, calls==1, drained | — | fire separate soft+hard callbacks |
| c/5910 held_reclaim_remeasures_concurrent_policy_and_slot_changes | STRONG | live postimage decides: (120,admit)/(130,deny) + revision 2 + exact semantic bytes | — | decide on the stale preimage |
| c/5999 concurrent_recharge_and_observe_never_expose_torn_axes | STRONG | validate_exact per sample + staged handshakes on both live vectors + ≥64 racing samples + drained | — | publish a torn axis vector |
| c/6094 forged_snapshot_has_fallible_totals_without_panicking | STRONG | SUT is checked_total_owner_bytes itself; forged u64::MAX+1 → Err(InvalidState), no panic | — | make checked totals wrap/panic |
| c/6116 concurrent_reserve_and_reconfigure_preserve_group_cap_truth | STRONG | admitted==0 / denied==128 + baseline 90k survives + balanced | — | admit an overflow mid-reconfigure |
| c/6252 soft_cap_crossing_triggers_reclaim_without_immediate_hard_deny | STRONG | exactly 1 request, SoftThreshold, max_attempts 1, attempts 1; admission Ok | — | hard-deny at the soft crossing |
| c/6282 owner_and_group_soft_caps_each_trigger_the_same_reclaim_lane | WEAK | F2: lane/trigger kind never asserted on either side (len==1 + group field only) | assert trigger == SoftThreshold both sides | emit a non-soft lane for soft crossings |
| c/6339 hard_cap_denial_reclaims_outside_lock_remeasures_and_retries_once | STRONG | trigger CapacityDenial + attempts 2 + reported 80 + ledger 30 | — | retry more than once / trust the report |
| c/6382 group_reserved_floor_is_unavailable_to_peer_group | STRONG | ±1 boundary: 601 denied vs 600/400 admitted; releases clean | — | let peers spend the reserved floor |
| c/6440 grouped_headroom_preserves_protected_floor_across_reconfigure_and_release | STRONG | multi-stage floor probes across identity reconfigure + mixed batch release + validate_exact + tickets 0 | — | leak the protected floor on release |
| c/6541 accepted_reconfigure_updates_revision_source_and_matches_headroom_predicate | STRONG | revision 2 + source runtime_reconfigure + exact provenance record + headroom 200 | — | bump nothing / wrong source |
| c/6580 identity_reconfigure_succeeds_during_legal_class_borrow | STRONG | Ok-oracle is the named claim (borrow stays live; usage-preservation not re-asserted, not named) | — | deny identity reconfigure during a borrow |
| c/6600 identity_reconfigure_succeeds_during_legal_group_burst | STRONG | same shape as 6580 for group burst | — | deny identity reconfigure during a burst |
| c/6630 reclaim_priority_orders_victims_deterministically | STRONG | concrete positional order last<normal<first + group order (single observation; repeat-determinism not separately asserted) | — | reorder victims by insertion |
| c/6746 sep23_fup007_zero_byte_committed_blocks_group_removal_until_explicit_release | STRONG | denial + explicit release + drained + removal now legal (positive pole) | — | treat zero usage as no authority |
| c/6770 sep23_fup007_zero_byte_committed_drop_after_denied_reconfigure_releases | STRONG | denial → Drop → drained → removal legal | — | strand zero-byte authority on drop |
| c/6783 sep23_fup007_zero_byte_committed_rebuild_fence_drop_drains_on_finish | STRONG | ticket deferred under fence (count 1) then finish drains; fully drained | — | drop the deferred ticket |
| c/6799 sep23_fup007_zero_byte_pending_blocks_removal_explicit_rollback_unblocks | STRONG | pending denial + rollback drains + removal legal | — | ignore pending zero-byte authority |
| c/6823 sep23_fup007_zero_byte_pending_drop_rolls_back_without_stranding | STRONG | denial → Drop → fully drained | — | strand the pending rollback |
| c/6843 sep23_fup007_denied_reconfigure_preserves_policy_revision_scheduler_ledger | STRONG | revision/policy/provenance + snapshot + group probe all frozen; observe group 0 | — | bump revision on denial |
| c/6863 sep23_fup007_mixed_zero_and_nonzero_partial_termination | STRONG | nonzero release leaves 0 bytes but zero-byte ticket still blocks; then unblocks | — | let partial termination unblock removal |
| c/6895 sep23_fup007_scope_matrix_remap_cap_ungrouped_terminated | STRONG | 4-leg matrix: remap denied, cap-only accepted, terminated never blocks, ungrouped doesn't pin | — | pin group removal on an ungrouped ticket |
| c/6988 sep23_fup005_ten_thousand_reconfigures_keep_provenance_bounded | STRONG | revision +10k; provenance ≤ +1 with prefix equality; source pinned; observe==report; drained | — | grow provenance per reconfigure |
| c/7031 sep23_fup005_denied_reconfigure_preserves_provenance_byte_for_byte | STRONG | provenance/revision/source byte-for-byte after denial (with a planted active record) | — | mutate the receipt on denial |
| c/7095 sep23_fup003_terminal_matrix_never_fires_callback | STRONG | 6 fixed cases: typed denial + terminal counter + empty requests + pristine snapshot equality + attempts 0 + deterministic verdicts | — | fire a callback on a terminal denial |
| c/7221 sep23_fup003_borrowing_class_hard_is_not_terminal_until_pool_exhausted | STRONG | borrow admits (soft-only requests) then beyond pool is terminal with no new callback | — | treat borrowing class hard as terminal early |
| c/7272 sep23_fup003_blocked_matrix_never_fires_callback | STRONG | byte-equal legacy errors (unconfigured class, fenced reserve) + blocked counters + drained | — | fire a callback for a blocked request |
| c/7313 sep23_fup003_ticket_exhaustion_blocks_without_callback | WEAK | F9: pure preflight with fabricated inputs; broker wiring of the flag untested; "without callback" structural | broker leg: poke ticket-id to u64::MAX, assert typed reserve error | stop setting the flag in the broker's PreflightInputs |
| c/7377 live_ticket_count_cap_runs_one_reclaim_then_remeasures | STRONG | exact legacy message (ungrouped), snapshot before==after, exactly 1 request, drained; both group poles | — | loop callbacks at the ticket cap |
| c/7467 live_ticket_count_cap_can_recover_when_host_releases_zero_byte_charge | STRONG | host release → admission Ok + cap exact + zero-byte victim candidacy + drained | — | never remeasure after the callback |
| c/7534 sep23_fup003_predicted_failure_skips_soft_and_runs_single_hard | STRONG | no request at 80; exactly one CapacityDenial plan for the predicted failure; attempts 1, terminals 0 | — | run soft-first on a predicted failure |
| c/7589 sep23_fup003_hard_remeasure_still_denied_is_terminal_without_second_callback | STRONG | 1 request (hard), terminal on remeasure, no second attempt | — | retry the callback after remeasure |
| c/7654 sep23_fup006_accepted_reconfigure_inside_callback_remeasures_under_new_policy | STRONG | revision brackets exactly one accepted reconfigure; ticket/owner/class/group truth + drained | — | remeasure under the stale policy |
| c/7729 sep23_audit_max_scale_plus_one_byte_denies_clean_without_callback | STRONG | exact "prospective usage exceeds u64" error + attempts 0 + u64::MAX total + drain + broker reusable | — | wrap the prospective aggregate |
| c/7804 sep23_audit_failing_pending_drain_preserves_ticket_for_retry | STRONG | queue seeded and exact-id preserved across retries; drain_error surfaced; snapshot equality | — | drop the failing ticket from the queue |
| c/7860 sep23_audit_failing_committed_drain_restores_whole_batch | STRONG | whole batch restored by id + drain_error + snapshot equality + totals 0 | — | drain partially then discard |
| c/7885 competing_rebuild_fence_preserves_deferred_authority_and_original_error | STRONG | re-entry error pinned + preserved ticket id + finish drains to 0 | — | let the competing drain discard authority |
| c/7922 sep23_fup006_denied_reconfigure_inside_callback_preserves_receipt_and_admits | STRONG | denial typed + revision/provenance/source frozen + admission under old policy + drained | — | bump the receipt on denial |
| c/8008 sep23_audit_group_denial_runs_single_hard_regardless_of_soft_crossing | STRONG | both soft settings: exactly one plan, Hard trigger; drained | — | plan soft-first on a group denial |
| c/8056 sep23_audit_wrong_victim_release_does_not_admit | STRONG | wrong-class victim freed; remeasure denies terminally; ledger exactly 80 | — | admit on the callback's report |
| c/8108 sep23_audit_forged_reclaimed_bytes_are_recorded_but_never_trusted | STRONG | telemetry records the lie (30) AND ledger 80 AND terminal denial — the anti-A3 exemplar | — | trust reported bytes for admission |
| c/8146 sep23_audit_panicking_reclaimer_is_contained_and_broker_survives | STRONG | panic counted as failure; quarantined (calls stay 1); ledger exact; drained twice | — | retry or propagate the callback panic |
| c/8209 reclaimer_release_then_panic_uses_remeasured_ledger_not_callback_result | STRONG | release-then-panic: admission Ok from remeasured ledger; failures 1; validate_exact; totals/tickets | — | let the panic veto made progress |
| c/8272 sep23_audit_zero_byte_reserve_fires_no_callback_even_over_soft | STRONG | over soft, a 0-byte reserve adds no callback (len stays 1) + drained | — | fire a callback for zero-byte edges |
| c/8313 terminal_preflight_preserves_the_live_first_breach_when_zero_axis_differs | STRONG | 3-way byte-equal errors (preflight / internal attempt / public) + axis message + snapshot equality | — | swap in the fresh-broker breach |
| c/8447 sep23_fup003_preflight_attempt_agreement_randomized | STRONG | 4 shapes × 500 steps: verdict/attempt byte-identical errors incl. fenced legs; terminality spot checks on fresh brokers; drained | — | any verdict/attempt divergence |
| b/171 build_without_manifest_returns_build_error | STRONG | both build() and build_report_only() typed Build | — | default a manifest silently |
| b/183 build_with_invalid_residency_returns_config_error | STRONG | class hard > global → typed Config | — | skip residency validation |
| b/203 build_with_soft_above_hard_returns_config_error | STRONG | soft>hard → typed Config | — | allow incoherent caps |
| b/225 build_with_valid_residency_succeeds_and_reports_that_residency | STRONG | global + class hard cap verbatim in the report (positive pair) | — | mangle the resolved policy |
| b/249 builder_rejects_manifest_missing_mandatory_class | STRONG | mandatory-class removal → typed Config (positive pair lives at b/225) | — | accept an incomplete manifest |
| b/266 builder_reclaimer_reaches_build_live_broker | STRONG | full behavioral: request fields exact, attempts 1, ledger 60 → 0 via drop | — | drop the reclaimer during build |
| b/332 report_only_builder_applies_override_layers | STRONG | override value 5e9 + provenance source last | — | ignore the override layer |
| s/96 names_every_public_reexport | STRONG (compile-pin, **not a test**) | grep false positive: the 9th `#[test]` hit is a doc-comment mention (line 11); this dead_code fn pins re-export *resolution* at compile time — shape is pinned by s/182/231/256/274 | — | remove/rename any pinned re-export (build breaks) |
| s/182 memory_error_has_exactly_the_nine_documented_variants | STRONG | construct 9 + exhaustive match (compile) + count + helper variants + 2 Display strings | — | add/rename/remove a variant |
| s/231 memory_class_taxonomy_is_stable | STRONG | counts 14/4 pinned before the loops (empty-bucket escape impossible) + disjoint + duplicate scan | — | move a class between buckets |
| s/256 charge_kind_has_exactly_three_partitions | STRONG | exhaustive match (compile) + len 3 | — | add a 4th kind |
| s/274 memory_preset_has_exactly_the_four_documented_variants | STRONG | exhaustive match (compile) + len 4 | — | add a 5th preset |
| s/298 builder_first_surface_produces_a_broker_from_a_preset | STRONG | public builder end-to-end: preset identity + full mandatory class count (behavior claim inside a surface test — noted) | — | return a stub report |
| s/322 builder_without_manifest_fails_closed_with_build_variant | STRONG | typed Build variant | — | panic instead of typed error |
| s/331 report_only_builder_returns_data_without_runtime_authority | WEAK | F10: preset field only; "without runtime authority" is type-structural, asserted nowhere | assert richer report content or rename | wrong preset for Production (authority leak invisible regardless) |
| s/346 pressure_thresholds_construct_and_getters_reach_the_external_path | STRONG | getter round-trip 75/90/98 + defaults + inverted & out-of-range negative poles | — | clamp instead of reject |

## Footer — risk summary

1. This area's signature risk (A3, advisory-vs-authority in the
   reserve→commit→snapshot→release→rebuild pipeline) is **well defended**: nearly
   every control.rs test pairs its telemetry/callback observation with a ledger
   equality or exact-total oracle (`sep23_assert_fully_drained`,
   before==after snapshots, recovery enums). The one genuine hole is F1:
   `pressure_classifies_from_semantic_usage` never installs an allocator
   snapshot, so "not allocator-driven" is asserted by absence, not by pole.
2. The remaining nine findings are WEAK-oracle gaps, not broken tests: five
   denial tests skip the post-state check their siblings carry (F3/F4/F6/F7,
   F5), one lane test never pins the lane kind (F2), one fence test names a
   clause only the type system can enforce (F8), one exhaustion test stops at
   the pure function without the broker leg (F9), one surface test asserts a
   single field (F10). All have one-line fixes that copy patterns already
   present in neighboring tests.
3. No VACUOUS bodies and no A2 (all loops iterate fixed non-empty cases or are
   preceded by count pins); catch_unwind appears only in production code, and
   the panic-containment tests assert containment through recovered ledger
   state, not through "did not panic" wrappers. Biggest residual risk is drift:
   the strong house style here (exact variant + snapshot equality) is enforced
   only by review, so new tests can quietly regress to variant-only oracles.
