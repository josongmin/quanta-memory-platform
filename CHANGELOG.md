# Changelog

## Unreleased (after 0.2.0)

- Made rebuild fencing permit-owned and pressure thresholds constructible
  only through validation. Both are source API changes; see
  [MIGRATION.md](MIGRATION.md).
- Kept zero-byte tickets, group/owner attribution and deferred cleanup under
  bounded single-owner authority. Admission, reconfigure, recharge and batch
  release use fail-closed postimages; scratch peak observation is monotonic.
- Consolidated facade read/mutation lifecycle containment for supported
  caller-trait panics and reentry. Payload lookup buckets use a randomized
  per-store hash seed plus full equality; prepared publication separates
  active payload from persistent metadata and bounds raw orphan-root input.
- Strengthened the optional Redb custody protocol with exact record
  disposition, claim-generation ownership, schema-v2 migration and resource
  limits. `SIGKILL`, syscall-error and disposable block-I/O tests do not
  establish physical power-loss qualification. Redb is outside the current
  in-memory product scope.
- Consolidated the canonical workspace rail, feature checks, negative
  conformance tests, public examples and real standalone extraction in CI.
  Scaling receipts use a versioned verifier; a passing structural benchmark
  is not an approved performance SLO or a host-product receipt.

Accepted boundaries are in [the ADRs](docs/adr/README.md). Exact test
selectors remain in [verification scenarios](docs/verification-scenarios.md).
This changelog does not qualify a later source revision.

## 0.2.0 — 2026-09-21

Breaking pre-1.0 internal release.

- Made builder-issued live accounting the default public authority; removed
  raw ledger/broker/ticket and late-attachment consumer paths.
- Added fail-closed semantic ledger axes, policy reconfiguration, group
  scheduling, pressure/reclaim and charged runtime scratch.
- Added versioned retention costs and charged payload, retained-map,
  revocable, transcript and prepared-publication stores.
- Added accounting-safe in-process transfer and an opt-in reconciled durable
  spill backing, plus public conformance, feature and extraction rails.
- Distribution remains internal (`publish = false`, `UNLICENSED`) with Rust
  1.92 as the declared MSRV.

Consumer actions and feature policy are in [MIGRATION.md](MIGRATION.md).
