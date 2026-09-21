#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

run_lib() {
  local features="$1"
  local selector="$2"
  echo "[feature-capability] ${features}: ${selector}"
  scripts/run-ticket-test --locked \
    -p quanta-memory-platform-facade \
    --no-default-features \
    --features "${features}" \
    --lib "${selector}" -- --nocapture
}

run_test() {
  local features="$1"
  local target="$2"
  local selector="$3"
  echo "[feature-capability] ${features}: ${target}::${selector}"
  scripts/run-ticket-test --locked \
    -p quanta-memory-platform-facade \
    --no-default-features \
    --features "${features}" \
    --test "${target}" "${selector}" -- --nocapture
}

run_test "core-live-accounting" "public_api_surface" \
  "live_broker_bridge_is_present_only_with_core_live_accounting"
run_test "unbudgeted-stores,retained_map" "public_api_surface" \
  "explicit_unbudgeted_store_is_a_distinct_product_and_does_not_charge_the_platform"
run_lib "payload_store" \
  "canonical_put_deduplicates_and_charges_once"
run_lib "retained_map" \
  "put_get_remove_get_none_round_trip_corrects_bytes"
run_lib "revocable_store" \
  "insert_tracks_reclaimable_bytes"
run_lib "transcript" \
  "exact_request_retried_n_times_applies_once"
run_lib "publish_pipeline" \
  "default_pipeline_runs_stage_then_commit_each_exactly_once_no_rollback"
run_lib "spill" \
  "in_memory_target_write_then_restore_round_trips_the_payload"
run_lib "revocable_store,spill-redb" \
  "durable_redb_round_trip_transfers_charge_exactly_once"
run_lib "full-backings" \
  "many_spills_share_large_binding_identity_without_unbilled_clones"

echo "feature-capability: OK (10 advertised feature paths executed non-zero tests)"
