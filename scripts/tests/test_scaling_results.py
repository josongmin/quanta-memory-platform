import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "verify-scaling-results.py"
SPEC = importlib.util.spec_from_file_location("verify_scaling_results", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def sample(mode: str, threads: int, repetition: int) -> dict[str, object]:
    return {
        "schema": MODULE.SAMPLE_SCHEMA,
        "operation": "payload_contention",
        "mode": mode,
        "entries": 1000,
        "threads": threads,
        "repetition": repetition,
        "elapsed_ns": 1000,
        "throughput_ops_per_sec": 1000,
        "p50_ns": 10,
        "p95_ns": 20,
        "p99_ns": 30,
        "semantic_peak_bytes": 100,
        "semantic_after_close_bytes": 0,
        "allocator_allocated_bytes": 200,
        "allocator_deallocated_bytes": 200,
        "allocator_live_bytes": 0,
        "allocator_high_water_bytes": 200,
        "busy_retries": 0,
    }


def valid_records() -> list[dict[str, object]]:
    records: list[dict[str, object]] = [
        {
            "schema": MODULE.RUN_SCHEMA,
            "source_sha": "a" * 40,
            "source_tree": "b" * 40,
            "rustc": "rustc test",
            "cargo": "cargo test",
            "machine": "test-machine",
        }
    ]
    for operation in (
        "payload_unique_fill",
        "payload_dedup_lookup",
        "revocable_fill_and_drain",
        "broker_release_owner_cardinality",
    ):
        for repetition in range(3):
            value = sample("single-thread", 1, repetition)
            value["operation"] = operation
            records.append(value)
    for mode in MODULE.MODES:
        for threads in (1, 2):
            records.extend(sample(mode, threads, repetition) for repetition in range(3))
    return records


class ScalingResultTests(unittest.TestCase):
    def test_valid_smoke_matrix_passes(self) -> None:
        self.assertEqual(MODULE.validate(valid_records(), {1, 2}), 30)

    def test_allocator_drift_is_rejected(self) -> None:
        records = valid_records()
        records[-1]["allocator_live_bytes"] = 1
        with self.assertRaisesRegex(MODULE.ValidationError, "allocator drift"):
            MODULE.validate(records, {1, 2})

    def test_intentionally_regressed_p99_is_rejected(self) -> None:
        records = valid_records()
        records[-1]["p99_ns"] = 1_000_000_001
        with self.assertRaisesRegex(MODULE.ValidationError, "latency distribution"):
            MODULE.validate(records, {1, 2})

    def test_missing_contention_cell_is_rejected(self) -> None:
        records = [
            record
            for record in valid_records()
            if not (
                record.get("operation") == "payload_contention"
                and record.get("mode") == "mixed"
                and record.get("threads") == 2
            )
        ]
        with self.assertRaisesRegex(MODULE.ValidationError, "matrix is incomplete"):
            MODULE.validate(records, {1, 2})

    def test_baseline_regression_is_rejected(self) -> None:
        baseline = valid_records()
        current = valid_records()
        for record in current:
            if record.get("schema") == MODULE.SAMPLE_SCHEMA:
                record["throughput_ops_per_sec"] = 400
        with self.assertRaisesRegex(MODULE.ValidationError, "throughput regressed"):
            MODULE.validate(
                current,
                {1, 2},
                baseline,
                max_throughput_regression_percent=50,
            )


if __name__ == "__main__":
    unittest.main()
