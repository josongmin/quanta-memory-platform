// This benchmark alone instruments GlobalAlloc. Product crates forbid unsafe
// code; the workspace deny policy permits only the delegated GlobalAlloc item.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use quanta_memory_platform_core::domain::management_limits::MAX_RESOURCE_GROUPS_PER_POLICY;
use quanta_memory_platform_core::{
    ChargeKind, ExecutionBudget, ManagedCharge, MemoryBudgetManifest, MemoryClass,
    MemoryClassBudget, MemoryContextKind, MemoryError, MemoryPreset, ReclaimPriority,
    ResidencyPolicy, ResourceGroup, RetainedEnvelope, StoreBindingHandle,
};
use quanta_memory_platform_facade::{
    MemoryPlatformBuilder, PayloadStore, PortableMemoryPresets, RetainedMapStore, RevocableStore,
};
#[global_allocator]
static GLOBAL: BenchAllocator = BenchAllocator;

static ALLOCATION_TRACKING_ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOCATION_COUNTERS: AllocationCounters = AllocationCounters::new();

struct BenchAllocator;

// SAFETY: every operation delegates to `System` with the exact layout supplied
// by the caller. Accounting is allocation-free atomic bookkeeping and does not
// alter the returned pointer or allocation lifetime.
#[allow(unsafe_code, reason = "GlobalAlloc instrumentation delegates to System")]
unsafe impl GlobalAlloc for BenchAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && ALLOCATION_TRACKING_ENABLED.load(Ordering::Relaxed) {
            ALLOCATION_COUNTERS.record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && ALLOCATION_TRACKING_ENABLED.load(Ordering::Relaxed) {
            ALLOCATION_COUNTERS.record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if ALLOCATION_TRACKING_ENABLED.load(Ordering::Relaxed) {
            ALLOCATION_COUNTERS.record_deallocation(layout.size());
        }
        // SAFETY: delegated with the original pointer and layout from the caller.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: delegated with the original pointer/layout and requested size.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if !replacement.is_null() && ALLOCATION_TRACKING_ENABLED.load(Ordering::Relaxed) {
            ALLOCATION_COUNTERS.record_deallocation(layout.size());
            ALLOCATION_COUNTERS.record_allocation(new_size);
        }
        replacement
    }
}

#[derive(Debug)]
struct AllocationCounters {
    allocated: AtomicU64,
    deallocated: AtomicU64,
    live: AtomicU64,
    high_water: AtomicU64,
    underflowed: AtomicBool,
}

impl AllocationCounters {
    const fn new() -> Self {
        Self {
            allocated: AtomicU64::new(0),
            deallocated: AtomicU64::new(0),
            live: AtomicU64::new(0),
            high_water: AtomicU64::new(0),
            underflowed: AtomicBool::new(false),
        }
    }

    fn record_allocation(&self, object_size: usize) {
        let bytes = object_size as u64;
        self.allocated.fetch_add(bytes, Ordering::Relaxed);
        let live = self.live.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.high_water.fetch_max(live, Ordering::Relaxed);
    }

    fn record_deallocation(&self, object_size: usize) {
        let bytes = object_size as u64;
        self.deallocated.fetch_add(bytes, Ordering::Relaxed);
        if self
            .live
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |live| live.checked_sub(bytes))
            .is_err()
        {
            self.underflowed.store(true, Ordering::Relaxed);
        }
    }
}

#[derive(Clone, Debug)]
struct Options {
    sizes: Vec<usize>,
    threads: Vec<usize>,
    repetitions: usize,
    profile: BenchProfile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BenchProfile {
    Standard,
    InMemoryPressure,
}

#[derive(Clone, Copy, Debug)]
enum ContentionMode {
    MetadataStatsHeavy,
    WriteHeavy,
    MixedStatsPut,
}

impl ContentionMode {
    fn label(self) -> &'static str {
        match self {
            Self::MetadataStatsHeavy => "metadata-stats-heavy",
            Self::WriteHeavy => "write-heavy",
            Self::MixedStatsPut => "mixed-stats-put",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum BrokerPairwiseMode {
    SingleOwner,
    PerThreadOwner,
    IndependentOwners,
}

impl BrokerPairwiseMode {
    fn label(self) -> &'static str {
        match self {
            Self::SingleOwner => "shared-single-owner",
            Self::PerThreadOwner => "shared-per-thread-owner",
            Self::IndependentOwners => "independent-per-thread-owner",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct AllocationSample {
    allocated: u64,
    deallocated: u64,
    live: u64,
    high_water: u64,
    underflowed: bool,
}

#[derive(Clone, Copy, Debug)]
struct LatencySample {
    p50: u64,
    p95: u64,
    p99: u64,
}

#[derive(Debug)]
struct WorkerSample {
    latencies: Vec<u64>,
    busy_retries: u64,
}

#[derive(Debug)]
struct PressureWorkerSample {
    latencies: Vec<u64>,
    admitted: Vec<u64>,
    denied: Vec<u64>,
    reclaim: Vec<u64>,
    reclaimed_operations: usize,
    busy_retries: u64,
}

#[derive(Clone, Copy, Debug)]
struct OperationCounts {
    successful: usize,
    denied: usize,
    outcome_latencies: Option<(LatencySample, LatencySample)>,
    detail_latencies: DetailLatencies,
}

#[derive(Clone, Copy, Debug, Default)]
enum DetailLatencies {
    #[default]
    None,
    Revocable {
        fill: LatencySample,
        reclaim: LatencySample,
    },
    ContendedPressure {
        reclaim: LatencySample,
        reclaimed_operations: usize,
    },
}

impl OperationCounts {
    fn all_successful(successful: usize) -> Self {
        Self {
            successful,
            denied: 0,
            outcome_latencies: None,
            detail_latencies: DetailLatencies::None,
        }
    }

    fn timed(self) -> usize {
        self.successful.checked_add(self.denied).expect("timed operation count")
    }
}

fn options() -> Options {
    let mut profile = BenchProfile::Standard;
    let mut smoke = false;
    let mut sizes = None;
    let mut threads = None;
    let mut repetitions = None;
    for argument in std::env::args().skip(1) {
        if argument == "--bench" {
            // Cargo passes this to harness=false bench targets.
            continue;
        } else if argument == "--smoke" {
            smoke = true;
        } else if let Some(raw) = argument.strip_prefix("--sizes=") {
            sizes = Some(parse_usize_list(raw, "--sizes"));
        } else if let Some(raw) = argument.strip_prefix("--threads=") {
            threads = Some(parse_usize_list(raw, "--threads"));
        } else if let Some(raw) = argument.strip_prefix("--repetitions=") {
            repetitions = Some(raw.parse().expect("--repetitions must be a usize"));
        } else if argument == "--profile=in-memory-pressure" {
            profile = BenchProfile::InMemoryPressure;
        } else {
            panic!("unknown benchmark argument {argument}");
        }
    }
    let default_threads = match (profile, smoke) {
        (BenchProfile::Standard, false) => vec![1, 2, 8, 32],
        (BenchProfile::Standard, true) => vec![1, 2],
        (BenchProfile::InMemoryPressure, false) => vec![1, 8, 32],
        (BenchProfile::InMemoryPressure, true) => vec![1, 8],
    };
    let options = Options {
        sizes: sizes.unwrap_or_else(|| if smoke { vec![1_000] } else { vec![1_000, 10_000] }),
        threads: threads.unwrap_or(default_threads),
        repetitions: repetitions.unwrap_or(if smoke { 3 } else { 5 }),
        profile,
    };
    assert!(options.sizes.iter().all(|entries| *entries > 0), "entries must be > 0");
    assert!(options.threads.iter().all(|threads| *threads > 0), "threads must be > 0");
    assert!(options.repetitions >= 3, "at least three repetitions are required");
    if options.profile == BenchProfile::InMemoryPressure {
        for threads in options.threads.iter().copied().filter(|threads| *threads > 1) {
            assert!(
                options.sizes.iter().all(|entries| *entries >= threads.saturating_mul(2)),
                "contended pressure requires at least two admission attempts per worker"
            );
        }
    }
    options
}

fn parse_usize_list(raw: &str, option: &str) -> Vec<usize> {
    raw.split(',')
        .map(|value| {
            value.parse::<usize>().unwrap_or_else(|_| panic!("{option} must contain usize values"))
        })
        .collect()
}

// `entries` is the number of timed admission pairs. The production policy
// bounds configured groups at 256, so benchmark cardinality scales within
// that bound: standard 1K/10K runs use 25/250 configured groups.
fn broker_group_cardinality(entries: usize) -> usize {
    entries.div_ceil(40).clamp(1, MAX_RESOURCE_GROUPS_PER_POLICY)
}

fn begin_allocation_epoch(counters: &AllocationCounters) {
    assert_eq!(
        counters.live.load(Ordering::Relaxed),
        0,
        "previous benchmark scenario leaked tracked allocations"
    );
    counters.allocated.store(0, Ordering::Relaxed);
    counters.deallocated.store(0, Ordering::Relaxed);
    counters.high_water.store(0, Ordering::Relaxed);
    counters.underflowed.store(false, Ordering::Relaxed);
    ALLOCATION_TRACKING_ENABLED.store(true, Ordering::Release);
}

fn end_allocation_epoch(counters: &AllocationCounters) -> AllocationSample {
    ALLOCATION_TRACKING_ENABLED.store(false, Ordering::Release);
    AllocationSample {
        allocated: counters.allocated.load(Ordering::Relaxed),
        deallocated: counters.deallocated.load(Ordering::Relaxed),
        live: counters.live.load(Ordering::Relaxed),
        high_water: counters.high_water.load(Ordering::Relaxed),
        underflowed: counters.underflowed.load(Ordering::Relaxed),
    }
}

/// Bench-local scoped allocation-epoch guard. The plain `begin`/`end` pair
/// leaves tracking enabled when the measured region panics, leaking the flag
/// into the next scenario. The guard fail-safes `Drop` to tracking-disabled
/// only: it never resets global counters and never consumes another
/// scenario's allocations. The normal path keeps the explicit `finish()` read.
struct AllocationEpochGuard {
    finished: bool,
}

impl AllocationEpochGuard {
    fn begin(counters: &AllocationCounters) -> Self {
        begin_allocation_epoch(counters);
        Self { finished: false }
    }

    fn finish(mut self, counters: &AllocationCounters) -> AllocationSample {
        self.finished = true;
        end_allocation_epoch(counters)
    }
}

impl Drop for AllocationEpochGuard {
    fn drop(&mut self) {
        if !self.finished {
            ALLOCATION_TRACKING_ENABLED.store(false, Ordering::Release);
        }
    }
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    let index = ((sorted.len() - 1) * percentile).div_ceil(100);
    sorted[index]
}

fn summarize_latencies(mut latencies: Vec<u64>) -> LatencySample {
    summarize_latencies_in_place(&mut latencies)
}

fn summarize_latencies_in_place(latencies: &mut [u64]) -> LatencySample {
    latencies.sort_unstable();
    LatencySample {
        p50: percentile(latencies, 50),
        p95: percentile(latencies, 95),
        p99: percentile(latencies, 99),
    }
}

#[allow(clippy::too_many_arguments)]
fn emit(
    operation: &str,
    mode: &str,
    entries: usize,
    threads: usize,
    repetition: usize,
    elapsed: Duration,
    latency: LatencySample,
    semantic_peak_bytes: u64,
    semantic_after_close_bytes: u64,
    allocation: AllocationSample,
    operations: OperationCounts,
    busy_retries: u64,
) {
    let elapsed_ns = elapsed.as_nanos();
    let timed_operations = operations.timed();
    let throughput = (timed_operations as f64 / elapsed.as_secs_f64()).round() as u64;
    let successful_throughput =
        (operations.successful as f64 / elapsed.as_secs_f64()).round() as u64;
    let group_cardinality_field = (operation == "broker_group_admission_cardinality")
        .then(|| format!(",\"group_cardinality\":{}", broker_group_cardinality(entries)))
        .unwrap_or_default();
    let outcome_latency_fields = operations.outcome_latencies.map_or_else(String::new, |(admitted, denied)| {
        format!(
            ",\"admitted_p50_ns\":{},\"admitted_p95_ns\":{},\"admitted_p99_ns\":{},\"denied_p50_ns\":{},\"denied_p95_ns\":{},\"denied_p99_ns\":{}",
            admitted.p50, admitted.p95, admitted.p99, denied.p50, denied.p95, denied.p99,
        )
    });
    let detail_latency_fields = match operations.detail_latencies {
        DetailLatencies::None => String::new(),
        DetailLatencies::Revocable { fill, reclaim } => format!(
            ",\"fill_p50_ns\":{},\"fill_p95_ns\":{},\"fill_p99_ns\":{},\"reclaim_p50_ns\":{},\"reclaim_p95_ns\":{},\"reclaim_p99_ns\":{}",
            fill.p50, fill.p95, fill.p99, reclaim.p50, reclaim.p95, reclaim.p99,
        ),
        DetailLatencies::ContendedPressure { reclaim, reclaimed_operations } => format!(
            ",\"reclaim_p50_ns\":{},\"reclaim_p95_ns\":{},\"reclaim_p99_ns\":{},\"reclaimed_operations\":{}",
            reclaim.p50, reclaim.p95, reclaim.p99, reclaimed_operations,
        ),
    };
    println!(
        concat!(
            "{{\"schema\":\"qmp-scaling-sample-v4\",",
            "\"operation\":\"{operation}\",\"mode\":\"{mode}\",",
            "\"entries\":{entries},\"threads\":{threads},\"repetition\":{repetition},",
            "\"timed_operations\":{timed_operations},",
            "\"successful_operations\":{successful_operations},",
            "\"denied_operations\":{denied_operations},",
            "\"elapsed_ns\":{elapsed_ns},\"throughput_ops_per_sec\":{throughput},",
            "\"successful_throughput_ops_per_sec\":{successful_throughput},",
            "\"p50_ns\":{p50},\"p95_ns\":{p95},\"p99_ns\":{p99},",
            "\"semantic_peak_bytes\":{semantic_peak_bytes},",
            "\"semantic_after_close_bytes\":{semantic_after_close_bytes},",
            "\"allocator_allocated_bytes\":{allocated},",
            "\"allocator_deallocated_bytes\":{deallocated},",
            "\"allocator_live_bytes\":{live},",
            "\"allocator_high_water_bytes\":{high_water},",
            "\"allocator_underflowed\":{underflowed},",
            "\"busy_retries\":{busy_retries}{group_cardinality_field}{outcome_latency_fields}{detail_latency_fields}}}"
        ),
        operation = operation,
        mode = mode,
        entries = entries,
        threads = threads,
        repetition = repetition,
        timed_operations = timed_operations,
        successful_operations = operations.successful,
        denied_operations = operations.denied,
        elapsed_ns = elapsed_ns,
        throughput = throughput,
        successful_throughput = successful_throughput,
        p50 = latency.p50,
        p95 = latency.p95,
        p99 = latency.p99,
        semantic_peak_bytes = semantic_peak_bytes,
        semantic_after_close_bytes = semantic_after_close_bytes,
        allocated = allocation.allocated,
        deallocated = allocation.deallocated,
        live = allocation.live,
        high_water = allocation.high_water,
        underflowed = allocation.underflowed,
        busy_retries = busy_retries,
        group_cardinality_field = group_cardinality_field,
        outcome_latency_fields = outcome_latency_fields,
        detail_latency_fields = detail_latency_fields,
    );
}

fn main() {
    let options = options();
    warm_up_process_runtime();
    let allocations = &ALLOCATION_COUNTERS;
    if options.profile == BenchProfile::InMemoryPressure {
        for entries in options.sizes {
            for repetition in 0..options.repetitions {
                broker_pressure_cycle(entries, repetition, allocations);
            }
            for threads in &options.threads {
                for repetition in 0..options.repetitions {
                    dual_store_write_contention(entries, *threads, repetition, allocations);
                    if *threads > 1 {
                        contended_pressure_reclaim(entries, *threads, repetition, allocations);
                    }
                }
            }
        }
        return;
    }
    for entries in options.sizes {
        for repetition in 0..options.repetitions {
            payload_unique_fill_and_dedup_put(entries, repetition, allocations);
            revocable_fill_and_drain(entries, repetition, allocations);
            broker_release_owner_cardinality(entries, repetition, allocations);
            broker_group_admission_cardinality(entries, repetition, allocations);
            retained_map_point_read(entries, repetition, true, allocations);
            retained_map_point_read(entries, repetition, false, allocations);
            for mode in [
                BrokerPairwiseMode::SingleOwner,
                BrokerPairwiseMode::PerThreadOwner,
                BrokerPairwiseMode::IndependentOwners,
            ] {
                broker_release_pairwise(entries, repetition, mode, allocations);
            }
        }
        for threads in &options.threads {
            for mode in [
                ContentionMode::MetadataStatsHeavy,
                ContentionMode::WriteHeavy,
                ContentionMode::MixedStatsPut,
            ] {
                for repetition in 0..options.repetitions {
                    payload_contention(entries, *threads, repetition, mode, allocations);
                }
            }
        }
    }
}

/// Force process-lifetime standard-library/runtime allocations to happen while
/// tracking is disabled. The benchmark owns scenario allocations, not one-time
/// main-thread hash seeding that intentionally survives the first store.
fn warm_up_process_runtime() {
    thread::spawn(|| {}).join().expect("warmup thread");
    let platform = new_platform();
    let store = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "bench:warmup".into(), None))
        .build()
        .expect("warmup payload store");
    store
        .put_canonical(RetainedEnvelope::new(0u64.to_be_bytes().to_vec()))
        .expect("warmup payload");
    store.close().expect("close warmup payload store");
    drop(store);
    assert_eq!(
        platform.broker().ledger_snapshot().expect("warmup ledger").total_owner_bytes(),
        0,
        "warmup must leave semantic accounting empty"
    );
}

fn new_platform() -> quanta_memory_platform_facade::LiveMemoryPlatform {
    MemoryPlatformBuilder::new()
        .with_manifest(PortableMemoryPresets::production())
        .build()
        .expect("production platform")
}

mod pressure;
mod throughput;
use pressure::*;
use throughput::*;
