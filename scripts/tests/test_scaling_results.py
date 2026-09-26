from __future__ import annotations

import copy
import contextlib
import io
import importlib.util
import itertools
import json
import sys
import unittest.mock
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "verify-scaling-results.py"
SPEC = importlib.util.spec_from_file_location("verify_scaling_results", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

# Independent fixture oracle: do not construct the sample matrix from the validator.
SINGLE = (
    "payload_unique_fill", "payload_dedup_put", "revocable_fill_and_drain",
    "broker_release_owner_cardinality", "scheduler_admission_group_cardinality",
)
CONTENTIONS = ("metadata-stats-heavy", "write-heavy", "mixed-stats-put")
PAIRWISE = (
    "shared-single-owner", "shared-per-thread-owner", "independent-per-thread-owner",
)
RUN_SEQUENCE = itertools.count()


def sample(
    operation: str, mode: str, entries: int, threads: int, repetition: int,
) -> dict[str, object]:
    timed = entries * (2 if operation in {
        "revocable_fill_and_drain", "scheduler_admission_group_cardinality",
        "broker_pressure_cycle",
    } else 1)
    successful = entries // 2 if operation == "contended_pressure_reclaim" else entries if operation in {
        "scheduler_admission_group_cardinality", "broker_pressure_cycle",
    } else timed
    result = {
        "schema": MODULE.SAMPLE_SCHEMA,
        "operation": operation,
        "mode": mode,
        "entries": entries,
        "threads": threads,
        "repetition": repetition,
        "elapsed_ns": 20_000_000,
        "throughput_ops_per_sec": round(timed * 1_000_000_000 / 20_000_000),
        "successful_throughput_ops_per_sec": round(successful * 1_000_000_000 / 20_000_000),
        "timed_operations": timed,
        "successful_operations": successful,
        "denied_operations": timed - successful,
        "p50_ns": 10,
        "p95_ns": 20,
        "p99_ns": 30,
        "semantic_peak_bytes": (
            0 if operation == "scheduler_admission_group_cardinality"
            else 1 if operation == "broker_pressure_cycle"
            else 96 if operation == "contended_pressure_reclaim"
            else 100
        ),
        "semantic_after_close_bytes": 0,
        "allocator_allocated_bytes": 200,
        "allocator_deallocated_bytes": 200,
        "allocator_live_bytes": 0,
        "allocator_high_water_bytes": 200,
        "allocator_underflowed": False,
        "busy_retries": 0,
    }
    if operation in {
        "scheduler_admission_group_cardinality", "broker_pressure_cycle",
        "contended_pressure_reclaim",
    }:
        result.update({
            "admitted_p50_ns": 8, "admitted_p95_ns": 16, "admitted_p99_ns": 24,
            "denied_p50_ns": 12, "denied_p95_ns": 22, "denied_p99_ns": 32,
        })
    if operation == "revocable_fill_and_drain":
        result.update({
            "fill_p50_ns": 8, "fill_p95_ns": 16, "fill_p99_ns": 24,
            "reclaim_p50_ns": 12, "reclaim_p95_ns": 22, "reclaim_p99_ns": 32,
        })
    if operation == "contended_pressure_reclaim":
        result.update({
            "reclaim_p50_ns": 12, "reclaim_p95_ns": 22, "reclaim_p99_ns": 32,
            "reclaimed_operations": entries // 4,
        })
    return result


def set_elapsed(row: dict[str, object], elapsed_ns: int) -> None:
    row["elapsed_ns"] = elapsed_ns
    row["throughput_ops_per_sec"] = round(int(row["timed_operations"]) * 1_000_000_000 / elapsed_ns)
    row["successful_throughput_ops_per_sec"] = round(
        int(row["successful_operations"]) * 1_000_000_000 / elapsed_ns
    )


def valid_records(
    benchmark_args: list[str] | None = None,
    sizes: tuple[int, ...] | None = None,
    threads: tuple[int, ...] | None = None,
    repetitions: int | None = None,
) -> list[dict[str, object]]:
    benchmark_args = list(benchmark_args or [])
    pressure = "--profile=in-memory-pressure" in benchmark_args
    if sizes is None:
        sizes = (1_000,) if "--smoke" in benchmark_args else (1_000, 10_000)
    if threads is None:
        if pressure:
            threads = (1, 8) if "--smoke" in benchmark_args else (1, 8, 32)
        else:
            threads = (1, 2) if "--smoke" in benchmark_args else (1, 2, 8, 32)
    if repetitions is None:
        repetitions = 3 if "--smoke" in benchmark_args else 5
    run_id = f"fixture-{next(RUN_SEQUENCE)}"
    records: list[dict[str, object]] = [{
        "schema": MODULE.RUN_SCHEMA,
        "source_sha": "a" * 40,
        "source_tree": "b" * 40,
        "rustc": "rustc test",
        "cargo": "cargo test",
        "machine": "test-machine",
        "feature_set": "full" if pressure else "all-features",
        "feature_graph_sha256": "f" * 64,
        "dependency_lock_sha256": "e" * 64,
        "benchmark_source_sha256": "9" * 64,
        "bench_build_config_sha256": "7" * 64,
        "package": "quanta-memory-platform-facade",
        "bench_target": "scaling",
        "benchmark_args": benchmark_args,
        "run_id": run_id,
    }]
    for entries in sizes:
        for repetition in range(repetitions):
            if pressure:
                records.append(sample("broker_pressure_cycle", "single-thread", entries, 1, repetition))
                records.extend(
                    sample("dual_store_write_contention", "shared-broker", entries, worker_count, repetition)
                    for worker_count in threads
                )
                records.extend(
                    sample("contended_pressure_reclaim", "two-revocable-stores", entries, worker_count, repetition)
                    for worker_count in threads if worker_count > 1
                )
                continue
            records.extend(sample(operation, "single-thread", entries, 1, repetition) for operation in SINGLE)
            records.extend(
                sample("retained_map_point_read", mode, entries, 1, repetition)
                for mode in ("hit", "miss")
            )
            records.extend(
                sample("payload_contention", mode, entries, worker_count, repetition)
                for worker_count in threads for mode in CONTENTIONS
            )
            records.extend(
                sample("broker_release_pairwise", mode, entries, 2, repetition)
                for mode in PAIRWISE
            )
    records.append({
        "schema": MODULE.END_SCHEMA,
        "source_sha": "a" * 40,
        "source_tree": "b" * 40,
        "source_clean": True,
        "bench_build_config_sha256": "7" * 64,
        "bench_exit_code": 0,
        "run_id": run_id,
    })
    return records


def release_receipts(
    benchmark_args: list[str] | None = None,
) -> tuple[list[list[dict[str, object]]], list[list[dict[str, object]]]]:
    def group(prefix: str, sha: str, tree: str) -> list[list[dict[str, object]]]:
        runs = []
        for index in range(3):
            records = valid_records(benchmark_args)
            records[0]["source_sha"] = records[-1]["source_sha"] = sha
            records[0]["source_tree"] = records[-1]["source_tree"] = tree
            records[0]["run_id"] = records[-1]["run_id"] = f"{prefix}-{index}"
            records[0]["logical_cpus"] = 8
            records[0]["load_per_cpu_start"] = 0.1
            records[-1]["load_per_cpu_end"] = 0.1
            runs.append(records)
        return runs

    return group("candidate", "c" * 40, "d" * 40), group("baseline", "a" * 40, "b" * 40)


RELEASE_POLICY = MODULE.ReleasePolicy(
    max_p99_ns=1_000_000_000,
    max_throughput_regression_percent=10,
    max_p99_regression_percent=20,
    p99_noise_floor_ns=10,
    max_busy_retries_per_operation=1,
    max_run_noise_percent=20,
    max_load_per_cpu=1,
)


class ScalingResultTests(unittest.TestCase):
    def test_complete_default_and_smoke_matrix(self) -> None:
        self.assertEqual(MODULE.validate(valid_records()), 220)
        self.assertEqual(MODULE.validate(valid_records(["--smoke"])), 48)
        self.assertEqual(MODULE.validate(valid_records(["--profile=in-memory-pressure"])), 60)
        self.assertEqual(
            MODULE.validate(valid_records(["--profile=in-memory-pressure", "--smoke"])), 12,
        )

    def test_phase_and_outcome_p99_cannot_exceed_whole_run(self) -> None:
        for operation, field in (
            ("revocable_fill_and_drain", "fill_p99_ns"),
            ("scheduler_admission_group_cardinality", "denied_p99_ns"),
        ):
            with self.subTest(operation=operation, field=field):
                records = valid_records(["--smoke"])
                row = next(record for record in records if record.get("operation") == operation)
                row[field] = int(row["elapsed_ns"]) + 1
                with self.assertRaisesRegex(MODULE.ValidationError, "p99 exceeds measured wall time"):
                    MODULE.validate(records)

    def test_provenance_fields_require_full_git_ids_and_sha256_digests(self) -> None:
        for field in (
            "source_sha", "source_tree", "feature_graph_sha256", "dependency_lock_sha256",
        ):
            with self.subTest(field=field):
                records = valid_records(["--smoke"])
                records[0][field] = "not-a-digest"
                with self.assertRaisesRegex(MODULE.ValidationError, field):
                    MODULE.validate(records)

    def test_release_requires_independent_full_compatible_runs(self) -> None:
        candidates, baselines = release_receipts()
        self.assertEqual(MODULE.validate_release(candidates, baselines, RELEASE_POLICY), 220)
        candidates[1][0]["run_id"] = candidates[1][-1]["run_id"] = "candidate-0"
        with self.assertRaisesRegex(MODULE.ValidationError, "reused a run identity"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        candidates, baselines = release_receipts()
        candidates[1][0]["source_sha"] = candidates[1][-1]["source_sha"] = "d" * 40
        with self.assertRaisesRegex(MODULE.ValidationError, "candidate runs disagree on source_sha"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        with self.assertRaisesRegex(MODULE.ValidationError, "three candidate"):
            MODULE.validate_release(candidates[:2], baselines, RELEASE_POLICY)

    def test_release_rejects_same_source_tree_on_both_sides(self) -> None:
        candidates, baselines = release_receipts()
        for records in candidates:
            records[0]["source_tree"] = records[-1]["source_tree"] = baselines[0][0]["source_tree"]
        with self.assertRaisesRegex(MODULE.ValidationError, "same source tree"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_rejects_changed_or_missing_benchmark_implementation(self) -> None:
        candidates, baselines = release_receipts()
        candidates[0][0]["benchmark_source_sha256"] = "8" * 64
        with self.assertRaisesRegex(MODULE.ValidationError, "benchmark_source_sha256"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        candidates, baselines = release_receipts()
        for records in candidates:
            records[0]["benchmark_source_sha256"] = "8" * 64
        with self.assertRaisesRegex(MODULE.ValidationError, "benchmark_source_sha256"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        candidates, baselines = release_receipts()
        del candidates[0][0]["benchmark_source_sha256"]
        with self.assertRaisesRegex(MODULE.ValidationError, "benchmark_source_sha256"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_rejects_changed_or_missing_build_configuration(self) -> None:
        candidates, baselines = release_receipts()
        for records in candidates:
            records[0]["bench_build_config_sha256"] = records[-1]["bench_build_config_sha256"] = "6" * 64
        with self.assertRaisesRegex(MODULE.ValidationError, "bench_build_config_sha256"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        candidates, baselines = release_receipts()
        del candidates[0][0]["bench_build_config_sha256"]
        del candidates[0][-1]["bench_build_config_sha256"]
        with self.assertRaisesRegex(MODULE.ValidationError, "bench_build_config_sha256"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_run_rejects_build_configuration_change_mid_benchmark(self) -> None:
        records = valid_records(["--smoke"])
        records[-1]["bench_build_config_sha256"] = "6" * 64
        with self.assertRaisesRegex(MODULE.ValidationError, "build configuration changed"):
            MODULE.validate(records)

    def test_release_requires_successful_recorded_bench_exit(self) -> None:
        candidates, baselines = release_receipts()
        candidates[0][-1]["bench_exit_code"] = 1
        with self.assertRaisesRegex(MODULE.ValidationError, "bench exited nonzero"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        candidates, baselines = release_receipts()
        del candidates[0][-1]["bench_exit_code"]
        with self.assertRaisesRegex(MODULE.ValidationError, "bench exit status is missing"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_current_receipt_missing_exit_status_is_not_ordinary_green(self) -> None:
        records = valid_records(["--smoke"])
        records[0]["run_id"] = records[-1]["run_id"] = "new-run"
        del records[-1]["bench_exit_code"]
        with self.assertRaisesRegex(MODULE.ValidationError, "bench exit status is missing"):
            MODULE.validate(records)

    def test_current_receipt_requires_nonempty_string_run_identity(self) -> None:
        for invalid in ([], ""):
            with self.subTest(invalid=invalid):
                records = valid_records(["--smoke"])
                records[0]["run_id"] = records[-1]["run_id"] = invalid
                with self.assertRaisesRegex(MODULE.ValidationError, "run identity is invalid"):
                    MODULE.validate(records)

    def test_ordinary_receipt_requires_all_provenance_fields(self) -> None:
        for record_index, field, message in (
            (0, "run_id", "run identity"),
            (0, "benchmark_source_sha256", "benchmark_source_sha256"),
            (0, "bench_build_config_sha256", "bench_build_config_sha256"),
            (-1, "bench_exit_code", "bench exit status"),
        ):
            with self.subTest(field=field):
                records = valid_records(["--smoke"])
                del records[record_index][field]
                with self.assertRaisesRegex(MODULE.ValidationError, message):
                    MODULE.validate(records)

    def test_impossible_wall_and_outcome_latencies_are_rejected(self) -> None:
        records = valid_records(["--smoke"])
        set_elapsed(records[1], 1)
        with self.assertRaisesRegex(MODULE.ValidationError, "p99 exceeds measured wall time"):
            MODULE.validate(records)
        records = valid_records(["--smoke"])
        scheduler = next(row for row in records if row.get("operation") == "scheduler_admission_group_cardinality")
        scheduler["admitted_p50_ns"] = scheduler["admitted_p95_ns"] = scheduler["admitted_p99_ns"] = 100
        with self.assertRaisesRegex(MODULE.ValidationError, "outcome median exceeds combined p99"):
            MODULE.validate(records)

    def test_ordinary_baseline_requires_distinct_run_and_comparable_cells(self) -> None:
        records = valid_records(["--smoke"])
        with self.assertRaisesRegex(MODULE.ValidationError, "reuses candidate run identity"):
            MODULE.validate(records, records)
        baseline = valid_records(["--smoke"])
        for row in baseline[1:-1]:
            set_elapsed(row, 9_000_000)
        with self.assertRaisesRegex(MODULE.InconclusiveError, "skipped .* short cells"):
            MODULE.validate(records, baseline)
        baseline = valid_records(["--smoke"])
        short_key = MODULE.scenario_key(baseline[1])
        for row in baseline[1:-1]:
            if MODULE.scenario_key(row) == short_key:
                set_elapsed(row, 9_000_000)
        with self.assertRaisesRegex(MODULE.InconclusiveError, "skipped 1 short cells"):
            MODULE.validate(records, baseline)

    def test_ordinary_limits_reject_100_percent_throughput_regression(self) -> None:
        with self.assertRaisesRegex(MODULE.ValidationError, "invalid ordinary benchmark limits"):
            MODULE.validate(valid_records(["--smoke"]), max_throughput_regression_percent=100)

    def test_cli_rejects_vacuous_baseline_and_release_option_mix(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "receipt.jsonl"
            path.write_text("\n".join(json.dumps(row) for row in valid_records(["--smoke"])) + "\n", encoding="utf-8")
            with unittest.mock.patch.object(sys, "argv", ["verify", "--input", str(path), "--baseline", str(path)]), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(MODULE.main(), 1)
            with unittest.mock.patch.object(sys, "argv", ["verify", "--input", str(path), "--release", "--max-p99-ns", "1"]), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(MODULE.main(), 1)

    def test_pressure_one_byte_peak_cannot_be_forged_as_two(self) -> None:
        records = valid_records(["--profile=in-memory-pressure", "--smoke"])
        pressure = next(row for row in records if row.get("operation") == "broker_pressure_cycle")
        pressure["semantic_peak_bytes"] = 2
        with self.assertRaisesRegex(MODULE.ValidationError, "exactly one semantic byte"):
            MODULE.validate(records)

    def test_malformed_record_identity_is_a_validation_error(self) -> None:
        for field in ("schema", "operation", "mode"):
            with self.subTest(field=field):
                records = valid_records(["--smoke"])
                records[1][field] = ["invalid"]
                with self.assertRaises(MODULE.ValidationError):
                    MODULE.validate(records)

    def test_release_rejects_custom_small_matrix_without_smoke_flag(self) -> None:
        for pressure in (False, True):
            with self.subTest(pressure=pressure):
                args = (["--profile=in-memory-pressure"] if pressure else []) + [
                    "--sizes=1000", "--threads=1", "--repetitions=3",
                ]
                candidates, baselines = release_receipts()
                for records in (*candidates, *baselines):
                    records[0]["benchmark_args"] = args
                    records[1:-1] = [
                        row for row in records[1:-1]
                        if row["entries"] == 1_000 and row["repetition"] < 3 and (
                            row["operation"] != "payload_contention" or row["threads"] == 1
                        )
                    ]
                    if pressure:
                        records[1:-1] = [
                            sample("broker_pressure_cycle", "single-thread", 1_000, 1, repetition)
                            for repetition in range(3)
                        ] + [
                            sample("dual_store_write_contention", "shared-broker", 1_000, 1, repetition)
                            for repetition in range(3)
                        ]
                        records[0]["feature_set"] = "full"
                with self.assertRaisesRegex(MODULE.ValidationError, "canonical full matrix"):
                    MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_rejects_real_regression_and_busy_storm(self) -> None:
        candidates, baselines = release_receipts()
        for records in candidates:
            for row in records:
                if row.get("operation") == "payload_unique_fill" and row.get("entries") == 1_000:
                    set_elapsed(row, int(row["elapsed_ns"]) * 2)
        with self.assertRaisesRegex(MODULE.ValidationError, "release throughput regressed"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        candidates, baselines = release_receipts()
        for records in candidates:
            for row in records:
                if row.get("operation") == "payload_unique_fill" and row.get("entries") == 1_000:
                    row["p99_ns"] = 100
        with self.assertRaisesRegex(MODULE.ValidationError, "release p99 regressed"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        candidates, baselines = release_receipts()
        for records in candidates:
            for row in records:
                if row.get("operation") == "payload_contention" and row.get("mode") == "write-heavy":
                    row["busy_retries"] = int(row["timed_operations"]) * 2
        with self.assertRaisesRegex(MODULE.ValidationError, "Busy retries per operation exceeded"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_rejects_recurrent_single_repetition_tail_regression(self) -> None:
        candidates, baselines = release_receipts()
        for repetition, records in enumerate(candidates):
            row = next(
                row for row in records if row.get("operation") == "payload_unique_fill"
                and row.get("entries") == 1_000 and row.get("repetition") == repetition
            )
            row["p99_ns"] = 100
        with self.assertRaisesRegex(MODULE.ValidationError, "release p99 regressed"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_rejects_recurrent_single_repetition_throughput_regression(self) -> None:
        candidates, baselines = release_receipts()
        for repetition, records in enumerate(candidates):
            row = next(
                row for row in records if row.get("operation") == "payload_unique_fill"
                and row.get("entries") == 1_000 and row.get("repetition") == repetition
            )
            set_elapsed(row, int(row["elapsed_ns"]) * 2)
        with self.assertRaisesRegex(MODULE.ValidationError, "release throughput regressed"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_single_process_tail_spike_is_inconclusive(self) -> None:
        candidates, baselines = release_receipts()
        row = next(
            row for row in candidates[0] if row.get("operation") == "payload_unique_fill"
            and row.get("entries") == 1_000 and row.get("repetition") == 0
        )
        row["p99_ns"] = 100
        with self.assertRaisesRegex(MODULE.InconclusiveError, "varies beyond run-noise"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_rejects_recurrent_single_repetition_denied_tail_regression(self) -> None:
        candidates, baselines = release_receipts(["--profile=in-memory-pressure"])
        for repetition, records in enumerate(candidates):
            row = next(
                row for row in records if row.get("operation") == "broker_pressure_cycle"
                and row.get("entries") == 1_000 and row.get("repetition") == repetition
            )
            row["denied_p99_ns"] = 100
        with self.assertRaisesRegex(MODULE.ValidationError, "release denied_p99_ns regressed"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_classifies_noisy_or_loaded_host_as_inconclusive(self) -> None:
        candidates, baselines = release_receipts()
        for row in candidates[0]:
            if row.get("operation") == "payload_unique_fill" and row.get("entries") == 1_000:
                set_elapsed(row, int(row["elapsed_ns"]) * 2)
        with self.assertRaisesRegex(MODULE.InconclusiveError, "varies beyond run-noise"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        candidates, baselines = release_receipts()
        candidates[0][0]["load_per_cpu_start"] = 2.0
        with self.assertRaisesRegex(MODULE.InconclusiveError, "host load exceeded"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_requires_explicit_policy_and_pressure_outcomes(self) -> None:
        candidates, baselines = release_receipts(["--profile=in-memory-pressure"])
        self.assertEqual(MODULE.validate_release(candidates, baselines, RELEASE_POLICY), 60)
        del baselines[0][1]["denied_p99_ns"]
        with self.assertRaisesRegex(MODULE.ValidationError, "incomplete admission outcome"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        with self.assertRaisesRegex(MODULE.ValidationError, "invalid release throughput regression"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY._replace(
                max_throughput_regression_percent=90 + 10,
            ))

    def test_release_limits_without_release_mode_are_rejected_before_loading(self) -> None:
        options = (
            "--release-max-p99-ns", "--release-max-throughput-regression-percent",
            "--release-max-p99-regression-percent", "--release-p99-noise-floor-ns",
            "--release-max-busy-retries-per-operation", "--release-max-run-noise-percent",
            "--release-max-load-per-cpu",
        )
        for option in options:
            for arguments in ([option, "1"], [option + "=1"]):
                with self.subTest(arguments=arguments), unittest.mock.patch(
                    "sys.argv", ["verify-scaling-results.py", "--input", "missing.jsonl", *arguments],
                ), unittest.mock.patch.object(MODULE, "load_jsonl") as load:
                    output = io.StringIO()
                    with contextlib.redirect_stderr(output):
                        self.assertEqual(MODULE.main(), 1)
                    self.assertIn("release limits require --release", output.getvalue())
                    load.assert_not_called()

    def test_abbreviated_policy_options_cannot_bypass_mode_checks(self) -> None:
        with unittest.mock.patch(
            "sys.argv", ["verify-scaling-results.py", "--release", "--input", "missing.jsonl", "--max-p99=1"],
        ), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as raised:
            MODULE.main()
        self.assertEqual(raised.exception.code, 2)

    def test_release_cli_does_not_default_unapproved_limits(self) -> None:
        output = io.StringIO()
        with unittest.mock.patch("sys.argv", ["verify-scaling-results.py", "--release", "--input", "missing.jsonl"]):
            with contextlib.redirect_stderr(output):
                self.assertEqual(MODULE.main(), 1)
        self.assertIn("requires every explicit release limit", output.getvalue())

    def test_release_cli_reports_green_and_inconclusive_distinctly(self) -> None:
        candidates, baselines = release_receipts(["--profile=in-memory-pressure"])
        paths = {
            **{f"candidate-{index}.jsonl": records for index, records in enumerate(candidates)},
            **{f"baseline-{index}.jsonl": records for index, records in enumerate(baselines)},
        }
        argv = ["verify-scaling-results.py", "--release"]
        for index in range(3):
            argv.extend(("--input", f"candidate-{index}.jsonl"))
            argv.extend(("--baseline", f"baseline-{index}.jsonl"))
        argv.extend((
            "--release-max-p99-ns", "1000000000",
            "--release-max-throughput-regression-percent", "10",
            "--release-max-p99-regression-percent", "20",
            "--release-p99-noise-floor-ns", "10",
            "--release-max-busy-retries-per-operation", "1",
            "--release-max-run-noise-percent", "20",
            "--release-max-load-per-cpu", "1",
        ))
        with unittest.mock.patch("sys.argv", argv), unittest.mock.patch.object(
            MODULE, "load_jsonl", side_effect=lambda path: paths[str(path)],
        ):
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(MODULE.main(), 0)
            candidates[0][0]["load_per_cpu_start"] = 2.0
            output = io.StringIO()
            with contextlib.redirect_stderr(output):
                self.assertEqual(MODULE.main(), 2)
            self.assertIn("INCONCLUSIVE", output.getvalue())

    def test_custom_args_matrix(self) -> None:
        args = ["--sizes=25,250", "--threads=1,4", "--repetitions=3"]
        records = valid_records(args, (25, 250), (1, 4), 3)
        self.assertEqual(MODULE.validate(records), 96)

    def test_runner_rejects_number_spellings_rust_usize_will_not_parse(self) -> None:
        for args in (
            ["--sizes=1, 2"],
            ["--threads=1_000"],
            ["--repetitions=3 "],
            [f"--sizes={2 * sys.maxsize + 2}"],
        ):
            with self.subTest(args=args), self.assertRaises(MODULE.ValidationError):
                MODULE.parse_benchmark_args(args)

    def test_contended_pressure_requires_two_attempts_per_worker(self) -> None:
        with self.assertRaisesRegex(MODULE.ValidationError, "at least two admission attempts"):
            MODULE.parse_benchmark_args([
                "--profile=in-memory-pressure", "--sizes=15", "--threads=1,8",
            ])
        spec = MODULE.parse_benchmark_args([
            "--profile=in-memory-pressure", "--sizes=16", "--threads=1,8",
        ])
        self.assertEqual(spec.sizes, (16,))

    def test_pressure_profile_is_independent_and_fail_closed(self) -> None:
        args = ["--smoke", "--profile=in-memory-pressure"]
        records = valid_records(args)
        self.assertEqual(MODULE.validate(records), 12)
        self.assertEqual(
            MODULE.parse_benchmark_args(list(reversed(args))),
            MODULE.parse_benchmark_args(args),
        )
        missing = copy.deepcopy(records)
        missing.pop(2)
        with self.assertRaisesRegex(MODULE.ValidationError, "matrix is incomplete"):
            MODULE.validate(missing)
        wrong_feature = copy.deepcopy(records)
        wrong_feature[0]["feature_set"] = "all-features"
        with self.assertRaisesRegex(MODULE.ValidationError, "requires feature_set full"):
            MODULE.validate(wrong_feature)
        wrong_outcome = copy.deepcopy(records)
        pressure = next(
            row for row in wrong_outcome if row.get("operation") == "broker_pressure_cycle"
        )
        pressure["denied_operations"] = 0
        with self.assertRaisesRegex(MODULE.ValidationError, "counts disagree"):
            MODULE.validate(wrong_outcome)
        missing_split = copy.deepcopy(records)
        pressure = next(
            row for row in missing_split if row.get("operation") == "broker_pressure_cycle"
        )
        del pressure["denied_p99_ns"]
        with self.assertRaisesRegex(MODULE.ValidationError, "incomplete admission outcome latency"):
            MODULE.validate(missing_split)

    def test_contended_pressure_reclaim_rows_and_metrics_are_required(self) -> None:
        args = ["--smoke", "--profile=in-memory-pressure"]
        records = valid_records(args)
        missing = [row for row in records if row.get("operation") != "contended_pressure_reclaim"]
        with self.assertRaisesRegex(MODULE.ValidationError, "matrix is incomplete"):
            MODULE.validate(missing)
        for field, value, message in (
            ("reclaim_p99_ns", None, "phase latency fields"),
            ("reclaim_p95_ns", 40, "reclaim latency distribution"),
            ("reclaimed_operations", 0, "reclaimed operation count"),
            ("semantic_peak_bytes", 95, "96-byte hard cap"),
            ("denied_operations", 0, "counts disagree"),
        ):
            with self.subTest(field=field):
                malformed = valid_records(args)
                row = next(row for row in malformed if row.get("operation") == "contended_pressure_reclaim")
                if value is None:
                    del row[field]
                else:
                    row[field] = value
                with self.assertRaisesRegex(MODULE.ValidationError, message):
                    MODULE.validate(malformed)

    def test_revocable_fill_and_reclaim_tails_are_independently_validated(self) -> None:
        for field, value, message in (
            ("fill_p99_ns", None, "phase latency fields"),
            ("fill_p95_ns", 25, "fill latency distribution"),
            ("reclaim_p99_ns", 1_000_000_001, "reclaim latency distribution"),
        ):
            with self.subTest(field=field):
                records = valid_records(["--smoke"])
                row = next(row for row in records if row.get("operation") == "revocable_fill_and_drain")
                if value is None:
                    del row[field]
                else:
                    row[field] = value
                with self.assertRaisesRegex(MODULE.ValidationError, message):
                    MODULE.validate(records)

    def test_release_rejects_reclaim_tail_regression_even_if_aggregate_is_stable(self) -> None:
        candidates, baselines = release_receipts(["--profile=in-memory-pressure"])
        for records in candidates:
            for row in records:
                if row.get("operation") == "contended_pressure_reclaim":
                    row["reclaim_p99_ns"] = 200
        with self.assertRaisesRegex(MODULE.ValidationError, "release reclaim_p99_ns regressed"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_rejects_denial_as_speed_even_if_total_throughput_is_stable(self) -> None:
        candidates, baselines = release_receipts(["--profile=in-memory-pressure"])
        for records in candidates:
            for row in records:
                if row.get("operation") == "contended_pressure_reclaim":
                    row["successful_operations"] = row["entries"] // 10
                    row["denied_operations"] = row["entries"] - row["successful_operations"]
                    row["reclaimed_operations"] = row["successful_operations"]
                    row["successful_throughput_ops_per_sec"] = round(
                        int(row["successful_operations"]) * 1_000_000_000 / int(row["elapsed_ns"])
                    )
        with self.assertRaisesRegex(MODULE.ValidationError, "release successful throughput regressed"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_default_metadata_with_smoke_only_rows_is_rejected(self) -> None:
        records = valid_records([], (1_000,), (1, 2), 3)
        with self.assertRaisesRegex(MODULE.ValidationError, "matrix is incomplete"):
            MODULE.validate(records)

    def test_historical_51_row_false_green_is_rejected(self) -> None:
        full = valid_records()
        only_1k = [record for record in full[1:-1] if record["entries"] == 1_000][:51]
        assert len(only_1k) == 51
        with self.assertRaisesRegex(MODULE.ValidationError, "matrix is incomplete"):
            MODULE.validate([full[0], *only_1k, full[-1]])

    def test_missing_repetition_and_extra_cell_are_rejected(self) -> None:
        records = valid_records(["--smoke"])
        records.pop(2)
        with self.assertRaisesRegex(MODULE.ValidationError, "matrix is incomplete"):
            MODULE.validate(records)
        records = valid_records(["--smoke"])
        records.insert(-1, sample("unexpected", "single-thread", 1_000, 1, 0))
        with self.assertRaisesRegex(MODULE.ValidationError, "extra cells"):
            MODULE.validate(records)

    def test_duplicate_cell_and_repetition_are_rejected(self) -> None:
        records = valid_records(["--smoke"])
        records.insert(-1, copy.deepcopy(records[1]))
        with self.assertRaisesRegex(MODULE.ValidationError, "duplicate benchmark cell"):
            MODULE.validate(records)

    def test_missing_pairwise_or_point_read_is_rejected(self) -> None:
        for operation in ("broker_release_pairwise", "retained_map_point_read"):
            records = [r for r in valid_records(["--smoke"]) if r.get("operation") != operation]
            with self.assertRaisesRegex(MODULE.ValidationError, "matrix is incomplete"):
                MODULE.validate(records)

    def test_quadratic_revocable_growth_is_rejected(self) -> None:
        records = valid_records()
        for record in records:
            if record.get("operation") == "revocable_fill_and_drain" and record.get("entries") == 10_000:
                record["p50_ns"] = 100
                record["p95_ns"] = 200
                record["p99_ns"] = 300
        with self.assertRaisesRegex(MODULE.ValidationError, "revocable_fill_and_drain growth"):
            MODULE.validate(records)

    def test_wall_clock_gap_does_not_masquerade_as_operation_growth(self) -> None:
        records = valid_records()
        for record in records:
            if record.get("operation") == "revocable_fill_and_drain" and record.get("entries") == 10_000:
                set_elapsed(record, 100_000)
        self.assertEqual(MODULE.validate(records), 220)

    def test_accounting_and_operation_counts_are_rejected(self) -> None:
        for field, value, message in (
            ("allocator_live_bytes", 1, "allocator drift"),
            ("semantic_after_close_bytes", 1, "did not drain"),
            ("timed_operations", 0, "counts disagree"),
            ("busy_retries", -1, "busy retry"),
            ("successful_throughput_ops_per_sec", 0, "successful throughput"),
        ):
            with self.subTest(field=field):
                records = valid_records(["--smoke"])
                records[1][field] = value
                with self.assertRaisesRegex(MODULE.ValidationError, message):
                    MODULE.validate(records)

    def test_allocator_telemetry_cannot_silently_disappear(self) -> None:
        records = valid_records(["--smoke"])
        payload = next(record for record in records if record.get("operation") == "payload_unique_fill")
        for field in (
            "allocator_allocated_bytes", "allocator_deallocated_bytes",
            "allocator_high_water_bytes",
        ):
            payload[field] = 0
        with self.assertRaisesRegex(MODULE.ValidationError, "missing allocator telemetry"):
            MODULE.validate(records)

        records = valid_records(["--smoke"])
        records[1]["allocator_high_water_bytes"] = 0
        with self.assertRaisesRegex(MODULE.ValidationError, "allocator high-water"):
            MODULE.validate(records)

        # Scheduler admission deliberately starts allocation tracking after
        # fixture construction and may have an allocation-free measured loop.
        records = valid_records(["--smoke"])
        scheduler = next(
            record for record in records
            if record.get("operation") == "scheduler_admission_group_cardinality"
        )
        for field in (
            "allocator_allocated_bytes", "allocator_deallocated_bytes",
            "allocator_high_water_bytes",
        ):
            scheduler[field] = 0
        self.assertEqual(MODULE.validate(records), 48)

    def test_falsified_outcome_counts_are_rejected(self) -> None:
        records = valid_records(["--smoke"])
        scheduler = next(
            record for record in records
            if record.get("operation") == "scheduler_admission_group_cardinality"
        )
        scheduler["successful_operations"] = scheduler["timed_operations"]
        scheduler["denied_operations"] = 0
        with self.assertRaisesRegex(MODULE.ValidationError, "outcome counts"):
            MODULE.validate(records)

        records = valid_records(["--smoke"])
        payload = next(record for record in records if record.get("operation") == "payload_unique_fill")
        payload["successful_operations"] = 0
        payload["denied_operations"] = payload["timed_operations"]
        with self.assertRaisesRegex(MODULE.ValidationError, "outcome counts"):
            MODULE.validate(records)

    def test_semantic_peak_matches_workload_role(self) -> None:
        records = valid_records(["--smoke"])
        records[1]["semantic_peak_bytes"] = 0
        with self.assertRaisesRegex(MODULE.ValidationError, "retained workload has no semantic peak"):
            MODULE.validate(records)
        records = valid_records(["--smoke"])
        scheduler = next(
            record for record in records
            if record.get("operation") == "scheduler_admission_group_cardinality"
        )
        scheduler["semantic_peak_bytes"] = 1
        with self.assertRaisesRegex(MODULE.ValidationError, "scheduler admission must not retain"):
            MODULE.validate(records)

    def test_latency_limit_is_rejected(self) -> None:
        records = valid_records(["--smoke"])
        records[1]["p99_ns"] = 1_000_000_001
        with self.assertRaisesRegex(MODULE.ValidationError, "latency distribution"):
            MODULE.validate(records)

    def test_scheduler_outcome_latency_is_required_and_bounded(self) -> None:
        for field, value, message in (
            ("denied_p99_ns", None, "incomplete admission outcome latency"),
            ("admitted_p95_ns", 25, "admitted latency distribution"),
            ("denied_p99_ns", 1_000_000_001, "denied latency distribution"),
            ("admitted_p50_ns", True, "must be an integer"),
        ):
            with self.subTest(field=field, value=value):
                records = valid_records(["--smoke"])
                scheduler = next(
                    record for record in records
                    if record.get("operation") == "scheduler_admission_group_cardinality"
                )
                if value is None:
                    del scheduler[field]
                else:
                    scheduler[field] = value
                with self.assertRaisesRegex(MODULE.ValidationError, message):
                    MODULE.validate(records)

    def test_v4_baseline_requires_outcome_split(self) -> None:
        baseline = valid_records(["--smoke"])
        current = valid_records(["--smoke"])
        for record in baseline:
            if record.get("operation") == "scheduler_admission_group_cardinality":
                for fields in MODULE.OUTCOME_LATENCY_FIELDS.values():
                    for field in fields:
                        del record[field]
        with self.assertRaisesRegex(MODULE.ValidationError, "incomplete admission outcome latency"):
            MODULE.validate(current, baseline)

    def test_partial_baseline_outcome_split_is_rejected(self) -> None:
        baseline = valid_records(["--smoke"])
        scheduler = next(
            record for record in baseline
            if record.get("operation") == "scheduler_admission_group_cardinality"
        )
        for fields in MODULE.OUTCOME_LATENCY_FIELDS.values():
            for field in fields:
                del scheduler[field]
        with self.assertRaisesRegex(MODULE.ValidationError, "incomplete admission outcome latency"):
            MODULE.validate(valid_records(["--smoke"]), baseline)

    def test_v3_receipt_is_not_misread_as_v4(self) -> None:
        records = valid_records(["--smoke"])
        records[0]["schema"] = "qmp-scaling-run-v3"
        with self.assertRaisesRegex(MODULE.ValidationError, "benchmark run header"):
            MODULE.validate(records)

    def test_outcome_specific_p99_regressions_are_rejected(self) -> None:
        for field, label in (("admitted_p99_ns", "admitted"), ("denied_p99_ns", "denied")):
            with self.subTest(field=field):
                current = valid_records(["--smoke"])
                for record in current:
                    if record.get("operation") == "scheduler_admission_group_cardinality":
                        record[field] = 2_000_000
                with self.assertRaisesRegex(MODULE.ValidationError, f"{label} p99 regressed"):
                    MODULE.validate(current, valid_records(["--smoke"]))

    def test_throughput_denominator_mismatch_is_rejected(self) -> None:
        records = valid_records(["--smoke"])
        records[1]["throughput_ops_per_sec"] = 1
        with self.assertRaisesRegex(MODULE.ValidationError, "throughput does not match"):
            MODULE.validate(records)

    def test_source_change_and_missing_end_are_rejected(self) -> None:
        records = valid_records(["--smoke"])
        records[-1]["source_tree"] = "c" * 40
        with self.assertRaisesRegex(MODULE.ValidationError, "source changed"):
            MODULE.validate(records)
        records = valid_records(["--smoke"])[:-1]
        with self.assertRaisesRegex(MODULE.ValidationError, "completion footer last"):
            MODULE.validate(records)

    def test_footer_order_and_feature_graph_are_fail_closed(self) -> None:
        records = valid_records(["--smoke"])
        records[-1], records[-2] = records[-2], records[-1]
        with self.assertRaisesRegex(MODULE.ValidationError, "completion footer last"):
            MODULE.validate(records)
        current = valid_records(["--smoke"])
        baseline = valid_records(["--smoke"])
        baseline[0]["feature_graph_sha256"] = "0" * 64
        with self.assertRaisesRegex(MODULE.ValidationError, "baseline incompatible feature_graph_sha256"):
            MODULE.validate(current, baseline)

    def test_invalid_args_are_rejected(self) -> None:
        for args in (["--sizes=0"], ["--threads=1,1"], ["--repetitions=2"], ["--unknown"]):
            with self.subTest(args=args):
                with self.assertRaises(MODULE.ValidationError):
                    MODULE.parse_benchmark_args(args)

    def test_baseline_incompatible_metadata_and_matrix_are_rejected(self) -> None:
        current = valid_records(["--smoke"])
        for field in ("machine", "rustc", "cargo", "feature_set", "bench_target"):
            with self.subTest(field=field):
                baseline = valid_records(["--smoke"])
                baseline[0][field] = "incompatible"
                message = (
                    "requires feature_set all-features" if field == "feature_set"
                    else f"baseline incompatible {field}"
                )
                with self.assertRaisesRegex(MODULE.ValidationError, message):
                    MODULE.validate(current, baseline)
        with self.assertRaisesRegex(MODULE.ValidationError, "baseline benchmark matrix differs"):
            MODULE.validate(current, valid_records())

    def test_baseline_source_difference_is_allowed(self) -> None:
        current = valid_records(["--smoke"])
        baseline = valid_records(["--smoke"])
        baseline[0]["source_sha"] = baseline[-1]["source_sha"] = "c" * 40
        baseline[0]["source_tree"] = baseline[-1]["source_tree"] = "d" * 40
        self.assertEqual(MODULE.validate(current, baseline), 48)

    def test_baseline_requires_same_benchmark_implementation(self) -> None:
        current = valid_records(["--smoke"])
        baseline = valid_records(["--smoke"])
        baseline[0]["benchmark_source_sha256"] = "8" * 64
        with self.assertRaisesRegex(MODULE.ValidationError, "baseline incompatible benchmark_source_sha256"):
            MODULE.validate(current, baseline)
        del baseline[0]["benchmark_source_sha256"]
        with self.assertRaisesRegex(MODULE.ValidationError, "benchmark_source_sha256.*missing or invalid"):
            MODULE.validate(current, baseline)
        baseline[0]["benchmark_source_sha256"] = "not-a-digest"
        with self.assertRaisesRegex(MODULE.ValidationError, "benchmark_source_sha256.*invalid"):
            MODULE.validate(current, baseline)

    def test_baseline_requires_same_build_configuration(self) -> None:
        current = valid_records(["--smoke"])
        baseline = valid_records(["--smoke"])
        baseline[0]["bench_build_config_sha256"] = baseline[-1]["bench_build_config_sha256"] = "6" * 64
        with self.assertRaisesRegex(MODULE.ValidationError, "baseline incompatible bench_build_config_sha256"):
            MODULE.validate(current, baseline)
        del baseline[0]["bench_build_config_sha256"]
        del baseline[-1]["bench_build_config_sha256"]
        with self.assertRaisesRegex(MODULE.ValidationError, "bench_build_config_sha256.*missing or invalid"):
            MODULE.validate(current, baseline)

    def test_baseline_regressions_are_rejected(self) -> None:
        baseline = valid_records(["--smoke"])
        current = valid_records(["--smoke"])
        for record in baseline + current:
            if record.get("schema") == MODULE.SAMPLE_SCHEMA:
                set_elapsed(record, 20_000_000)
        for record in current:
            if record.get("schema") == MODULE.SAMPLE_SCHEMA:
                set_elapsed(record, 2_000_000_000)
        with self.assertRaisesRegex(MODULE.ValidationError, "throughput regressed"):
            MODULE.validate(current, baseline)
        current = valid_records(["--smoke"])
        for record in current:
            if record.get("schema") == MODULE.SAMPLE_SCHEMA:
                record["p99_ns"] = 2_000_000
        with self.assertRaisesRegex(MODULE.ValidationError, "p99 regressed"):
            MODULE.validate(current, valid_records(["--smoke"]))

    def test_truncated_jsonl_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.jsonl"
            path.write_text('{"schema":"qmp-scaling-run-v4"}', encoding="utf-8")
            with self.assertRaisesRegex(MODULE.ValidationError, "truncated final line"):
                MODULE.load_jsonl(path)
            path.write_text('{"schema":"qmp-scaling-run-v4"\n', encoding="utf-8")
            with self.assertRaisesRegex(MODULE.ValidationError, "invalid JSON"):
                MODULE.load_jsonl(path)
            path.write_bytes(b"\xff\n")
            with self.assertRaisesRegex(MODULE.ValidationError, "invalid UTF-8"):
                MODULE.load_jsonl(path)

    def test_duplicate_json_keys_cannot_rewrite_release_evidence(self) -> None:
        records = valid_records(["--smoke"])
        lines = [json.dumps(record, separators=(",", ":")) for record in records]
        conflicts = (
            (0, '"source_sha":"' + "a" * 40 + '"',
             '"source_sha":"' + "0" * 40 + '","source_sha":"' + "a" * 40 + '"'),
            (1, '"successful_operations":1000',
             '"successful_operations":0,"successful_operations":1000'),
            (-1, '"bench_exit_code":0', '"bench_exit_code":1,"bench_exit_code":0'),
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "result.jsonl"
            for index, old, new in conflicts:
                with self.subTest(index=index):
                    conflicted = lines.copy()
                    self.assertIn(old, conflicted[index])
                    conflicted[index] = conflicted[index].replace(old, new, 1)
                    path.write_text("\n".join(conflicted) + "\n", encoding="utf-8")
                    with self.assertRaisesRegex(MODULE.ValidationError, "duplicate JSON key"):
                        MODULE.validate(MODULE.load_jsonl(path))
            path.write_text('{"schema":"sample","detail":{"bytes":0,"bytes":1}}\n', encoding="utf-8")
            with self.assertRaisesRegex(MODULE.ValidationError, "duplicate JSON key"):
                MODULE.load_jsonl(path)


if __name__ == "__main__":
    unittest.main()
