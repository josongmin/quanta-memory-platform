from __future__ import annotations

import copy
import importlib.util
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
    } else 1)
    return {
        "schema": MODULE.SAMPLE_SCHEMA,
        "operation": operation,
        "mode": mode,
        "entries": entries,
        "threads": threads,
        "repetition": repetition,
        "elapsed_ns": entries,
        "throughput_ops_per_sec": round(timed * 1_000_000_000 / entries),
        "timed_operations": timed,
        "successful_operations": timed,
        "denied_operations": 0,
        "p50_ns": 10,
        "p95_ns": 20,
        "p99_ns": 30,
        "semantic_peak_bytes": 0 if operation == "scheduler_admission_group_cardinality" else 100,
        "semantic_after_close_bytes": 0,
        "allocator_allocated_bytes": 200,
        "allocator_deallocated_bytes": 200,
        "allocator_live_bytes": 0,
        "allocator_high_water_bytes": 200,
        "allocator_underflowed": False,
        "busy_retries": 0,
    }


def valid_records(
    benchmark_args: list[str] | None = None,
    sizes: tuple[int, ...] | None = None,
    threads: tuple[int, ...] | None = None,
    repetitions: int | None = None,
) -> list[dict[str, object]]:
    benchmark_args = list(benchmark_args or [])
    if sizes is None:
        sizes = (1_000,) if "--smoke" in benchmark_args else (1_000, 10_000)
    if threads is None:
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
        "feature_set": "all-features",
        "feature_graph_sha256": "f" * 64,
        "dependency_lock_sha256": "e" * 64,
        "package": "quanta-memory-platform-facade",
        "bench_target": "scaling",
        "benchmark_args": benchmark_args,
    }]
    for entries in sizes:
        for repetition in range(repetitions):
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
    })
    return records


class ScalingResultTests(unittest.TestCase):
    def test_complete_default_and_smoke_matrix(self) -> None:
        self.assertEqual(MODULE.validate(valid_records()), 220)
        self.assertEqual(MODULE.validate(valid_records(["--smoke"])), 48)

    def test_custom_args_matrix(self) -> None:
        args = ["--sizes=25,250", "--threads=1,4", "--repetitions=3"]
        records = valid_records(args, (25, 250), (1, 4), 3)
        self.assertEqual(MODULE.validate(records), 96)

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
                record["elapsed_ns"] = 100_000
                record["throughput_ops_per_sec"] = round(
                    int(record["timed_operations"]) * 1_000_000_000 / 100_000
                )
        with self.assertRaisesRegex(MODULE.ValidationError, "revocable_fill_and_drain growth"):
            MODULE.validate(records)

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
                with self.assertRaisesRegex(MODULE.ValidationError, f"baseline incompatible {field}"):
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


if __name__ == "__main__":
    unittest.main()
