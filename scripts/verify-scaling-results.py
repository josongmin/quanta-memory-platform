#!/usr/bin/env python3
"""Fail-closed validator for complete QMP scaling benchmark runs."""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path
from typing import NamedTuple

SAMPLE_SCHEMA = "qmp-scaling-sample-v3"
RUN_SCHEMA = "qmp-scaling-run-v3"
END_SCHEMA = "qmp-scaling-run-end-v3"
SINGLE = {
    "payload_unique_fill", "payload_dedup_put", "revocable_fill_and_drain",
    "broker_release_owner_cardinality", "scheduler_admission_group_cardinality",
}
READ_MODES = {"hit", "miss"}
CONTENTION_MODES = {"metadata-stats-heavy", "write-heavy", "mixed-stats-put"}
PAIRWISE_MODES = {
    "shared-single-owner", "shared-per-thread-owner", "independent-per-thread-owner",
}
COMPAT_FIELDS = (
    "rustc", "cargo", "machine", "feature_set", "feature_graph_sha256",
    "dependency_lock_sha256", "package", "bench_target",
)


class ValidationError(RuntimeError):
    pass


class BenchmarkSpec(NamedTuple):
    sizes: tuple[int, ...]
    threads: tuple[int, ...]
    repetitions: int


def positive_list(raw: str, option: str) -> tuple[int, ...]:
    try:
        values = tuple(int(part) for part in raw.split(","))
    except ValueError as error:
        raise ValidationError(f"{option} must contain positive integers") from error
    if not values or any(value <= 0 for value in values) or len(values) != len(set(values)):
        raise ValidationError(f"{option} must contain distinct positive integers")
    return values


def parse_benchmark_args(arguments: object) -> BenchmarkSpec:
    if not isinstance(arguments, list) or any(not isinstance(arg, str) for arg in arguments):
        raise ValidationError("benchmark_args must be a list of strings")
    sizes, threads, repetitions = (1_000, 10_000), (1, 2, 8, 32), 5
    seen: set[str] = set()
    for argument in arguments:
        if argument == "--smoke":
            option = "--smoke"
            sizes, threads, repetitions = (1_000,), (1, 2), 3
        elif argument.startswith("--sizes="):
            option = "--sizes"
            sizes = positive_list(argument.split("=", 1)[1], option)
        elif argument.startswith("--threads="):
            option = "--threads"
            threads = positive_list(argument.split("=", 1)[1], option)
        elif argument.startswith("--repetitions="):
            option = "--repetitions"
            try:
                repetitions = int(argument.split("=", 1)[1])
            except ValueError as error:
                raise ValidationError("--repetitions must be an integer") from error
            if repetitions < 3:
                raise ValidationError("--repetitions must be at least three")
        else:
            raise ValidationError(f"unknown benchmark argument {argument!r}")
        if option in seen:
            raise ValidationError(f"duplicate benchmark argument {option}")
        seen.add(option)
    return BenchmarkSpec(sizes, threads, repetitions)


def load_jsonl(path: Path) -> list[dict[str, object]]:
    content = path.read_text(encoding="utf-8")
    if content and not content.endswith("\n"):
        raise ValidationError(f"{path}: truncated final line")
    records: list[dict[str, object]] = []
    for line_number, raw in enumerate(content.splitlines(), 1):
        raw = raw.strip()
        if not raw.startswith("{"):
            continue  # cargo's human-readable lines are not benchmark records
        try:
            value = json.loads(raw)
        except json.JSONDecodeError as error:
            raise ValidationError(f"line {line_number}: invalid JSON: {error}") from error
        if not isinstance(value, dict):
            raise ValidationError(f"line {line_number}: JSON record must be an object")
        records.append(value)
    return records


def integer(sample: dict[str, object], field: str) -> int:
    value = sample.get(field)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValidationError(f"sample field {field!r} must be an integer")
    return value


def scenario_key(sample: dict[str, object]) -> tuple[object, ...]:
    return sample.get("operation"), sample.get("mode"), integer(sample, "entries"), integer(sample, "threads")


def expected_keys(spec: BenchmarkSpec) -> set[tuple[object, ...]]:
    keys: set[tuple[object, ...]] = set()
    for entries in spec.sizes:
        for repetition in range(spec.repetitions):
            keys.update((op, "single-thread", entries, 1, repetition) for op in SINGLE)
            keys.update(("retained_map_point_read", mode, entries, 1, repetition) for mode in READ_MODES)
            keys.update(
                ("payload_contention", mode, entries, threads, repetition)
                for threads in spec.threads for mode in CONTENTION_MODES
            )
            keys.update(
                ("broker_release_pairwise", mode, entries, 2, repetition)
                for mode in PAIRWISE_MODES
            )
    return keys


def run_metadata(records: list[dict[str, object]]) -> tuple[dict[str, object], BenchmarkSpec]:
    if not records or records[0].get("schema") != RUN_SCHEMA or records[-1].get("schema") != END_SCHEMA:
        raise ValidationError("benchmark run header must be first and completion footer last")
    for record in records:
        if record.get("schema") not in {RUN_SCHEMA, END_SCHEMA, SAMPLE_SCHEMA}:
            raise ValidationError(f"unexpected JSON record schema {record.get('schema')!r}")
    runs = [record for record in records if record.get("schema") == RUN_SCHEMA]
    ends = [record for record in records if record.get("schema") == END_SCHEMA]
    if len(runs) != 1 or len(ends) != 1:
        raise ValidationError(f"expected one run and one end record, found {len(runs)} and {len(ends)}")
    if any(record.get("schema") != SAMPLE_SCHEMA for record in records[1:-1]):
        raise ValidationError("only sample records may occur between run header and completion footer")
    run, end = runs[0], ends[0]
    for field in ("source_sha", "source_tree", *COMPAT_FIELDS):
        if not isinstance(run.get(field), str) or not run[field]:
            raise ValidationError(f"run metadata field {field!r} is missing")
    for field in ("source_sha", "source_tree"):
        if end.get(field) != run[field]:
            raise ValidationError(f"source changed during benchmark: {field}")
    if end.get("source_clean") is not True:
        raise ValidationError("source changed during benchmark: working tree is dirty")
    return run, parse_benchmark_args(run.get("benchmark_args"))


def validate_samples(
    records: list[dict[str, object]], spec: BenchmarkSpec, max_p99_ns: int,
) -> tuple[list[dict[str, object]], dict[tuple[object, ...], list[dict[str, object]]]]:
    samples = [record for record in records if record.get("schema") == SAMPLE_SCHEMA]
    expected = expected_keys(spec)
    observed: dict[tuple[object, ...], dict[str, object]] = {}
    grouped: dict[tuple[object, ...], list[dict[str, object]]] = defaultdict(list)
    for sample in samples:
        key = (*scenario_key(sample), integer(sample, "repetition"))
        if key in observed:
            raise ValidationError(f"duplicate benchmark cell/repetition: {key}")
        observed[key] = sample
        grouped[key[:-1]].append(sample)
        if integer(sample, "entries") <= 0 or integer(sample, "threads") <= 0:
            raise ValidationError(f"{key}: entries/threads must be positive")
        if integer(sample, "elapsed_ns") <= 0:
            raise ValidationError(f"{key}: elapsed_ns must be positive")
        p50, p95, p99 = (integer(sample, field) for field in ("p50_ns", "p95_ns", "p99_ns"))
        if not (0 <= p50 <= p95 <= p99 <= max_p99_ns):
            raise ValidationError(f"{key}: invalid/over-limit latency distribution {p50}/{p95}/{p99}")
        throughput = integer(sample, "throughput_ops_per_sec")
        if throughput <= 0:
            raise ValidationError(f"{key}: throughput must be positive")
        timed = integer(sample, "timed_operations")
        succeeded = integer(sample, "successful_operations")
        denied = integer(sample, "denied_operations")
        if timed <= 0 or succeeded < 0 or denied < 0 or timed != succeeded + denied:
            raise ValidationError(f"{key}: timed/successful/denied operation counts disagree")
        expected_timed = integer(sample, "entries") * (
            2 if sample.get("operation") in {"revocable_fill_and_drain", "scheduler_admission_group_cardinality"} else 1
        )
        if timed != expected_timed:
            raise ValidationError(f"{key}: timed operations {timed} != expected {expected_timed}")
        if sample.get("operation") == "scheduler_admission_group_cardinality":
            expected_succeeded = expected_denied = integer(sample, "entries")
        else:
            expected_succeeded, expected_denied = expected_timed, 0
        if (succeeded, denied) != (expected_succeeded, expected_denied):
            raise ValidationError(
                f"{key}: outcome counts {succeeded}/{denied} != expected "
                f"{expected_succeeded}/{expected_denied}"
            )
        derived_throughput = round(timed * 1_000_000_000 / integer(sample, "elapsed_ns"))
        if abs(throughput - derived_throughput) > max(1, derived_throughput // 1_000):
            raise ValidationError(f"{key}: throughput does not match timed operations / elapsed time")
        semantic_peak = integer(sample, "semantic_peak_bytes")
        if sample.get("operation") == "scheduler_admission_group_cardinality":
            if semantic_peak != 0:
                raise ValidationError(f"{key}: scheduler admission must not retain semantic bytes")
        elif semantic_peak <= 0:
            raise ValidationError(f"{key}: retained workload has no semantic peak")
        if integer(sample, "semantic_after_close_bytes") != 0:
            raise ValidationError(f"{key}: semantic accounting did not drain to zero")
        allocated, deallocated, live = (
            integer(sample, field)
            for field in ("allocator_allocated_bytes", "allocator_deallocated_bytes", "allocator_live_bytes")
        )
        if allocated < 0 or deallocated < 0 or allocated != deallocated or live != 0:
            raise ValidationError(
                f"{key}: allocator drift allocated={allocated} deallocated={deallocated} live={live}"
            )
        if sample.get("allocator_underflowed") is not False:
            raise ValidationError(f"{key}: allocator live-byte counter underflowed")
        if integer(sample, "allocator_high_water_bytes") < live:
            raise ValidationError(f"{key}: allocator high-water is below live bytes")
        if integer(sample, "busy_retries") < 0:
            raise ValidationError(f"{key}: busy retry count is negative")
    observed_keys = set(observed)
    if missing := expected - observed_keys:
        raise ValidationError(f"benchmark matrix is incomplete: {len(missing)} missing; first {sorted(missing)[:3]}")
    if extra := observed_keys - expected:
        raise ValidationError(f"benchmark matrix has {len(extra)} extra cells; first {sorted(extra)[:3]}")
    return samples, grouped


def validate_growth(samples: list[dict[str, object]]) -> None:
    by_entries: dict[int, list[dict[str, object]]] = defaultdict(list)
    for sample in samples:
        if sample.get("operation") == "revocable_fill_and_drain":
            by_entries[integer(sample, "entries")].append(sample)
    sizes = sorted(by_entries)
    if len(sizes) < 2 or sizes[-1] < sizes[0] * 10:
        return
    smallest, largest = sizes[0], sizes[-1]
    small = statistics.median(integer(sample, "elapsed_ns") / smallest for sample in by_entries[smallest])
    large = statistics.median(integer(sample, "elapsed_ns") / largest for sample in by_entries[largest])
    ratio = large / small if small > 0 else float("inf")
    if ratio > 4.0:
        raise ValidationError(
            f"revocable_fill_and_drain growth: per-entry cost grew {ratio:.2f}x "
            f"from {smallest} to {largest} entries (allowed 4.0x)"
        )


def validate(
    records: list[dict[str, object]],
    baseline_records: list[dict[str, object]] | None = None,
    max_p99_ns: int = 1_000_000_000,
    max_throughput_regression_percent: int = 90,
    max_p99_regression_percent: int = 200,
    p99_noise_floor_ns: int = 1_000_000,
    baseline_min_elapsed_ns: int = 10_000_000,
) -> int:
    run, spec = run_metadata(records)
    samples, grouped = validate_samples(records, spec, max_p99_ns)
    validate_growth(samples)
    if baseline_records is not None:
        baseline_run, baseline_spec = run_metadata(baseline_records)
        if baseline_spec != spec:
            raise ValidationError(f"baseline benchmark matrix differs: {baseline_spec} != {spec}")
        for field in COMPAT_FIELDS:
            if baseline_run[field] != run[field]:
                raise ValidationError(f"baseline incompatible {field}: {baseline_run[field]!r} != {run[field]!r}")
        baseline_samples, baseline_groups = validate_samples(baseline_records, spec, max_p99_ns)
        validate_growth(baseline_samples)
        throughput_allowed = max_throughput_regression_percent / 100.0
        p99_allowed = max_p99_regression_percent / 100.0
        for key, current in grouped.items():
            baseline = baseline_groups[key]
            current_throughput = statistics.median(integer(sample, "throughput_ops_per_sec") for sample in current)
            baseline_throughput = statistics.median(integer(sample, "throughput_ops_per_sec") for sample in baseline)
            baseline_elapsed = statistics.median(integer(sample, "elapsed_ns") for sample in baseline)
            if (
                baseline_elapsed >= baseline_min_elapsed_ns
                and current_throughput < baseline_throughput * (1.0 - throughput_allowed)
            ):
                raise ValidationError(f"{key}: throughput regressed beyond {max_throughput_regression_percent}%")
            current_p99 = statistics.median(integer(sample, "p99_ns") for sample in current)
            baseline_p99 = statistics.median(integer(sample, "p99_ns") for sample in baseline)
            p99_limit = max(baseline_p99 * (1.0 + p99_allowed), baseline_p99 + p99_noise_floor_ns)
            if current_p99 > p99_limit:
                raise ValidationError(f"{key}: p99 regressed beyond {max_p99_regression_percent}%")
    return len(samples)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--smoke", action="store_true", help="require a smoke invocation in run metadata")
    parser.add_argument("--max-p99-ns", type=int, default=1_000_000_000)
    parser.add_argument("--max-throughput-regression-percent", type=int, default=90)
    parser.add_argument("--max-p99-regression-percent", type=int, default=200)
    parser.add_argument("--p99-noise-floor-ns", type=int, default=1_000_000)
    parser.add_argument("--baseline-min-elapsed-ns", type=int, default=10_000_000)
    args = parser.parse_args()
    try:
        records = load_jsonl(args.input)
        if args.smoke:
            run, _ = run_metadata(records)
            if "--smoke" not in run["benchmark_args"]:
                raise ValidationError("--smoke was requested but run metadata is not smoke")
        baseline = load_jsonl(args.baseline) if args.baseline else None
        sample_count = validate(
            records, baseline, args.max_p99_ns, args.max_throughput_regression_percent,
            args.max_p99_regression_percent, args.p99_noise_floor_ns, args.baseline_min_elapsed_ns,
        )
    except (OSError, ValidationError) as error:
        print(f"verify-scaling-results: FAIL: {error}", file=sys.stderr)
        return 1
    print(f"verify-scaling-results: OK ({sample_count} samples)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
