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

verify_external_spill_boundary() {
  local fixture_root
  fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/qmp-spill-feature.XXXXXX")"
  trap 'rm -rf "${fixture_root}"' EXIT
  mkdir -p "${fixture_root}/src"
  cat > "${fixture_root}/Cargo.toml" <<EOF
[package]
name = "qmp-spill-feature-boundary-fixture"
version = "0.0.0"
edition = "2021"

[features]
spill = ["qmp/spill"]
full = ["qmp/full"]
spill-durable = ["qmp/spill-durable"]

[dependencies]
qmp = { package = "quanta-memory-platform-facade", path = "${repo_root}/crates/quanta-memory-platform-facade", default-features = false }
EOF
  cat > "${fixture_root}/src/main.rs" <<'EOF'
use qmp::{DurableRecordId, SpillReceipt};

fn main() {
    let _ = DurableRecordId::new(7);
    let _ = SpillReceipt::Durable { sequence: 7, inventory_after: 1 };
}
EOF

  local features
  for features in spill full; do
    echo "[feature-capability] external ${features}: durable API must be absent"
    if cargo check --offline --manifest-path "${fixture_root}/Cargo.toml" \
      --no-default-features --features "${features}" > "${fixture_root}/check.log" 2>&1; then
      echo "durable API compiled under ${features}" >&2
      exit 1
    fi
    if ! grep -q 'E0432' "${fixture_root}/check.log" || \
       ! grep -q 'DurableRecordId' "${fixture_root}/check.log" || \
       ! grep -q 'src/main.rs:1:' "${fixture_root}/check.log"; then
      cat "${fixture_root}/check.log" >&2
      echo "${features} failed for a reason other than the missing durable API" >&2
      exit 1
    fi
  done

  echo "[feature-capability] external spill-durable: durable API must compile"
  cargo check --offline --manifest-path "${fixture_root}/Cargo.toml" \
    --no-default-features --features spill-durable
  echo "[feature-capability] external full: Redb dependency must be absent"
  cargo tree --offline --manifest-path "${fixture_root}/Cargo.toml" \
    --no-default-features --features full --edges normal > "${fixture_root}/tree.log"
  if grep -Eq '(^|[[:space:]])redb v[0-9]' "${fixture_root}/tree.log"; then
    cat "${fixture_root}/tree.log" >&2
    echo "full unexpectedly resolves Redb" >&2
    exit 1
  fi
  rm -rf "${fixture_root}"
  trap - EXIT
}

run_test "core-live-accounting" "public_api_surface" \
  "live_broker_bridge_is_present_only_with_core_live_accounting"
run_member_lib "quanta-memory-platform-core" "pressure" \
  "application::controller::tests::classification_is_deterministic_across_the_ladder"
run_member_lib "quanta-memory-platform-core" "scheduler" \
  "application::scheduler::tests::admission_reuses_policy_static_fair_share_table_at_high_group_cardinality"
run_member_lib "quanta-memory-platform-core" "telemetry" \
  "adapters::metrics::recording_tests::emitter_records_exact_counter_labels_and_optional_byte_samples"
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
run_member_lib "quanta-memory-platform-facade" "publish_pipeline" \
  "publish_pipeline::prepared_publication::tests::cases_01::identical_prepare_replays_same_token_without_second_charge"
run_lib "spill" "spill" \
  "in_memory_target_write_then_restore_round_trips_the_payload"
if [[ "${mode}" == "full" ]]; then
  verify_external_spill_boundary
  run_lib "revocable_store,spill-durable" "revocable_store" \
    "cases_02::bounded_codec_keeps_output_charged_until_durable_commit_returns"
  run_member_lib "quanta-memory-platform-testkit" "loom" \
    "loom_models::loom_ticket_transition_has_single_terminal_owner"
  run_lib "revocable_store,spill-redb" "revocable_store" \
    "cases_02::durable_redb_round_trip_transfers_charge_exactly_once"
  run_lib "full-backings" "revocable_store" \
    "cases_01::many_spills_share_large_binding_identity_without_unbilled_clones"
  echo "feature-capability: OK (18 advertised feature paths executed non-zero tests; external spill boundary checked)"
else
  echo "feature-capability: OK (14 in-memory/fixture feature paths executed non-zero tests)"
fi
