# Changelog

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
