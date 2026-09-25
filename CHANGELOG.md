# Changelog

## Unreleased - 2026-09-26

- Store read boundary (ENG-01): every facade public read now runs through
  one owned `StoreLifecycle::with_read_state` boundary (lifecycle preflight,
  nonblocking state lock, unwind containment), so a caller-trait panic in a
  read such as transcript `session_bytes` can no longer poison an intact
  store. Regression: `session_bytes_ord_panic_preserves_store_and_ledger`.
- Prepared-publication kind buckets (ENG-13): the persistent
  identity/index/tombstone lease is charged as `RetainedMetadata` (previously
  `RetainedPayload`); the active lease stays `RetainedPayload`. Admission
  totals and ticket counts are unchanged. Independent numeric oracle:
  `prepared_identity_index_and_tombstone_keep_exact_independent_cost`.
- Bounded orphan roots (ENG-10): `orphan_tokens`/`gc_orphans` reject a raw
  reference-root slice above 16,384 tokens (duplicates counted) with a typed
  storage capacity error before acquiring any state lock or mutation permit.
  Regression: `orphan_roots_bound_and_fail_closed`.
- Broker batch preflight (ENG-08): `release_managed_many` validates batch
  length (against the live-ticket cap), broker affinity, active lease state,
  and the checked byte sum before building the ticket vector; the byte sum
  can no longer mask an inactive lease as zero. Regression:
  `managed_batch_invalid_lease_rejects_before_ticket_collection`.
- Reclaim contract documented (ENG-02/07): `MemoryReclaimer` callback
  contract rustdoc (synchronous, same-store reentry is bounded
  `OperationInProgress`, claimed bytes advisory with broker remeasurement, no
  deadline/RSS guarantee). Owner regressions:
  `same_store_reclaimer_returns_busy_without_losing_fifo` and
  `other_store_reclaim_frees_admission_and_drains_every_ticket`.
- Scratch peak monotonicity (ENG-11): `grow_to_observed` keeps the recorded
  high-water mark (`peak = max(peak, observed)`). Regressions:
  `scratch_peak_never_decreases_on_smaller_observation` and
  `scratch_peak_denial_release_and_drop_keep_authority_exact`.
- Validated pressure thresholds (ENG-12, source-breaking): `PressureThresholds`
  fields are private; construction goes through validating `new`/`Default`
  with read-only getters. External-path proof:
  `pressure_thresholds_construct_and_getters_reach_the_external_path` plus a
  `compile_fail` doc on the type.

## Unreleased - 2026-09-24

- Added two in-memory cross-store accounting E2E cases and strengthened the
  scaling-result verifier to reject inconsistent success/denial outcome counts.
  Existing 48-row smoke and 220-row full artifacts passed the revised verifier;
  a clean `db0b03c` smoke subsequently passed 48/48. Fresh full benchmark
  generation and repository-wide qualification belong to a clean exact-source
  candidate. The current status is recorded in the annotated
  `qmp-local-qualified-*` tag only after canonical, full benchmark, and
  extraction dry-run gates pass.
- Two fresh full runs at `fdf34cf` emitted 220 rows but failed the revocable
  fill/drain growth verifier at 9.80x and 12.25x versus a 4.0x allowance while
  other Rust builds were active on the shared host. The artifacts and hashes
  are recorded in the qualification receipt. Cause was not established, and no
  code or threshold change followed from those runs. A later exact-source full
  benchmark passed; qualification status is determined by the tag that peels
  to current HEAD.

## Unreleased - 2026-09-23

- Sep-23 follow-up hardening (FUP-001 through FUP-007): exact durable
  record disposition with schema-v2 migration, backend-scoped
  single-owner recovery leases (`DurableRestoreClaim` replaces bare
  sequence permission changes and claim-bearing values are non-cloneable),
  0-byte ticket authority indexes, a
  shared admission preflight with single-plan reclaim, per-axis kind
  validation plus oracle matrices, and bounded reconfigure provenance.
  See `MIGRATION.md` for the breaking claim-API change and the storage
  migration notes.
- Added a real SIGKILL/reopen qualification rail for the durable spill
  tier (public API only), a randomized preflight/attempt agreement
  oracle, reclaim/reconfigure coupling oracles, and deferred-authority
  preservation oracles. Power loss and mid-finalize instruction-point
  kills remain explicitly unverified.
- Restored zero-allocation scheduler admission probes after the shared
  admission engine briefly formatted denial messages on the boolean
  path; the scaling bench pins the contract.
- Bounded management authority independently of byte budgets: live ticket
  count and identifier-byte caps prevent zero-byte metadata growth, and
  deferred cleanup now uses bounded batches with no-progress failures surfaced
  while preserving exact ticket authority.
- Bounded callback recursion: same-broker reserve reentry from a reclaimer now
  returns `OperationInProgress`; only one reclaimer callback runs concurrently
  per broker. Event-sink callbacks are single-flight across threads; recursive
  or concurrent advisory events during an active callback are dropped, not
  queued.
- Read-side facade callbacks that must clone or compare caller-owned values
  now release the store mutex before propagating a panic. This preserves the
  caller-visible panic while preventing an otherwise intact store from becoming
  permanently unavailable due only to mutex poisoning; callback execution time
  remains caller-controlled.

## Unreleased - 2026-09-22

- Removed campaign-only ticket verification tooling and ticket-campaign
  artifacts. The retained bugbash reports are a bounded source-audit record;
  repo-local rails validate source, feature capability, documented test
  selectors, scaling results, and standalone extraction directly.
- Consolidated duplicate doctor/CI wrappers into `scripts/check-workspace.sh`,
  moved every portable policy check under `scripts/`, and made feature coverage
  metadata-driven. The guidance generator is now standard-library-only and
  rejects orphan sources, duplicate outputs, path escapes, and drift.
- Replaced manifest-text dependency checks with a resolved Cargo-graph contract,
  added fail-closed ShellCheck validation to the canonical rail, and hardened
  generated guidance against source/output symlink escapes and reserved Git
  paths.
- Removed duplicate shell parsing and repeated agent-specific guidance;
  centralized low-noise Clippy policy in the workspace manifest while retaining
  the crate-local non-overridable unsafe boundary, and corrected review-only
  rules that were mislabeled as automated gates.
- Consolidated repeated architecture, test, dependency, and error-handling
  guidance in generated agent rules; aligned the documented `MemoryError`
  taxonomy with all nine current variants; removed duplicate live-policy and
  extracted-destination graph checks; deduplicated identical default/bare/
  singleton feature builds; and separated dirty-tree local verification from
  clean exact-HEAD extraction qualification.
- Added a workspace-wide `redundant_clone` guard and removed the existing
  unnecessary policy/configuration clones; rejected broader allocation lints
  that misclassify intentional collect-then-join concurrency tests.
- Consolidated four Semgrep fixture processes into one exact rule-ID scan,
  reducing lint startup cost while strengthening the negative contract so each
  of the six regression rules must independently produce evidence.
- Reused the authoritative all-feature test output for documented-selector
  validation and removed the workspace bare Clippy union already covered by
  each crate's bare all-target sweep.

## 0.2.0 - 2026-09-21

Breaking pre-1.0 hardening release.

- Made builder-issued live accounting the default public authority and removed
  raw ledger/broker/scratch/ticket and late-attachment paths.
- Added exact fail-closed ledger aggregates, policy reconfiguration, resource
  group scheduling, bounded pressure/reclaim, and charged runtime scratch.
- Added `RetainedCostV1` across payload, retained-map, revocable, transcript,
  and prepared-publication retention. Payload references now require `Eq + Hash`
  and retain two digest-index slots.
- Replaced unbounded transcript retention with
  `TranscriptRetentionPolicyV2` and typed saturation/reconfiguration outcomes.
- Added accounting-safe in-process and reconciled durable spill/restore; large
  store binding identity is shared across in-process handles.
- Added public compile-fail contracts, conformance fakes that must be rejected,
  Loom/property/Miri lanes, feature capability tests, scaling gates, exact-tree
  extraction, and source-bound verification rails.
- Removed non-capability moka/scc/bumpalo feature claims. The only durable
  backing feature is `spill-redb`.
- Declared Rust 1.92 as MSRV to match the current Semantica host. Distribution
  remains internal-only, `UNLICENSED`, and `publish = false`.

See [MIGRATION.md](./MIGRATION.md) for consumer action and compatibility policy.
