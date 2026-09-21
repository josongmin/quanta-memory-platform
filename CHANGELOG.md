# Changelog

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
  extraction, and machine-validated ticket receipts.
- Removed non-capability moka/scc/bumpalo feature claims. The only durable
  backing feature is `spill-redb`.
- Declared Rust 1.92 as MSRV to match the current Semantica host. Distribution
  remains internal-only, `UNLICENSED`, and `publish = false`.

See [MIGRATION.md](./MIGRATION.md) for consumer action and compatibility policy.
