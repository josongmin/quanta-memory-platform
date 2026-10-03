#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

mode="full"
if [[ "${1:-}" == "--in-memory" ]]; then
  mode="in-memory"
  shift
fi
if [[ "$#" -ne 0 ]]; then
  echo "usage: scripts/verify-feature-capabilities.sh [--in-memory]" >&2
  exit 2
fi

run_lib() {
  local features="$1"
  local module="$2"
  local selector="$3"
  echo "[feature-capability] ${features}: ${selector}"
  python3 scripts/lib/run_cargo_test.py --locked \
    -p quanta-memory-platform-facade \
    --no-default-features \
    --features "${features}" \
    --lib "${module}::service::tests::${selector}" -- --nocapture --exact
}

run_member_lib() {
  local crate="$1"
  local features="$2"
  local selector="$3"
  echo "[feature-capability] ${crate} ${features}: ${selector}"
  python3 scripts/lib/run_cargo_test.py --locked \
    -p "${crate}" --no-default-features --features "${features}" \
    --lib "${selector}" -- --nocapture --exact
}

run_test() {
  local features="$1"
  local target="$2"
  local selector="$3"
  echo "[feature-capability] ${features}: ${target}::${selector}"
  python3 scripts/lib/run_cargo_test.py --locked \
    -p quanta-memory-platform-facade \
    --no-default-features \
    --features "${features}" \
    --test "${target}" "${selector}" -- --nocapture --exact
}

run_test "core-live-accounting" "public_api_surface" \
  "live_broker_bridge_is_present_only_with_core_live_accounting"
run_member_lib "quanta-memory-platform-core" "pressure" \
  "application::controller::tests::classification_is_deterministic_across_the_ladder"
run_member_lib "quanta-memory-platform-core" "scheduler" \
  "application::scheduler::tests::admission_reuses_policy_static_fair_share_table_at_high_group_cardinality"
run_member_lib "quanta-memory-platform-core" "telemetry" \
  "adapters::metrics::tests::emitter_holds_no_accounting_state"
run_member_lib "quanta-memory-platform-core" "unbudgeted_pin" \
  "adapters::pin::tests::pin_invalidate_round_trip_tracks_dependent_count"
run_member_lib "quanta-memory-platform-testkit" "conformance" \
  "self_tests::conformance_suite_rejects_a_fake_that_under_reports_usage"
run_member_lib "quanta-memory-platform-testkit" "alloc_tracking" \
  "alloc_tracking::tests::recorded_signals_accumulate_and_net_live_is_correct"
run_test "unbudgeted-stores,retained_map" "public_api_surface" \
  "explicit_unbudgeted_store_is_a_distinct_product_and_does_not_charge_the_platform"
run_lib "payload_store" "payload_store" \
  "cases_01::canonical_put_deduplicates_and_charges_once"
run_lib "retained_map" "retained_map" \
  "cases_01::put_get_remove_get_none_round_trip_corrects_bytes"
run_lib "revocable_store" "revocable_store" \
  "cases_01::insert_tracks_reclaimable_bytes"
run_lib "transcript" "transcript_store" \
  "cases_01::exact_request_retried_n_times_applies_once"
run_lib "publish_pipeline" "publish_pipeline" \
  "default_pipeline_runs_stage_then_commit_each_exactly_once_no_rollback"
run_lib "spill" "spill" \
  "in_memory_target_write_then_restore_round_trips_the_payload"
if [[ "${mode}" == "full" ]]; then
  run_member_lib "quanta-memory-platform-testkit" "loom" \
    "loom_models::loom_ticket_transition_has_single_terminal_owner"
  run_lib "revocable_store,spill-redb" "revocable_store" \
    "cases_02::durable_redb_round_trip_transfers_charge_exactly_once"
  run_lib "full-backings" "revocable_store" \
    "cases_01::many_spills_share_large_binding_identity_without_unbilled_clones"
  echo "feature-capability: OK (17 advertised feature paths executed non-zero tests)"
else
  echo "feature-capability: OK (14 in-memory/fixture feature paths executed non-zero tests)"
fi
