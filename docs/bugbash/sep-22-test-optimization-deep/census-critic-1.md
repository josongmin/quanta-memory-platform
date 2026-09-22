# Census critic 1 — t01..t06 verification

Method per report: (a) `grep -c '#[test]'` in stated scope vs census rows;
(b) 5 cited `file:line` bodies opened via grep; (c) v1 re-verification column present.
Line convention note: some reports cite the `#[test]` attribute line, some the
`fn` line; a ±1 difference is a convention, not an error. All "confirm" below
means the fn was found at the cited line or exactly +1 below it.

## t01-ledger.md — PASS
- (a) `ledger.rs` grep: 28 `#[test]`, 0 `tokio::test`. Census 28 rows. Match.
- (b) Confirmed: `:1083` reserve_denied_when_global, `:1132`
  headroom_usage_does_not, `:1499` single_release_preflight_10k,
  `:1654` ticket_ids_monotonic, `:1785` deterministic_fuzz. All exact.
- (c) `## V1 re-verification` table present (F1/F2/F3/F4/F5/F9/F11/Cluster 1,
  quick-wins marked SUPERSEDE-out-of-scope with reason).
- Timing cross-check: `timing/top40.md` rank 19 / 0.580s for the preflight test
  confirmed.
- Corrections: none.

## t02-domain-rest.md — PASS
- (a) `policy.rs` 14 + `config.rs` 21 + `policy_contracts.rs` 14 = 49.
  Census 49 rows. Match. No `#[tokio::test]` claim accepted (grep scope has none).
- (b) Confirmed: `policy.rs:95` check_helpers, `policy.rs:470`
  for_preset_populates, `config.rs:367` higher_precedence, `config.rs:451`
  validate_rejects_reserved_above_soft, `policy_contracts.rs:172`
  global_hard_limit_denies. All exact.
- (c) `## v1 re-verification` table present (F4/F5 SUPERSEDE with narrowing
  rationale, rest CONFIRM).
- Corrections: none.

## t03-control.md — PASS (with line-convention note)
- (a) `control.rs` grep: 76. Census 76 rows. Verdict sum 69+4+1+2=76. Match.
- (b) Confirmed with systematic +1 (report cites attribute line, fn is next
  line): cited `:1605`/fn 1606 scratch_reserve; cited `:1876`/fn 1877
  managed_charge_recharge_denial; cited `:2719`/fn 2720 pressure_thresholds;
  cited `:3191`/fn 3192 concurrent_reserves; cited `:4429`/fn 4430
  reclaim_priority. All resolve.
- (c) `## v1 re-verification` table present (F-01..F-26, no REFUTE/SUPERSEDE).
- Correction (documentation only, no FAIL): state the convention once at the
  census header — "line = `#[test]` attribute line; `fn` is +1" — so future
  critics do not flag the +1 as drift.

## t04-api-rest.md — PASS
- (a) `builder.rs` 6 + naive `public_api_surface.rs` 8 = 14 naive, but the
  report's anchored grep (`^[[:space:]]*#\[test\]`) gives 7 (excludes the
  `` `#[test]` `` mention in the doc comment at `public_api_surface.rs:11`).
  Re-ran both: anchored 7, naive 8. Census 6+7=13. Match; the report
  explicitly documents this trap. No correction.
- (b) Confirmed: `builder.rs:161` build_without_manifest, `builder.rs:215`
  build_with_valid_residency, `public_api_surface.rs:179`
  memory_error_has_exactly_nine (fn line; attribute at 178 — same ±1
  convention), `:295` builder_first_surface, `:328`
  report_only_builder_returns. All resolve.
- (c) `## v1 re-verification` table present (F-04/F-06/F-08/F-25 CONFIRM,
  out-of-scope exclusions named).
- Corrections: none.

## t05-app-adapters-ports.md — PASS
- (a) 13+13+5+4+2+9+8 = 54 = grep total. Each per-file count re-verified
  individually. Match.
- (b) Confirmed: `controller.rs:251` classification_is_deterministic,
  `scheduler.rs:299` within_hard_cap, `adapters/pin.rs:87` pin_invalidate,
  `ports/telemetry.rs:200` operational_labels, `ports/scratch.rs:198`
  reservation_maps_to_live. All exact.
- (c) `## v1 re-verification` table present (F-05/F-07/F-12/F-15/F-16/F-18/
  F-21/F-23/F-24/F-27/F-28 CONFIRM; out-of-scope list named).
- Corrections: none.

## t06-stores-a.md — PASS
- (a) `payload_store/service.rs` 27 + `retained_map/service.rs` 54 = 81.
  Census 81 rows. Match.
- (b) Confirmed: payload `:689` canonical_put, `:703` probe_count, `:1162`
  concurrent_mutation_and_close; retained `:756` owner_has_one, `:875`
  concurrent_checked_read. All exact.
- (c) `## 3. v1 re-verification` table present (F1–F14 all CONFIRM, plus
  01/02/04/05 NONE row with grep basis).
- Timing cross-check: rank 18 / 1.036s concurrent_checked_read and rank 31 /
  0.285s probe_count confirmed in `timing/top40.md`.
- Corrections: none.

## Summary
6/6 PASS. No census-count mismatch, no phantom file:line, no missing v1
column. Only follow-up: adopt the one-line line-convention note in t03
(attribute vs fn line) to prevent false drift flags.
