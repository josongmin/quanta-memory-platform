#!/usr/bin/env python3
"""Fail-closed validator for QMP scaling benchmark JSONL output."""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path


SAMPLE_SCHEMA = "qmp-scaling-sample-v2"
RUN_SCHEMA = "qmp-scaling-run-v2"
MODES = {"read-heavy", "write-heavy", "mixed"}


class ValidationError(RuntimeError):
    pass


def load_jsonl(path: Path) -> list[dict[str, object]]:
    records: list[dict[str, object]] = []
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        raw = raw.strip()
        if not raw.startswith("{"):
            continue
        try:
            value = json.loads(raw)
        except json.JSONDecodeError as error:
            raise ValidationError(f"line {line_number}: invalid JSON: {error}") from error
        if isinstance(value, dict):
            records.append(value)
    return records


def integer(sample: dict[str, object], field: str) -> int:
    value = sample.get(field)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValidationError(f"sample field {field!r} must be an integer")
    return value


def scenario_key(sample: dict[str, object]) -> tuple[object, ...]:
    return (
        sample.get("operation"),
        sample.get("mode"),
        integer(sample, "entries"),
        integer(sample, "threads"),
    )


def validate(
    records: list[dict[str, object]],
    expected_threads: set[int],
    baseline_records: list[dict[str, object]] | None = None,
    max_p99_ns: int = 1_000_000_000,
    max_throughput_regression_percent: int = 50,
    max_p99_regression_percent: int = 200,
    p99_noise_floor_ns: int = 1_000_000,
    baseline_min_elapsed_ns: int = 10_000_000,
) -> int:
    runs = [record for record in records if record.get("schema") == RUN_SCHEMA]
    if len(runs) != 1:
        raise ValidationError(f"expected exactly one {RUN_SCHEMA} record, found {len(runs)}")
    for field in ("source_sha", "source_tree", "rustc", "cargo", "machine"):
        if not isinstance(runs[0].get(field), str) or not runs[0][field]:
            raise ValidationError(f"run metadata field {field!r} is missing")

    samples = [record for record in records if record.get("schema") == SAMPLE_SCHEMA]
    if not samples:
        raise ValidationError("no scaling samples found")

    grouped: dict[tuple[object, ...], list[dict[str, object]]] = defaultdict(list)
    for sample in samples:
        key = scenario_key(sample)
        grouped[key].append(sample)
        p50 = integer(sample, "p50_ns")
        p95 = integer(sample, "p95_ns")
        p99 = integer(sample, "p99_ns")
        if not (0 <= p50 <= p95 <= p99 <= max_p99_ns):
            raise ValidationError(f"{key}: invalid/over-limit latency distribution {p50}/{p95}/{p99}")
        if integer(sample, "throughput_ops_per_sec") <= 0:
            raise ValidationError(f"{key}: throughput must be positive")
        if integer(sample, "semantic_after_close_bytes") != 0:
            raise ValidationError(f"{key}: semantic accounting did not drain to zero")
        allocated = integer(sample, "allocator_allocated_bytes")
        deallocated = integer(sample, "allocator_deallocated_bytes")
        live = integer(sample, "allocator_live_bytes")
        if allocated != deallocated or live != 0:
            raise ValidationError(
                f"{key}: allocator drift allocated={allocated} deallocated={deallocated} live={live}"
            )
        if integer(sample, "allocator_high_water_bytes") < live:
            raise ValidationError(f"{key}: allocator high-water is below live bytes")
        if integer(sample, "busy_retries") < 0:
            raise ValidationError(f"{key}: busy retry count is negative")

    required_operations = {
        "payload_unique_fill",
        "payload_dedup_lookup",
        "revocable_fill_and_drain",
        "broker_release_owner_cardinality",
    }
    observed_operations = {str(sample.get("operation")) for sample in samples}
    missing_operations = required_operations - observed_operations
    if missing_operations:
        raise ValidationError(f"missing operations: {sorted(missing_operations)}")

    for key, scenario_samples in grouped.items():
        if len(scenario_samples) < 3:
            raise ValidationError(f"{key}: fewer than three repetitions")
        repetitions = {integer(sample, "repetition") for sample in scenario_samples}
        if len(repetitions) != len(scenario_samples):
            raise ValidationError(f"{key}: duplicate repetition identifiers")

    contention = [sample for sample in samples if sample.get("operation") == "payload_contention"]
    observed_matrix = {
        (str(sample.get("mode")), integer(sample, "threads")) for sample in contention
    }
    required_matrix = {(mode, threads) for mode in MODES for threads in expected_threads}
    if missing := required_matrix - observed_matrix:
        raise ValidationError(f"contention matrix is incomplete: {sorted(missing)}")

    if baseline_records is not None:
        baselines = [
            record for record in baseline_records if record.get("schema") == SAMPLE_SCHEMA
        ]
        baseline_groups: dict[tuple[object, ...], list[dict[str, object]]] = defaultdict(list)
        for sample in baselines:
            baseline_groups[scenario_key(sample)].append(sample)
        throughput_allowed = max_throughput_regression_percent / 100.0
        p99_allowed = max_p99_regression_percent / 100.0
        for key, current in grouped.items():
            baseline = baseline_groups.get(key)
            if not baseline:
                raise ValidationError(f"{key}: missing from supplied baseline")
            current_throughput = statistics.median(
                integer(sample, "throughput_ops_per_sec") for sample in current
            )
            baseline_throughput = statistics.median(
                integer(sample, "throughput_ops_per_sec") for sample in baseline
            )
            baseline_elapsed = statistics.median(
                integer(sample, "elapsed_ns") for sample in baseline
            )
            if (
                baseline_elapsed >= baseline_min_elapsed_ns
                and current_throughput < baseline_throughput * (1.0 - throughput_allowed)
            ):
                raise ValidationError(
                    f"{key}: throughput regressed beyond {max_throughput_regression_percent}%"
                )
            current_p99 = statistics.median(integer(sample, "p99_ns") for sample in current)
            baseline_p99 = statistics.median(integer(sample, "p99_ns") for sample in baseline)
            p99_limit = max(
                baseline_p99 * (1.0 + p99_allowed),
                baseline_p99 + p99_noise_floor_ns,
            )
            if current_p99 > p99_limit:
                raise ValidationError(
                    f"{key}: p99 regressed beyond {max_p99_regression_percent}%"
                )
    return len(samples)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--max-p99-ns", type=int, default=1_000_000_000)
    parser.add_argument("--max-throughput-regression-percent", type=int, default=50)
    parser.add_argument("--max-p99-regression-percent", type=int, default=200)
    parser.add_argument("--p99-noise-floor-ns", type=int, default=1_000_000)
    parser.add_argument("--baseline-min-elapsed-ns", type=int, default=10_000_000)
    args = parser.parse_args()
    try:
        records = load_jsonl(args.input)
        baseline = load_jsonl(args.baseline) if args.baseline else None
        sample_count = validate(
            records,
            {1, 2} if args.smoke else {1, 2, 8, 32},
            baseline,
            args.max_p99_ns,
            args.max_throughput_regression_percent,
            args.max_p99_regression_percent,
            args.p99_noise_floor_ns,
            args.baseline_min_elapsed_ns,
        )
    except (OSError, ValidationError) as error:
        print(f"verify-scaling-results: FAIL: {error}", file=sys.stderr)
        return 1
    print(f"verify-scaling-results: OK ({sample_count} samples)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
