#!/usr/bin/env python3
"""Fail-closed validator for complete QMP scaling benchmark runs."""

from __future__ import annotations

import argparse
import json
import math
import re
import statistics
import sys
from collections import defaultdict
from pathlib import Path
from typing import NamedTuple

SAMPLE_SCHEMA = "qmp-scaling-sample-v4"
RUN_SCHEMA = "qmp-scaling-run-v4"
END_SCHEMA = "qmp-scaling-run-end-v4"
SINGLE = {
    "payload_unique_fill", "payload_dedup_put", "revocable_fill_and_drain",
    "broker_release_owner_cardinality", "scheduler_admission_group_cardinality",
}
READ_MODES = {"hit", "miss"}
CONTENTION_MODES = {"metadata-stats-heavy", "write-heavy", "mixed-stats-put"}
PAIRWISE_MODES = {
    "shared-single-owner", "shared-per-thread-owner", "independent-per-thread-owner",
}
PRESSURE_OUTCOME_OPERATION = "broker_pressure_cycle"
PRESSURE_CONTENTION_OPERATION = "dual_store_write_contention"
PRESSURE_RECLAIM_OPERATION = "contended_pressure_reclaim"
FIXED_OUTCOME_OPERATIONS = {"scheduler_admission_group_cardinality", PRESSURE_OUTCOME_OPERATION}
OUTCOME_OPERATIONS = FIXED_OUTCOME_OPERATIONS | {PRESSURE_RECLAIM_OPERATION}
OUTCOME_LATENCY_FIELDS = {
    outcome: tuple(f"{outcome}_{percentile}_ns" for percentile in ("p50", "p95", "p99"))
    for outcome in ("admitted", "denied")
}
ALL_OUTCOME_LATENCY_FIELDS = tuple(
    field for fields in OUTCOME_LATENCY_FIELDS.values() for field in fields
)
DETAIL_LATENCY_FIELDS = {
    operation: tuple(
        f"{phase}_{percentile}_ns" for phase in phases for percentile in ("p50", "p95", "p99")
    )
    for operation, phases in {
        "revocable_fill_and_drain": ("fill", "reclaim"),
        PRESSURE_RECLAIM_OPERATION: ("reclaim",),
    }.items()
}
ALL_DETAIL_LATENCY_FIELDS = set().union(*map(set, DETAIL_LATENCY_FIELDS.values()))
COMPAT_FIELDS = (
    "rustc", "cargo", "machine", "feature_set", "feature_graph_sha256",
    "dependency_lock_sha256", "package", "bench_target",
)
BENCHMARK_SOURCE_FIELD = "benchmark_source_sha256"
BUILD_CONFIG_FIELD = "bench_build_config_sha256"


class ValidationError(RuntimeError):
    pass


class InconclusiveError(RuntimeError):
    pass


class BenchmarkSpec(NamedTuple):
    sizes: tuple[int, ...]
    threads: tuple[int, ...]
    repetitions: int
    profile: str


class ReleasePolicy(NamedTuple):
    max_p99_ns: int
    max_throughput_regression_percent: float
    max_p99_regression_percent: float
    p99_noise_floor_ns: int
    max_busy_retries_per_operation: float
    max_run_noise_percent: float
    max_load_per_cpu: float


def positive_list(raw: str, option: str) -> tuple[int, ...]:
    values = tuple(positive_usize(part, option) for part in raw.split(","))
    if len(values) != len(set(values)):
        raise ValidationError(f"{option} must contain distinct positive integers")
    return values


def positive_usize(raw: str, option: str) -> int:
    if not raw or not raw.isascii() or not raw.isdecimal():
        raise ValidationError(f"{option} must be a positive usize decimal")
    try:
        value = int(raw)
    except ValueError as error:
        raise ValidationError(f"{option} must be a positive usize decimal") from error
    if value <= 0 or value > 2 * sys.maxsize + 1:
        raise ValidationError(f"{option} is outside the positive usize range")
    return value


def parse_benchmark_args(arguments: object) -> BenchmarkSpec:
    if not isinstance(arguments, list) or any(not isinstance(arg, str) for arg in arguments):
        raise ValidationError("benchmark_args must be a list of strings")
    sizes, repetitions, profile = (1_000, 10_000), 5, "standard"
    smoke = False
    sizes_override: tuple[int, ...] | None = None
    threads_override: tuple[int, ...] | None = None
    repetitions_override: int | None = None
    seen: set[str] = set()
    for argument in arguments:
        if argument == "--smoke":
            option = "--smoke"
            smoke = True
        elif argument.startswith("--sizes="):
            option = "--sizes"
            sizes_override = positive_list(argument.split("=", 1)[1], option)
        elif argument.startswith("--threads="):
            option = "--threads"
            threads_override = positive_list(argument.split("=", 1)[1], option)
        elif argument.startswith("--repetitions="):
            option = "--repetitions"
            repetitions_override = positive_usize(argument.split("=", 1)[1], option)
            if repetitions_override < 3:
                raise ValidationError("--repetitions must be at least three")
        elif argument == "--profile=in-memory-pressure":
            option = "--profile"
            profile = "in-memory-pressure"
        else:
            raise ValidationError(f"unknown benchmark argument {argument!r}")
        if option in seen:
            raise ValidationError(f"duplicate benchmark argument {option}")
        seen.add(option)
    if smoke:
        sizes, repetitions = (1_000,), 3
    threads = (1, 8, 32) if profile == "in-memory-pressure" else (1, 2, 8, 32)
    if smoke:
        threads = (1, 8) if profile == "in-memory-pressure" else (1, 2)
    spec = BenchmarkSpec(
        sizes_override or sizes,
        threads_override or threads,
        repetitions_override or repetitions,
        profile,
    )
    if profile == "in-memory-pressure" and any(
        entries < 2 * worker_count
        for entries in spec.sizes for worker_count in spec.threads if worker_count > 1
    ):
        raise ValidationError("contended pressure requires at least two admission attempts per worker")
    return spec


def load_jsonl(path: Path) -> list[dict[str, object]]:
    try:
        content = path.read_text(encoding="utf-8")
    except UnicodeDecodeError as error:
        raise ValidationError(f"{path}: invalid UTF-8") from error
    if content and not content.endswith("\n"):
        raise ValidationError(f"{path}: truncated final line")
    records: list[dict[str, object]] = []
    for line_number, raw in enumerate(content.splitlines(), 1):
        raw = raw.strip()
        if not raw.startswith("{"):
            continue  # cargo's human-readable lines are not benchmark records

        def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
            result: dict[str, object] = {}
            for key, value in pairs:
                if key in result:
                    raise ValidationError(f"line {line_number}: duplicate JSON key {key!r}")
                result[key] = value
            return result
        try:
            value = json.loads(raw, object_pairs_hook=unique_object)
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
    identity = (sample.get("operation"), sample.get("mode"))
    if any(not isinstance(value, str) or not value for value in identity):
        raise ValidationError("sample operation/mode must be nonempty strings")
    return *identity, integer(sample, "entries"), integer(sample, "threads")


def expected_keys(spec: BenchmarkSpec) -> set[tuple[object, ...]]:
    keys: set[tuple[object, ...]] = set()
    for entries in spec.sizes:
        for repetition in range(spec.repetitions):
            if spec.profile == "in-memory-pressure":
                keys.add((PRESSURE_OUTCOME_OPERATION, "single-thread", entries, 1, repetition))
                keys.update(
                    (PRESSURE_CONTENTION_OPERATION, "shared-broker", entries, threads, repetition)
                    for threads in spec.threads
                )
                keys.update(
                    (PRESSURE_RECLAIM_OPERATION, "two-revocable-stores", entries, threads, repetition)
                    for threads in spec.threads if threads > 1
                )
                continue
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
        schema = record.get("schema")
        if not isinstance(schema, str) or schema not in {RUN_SCHEMA, END_SCHEMA, SAMPLE_SCHEMA}:
            raise ValidationError(f"unexpected JSON record schema {schema!r}")
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
    for field in (BENCHMARK_SOURCE_FIELD, BUILD_CONFIG_FIELD):
        digest = run.get(field)
        if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
            raise ValidationError(f"run metadata field {field!r} is missing or invalid")
    for field in ("source_sha", "source_tree"):
        if end.get(field) != run[field]:
            raise ValidationError(f"source changed during benchmark: {field}")
    if end.get(BUILD_CONFIG_FIELD) != run[BUILD_CONFIG_FIELD]:
        raise ValidationError("build configuration changed during benchmark")
    if end.get("source_clean") is not True:
        raise ValidationError("source changed during benchmark: working tree is dirty")
    if "bench_exit_code" not in end:
        raise ValidationError("bench exit status is missing")
    if (
        not isinstance(end["bench_exit_code"], int)
        or isinstance(end["bench_exit_code"], bool)
        or end["bench_exit_code"] != 0
    ):
        raise ValidationError("bench exited nonzero or has invalid exit status")
    if (
        not isinstance(run.get("run_id"), str) or not run["run_id"]
        or not isinstance(end.get("run_id"), str) or not end["run_id"]
    ):
        raise ValidationError("run identity is invalid")
    if run["run_id"] != end["run_id"]:
        raise ValidationError("run identity changed during benchmark")
    spec = parse_benchmark_args(run.get("benchmark_args"))
    expected_features = "full" if spec.profile == "in-memory-pressure" else "all-features"
    if run.get("feature_set") != expected_features:
        raise ValidationError(
            f"benchmark profile {spec.profile} requires feature_set {expected_features}"
        )
    return run, spec


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
        if p99 > integer(sample, "elapsed_ns"):
            raise ValidationError(f"{key}: operation p99 exceeds measured wall time")
        throughput = integer(sample, "throughput_ops_per_sec")
        if throughput <= 0:
            raise ValidationError(f"{key}: throughput must be positive")
        timed = integer(sample, "timed_operations")
        succeeded = integer(sample, "successful_operations")
        denied = integer(sample, "denied_operations")
        if timed <= 0 or succeeded < 0 or denied < 0 or timed != succeeded + denied:
            raise ValidationError(f"{key}: timed/successful/denied operation counts disagree")
        expected_timed = integer(sample, "entries") * (
            2 if sample.get("operation") in {"revocable_fill_and_drain", *FIXED_OUTCOME_OPERATIONS} else 1
        )
        if timed != expected_timed:
            raise ValidationError(f"{key}: timed operations {timed} != expected {expected_timed}")
        if sample.get("operation") == PRESSURE_RECLAIM_OPERATION:
            if not (0 < succeeded < timed and 0 < denied < timed):
                raise ValidationError(f"{key}: contended pressure requires admitted and denied operations")
        elif sample.get("operation") in FIXED_OUTCOME_OPERATIONS:
            expected_succeeded = expected_denied = integer(sample, "entries")
            if (succeeded, denied) != (expected_succeeded, expected_denied):
                raise ValidationError(
                    f"{key}: outcome counts {succeeded}/{denied} != expected "
                    f"{expected_succeeded}/{expected_denied}"
                )
        else:
            expected_succeeded, expected_denied = expected_timed, 0
            if (succeeded, denied) != (expected_succeeded, expected_denied):
                raise ValidationError(
                    f"{key}: outcome counts {succeeded}/{denied} != expected "
                    f"{expected_succeeded}/{expected_denied}"
                )
        present = tuple(field for field in ALL_OUTCOME_LATENCY_FIELDS if field in sample)
        if sample.get("operation") in OUTCOME_OPERATIONS:
            if len(present) != len(ALL_OUTCOME_LATENCY_FIELDS):
                raise ValidationError(f"{key}: incomplete admission outcome latency fields")
            for outcome, fields in OUTCOME_LATENCY_FIELDS.items():
                values = tuple(integer(sample, field) for field in fields)
                if not (0 <= values[0] <= values[1] <= values[2] <= max_p99_ns):
                    raise ValidationError(f"{key}: invalid/over-limit {outcome} latency distribution {values}")
            if sample.get("operation") in FIXED_OUTCOME_OPERATIONS and p99 < max(
                integer(sample, "admitted_p50_ns"), integer(sample, "denied_p50_ns")
            ):
                raise ValidationError(f"{key}: outcome median exceeds combined p99")
        elif present:
            raise ValidationError(f"{key}: outcome latency fields are only valid for admission operations")
        detail_fields = DETAIL_LATENCY_FIELDS.get(sample.get("operation"), ())
        present_detail = {field for field in ALL_DETAIL_LATENCY_FIELDS if field in sample}
        if present_detail != set(detail_fields):
            raise ValidationError(f"{key}: missing or unexpected phase latency fields")
        for phase in ("fill", "reclaim"):
            fields = tuple(f"{phase}_{percentile}_ns" for percentile in ("p50", "p95", "p99"))
            if fields[0] in detail_fields:
                values = tuple(integer(sample, field) for field in fields)
                if not (0 <= values[0] <= values[1] <= values[2] <= max_p99_ns):
                    raise ValidationError(f"{key}: invalid/over-limit {phase} latency distribution {values}")
        if sample.get("operation") == PRESSURE_RECLAIM_OPERATION:
            reclaimed = integer(sample, "reclaimed_operations")
            if not 0 < reclaimed <= denied or reclaimed > succeeded + 2:
                raise ValidationError(f"{key}: reclaimed operation count is invalid")
        elif "reclaimed_operations" in sample:
            raise ValidationError(f"{key}: reclaimed operation count is unexpected")
        derived_throughput = round(timed * 1_000_000_000 / integer(sample, "elapsed_ns"))
        if abs(throughput - derived_throughput) > max(1, derived_throughput // 1_000):
            raise ValidationError(f"{key}: throughput does not match timed operations / elapsed time")
        successful_throughput = integer(sample, "successful_throughput_ops_per_sec")
        derived_successful_throughput = round(succeeded * 1_000_000_000 / integer(sample, "elapsed_ns"))
        if successful_throughput <= 0 or abs(successful_throughput - derived_successful_throughput) > max(1, derived_successful_throughput // 1_000):
            raise ValidationError(f"{key}: successful throughput does not match admitted operations / elapsed time")
        semantic_peak = integer(sample, "semantic_peak_bytes")
        if sample.get("operation") == PRESSURE_OUTCOME_OPERATION:
            if semantic_peak != 1:
                raise ValidationError(f"{key}: pressure cycle must retain exactly one semantic byte")
        elif sample.get("operation") == PRESSURE_RECLAIM_OPERATION:
            if semantic_peak != 96:
                raise ValidationError(f"{key}: contended pressure must start at its 96-byte hard cap")
        elif sample.get("operation") == "scheduler_admission_group_cardinality":
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
        high_water = integer(sample, "allocator_high_water_bytes")
        if high_water < live or high_water > allocated or (allocated > 0 and high_water == 0):
            raise ValidationError(f"{key}: allocator high-water is inconsistent with allocated/live bytes")
        if sample.get("operation") != "scheduler_admission_group_cardinality" and allocated == 0:
            raise ValidationError(f"{key}: missing allocator telemetry for retained workload")
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
    # Wall elapsed includes time between calls, including host scheduling gaps.
    # The row already times each store operation independently; its p95 is the
    # stable signal for a systemic fill/drain complexity regression. Keep wall
    # elapsed and throughput in the receipt and in optional baseline checks.
    small = statistics.median(integer(sample, "p95_ns") for sample in by_entries[smallest])
    large = statistics.median(integer(sample, "p95_ns") for sample in by_entries[largest])
    ratio = large / small if small > 0 else float("inf")
    if ratio > 4.0:
        raise ValidationError(
            f"revocable_fill_and_drain growth: operation p95 grew {ratio:.2f}x "
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
    comparison_stats: dict[str, int] | None = None,
) -> int:
    if max_p99_ns <= 0 or not 0 <= max_throughput_regression_percent < 100 or max_p99_regression_percent < 0 or p99_noise_floor_ns < 0 or baseline_min_elapsed_ns < 0:
        raise ValidationError("invalid ordinary benchmark limits")
    run, spec = run_metadata(records)
    samples, grouped = validate_samples(records, spec, max_p99_ns)
    validate_growth(samples)
    if baseline_records is not None:
        baseline_run, baseline_spec = run_metadata(baseline_records)
        if run["run_id"] == baseline_run["run_id"]:
            raise ValidationError("baseline reuses candidate run identity")
        if baseline_spec != spec:
            raise ValidationError(f"baseline benchmark matrix differs: {baseline_spec} != {spec}")
        for field in COMPAT_FIELDS:
            if baseline_run[field] != run[field]:
                raise ValidationError(f"baseline incompatible {field}: {baseline_run[field]!r} != {run[field]!r}")
        if (
            not run.get(BENCHMARK_SOURCE_FIELD)
            or baseline_run.get(BENCHMARK_SOURCE_FIELD) != run[BENCHMARK_SOURCE_FIELD]
        ):
            raise ValidationError(f"baseline incompatible {BENCHMARK_SOURCE_FIELD}")
        if (
            not run.get(BUILD_CONFIG_FIELD)
            or baseline_run.get(BUILD_CONFIG_FIELD) != run[BUILD_CONFIG_FIELD]
        ):
            raise ValidationError(f"baseline incompatible {BUILD_CONFIG_FIELD}")
        baseline_samples, baseline_groups = validate_samples(baseline_records, spec, max_p99_ns)
        validate_growth(baseline_samples)
        throughput_allowed = max_throughput_regression_percent / 100.0
        p99_allowed = max_p99_regression_percent / 100.0
        compared = skipped = 0
        for key, current in grouped.items():
            baseline = baseline_groups[key]
            baseline_elapsed = statistics.median(integer(sample, "elapsed_ns") for sample in baseline)
            if baseline_elapsed < baseline_min_elapsed_ns:
                skipped += 1
            else:
                compared += 1
            for field, label in (
                ("throughput_ops_per_sec", "throughput"),
                ("successful_throughput_ops_per_sec", "successful throughput"),
            ):
                current_throughput = statistics.median(integer(sample, field) for sample in current)
                baseline_throughput = statistics.median(integer(sample, field) for sample in baseline)
                if (
                    baseline_elapsed >= baseline_min_elapsed_ns
                    and current_throughput < baseline_throughput * (1.0 - throughput_allowed)
                ):
                    raise ValidationError(f"{key}: {label} regressed beyond {max_throughput_regression_percent}%")
            for field, label in (
                ("p99_ns", "p99"),
                ("admitted_p99_ns", "admitted p99"),
                ("denied_p99_ns", "denied p99"),
                ("fill_p99_ns", "fill p99"),
                ("reclaim_p99_ns", "reclaim p99"),
            ):
                # Only rows carrying this metric can be compared; every v4
                # operation-specific field is mandatory for its owning row.
                if field not in baseline[0]:
                    continue
                current_p99 = statistics.median(integer(sample, field) for sample in current)
                baseline_p99 = statistics.median(integer(sample, field) for sample in baseline)
                p99_limit = max(baseline_p99 * (1.0 + p99_allowed), baseline_p99 + p99_noise_floor_ns)
                if current_p99 > p99_limit:
                    raise ValidationError(f"{key}: {label} regressed beyond {max_p99_regression_percent}%")
        if skipped:
            raise InconclusiveError(
                f"baseline throughput comparison skipped {skipped} short cells "
                f"(compared {compared}); no complete regression verdict"
            )
        if comparison_stats is not None:
            comparison_stats.update(compared=compared, skipped=skipped)
    return len(samples)


def validate_release_policy(policy: ReleasePolicy) -> None:
    if policy.max_p99_ns <= 0 or policy.p99_noise_floor_ns < 0:
        raise ValidationError("release p99 limits must be positive and noise floor nonnegative")
    for label, value, upper in (
        ("throughput regression", policy.max_throughput_regression_percent, 100),
        ("p99 regression", policy.max_p99_regression_percent, None),
        ("run noise", policy.max_run_noise_percent, None),
        ("busy retries per operation", policy.max_busy_retries_per_operation, None),
        ("load per CPU", policy.max_load_per_cpu, None),
    ):
        if not math.isfinite(value) or value < 0 or (upper is not None and value >= upper):
            raise ValidationError(f"invalid release {label} limit")


def release_run(
    records: list[dict[str, object]], policy: ReleasePolicy,
) -> tuple[dict[str, object], BenchmarkSpec, dict[tuple[object, ...], list[dict[str, object]]]]:
    run, spec = run_metadata(records)
    if "--smoke" in run["benchmark_args"]:
        raise ValidationError("release comparison requires full, not smoke, benchmark runs")
    canonical_args = ["--profile=in-memory-pressure"] if spec.profile == "in-memory-pressure" else []
    canonical_full = parse_benchmark_args(canonical_args)
    if spec != canonical_full:
        raise ValidationError("release comparison requires the canonical full matrix")
    if not isinstance(run.get("run_id"), str) or not run["run_id"]:
        raise ValidationError("release run identity is missing")
    if not run.get(BENCHMARK_SOURCE_FIELD):
        raise ValidationError(f"release {BENCHMARK_SOURCE_FIELD} is missing")
    if not run.get(BUILD_CONFIG_FIELD):
        raise ValidationError(f"release {BUILD_CONFIG_FIELD} is missing")
    if "bench_exit_code" not in records[-1]:
        raise ValidationError("release bench exit status is missing")
    if (
        not isinstance(run.get("logical_cpus"), int)
        or isinstance(run["logical_cpus"], bool)
        or run["logical_cpus"] <= 0
    ):
        raise ValidationError("release logical CPU count is missing")
    samples, grouped = validate_samples(records, spec, policy.max_p99_ns)
    validate_growth(samples)
    end = records[-1]
    for record, field in ((run, "load_per_cpu_start"), (end, "load_per_cpu_end")):
        load = record.get(field)
        if not isinstance(load, (int, float)) or isinstance(load, bool) or not math.isfinite(load) or load < 0:
            raise ValidationError(f"release {field} is invalid")
        if load > policy.max_load_per_cpu:
            raise InconclusiveError(
                f"host load exceeded release limit: {field}={load:.2f} "
                f"> {policy.max_load_per_cpu:.2f}"
            )
    return run, spec, grouped


def run_cell_metrics(
    grouped: dict[tuple[object, ...], list[dict[str, object]]],
) -> dict[tuple[object, ...], dict[str, float]]:
    metrics: dict[tuple[object, ...], dict[str, float]] = {}
    for key, samples in grouped.items():
        # A recurring one-of-five collapse is not erased by a within-process
        # median. Cross-process noise remains a separate INCONCLUSIVE gate.
        cell = {
            "throughput": min(integer(row, "throughput_ops_per_sec") for row in samples),
            "successful_throughput": min(
                integer(row, "successful_throughput_ops_per_sec") for row in samples
            ),
            "p99": max(integer(row, "p99_ns") for row in samples),
            "busy_per_operation": max(
                integer(row, "busy_retries") / integer(row, "timed_operations") for row in samples
            ),
        }
        for field in ("admitted_p99_ns", "denied_p99_ns"):
            if field in samples[0]:
                cell[field] = max(integer(row, field) for row in samples)
        for field in ("fill_p99_ns", "reclaim_p99_ns"):
            if field in samples[0]:
                cell[field] = max(integer(row, field) for row in samples)
        metrics[key] = cell
    return metrics


def validate_release(
    candidates: list[list[dict[str, object]]],
    baselines: list[list[dict[str, object]]],
    policy: ReleasePolicy,
) -> int:
    """Require three independent full runs per source, then compare stable cells."""
    validate_release_policy(policy)
    if len(candidates) != 3 or len(baselines) != 3:
        raise ValidationError("release comparison requires three candidate and three baseline runs")
    baseline_runs = [release_run(records, policy) for records in baselines]
    candidate_runs = [release_run(records, policy) for records in candidates]
    all_runs = baseline_runs + candidate_runs
    run_ids = [run[0]["run_id"] for run in all_runs]
    if len(set(run_ids)) != len(run_ids):
        raise ValidationError("release comparison reused a run identity")
    baseline_ref, spec, _ = baseline_runs[0]
    candidate_ref = candidate_runs[0][0]
    for label, runs, reference in (
        ("baseline", baseline_runs, baseline_ref),
        ("candidate", candidate_runs, candidate_ref),
    ):
        for run, run_spec, _ in runs:
            if run_spec != spec:
                raise ValidationError(f"{label} benchmark matrix differs")
            for field in ("source_sha", "source_tree", "benchmark_args", BENCHMARK_SOURCE_FIELD, BUILD_CONFIG_FIELD, *COMPAT_FIELDS, "logical_cpus"):
                if run[field] != reference[field]:
                    raise ValidationError(f"{label} runs disagree on {field}")
    for field in (BENCHMARK_SOURCE_FIELD, BUILD_CONFIG_FIELD, *COMPAT_FIELDS, "logical_cpus"):
        if baseline_ref[field] != candidate_ref[field]:
            raise ValidationError(f"release baseline incompatible {field}")
    if (
        baseline_ref["source_sha"] == candidate_ref["source_sha"]
        or baseline_ref["source_tree"] == candidate_ref["source_tree"]
    ):
        raise ValidationError("release baseline and candidate use the same source tree or commit")

    baseline_metrics = [run_cell_metrics(grouped) for _, _, grouped in baseline_runs]
    candidate_metrics = [run_cell_metrics(grouped) for _, _, grouped in candidate_runs]
    noise_limit = policy.max_run_noise_percent / 100.0
    for label, runs in (("baseline", baseline_metrics), ("candidate", candidate_metrics)):
        for key in runs[0]:
            for field in runs[0][key]:
                if field == "busy_per_operation":
                    continue  # Absolute Busy ceiling is checked below.
                values = [run[key][field] for run in runs]
                center = statistics.median(values)
                floor = 1 if field in {"throughput", "successful_throughput"} else policy.p99_noise_floor_ns
                if max(values) - min(values) > noise_limit * max(center, floor):
                    raise InconclusiveError(f"{label} {key} {field} varies beyond run-noise limit")

    throughput_limit = policy.max_throughput_regression_percent / 100.0
    p99_limit = policy.max_p99_regression_percent / 100.0
    for key in baseline_metrics[0]:
        baseline = {
            field: statistics.median(run[key][field] for run in baseline_metrics)
            for field in baseline_metrics[0][key]
        }
        candidate = {
            field: statistics.median(run[key][field] for run in candidate_metrics)
            for field in candidate_metrics[0][key]
        }
        if candidate["throughput"] < baseline["throughput"] * (1.0 - throughput_limit):
            raise ValidationError(f"{key}: release throughput regressed")
        if candidate["successful_throughput"] < baseline["successful_throughput"] * (1.0 - throughput_limit):
            raise ValidationError(f"{key}: release successful throughput regressed")
        for field in ("p99", "admitted_p99_ns", "denied_p99_ns", "fill_p99_ns", "reclaim_p99_ns"):
            if field not in baseline:
                continue
            allowed = max(
                baseline[field] * (1.0 + p99_limit),
                baseline[field] + policy.p99_noise_floor_ns,
            )
            if candidate[field] > allowed:
                raise ValidationError(f"{key}: release {field} regressed")
        if max(run[key]["busy_per_operation"] for run in candidate_metrics) > policy.max_busy_retries_per_operation:
            raise ValidationError(f"{key}: release Busy retries per operation exceeded")
    return len(candidate_metrics[0]) * spec.repetitions


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, action="append", required=True)
    parser.add_argument("--baseline", type=Path, action="append")
    parser.add_argument("--release", action="store_true", help="compare three independent full runs per source")
    parser.add_argument("--smoke", action="store_true", help="require a smoke invocation in run metadata")
    parser.add_argument("--max-p99-ns", type=int, default=1_000_000_000)
    parser.add_argument("--max-throughput-regression-percent", type=int, default=90)
    parser.add_argument("--max-p99-regression-percent", type=int, default=200)
    parser.add_argument("--p99-noise-floor-ns", type=int, default=1_000_000)
    parser.add_argument("--baseline-min-elapsed-ns", type=int, default=10_000_000)
    parser.add_argument("--release-max-p99-ns", type=int)
    parser.add_argument("--release-max-throughput-regression-percent", type=float)
    parser.add_argument("--release-max-p99-regression-percent", type=float)
    parser.add_argument("--release-p99-noise-floor-ns", type=int)
    parser.add_argument("--release-max-busy-retries-per-operation", type=float)
    parser.add_argument("--release-max-run-noise-percent", type=float)
    parser.add_argument("--release-max-load-per-cpu", type=float)
    args = parser.parse_args()
    try:
        if args.release:
            ordinary_options = (
                "--smoke", "--max-p99-ns", "--max-throughput-regression-percent",
                "--max-p99-regression-percent", "--p99-noise-floor-ns",
                "--baseline-min-elapsed-ns",
            )
            if any(any(arg == option or arg.startswith(option + "=") for option in ordinary_options) for arg in sys.argv[1:]):
                raise ValidationError("release comparison cannot use ordinary or smoke options")
            policy_fields = (
                args.release_max_p99_ns,
                args.release_max_throughput_regression_percent,
                args.release_max_p99_regression_percent,
                args.release_p99_noise_floor_ns,
                args.release_max_busy_retries_per_operation,
                args.release_max_run_noise_percent,
                args.release_max_load_per_cpu,
            )
            if any(value is None for value in policy_fields):
                raise ValidationError("release comparison requires every explicit release limit")
            policy = ReleasePolicy(*policy_fields)
            sample_count = validate_release(
                [load_jsonl(path) for path in args.input],
                [load_jsonl(path) for path in (args.baseline or [])],
                policy,
            )
            print(f"verify-scaling-results: RELEASE OK ({sample_count} candidate samples; 3 runs)")
            return 0
        if len(args.input) != 1 or (args.baseline and len(args.baseline) != 1):
            raise ValidationError("ordinary validation requires one input and at most one baseline")
        if args.baseline and args.input[0].resolve() == args.baseline[0].resolve():
            raise ValidationError("baseline and candidate refer to the same receipt")
        records = load_jsonl(args.input[0])
        if args.smoke:
            run, _ = run_metadata(records)
            if "--smoke" not in run["benchmark_args"]:
                raise ValidationError("--smoke was requested but run metadata is not smoke")
        baseline = load_jsonl(args.baseline[0]) if args.baseline else None
        comparison_stats: dict[str, int] = {}
        sample_count = validate(
            records, baseline, args.max_p99_ns, args.max_throughput_regression_percent,
            args.max_p99_regression_percent, args.p99_noise_floor_ns, args.baseline_min_elapsed_ns,
            comparison_stats,
        )
    except (OSError, ValidationError) as error:
        print(f"verify-scaling-results: FAIL: {error}", file=sys.stderr)
        return 1
    except InconclusiveError as error:
        print(f"verify-scaling-results: INCONCLUSIVE: {error}", file=sys.stderr)
        return 2
    coverage = (f"; baseline throughput cells compared={comparison_stats['compared']} "
                f"skipped={comparison_stats['skipped']}") if comparison_stats else ""
    print(f"verify-scaling-results: OK ({sample_count} samples{coverage})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
