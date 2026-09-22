# Changelog

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
  added shell syntax validation to the canonical rail, and hardened generated
  guidance against source/output symlink escapes and reserved Git paths.

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
