from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "lib/run_cargo_test.py"
SPEC = importlib.util.spec_from_file_location("run_cargo_test", SCRIPT)
assert SPEC and SPEC.loader
module = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = module
SPEC.loader.exec_module(module)


class CargoTestOutputTest(unittest.TestCase):
    def test_sums_multiple_result_lines(self) -> None:
        output = "\n".join(
            [
                "test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s",
                "test result: ok. 3 passed; 0 failed; 0 ignored; 1 measured; 0 filtered out; finished in 0.01s",
            ]
        )
        counts = module.parse_cargo_test_counts(output)
        self.assertEqual(counts.selected, 5)
        self.assertEqual(counts.passed, 5)
        self.assertEqual(counts.result_lines, 2)

    def test_zero_selected_is_rejected(self) -> None:
        output = "test result: ok. 0 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.01s"
        with self.assertRaisesRegex(module.CargoTestEvidenceError, "zero executable"):
            module.parse_cargo_test_counts(output)

    def test_ignored_only_is_rejected(self) -> None:
        output = "test result: ok. 0 passed; 0 failed; 9 ignored; 1 measured; 0 filtered out; finished in 0.01s"
        with self.assertRaises(module.CargoTestEvidenceError):
            module.parse_cargo_test_counts(output)

    def test_failed_test_is_counted_as_selected(self) -> None:
        output = "test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s"
        counts = module.parse_cargo_test_counts(output)
        self.assertEqual(counts.selected, 3)
        self.assertEqual(counts.failed, 2)

    def test_malformed_output_is_rejected(self) -> None:
        with self.assertRaisesRegex(module.CargoTestEvidenceError, "no test result"):
            module.parse_cargo_test_counts("cargo emitted a new format")

    def test_build_warning_cannot_fake_a_selected_test(self) -> None:
        output = (
            "warning: qmp@0.1.0: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\n"
            "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; "
            "0 filtered out; finished in 0.00s\n"
        )
        with self.assertRaisesRegex(module.CargoTestEvidenceError, "zero executable"):
            module.parse_cargo_test_counts(output)

    def test_capability_rail_uses_exact_qualified_selectors(self) -> None:
        root = SCRIPT.parents[2]
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)
            fake_python = temporary / "python3"
            fake_python.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$*" >> "$QMP_TEST_CAPTURE"\n',
                encoding="utf-8",
            )
            fake_python.chmod(0o755)
            capture = temporary / "calls.txt"
            environment = dict(os.environ)
            # CircleCI's BASH_ENV prepends its tool paths in child Bash shells,
            # overriding this test's intentionally first PATH entry.
            environment.pop("BASH_ENV", None)
            environment["PATH"] = f"{temporary}:{environment['PATH']}"
            environment["QMP_TEST_CAPTURE"] = str(capture)
            for arguments, expected_count in ((["--in-memory"], 14), ([], 17)):
                capture.write_text("", encoding="utf-8")
                completed = subprocess.run(
                    [str(root / "scripts/verify-feature-capabilities.sh"), *arguments],
                    cwd=root,
                    env=environment,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )
                self.assertEqual(completed.returncode, 0, completed.stderr)
                calls = capture.read_text(encoding="utf-8").splitlines()
                self.assertEqual(len(calls), expected_count)
                for call in calls:
                    self.assertIn("-- --nocapture --exact", call)
                    if " -p quanta-memory-platform-facade " in call and " --lib " in call:
                        self.assertIn("::service::tests::", call)
                lib_selectors = {
                    call.split(" --lib ", 1)[1].split(" -- ", 1)[0]
                    for call in calls if " --lib " in call
                }
                expected_lib_selectors = {
                    "application::controller::tests::classification_is_deterministic_across_the_ladder",
                    "application::scheduler::tests::admission_reuses_policy_static_fair_share_table_at_high_group_cardinality",
                    "adapters::metrics::recording_tests::emitter_records_exact_counter_labels_and_optional_byte_samples",
                    "adapters::pin::tests::pin_invalidate_round_trip_tracks_dependent_count",
                    "self_tests::conformance_suite_rejects_a_fake_that_under_reports_usage",
                    "alloc_tracking::tests::recorded_signals_accumulate_and_net_live_is_correct",
                    "payload_store::service::tests::cases_01::canonical_put_deduplicates_and_charges_once",
                    "retained_map::service::tests::cases_01::put_get_remove_get_none_round_trip_corrects_bytes",
                    "revocable_store::service::tests::cases_01::insert_tracks_reclaimable_bytes",
                    "transcript_store::service::tests::cases_01::exact_request_retried_n_times_applies_once",
                    "publish_pipeline::service::tests::default_pipeline_runs_stage_then_commit_each_exactly_once_no_rollback",
                    "spill::service::tests::in_memory_target_write_then_restore_round_trips_the_payload",
                }
                if not arguments:
                    expected_lib_selectors.update({
                        "loom_models::loom_ticket_transition_has_single_terminal_owner",
                        "revocable_store::service::tests::cases_02::durable_redb_round_trip_transfers_charge_exactly_once",
                        "revocable_store::service::tests::cases_01::many_spills_share_large_binding_identity_without_unbilled_clones",
                    })
                self.assertEqual(lib_selectors, expected_lib_selectors)


if __name__ == "__main__":
    unittest.main()
