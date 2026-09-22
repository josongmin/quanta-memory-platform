# Census critic-2: t07–t12 verification (PASS/FAIL + corrections)

Method (same as critic-1): (1) re-ran `grep -c` per scope file vs census row
count; (2) opened 5+ bodies per report to check `file:line → fn name` and
quoted oracles; (3) confirmed a v1 re-verification column/section exists.
Timing sources checked at `timing/top40.md` + `timing/full.txt` (both present).

## Verdicts

| report | grep vs census | spot-checks (5, bodies opened) | v1 column | verdict |
|---|---|---|---|---|
| t07-stores-b | 46 / 47 match (revocable/transcript) | 1144 insert_tracks…, 1157 reclaim_one_is_fifo…, 1620 round_trip (attr), 2139 reclaim_until (attr), transcript 1229 reentrant…, 3007 racing_close… (nested attr) — all names resolve | §4 table F15–F26, each CONFIRM + evidence | PASS (minor correction C1) |
| t08-pipeline-shared | 27+5+9+7+11 = 59 match; 0 elsewhere confirmed | service.rs:162 identity_pipeline…, :226 default_pipeline…; prep 1055 identical_prepare…, 1510 replay_charges… (attr); measure.rs:240 caller-size — all resolve | V1 section F04–F21 + out-of-scope list | PASS (minor correction C2) |
| t09-spill-bench | 20+13+22+0+0+0 = 55 match | spill/service.rs:512 unavailable…, :629 routed_after_offline…, :739 retaining_target…; redb.rs:1213 concurrent_accepts…; surface.rs:663 spill_controller_fails… — all resolve | v1 table F01–F22 + NOT IN SCOPE rows | PASS (no correction; F17 correction verified right) |
| t10-conformance | 5+5+35+0 = 45 match | mod.rs:120 honest_fake…, :180 public_suite…; suites.rs:1623 deterministic…, :1837 property_ (proptest-nested); invariants.rs:198 drift_evidence… — all resolve | §3 v1 table F-1–F-17 + out-of-scope list | PASS (no correction) |
| t11-testkit-rest | 5+2+5+5+6+5+3 = 31 match, per-file exact | scenarios.rs:180 reclaim_returns_zero…; mod.rs:121 harness_covers…; broker.rs:709 reclaim_yield_zero…; fixtures 120 + 159 — all resolve | §3 v1 table V-0, F-1–F-17 | PASS (no correction) |
| t12-e2e | 18+4+1 = 23 match | hellgate 72 charged_store_bytes…, 332 concurrent_charged…, 559 dedup_pin…; bite 107 honest_fake…, 128 double_refund…; surface 110 fixture_mode… — all resolve | V1 section F-2/F-3/F-9/F-10/F-11/F-13/F-14 CONFIRM | PASS (no correction; Unresolved timing note is honest) |

## Concrete corrections

### C1 (t07, minor) — mixed line convention
§3 states "line = `#[test]` attribute line", but early revocable rows use fn
lines: row `1144 insert_tracks…` — `#[test]` is at 1143, fn at 1144
(`sed -n '1142,1146p'`); row `1157 reclaim_one…` — `#[test]` at 1156, fn at
1157. Later rows use attribute lines (`1620`, `2139` are `#[test]`; fns at
+1). Transcript rows use attribute lines (1229, 3007 are `#[test]`).
Fix: normalize all census rows to one convention (recommend `#[test]`
attribute line as stated: 1144→1143, 1157→1156, etc.) or restate the header
as "fn line, ±1 for attribute". No fn-name or verdict error — all names
resolve either way.

### C2 (t08, minor) — census vs findings off-by-one
Census table uses fn lines (`service.rs:162` = fn, attr at 161; prep
`1055` = fn, attr at 1054), while findings D01 cites attribute lines
(`service.rs:161`, `:210`, `:225`, `:246`) and budgeted rows use attribute
lines (`1510` = `#[test]`, fn at 1511). Same ±1 normalization as C1; links
still land on the right test. No content error.

## Verified correct (not corrections)
- t09 F17 "correction" of v1 is right: v1's "`:589` does not check the handle
  value" is wrong — `spill/service.rs:597`
  `assert_eq!(err.into_parts().1, "handle")` asserts it. Keep.
- t09 T09-04 quote verified: `spill/service.rs:639-645` fresh-offline comment
  admits the gap; DELETE verdict stands.
- t07 G1/F15 quotes verified: `("only", 9)` at 2175 vs `("first", 4)` at 1167;
  `source.contains("entries: VecDeque<RevocableEntry")` at 1179 + forbidden
  `remove(0)` concat at 1182.
- t08 D03 quote verified ×4: identical
  `assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");` at
  builder.rs:782/803/812/845.
- t10 G01 verified: `cases: 128, max_shrink_iters: 8_192` at suites.rs:1830-1832
  + dual-backend run; top40 rank 2, 4.973s (top40.md:18).
- t11 G09 verified: dead `assert_ne!(s.net_live_bytes(), u64::MAX - 898)` at
  alloc_tracking/mod.rs:161 and `assert_ne!(after, 1_200)` at
  fake/broker.rs:644, each after an exact assert.
- t12 T12-06 verified: `assert_balanced` at hellgate_e2e.rs:63-67 is three
  boolean asserts (class/owner/context), no drift magnitudes.
- t08 ledger constants verified: builder.rs:795 (105), :836 (236), :861 (49).
- Timing honesty: t10/t12 explicitly mark sub-top40 estimates as unmeasured —
  correct, not a gap.
