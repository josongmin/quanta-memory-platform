# Top 40 slowest tests — full workspace suite

Command (mirror of `scripts/check-workspace.sh` cargo-test line plus timing):

```
cargo +nightly test --locked --workspace --all-features -- -Z unstable-options --report-time
```

Notes:
- `scripts/check-workspace.sh` runs `cargo test --locked --workspace --all-features`.
- Plain `-- --report-time` fails on stable (`error: The "report-time" flag is only accepted on the nightly compiler with -Z unstable-options`), so this run used the nightly toolchain with `-Z unstable-options`.
- Suite result: all green, 639 tests passed, 0 failed (12 `test result: ok` binaries + doc-tests: 199 + 14 + 7 + 266 + 18 + 22 + 76 + 4 + 1 + 13 + 19 + 0).
- Timings below are per-test `<time>s` values reported by the nightly `--report-time` flag, sorted descending. Full log tail (last 300 of 823 lines) is in `full.txt`.

| rank | time | test path |
| --- | --- | --- |
| 1 | 10.946s | spill::backings::redb::tests::concurrent_accepts_lose_no_handle_and_assign_distinct_keys |
| 2 | 4.973s | conformance::suites::tests::property_trace_matches_independent_model |
| 3 | 2.946s | spill::backings::redb::tests::committed_restore_payload_survives_reopen_until_resident_ack |
| 4 | 2.918s | spill::backings::redb::tests::reopened_backing_resumes_sequence_and_restores_persisted_handles |
| 5 | 2.747s | revocable_store::service::tests::durable_redb_committed_restore_recovers_after_process_restart |
| 6 | 2.654s | revocable_store::service::tests::durable_redb_prepared_restore_recovers_after_process_restart |
| 7 | 2.472s | spill::backings::redb::tests::persisted_restore_claim_blocks_duplicate_prepare_and_recovers_after_reopen |
| 8 | 2.383s | revocable_store::service::tests::denied_durable_restore_leaves_redb_record_authoritative |
| 9 | 2.343s | spill::backings::redb::tests::injected_write_crash_points_converge_without_duplicate_records |
| 10 | 2.300s | spill::backings::redb::tests::drained_queue_never_reuses_sequence_or_accepts_stale_restore_token |
| 11 | 2.281s | spill::backings::redb::tests::reopened_redb_reconciles_controller_count_from_persisted_inventory |
| 12 | 2.246s | spill::backings::redb::tests::generic_write_is_non_mutating_and_returns_the_original_payload |
| 13 | 2.234s | spill::backings::redb::tests::generic_restore_is_non_destructive_and_requires_owned_two_phase_path |
| 14 | 2.230s | spill::backings::redb::tests::accept_at_exhausted_keyspace_fails_closed_without_overwriting |
| 15 | 2.162s | spill::backings::redb::tests::injected_restore_and_finalize_ack_loss_preserve_recovery_authority |
| 16 | 2.133s | revocable_store::service::tests::durable_redb_round_trip_transfers_charge_exactly_once |
| 17 | 2.000s | spill::backings::redb::tests::durable_spill_target_round_trips_fifo_through_the_trait |
| 18 | 1.036s | retained_map::service::tests::concurrent_checked_read_observes_only_complete_old_or_new_payloads_v1 |
| 19 | 0.580s | domain::ledger::tests::single_release_preflight_is_bounded_by_affected_slots_at_10k_owners |
| 20 | 0.508s | crates/quanta-memory-platform-facade/src/shared/builder.rs — shared::builder::PayloadStoreBuilder (line 251) — compile fail (doc-test) |
| 21 | 0.503s | crates/quanta-memory-platform-facade/src/payload_store/service.rs — payload_store::service::SimplePayloadStore (line 73) — compile fail (doc-test) |
| 22 | 0.488s | crates/quanta-memory-platform-facade/src/payload_store/service.rs — payload_store::service::SimplePayloadStore (line 81) — compile fail (doc-test) |
| 23 | 0.411s | crates/quanta-memory-platform-facade/src/shared/builder.rs — shared::builder::RetainedMapStoreBuilder (line 354) — compile fail (doc-test) |
| 24 | 0.401s | crates/quanta-memory-platform-facade/src/publish_pipeline/mod.rs — publish_pipeline::PublishPipeline (line 25) — compile fail (doc-test) |
| 25 | 0.356s | crates/quanta-memory-platform-facade/src/spill/mod.rs — spill::DurableSpillController (line 49) — compile fail (doc-test) |
| 26 | 0.350s | crates/quanta-memory-platform-facade/src/publish_pipeline/mod.rs — publish_pipeline::PublishPipeline (line 35) — compile fail (doc-test) |
| 27 | 0.327s | crates/quanta-memory-platform-core/src/ports/scratch.rs — ports::scratch::ScratchReservation (line 31) — compile fail (doc-test) |
| 28 | 0.311s | crates/quanta-memory-platform-core/src/lib.rs — (line 13) — compile fail (doc-test) |
| 29 | 0.304s | crates/quanta-memory-platform-facade/src/revocable_store/service.rs — revocable_store::service::SimpleRevocableStore (line 125) — compile fail (doc-test) |
| 30 | 0.298s | crates/quanta-memory-platform-core/src/lib.rs — (line 70) — compile fail (doc-test) |
| 31 | 0.285s | payload_store::service::tests::payload_lookup_probe_count_is_n_independent |
| 32 | 0.282s | crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs — publish_pipeline::prepared_publication::PreparedPublicationService (line 311) — compile fail (doc-test) |
| 33 | 0.274s | crates/quanta-memory-platform-facade/src/spill/mod.rs — spill::InProcessSpillController (line 31) — compile fail (doc-test) |
| 34 | 0.273s | crates/quanta-memory-platform-core/src/api/control.rs — api::control::ManagedCharge (line 240) — compile fail (doc-test) |
| 35 | 0.270s | crates/quanta-memory-platform-facade/src/retained_map/mod.rs — retained_map::RetainedMapStore (line 41) — compile fail (doc-test) |
| 36 | 0.268s | crates/quanta-memory-platform-facade/src/lib.rs — (line 7) — compile fail (doc-test) |
| 37 | 0.257s | crates/quanta-memory-platform-facade/src/shared/accounting.rs — shared::accounting::Unbudgeted (line 82) — compile fail (doc-test) |
| 38 | 0.237s | crates/quanta-memory-platform-facade/src/publish_pipeline/prepared_publication.rs — publish_pipeline::prepared_publication::PreparedPublicationService (line 304) — compile fail (doc-test) |
| 39 | 0.236s | crates/quanta-memory-platform-core/src/lib.rs — (line 50) — compile fail (doc-test) |
| 40 | 0.225s | crates/quanta-memory-platform-core/src/lib.rs — (line 54) — compile fail (doc-test) |
