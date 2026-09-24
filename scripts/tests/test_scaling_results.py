from __future__ import annotations

import copy
import contextlib
import io
import importlib.util
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


def sample(
    operation: str, mode: str, entries: int, threads: int, repetition: int,
) -> dict[str, object]:
    timed = entries * (2 if operation in {
        "revocable_fill_and_drain", "scheduler_admission_group_cardinality",
        "broker_pressure_cycle",
    } else 1)
    successful = entries if operation in {
        "scheduler_admission_group_cardinality", "broker_pressure_cycle",
    } else timed
    result = {
        "schema": MODULE.SAMPLE_SCHEMA,
        "operation": operation,
        "mode": mode,
        "entries": entries,
        "threads": threads,
        "repetition": repetition,
        "elapsed_ns": entries,
        "throughput_ops_per_sec": round(timed * 1_000_000_000 / entries),
        "timed_operations": timed,
        "successful_operations": successful,
        "denied_operations": timed - successful,
        "p50_ns": 10,
        "p95_ns": 20,
        "p99_ns": 30,
        "semantic_peak_bytes": (
            0 if operation == "scheduler_admission_group_cardinality"
            else 1 if operation == "broker_pressure_cycle"
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
    if operation in {"scheduler_admission_group_cardinality", "broker_pressure_cycle"}:
        result.update({
            "admitted_p50_ns": 8, "admitted_p95_ns": 16, "admitted_p99_ns": 24,
            "denied_p50_ns": 12, "denied_p95_ns": 22, "denied_p99_ns": 32,
        })
    return result


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
        "package": "quanta-memory-platform-facade",
        "bench_target": "scaling",
        "benchmark_args": benchmark_args,
    }]
    for entries in sizes:
        for repetition in range(repetitions):
            if pressure:
                records.append(sample("broker_pressure_cycle", "single-thread", entries, 1, repetition))
                records.extend(
                    sample("dual_store_write_contention", "shared-broker", entries, worker_count, repetition)
                    for worker_count in threads
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
        "bench_exit_code": 0,
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
        self.assertEqual(MODULE.validate(valid_records(["--profile=in-memory-pressure"])), 40)
        self.assertEqual(
            MODULE.validate(valid_records(["--profile=in-memory-pressure", "--smoke"])), 9,
        )

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
                    row["elapsed_ns"] *= 2
                    row["throughput_ops_per_sec"] = round(
                        int(row["timed_operations"]) * 1_000_000_000 / int(row["elapsed_ns"])
                    )
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

    def test_release_classifies_noisy_or_loaded_host_as_inconclusive(self) -> None:
        candidates, baselines = release_receipts()
        for row in candidates[0]:
            if row.get("operation") == "payload_unique_fill" and row.get("entries") == 1_000:
                row["elapsed_ns"] *= 2
                row["throughput_ops_per_sec"] = round(
                    int(row["timed_operations"]) * 1_000_000_000 / int(row["elapsed_ns"])
                )
        with self.assertRaisesRegex(MODULE.InconclusiveError, "varies beyond run-noise"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        candidates, baselines = release_receipts()
        candidates[0][0]["load_per_cpu_start"] = 2.0
        with self.assertRaisesRegex(MODULE.InconclusiveError, "host load exceeded"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)

    def test_release_requires_explicit_policy_and_pressure_outcomes(self) -> None:
        candidates, baselines = release_receipts(["--profile=in-memory-pressure"])
        self.assertEqual(MODULE.validate_release(candidates, baselines, RELEASE_POLICY), 40)
        del baselines[0][1]["denied_p99_ns"]
        with self.assertRaisesRegex(MODULE.ValidationError, "incomplete admission outcome"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY)
        with self.assertRaisesRegex(MODULE.ValidationError, "invalid release throughput regression"):
            MODULE.validate_release(candidates, baselines, RELEASE_POLICY._replace(
                max_throughput_regression_percent=90 + 10,
            ))

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

    def test_pressure_profile_is_independent_and_fail_closed(self) -> None:
        args = ["--smoke", "--profile=in-memory-pressure"]
        records = valid_records(args)
        self.assertEqual(MODULE.validate(records), 9)
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
                record["elapsed_ns"] = 100_000
                record["throughput_ops_per_sec"] = round(
                    int(record["timed_operations"]) * 1_000_000_000 / 100_000
                )
        self.assertEqual(MODULE.validate(records), 220)

    def test_accounting_and_operation_counts_are_rejected(self) -> None:
        for field, value, message in (
            ("allocator_live_bytes", 1, "allocator drift"),
            ("semantic_after_close_bytes", 1, "did not drain"),
            ("timed_operations", 0, "counts disagree"),
            ("busy_retries", -1, "busy retry"),
        ):
            with self.subTest(field=field):
                records = valid_records(["--smoke"])
                records[1][field] = value
                with self.assertRaisesRegex(MODULE.ValidationError, message):
                    MODULE.validate(records)

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

    def test_legacy_baseline_compares_aggregate_only(self) -> None:
        baseline = valid_records(["--smoke"])
        current = valid_records(["--smoke"])
        for record in baseline:
            if record.get("operation") == "scheduler_admission_group_cardinality":
                for fields in MODULE.OUTCOME_LATENCY_FIELDS.values():
                    for field in fields:
                        del record[field]
        self.assertEqual(MODULE.validate(current, baseline), 48)
        with self.assertRaisesRegex(MODULE.ValidationError, "incomplete admission outcome latency"):
            MODULE.validate(baseline)

    def test_mixed_baseline_outcome_generations_are_rejected(self) -> None:
        baseline = valid_records(["--smoke"])
        scheduler = next(
            record for record in baseline
            if record.get("operation") == "scheduler_admission_group_cardinality"
        )
        for fields in MODULE.OUTCOME_LATENCY_FIELDS.values():
            for field in fields:
                del scheduler[field]
        with self.assertRaisesRegex(MODULE.ValidationError, "mixed scheduler_admission_group_cardinality outcome latency generations"):
            MODULE.validate(valid_records(["--smoke"]), baseline)

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

    def test_baseline_regressions_are_rejected(self) -> None:
        baseline = valid_records(["--smoke"])
        current = valid_records(["--smoke"])
        for record in baseline + current:
            if record.get("schema") == MODULE.SAMPLE_SCHEMA:
                record["elapsed_ns"] = 20_000_000
                record["throughput_ops_per_sec"] = round(
                    int(record["timed_operations"]) * 1_000_000_000 / 20_000_000
                )
        for record in current:
            if record.get("schema") == MODULE.SAMPLE_SCHEMA:
                record["elapsed_ns"] = 2_000_000_000
                record["throughput_ops_per_sec"] = round(
                    int(record["timed_operations"]) * 1_000_000_000 / 2_000_000_000
                )
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
            path.write_text('{"schema":"qmp-scaling-run-v3"}', encoding="utf-8")
            with self.assertRaisesRegex(MODULE.ValidationError, "truncated final line"):
                MODULE.load_jsonl(path)
            path.write_text('{"schema":"qmp-scaling-run-v3"\n', encoding="utf-8")
            with self.assertRaisesRegex(MODULE.ValidationError, "invalid JSON"):
                MODULE.load_jsonl(path)
            path.write_bytes(b"\xff\n")
            with self.assertRaisesRegex(MODULE.ValidationError, "invalid UTF-8"):
                MODULE.load_jsonl(path)


if __name__ == "__main__":
    unittest.main()
