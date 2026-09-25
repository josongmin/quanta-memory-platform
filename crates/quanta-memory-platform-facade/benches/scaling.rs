// This benchmark alone instruments GlobalAlloc. Product crates forbid unsafe
// code; the workspace deny policy requires this explicit target-local exception.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use quanta_memory_platform_core::{
    ChargeKind, ExecutionBudget, ManagedCharge, MemoryBudgetManifest, MemoryClass,
    MemoryClassBudget, MemoryContextKind, MemoryError, MemoryPreset, ReclaimPriority,
    ResidencyPolicy, ResourceGroup, ResourceGroupScheduler, RetainedEnvelope, StoreBindingHandle,
    WeightedResourceGroupScheduler,
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
    options
}

fn parse_usize_list(raw: &str, option: &str) -> Vec<usize> {
    raw.split(',')
        .map(|value| {
            value.parse::<usize>().unwrap_or_else(|_| panic!("{option} must contain usize values"))
        })
        .collect()
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
            "\"p50_ns\":{p50},\"p95_ns\":{p95},\"p99_ns\":{p99},",
            "\"semantic_peak_bytes\":{semantic_peak_bytes},",
            "\"semantic_after_close_bytes\":{semantic_after_close_bytes},",
            "\"allocator_allocated_bytes\":{allocated},",
            "\"allocator_deallocated_bytes\":{deallocated},",
            "\"allocator_live_bytes\":{live},",
            "\"allocator_high_water_bytes\":{high_water},",
            "\"allocator_underflowed\":{underflowed},",
            "\"busy_retries\":{busy_retries}{outcome_latency_fields}{detail_latency_fields}}}"
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
            scheduler_admission_group_cardinality(entries, repetition, allocations);
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

/// One live byte exactly fills the global/class cap. Each iteration measures
/// one typed denial and one admission after an untimed refund; wall throughput
/// includes the refund needed to make the next admission possible.
fn broker_pressure_cycle(entries: usize, repetition: usize, allocations: &AllocationCounters) {
    let class_budget = MemoryClassBudget {
        reserved_bytes: 0,
        soft_cap_bytes: 1,
        hard_cap_bytes: 1,
        borrow_from_shared_pool: false,
        reclaim_priority: ReclaimPriority::Normal,
    };
    begin_allocation_epoch(allocations);
    let manifest = MemoryBudgetManifest::new(
        MemoryPreset::Production,
        ExecutionBudget::default(),
        ResidencyPolicy::new(1).with_class_budget(MemoryClass::ArtifactCache, class_budget),
    );
    let platform =
        MemoryPlatformBuilder::new().with_manifest(manifest).build().expect("pressure platform");
    let broker = platform.broker();
    let owner = "bench:pressure".to_string();
    let mut charge = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            owner.clone(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        )
        .expect("prefill cap");
    let mut latencies = Vec::with_capacity(entries.checked_mul(2).expect("operation count"));
    let mut admitted = Vec::with_capacity(entries);
    let mut denied = Vec::with_capacity(entries);
    let started = Instant::now();
    for _ in 0..entries {
        let operation = Instant::now();
        assert!(matches!(
            broker.reserve_managed_charge(
                MemoryClass::ArtifactCache,
                owner.clone(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1,
            ),
            Err(MemoryError::Capacity(_))
        ));
        let elapsed = operation.elapsed().as_nanos() as u64;
        latencies.push(elapsed);
        denied.push(elapsed);

        charge.release().expect("refund one-byte authority");
        let operation = Instant::now();
        charge = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                owner.clone(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1,
            )
            .expect("readmit after refund");
        let elapsed = operation.elapsed().as_nanos() as u64;
        latencies.push(elapsed);
        admitted.push(elapsed);
    }
    let elapsed = started.elapsed();
    let snapshot = broker.ledger_snapshot().expect("pressure peak snapshot");
    assert_eq!(snapshot.total_owner_bytes(), 1);
    assert_eq!(snapshot.live_ticket_count, 1);
    drop(snapshot);
    charge.release().expect("final pressure refund");
    drop(charge);
    let after = broker.ledger_snapshot().expect("pressure final snapshot");
    assert_eq!(after.live_ticket_count, 0);
    let semantic_after = after.total_owner_bytes();
    drop(after);
    drop(owner);
    drop(platform);
    let latency = summarize_latencies(latencies);
    let outcome_latencies = (summarize_latencies(admitted), summarize_latencies(denied));
    let allocation = end_allocation_epoch(allocations);
    emit(
        "broker_pressure_cycle",
        "single-thread",
        entries,
        1,
        repetition,
        elapsed,
        latency,
        1,
        semantic_after,
        allocation,
        OperationCounts {
            successful: entries,
            denied: entries,
            outcome_latencies: Some(outcome_latencies),
            detail_latencies: DetailLatencies::None,
        },
        0,
    );
}

/// Two different charged stores publish through one broker. This isolates the
/// cross-store accounting lock from single-store payload contention without
/// pretending that a synthetic write mix is a host workload.
fn dual_store_write_contention(
    entries: usize,
    threads: usize,
    repetition: usize,
    allocations: &AllocationCounters,
) {
    begin_allocation_epoch(allocations);
    let platform = new_platform();
    let map = Arc::new(
        platform
            .retained_map_store_builder::<String, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::MemoResidency,
                "bench:dual-map".into(),
                None,
            ))
            .build(),
    );
    let payload = Arc::new(
        platform
            .payload_store_builder::<String, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ArtifactCache,
                "bench:dual-payload".into(),
                None,
            ))
            .build()
            .expect("dual-store payload"),
    );
    let barrier = Arc::new(Barrier::new(threads + 1));
    let mut handles = Vec::with_capacity(threads);
    for thread_id in 0..threads {
        let map = Arc::clone(&map);
        let payload = Arc::clone(&payload);
        let barrier = Arc::clone(&barrier);
        let count = entries / threads + usize::from(thread_id < entries % threads);
        handles.push(thread::spawn(move || {
            let mut latencies = Vec::with_capacity(count);
            let mut busy_retries = 0u64;
            barrier.wait();
            for operation_id in 0..count {
                if operation_id % 2 == 0 {
                    let key = format!("key-{thread_id}-{operation_id}");
                    let value = vec![0; 8];
                    let operation = Instant::now();
                    loop {
                        match map.insert_checked(key.clone(), value.clone()) {
                            Ok(()) => break,
                            Err(MemoryError::OperationInProgress(_)) => {
                                busy_retries = busy_retries.checked_add(1).expect("retry count");
                                thread::yield_now();
                            }
                            Err(error) => panic!("dual-store map write: {error}"),
                        }
                    }
                    latencies.push(operation.elapsed().as_nanos() as u64);
                } else {
                    let unique = ((thread_id as u64) << 32) | (operation_id as u64 + 1);
                    let payload_bytes = unique.to_be_bytes().to_vec();
                    let operation = Instant::now();
                    loop {
                        match payload.put_canonical(RetainedEnvelope::new(payload_bytes.clone())) {
                            Ok(published) => {
                                black_box(published);
                                break;
                            }
                            Err(MemoryError::OperationInProgress(_)) => {
                                busy_retries = busy_retries.checked_add(1).expect("retry count");
                                thread::yield_now();
                            }
                            Err(error) => panic!("dual-store payload write: {error}"),
                        }
                    }
                    latencies.push(operation.elapsed().as_nanos() as u64);
                }
            }
            WorkerSample { latencies, busy_retries }
        }));
    }
    let started = Instant::now();
    barrier.wait();
    let mut latencies = Vec::with_capacity(entries);
    let mut busy_retries = 0u64;
    for handle in handles {
        let worker = handle.join().expect("dual-store worker");
        latencies.extend(worker.latencies);
        busy_retries = busy_retries.checked_add(worker.busy_retries).expect("retry count");
    }
    let elapsed = started.elapsed();
    assert_eq!(latencies.len(), entries);
    let map_items: usize = (0..threads)
        .map(|thread_id| {
            let count = entries / threads + usize::from(thread_id < entries % threads);
            count.div_ceil(2)
        })
        .sum();
    assert_eq!(map.stats_checked().expect("dual map stats").item_count, map_items);
    assert_eq!(payload.stats().expect("dual payload stats").item_count, entries - map_items);
    let semantic_peak =
        platform.broker().ledger_snapshot().expect("dual peak ledger").total_owner_bytes();
    assert!(semantic_peak > 0);
    map.close().expect("close dual map");
    payload.close().expect("close dual payload");
    drop(map);
    drop(payload);
    drop(barrier);
    let after = platform.broker().ledger_snapshot().expect("dual final ledger");
    assert_eq!(after.live_ticket_count, 0);
    let semantic_after = after.total_owner_bytes();
    drop(platform);
    let latency = summarize_latencies(latencies);
    let allocation = end_allocation_epoch(allocations);
    emit(
        "dual_store_write_contention",
        "shared-broker",
        entries,
        threads,
        repetition,
        elapsed,
        latency,
        semantic_peak,
        semantic_after,
        allocation,
        OperationCounts::all_successful(entries),
        busy_retries,
    );
}

/// Two revocable instances compete for one two-entry class/global cap. A
/// capacity denial triggers an explicit reclaim, so the fixture repeatedly
/// crosses the full/not-full boundary while writers contend. Admission calls
/// are the throughput denominator; wall time also includes reclaim episodes.
fn contended_pressure_reclaim(
    entries: usize,
    threads: usize,
    repetition: usize,
    allocations: &AllocationCounters,
) {
    const ENTRY_BYTES: u64 = 48; // 8-byte key + 8-byte value + 32-byte revocable overhead.
    const CAP_BYTES: u64 = ENTRY_BYTES * 2;
    const MAX_BUSY_RETRIES: u64 = 100_000;
    let class_budget = MemoryClassBudget {
        reserved_bytes: 0,
        soft_cap_bytes: CAP_BYTES,
        hard_cap_bytes: CAP_BYTES,
        borrow_from_shared_pool: false,
        reclaim_priority: ReclaimPriority::Normal,
    };
    begin_allocation_epoch(allocations);
    let manifest = MemoryBudgetManifest::new(
        MemoryPreset::Production,
        ExecutionBudget::default(),
        ResidencyPolicy::new(CAP_BYTES).with_class_budget(MemoryClass::ArtifactCache, class_budget),
    );
    let platform = MemoryPlatformBuilder::new()
        .with_manifest(manifest)
        .build()
        .expect("contended pressure platform");
    let left = Arc::new(
        platform
            .revocable_store_builder::<Vec<u8>, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ArtifactCache,
                "bench:pressure-left".into(),
                None,
            ))
            .build()
            .expect("left revocable store"),
    );
    let right = Arc::new(
        platform
            .revocable_store_builder::<Vec<u8>, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ArtifactCache,
                "bench:pressure-right".into(),
                None,
            ))
            .build()
            .expect("right revocable store"),
    );
    left.insert_checked(vec![0; 8], vec![0; 8]).expect("seed left slot");
    right.insert_checked(vec![1; 8], vec![1; 8]).expect("seed right slot");
    assert_eq!(left.stats().expect("left seed stats").estimated_bytes, ENTRY_BYTES);
    assert_eq!(right.stats().expect("right seed stats").estimated_bytes, ENTRY_BYTES);
    let seed = platform.broker().ledger_snapshot().expect("pressure seed ledger");
    assert_eq!(seed.total_owner_bytes(), CAP_BYTES);
    assert_eq!(seed.live_ticket_count, 2);
    drop(seed);

    let barrier = Arc::new(Barrier::new(threads + 1));
    let mut handles = Vec::with_capacity(threads);
    for thread_id in 0..threads {
        let left = Arc::clone(&left);
        let right = Arc::clone(&right);
        let barrier = Arc::clone(&barrier);
        let count = entries / threads + usize::from(thread_id < entries % threads);
        handles.push(thread::spawn(move || {
            let mut sample = PressureWorkerSample {
                latencies: Vec::with_capacity(count),
                admitted: Vec::with_capacity(count),
                denied: Vec::with_capacity(count),
                reclaim: Vec::with_capacity(count),
                reclaimed_operations: 0,
                busy_retries: 0,
            };
            let (own, peer) = if thread_id % 2 == 0 { (&left, &right) } else { (&right, &left) };
            barrier.wait();
            for operation_id in 0..count {
                let key =
                    (((thread_id as u64) << 32) | (operation_id as u64 + 2)).to_be_bytes().to_vec();
                let operation = Instant::now();
                let mut admission_busy_retries = 0;
                let result = loop {
                    match own.insert_checked(key.clone(), vec![0; 8]) {
                        Err(MemoryError::OperationInProgress(_)) => {
                            sample.busy_retries += 1;
                            admission_busy_retries += 1;
                            assert!(
                                admission_busy_retries <= MAX_BUSY_RETRIES,
                                "pressure insert stalled"
                            );
                            thread::yield_now();
                        }
                        result => break result,
                    }
                };
                let latency = operation.elapsed().as_nanos() as u64;
                sample.latencies.push(latency);
                match result {
                    Ok(()) => sample.admitted.push(latency),
                    Err(MemoryError::Capacity(_)) => {
                        sample.denied.push(latency);
                        let reclaim_started = Instant::now();
                        let mut reclaimed = false;
                        for store in [own, peer] {
                            let mut reclaim_busy_retries = 0;
                            let outcome = loop {
                                match store.reclaim_one() {
                                    Err(MemoryError::OperationInProgress(_)) => {
                                        sample.busy_retries += 1;
                                        reclaim_busy_retries += 1;
                                        assert!(
                                            reclaim_busy_retries <= MAX_BUSY_RETRIES,
                                            "pressure reclaim stalled"
                                        );
                                        thread::yield_now();
                                    }
                                    result => break result,
                                }
                            };
                            match outcome {
                                Ok(Some(entry)) => {
                                    drop(black_box(entry));
                                    reclaimed = true;
                                    break;
                                }
                                Ok(None) => {}
                                Err(error) => panic!("pressure reclaim: {error}"),
                            }
                        }
                        if reclaimed {
                            sample.reclaim.push(reclaim_started.elapsed().as_nanos() as u64);
                            sample.reclaimed_operations += 1;
                        }
                    }
                    Err(error) => panic!("pressure insert: {error}"),
                }
            }
            sample
        }));
    }
    let started = Instant::now();
    barrier.wait();
    let mut latencies = Vec::with_capacity(entries);
    let mut admitted = Vec::with_capacity(entries);
    let mut denied = Vec::with_capacity(entries);
    let mut reclaim = Vec::with_capacity(entries);
    let mut reclaimed_operations = 0;
    let mut busy_retries = 0;
    for handle in handles {
        let sample = handle.join().expect("pressure worker");
        latencies.extend(sample.latencies);
        admitted.extend(sample.admitted);
        denied.extend(sample.denied);
        reclaim.extend(sample.reclaim);
        reclaimed_operations += sample.reclaimed_operations;
        busy_retries += sample.busy_retries;
    }
    let elapsed = started.elapsed();
    assert_eq!(latencies.len(), entries);
    assert!(!admitted.is_empty(), "pressure churn must admit after reclaim");
    assert!(!denied.is_empty(), "pressure churn must hit the hard cap");
    assert!(reclaimed_operations > 0, "pressure churn must reclaim a live entry");
    assert_eq!(reclaim.len(), reclaimed_operations);
    let final_live = platform.broker().ledger_snapshot().expect("pressure final live ledger");
    let left_stats = left.stats().expect("pressure left final stats");
    let right_stats = right.stats().expect("pressure right final stats");
    assert_eq!(
        left_stats.estimated_bytes + right_stats.estimated_bytes,
        final_live.total_owner_bytes()
    );
    assert_eq!(left_stats.item_count + right_stats.item_count, final_live.live_ticket_count);
    assert!(final_live.total_owner_bytes() <= CAP_BYTES);
    assert!(final_live.live_ticket_count <= 2);
    drop(final_live);
    left.close().expect("close left pressure store");
    right.close().expect("close right pressure store");
    drop(left);
    drop(right);
    drop(barrier);
    let after = platform.broker().ledger_snapshot().expect("pressure closed ledger");
    assert_eq!(after.live_ticket_count, 0);
    let semantic_after = after.total_owner_bytes();
    drop(platform);
    let admitted_count = admitted.len();
    let denied_count = denied.len();
    let latency = summarize_latencies(latencies);
    let outcome_latencies = (summarize_latencies(admitted), summarize_latencies(denied));
    let reclaim_latency = summarize_latencies(reclaim);
    let allocation = end_allocation_epoch(allocations);
    emit(
        "contended_pressure_reclaim",
        "two-revocable-stores",
        entries,
        threads,
        repetition,
        elapsed,
        latency,
        CAP_BYTES,
        semantic_after,
        allocation,
        OperationCounts {
            successful: admitted_count,
            denied: denied_count,
            outcome_latencies: Some(outcome_latencies),
            detail_latencies: DetailLatencies::ContendedPressure {
                reclaim: reclaim_latency,
                reclaimed_operations,
            },
        },
        busy_retries,
    );
}

fn payload_unique_fill_and_dedup_put(
    entries: usize,
    repetition: usize,
    allocations: &AllocationCounters,
) {
    begin_allocation_epoch(allocations);
    let platform = new_platform();
    let store = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "bench:payload".into(), None))
        .build()
        .expect("payload store");
    let mut latencies = Vec::with_capacity(entries);
    let started = Instant::now();
    for value in 0..entries {
        let operation = Instant::now();
        black_box(
            store
                .put_canonical(RetainedEnvelope::new((value as u64).to_be_bytes().to_vec()))
                .expect("unique payload"),
        );
        latencies.push(operation.elapsed().as_nanos() as u64);
    }
    let elapsed = started.elapsed();
    let semantic_peak = store.charged_bytes().expect("charged bytes");
    store.close().expect("close payload store");
    drop(store);
    let semantic_after =
        platform.broker().ledger_snapshot().expect("ledger after close").total_owner_bytes();
    drop(platform);
    let latency = summarize_latencies(latencies);
    let allocation = end_allocation_epoch(allocations);
    emit(
        "payload_unique_fill",
        "single-thread",
        entries,
        1,
        repetition,
        elapsed,
        latency,
        semantic_peak,
        semantic_after,
        allocation,
        OperationCounts::all_successful(entries),
        0,
    );

    begin_allocation_epoch(allocations);
    let platform = new_platform();
    let store = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "bench:dedup".into(), None))
        .build()
        .expect("payload store");
    store.put_canonical(RetainedEnvelope::new(0u64.to_be_bytes().to_vec())).expect("dedup seed");
    let mut latencies = Vec::with_capacity(entries);
    let started = Instant::now();
    for _ in 0..entries {
        let operation = Instant::now();
        black_box(
            store
                .put_canonical(RetainedEnvelope::new(0u64.to_be_bytes().to_vec()))
                .expect("dedup payload"),
        );
        latencies.push(operation.elapsed().as_nanos() as u64);
    }
    let elapsed = started.elapsed();
    let semantic_peak = store.charged_bytes().expect("charged bytes");
    store.close().expect("close payload store");
    drop(store);
    let semantic_after =
        platform.broker().ledger_snapshot().expect("ledger after close").total_owner_bytes();
    drop(platform);
    let latency = summarize_latencies(latencies);
    let allocation = end_allocation_epoch(allocations);
    emit(
        "payload_dedup_put",
        "single-thread",
        entries,
        1,
        repetition,
        elapsed,
        latency,
        semantic_peak,
        semantic_after,
        allocation,
        OperationCounts::all_successful(entries),
        0,
    );
}

fn revocable_fill_and_drain(entries: usize, repetition: usize, allocations: &AllocationCounters) {
    begin_allocation_epoch(allocations);
    let platform = new_platform();
    let store = platform
        .revocable_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "bench:revocable".into(), None))
        .build()
        .expect("revocable store");
    let mut fill_latencies = Vec::with_capacity(entries);
    let mut reclaim_latencies = Vec::with_capacity(entries);
    let fill_started = Instant::now();
    for value in 0..entries {
        let operation = Instant::now();
        store.insert_checked(format!("key-{value}"), vec![0; 8]).expect("revocable insert");
        fill_latencies.push(operation.elapsed().as_nanos() as u64);
    }
    let fill_elapsed = fill_started.elapsed();
    let semantic_peak = store.stats().expect("stats").estimated_bytes;
    let drain_started = Instant::now();
    for _ in 0..entries {
        let operation = Instant::now();
        black_box(store.reclaim_one().expect("reclaim").expect("entry"));
        reclaim_latencies.push(operation.elapsed().as_nanos() as u64);
    }
    let elapsed = fill_elapsed + drain_started.elapsed();
    store.close().expect("close revocable store");
    drop(store);
    let semantic_after =
        platform.broker().ledger_snapshot().expect("ledger after close").total_owner_bytes();
    drop(platform);
    let fill_latency = summarize_latencies_in_place(&mut fill_latencies);
    let reclaim_latency = summarize_latencies_in_place(&mut reclaim_latencies);
    fill_latencies.extend(reclaim_latencies);
    let latency = summarize_latencies(fill_latencies);
    let allocation = end_allocation_epoch(allocations);
    emit(
        "revocable_fill_and_drain",
        "single-thread",
        entries,
        1,
        repetition,
        elapsed,
        latency,
        semantic_peak,
        semantic_after,
        allocation,
        OperationCounts {
            detail_latencies: DetailLatencies::Revocable {
                fill: fill_latency,
                reclaim: reclaim_latency,
            },
            ..OperationCounts::all_successful(entries.checked_mul(2).expect("operation count"))
        },
        0,
    );
}

fn broker_release_owner_cardinality(
    entries: usize,
    repetition: usize,
    allocations: &AllocationCounters,
) {
    begin_allocation_epoch(allocations);
    let platform = new_platform();
    let mut charges = Vec::with_capacity(entries);
    for owner in 0..entries {
        charges.push(
            platform
                .broker()
                .reserve_managed_charge(
                    MemoryClass::ArtifactCache,
                    format!("bench:owner:{owner}"),
                    MemoryContextKind::Request,
                    ChargeKind::RetainedPayload,
                    1,
                )
                .expect("owner-cardinality reserve"),
        );
    }
    let semantic_peak = entries as u64;
    let mut latencies = Vec::with_capacity(entries);
    let started = Instant::now();
    for charge in &mut charges {
        let operation = Instant::now();
        charge.release().expect("owner-cardinality release");
        latencies.push(operation.elapsed().as_nanos() as u64);
    }
    let elapsed = started.elapsed();
    drop(charges);
    let semantic_after =
        platform.broker().ledger_snapshot().expect("ledger after release").total_owner_bytes();
    drop(platform);
    let latency = summarize_latencies(latencies);
    let allocation = end_allocation_epoch(allocations);
    emit(
        "broker_release_owner_cardinality",
        "single-thread",
        entries,
        1,
        repetition,
        elapsed,
        latency,
        semantic_peak,
        semantic_after,
        allocation,
        OperationCounts::all_successful(entries),
        0,
    );
}

/// Only release is timed. One-byte charges are staged before the barrier so
/// broker sharing and owner cardinality are the variables under comparison,
/// not fixture construction or reserve/commit behavior.
fn broker_release_pairwise(
    entries: usize,
    repetition: usize,
    mode: BrokerPairwiseMode,
    allocations: &AllocationCounters,
) {
    const THREADS: usize = 2;
    begin_allocation_epoch(allocations);
    let platforms = if matches!(mode, BrokerPairwiseMode::IndependentOwners) {
        vec![new_platform(), new_platform()]
    } else {
        vec![new_platform()]
    };
    let mut charges_by_thread: Vec<Vec<ManagedCharge>> = Vec::with_capacity(THREADS);
    for thread_id in 0..THREADS {
        let count = entries / THREADS + usize::from(thread_id < entries % THREADS);
        let platform_index =
            if matches!(mode, BrokerPairwiseMode::IndependentOwners) { thread_id } else { 0 };
        let owner = if matches!(mode, BrokerPairwiseMode::SingleOwner) {
            "bench:pairwise:shared".to_string()
        } else {
            format!("bench:pairwise:owner:{thread_id}")
        };
        let mut charges = Vec::with_capacity(count);
        for _ in 0..count {
            charges.push(
                platforms[platform_index]
                    .broker()
                    .reserve_managed_charge(
                        MemoryClass::ArtifactCache,
                        owner.clone(),
                        MemoryContextKind::Request,
                        ChargeKind::RetainedPayload,
                        1,
                    )
                    .expect("pairwise reserve"),
            );
        }
        charges_by_thread.push(charges);
    }
    let semantic_peak: u64 = platforms
        .iter()
        .map(|platform| {
            platform.broker().ledger_snapshot().expect("peak ledger").total_owner_bytes()
        })
        .sum();
    assert_eq!(semantic_peak, entries as u64, "staged one-byte charges");

    let ready = Arc::new(Barrier::new(THREADS + 1));
    let start = Arc::new(Barrier::new(THREADS + 1));
    let mut handles = Vec::with_capacity(THREADS);
    for mut charges in charges_by_thread {
        let ready = Arc::clone(&ready);
        let start = Arc::clone(&start);
        handles.push(thread::spawn(move || {
            let mut latencies = Vec::with_capacity(charges.len());
            ready.wait();
            start.wait();
            for charge in &mut charges {
                let operation = Instant::now();
                charge.release().expect("pairwise release");
                latencies.push(operation.elapsed().as_nanos() as u64);
            }
            WorkerSample { latencies, busy_retries: 0 }
        }));
    }
    ready.wait();
    let started = Instant::now();
    start.wait();
    let mut latencies = Vec::with_capacity(entries);
    for handle in handles {
        let worker = handle.join().expect("pairwise worker");
        latencies.extend(worker.latencies);
    }
    let elapsed = started.elapsed();
    assert_eq!(latencies.len(), entries);
    let semantic_after: u64 = platforms
        .iter()
        .map(|platform| {
            platform.broker().ledger_snapshot().expect("post-release ledger").total_owner_bytes()
        })
        .sum();
    drop(platforms);
    drop(ready);
    drop(start);
    let latency = summarize_latencies(latencies);
    let allocation = end_allocation_epoch(allocations);
    emit(
        "broker_release_pairwise",
        mode.label(),
        entries,
        THREADS,
        repetition,
        elapsed,
        latency,
        semantic_peak,
        semantic_after,
        allocation,
        OperationCounts::all_successful(entries),
        0,
    );
}

/// Real point reads against a charged retained map. Queries are prebuilt so
/// formatting does not dominate the timed `get_checked` hit/miss measurement.
fn retained_map_point_read(
    entries: usize,
    repetition: usize,
    hit: bool,
    allocations: &AllocationCounters,
) {
    begin_allocation_epoch(allocations);
    let platform = new_platform();
    let store = platform
        .retained_map_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(
            MemoryClass::ArtifactCache,
            "bench:retained-read".into(),
            None,
        ))
        .build();
    for value in 0..entries {
        store.insert_checked(format!("key-{value}"), vec![0; 8]).expect("retained-map seed");
    }
    let semantic_peak = store.stats_checked().expect("retained-map stats").estimated_bytes;
    let queries: Vec<String> = (0..entries)
        .map(|value| if hit { format!("key-{value}") } else { format!("missing-{value}") })
        .collect();
    let mut latencies = Vec::with_capacity(entries);
    let started = Instant::now();
    for query in &queries {
        let operation = Instant::now();
        let found = store.get_checked(black_box(query)).expect("retained-map point read");
        latencies.push(operation.elapsed().as_nanos() as u64);
        assert_eq!(found.is_some(), hit, "point-read fixture");
        black_box(found);
    }
    let elapsed = started.elapsed();
    drop(queries);
    store.close().expect("close retained map");
    drop(store);
    let semantic_after =
        platform.broker().ledger_snapshot().expect("ledger after close").total_owner_bytes();
    drop(platform);
    let latency = summarize_latencies(latencies);
    let allocation = end_allocation_epoch(allocations);
    emit(
        "retained_map_point_read",
        if hit { "hit" } else { "miss" },
        entries,
        1,
        repetition,
        elapsed,
        latency,
        semantic_peak,
        semantic_after,
        allocation,
        OperationCounts::all_successful(entries),
        0,
    );
}

fn scheduler_admission_group_cardinality(
    entries: usize,
    repetition: usize,
    allocations: &AllocationCounters,
) {
    // Phase 1: fixture construction with tracking disabled. Everything
    // allocated here (groups, target, scheduler, latency buffer) must be
    // dropped after the epoch closes, never while tracked.
    let mut groups = BTreeMap::new();
    for index in 0..entries {
        groups.insert(
            format!("group-{index:08}"),
            ResourceGroup {
                reserved_bytes: 0,
                soft_cap_bytes: 500,
                hard_cap_bytes: 1_000,
                burst_percent: 100,
                reclaim_priority: ReclaimPriority::Normal,
                admission_weight: 1,
            },
        );
    }
    let target = format!("group-{:08}", entries / 2);
    let mut scheduler = WeightedResourceGroupScheduler::new(groups, entries as u64);
    scheduler.record_usage(&target, 1_000).expect("record target hard-cap usage");
    let mut latencies = Vec::with_capacity(entries.checked_mul(2).expect("operation count"));

    // Phase 2: open the epoch immediately before the admission loop and
    // require a clean precondition.
    let epoch = AllocationEpochGuard::begin(allocations);
    assert_eq!(
        allocations.allocated.load(Ordering::Relaxed),
        0,
        "allocation epoch must start with zero tracked allocations"
    );
    assert_eq!(
        allocations.deallocated.load(Ordering::Relaxed),
        0,
        "allocation epoch must start with zero tracked deallocations"
    );
    assert_eq!(
        allocations.live.load(Ordering::Relaxed),
        0,
        "allocation epoch must start with zero tracked live bytes"
    );

    // Phase 3: admission loop only.
    let started = Instant::now();
    for _ in 0..entries {
        let operation = Instant::now();
        assert!(scheduler.admits(&target, 1), "cached one-byte fair share must admit");
        let latency = operation.elapsed().as_nanos() as u64;
        latencies.push(latency);
        let operation = Instant::now();
        assert!(!scheduler.admits(&target, 2), "request beyond cached share must deny");
        let latency = operation.elapsed().as_nanos() as u64;
        latencies.push(latency);
    }
    let elapsed = started.elapsed();
    assert_eq!(
        allocations.allocated.load(Ordering::Relaxed),
        0,
        "scheduler admission must not allocate after policy construction"
    );
    assert_eq!(
        allocations.deallocated.load(Ordering::Relaxed),
        0,
        "scheduler admission must not deallocate policy state"
    );

    // Phase 4: stop tracking before touching fixture memory again.
    let allocation = epoch.finish(allocations);
    assert!(
        !ALLOCATION_TRACKING_ENABLED.load(Ordering::Relaxed),
        "tracker must be inactive after the admission measurement"
    );

    // Phase 5: summarize and drop with tracking disabled.
    // Split the existing alternating samples only after the timed loop and
    // allocation epoch. This keeps throughput comparable to prior v3 runs.
    let admitted_latencies: Vec<u64> = latencies.iter().step_by(2).copied().collect();
    let denied_latencies: Vec<u64> = latencies.iter().skip(1).step_by(2).copied().collect();
    let latency = summarize_latencies(latencies);
    let outcome_latencies =
        (summarize_latencies(admitted_latencies), summarize_latencies(denied_latencies));
    drop(scheduler);
    drop(target);
    emit(
        "scheduler_admission_group_cardinality",
        "single-thread",
        entries,
        1,
        repetition,
        elapsed,
        latency,
        0,
        0,
        allocation,
        OperationCounts {
            successful: entries,
            denied: entries,
            outcome_latencies: Some(outcome_latencies),
            detail_latencies: DetailLatencies::None,
        },
        0,
    );
}

fn payload_contention(
    entries: usize,
    threads: usize,
    repetition: usize,
    mode: ContentionMode,
    allocations: &AllocationCounters,
) {
    begin_allocation_epoch(allocations);
    let platform = new_platform();
    let store = Arc::new(
        platform
            .payload_store_builder::<String, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ArtifactCache,
                format!("bench:contention:{}:{threads}", mode.label()),
                None,
            ))
            .build()
            .expect("payload store"),
    );
    if matches!(mode, ContentionMode::MetadataStatsHeavy | ContentionMode::MixedStatsPut) {
        store
            .put_canonical(RetainedEnvelope::new(0u64.to_be_bytes().to_vec()))
            .expect("dedup seed");
    }
    let barrier = Arc::new(Barrier::new(threads + 1));
    let mut handles = Vec::with_capacity(threads);
    for thread_id in 0..threads {
        let store = store.clone();
        let barrier = barrier.clone();
        let count = entries / threads + usize::from(thread_id < entries % threads);
        handles.push(thread::spawn(move || {
            let mut latencies = Vec::with_capacity(count);
            let mut busy_retries = 0u64;
            barrier.wait();
            for operation_id in 0..count {
                let operation = Instant::now();
                let thread_slot = u32::try_from(thread_id).expect("thread ID fits payload key");
                let operation_slot = u32::try_from(operation_id)
                    .expect("operation ID fits payload key")
                    .checked_add(1)
                    .expect("nonzero operation slot fits payload key");
                let unique = (u64::from(thread_slot) << 32) | u64::from(operation_slot);
                let is_metadata_stats = matches!(mode, ContentionMode::MetadataStatsHeavy)
                    || matches!(mode, ContentionMode::MixedStatsPut) && operation_id % 2 == 0;
                if is_metadata_stats {
                    loop {
                        match store.stats() {
                            Ok(stats) => {
                                black_box(stats);
                                break;
                            }
                            Err(MemoryError::OperationInProgress(_)) => {
                                busy_retries = busy_retries.checked_add(1).expect("retry count");
                                thread::yield_now();
                            }
                            Err(error) => panic!("contention metadata stats: {error}"),
                        }
                    }
                } else {
                    loop {
                        match store
                            .put_canonical(RetainedEnvelope::new(unique.to_be_bytes().to_vec()))
                        {
                            Ok(published) => {
                                black_box(published);
                                break;
                            }
                            Err(MemoryError::OperationInProgress(_)) => {
                                busy_retries = busy_retries.checked_add(1).expect("retry count");
                                thread::yield_now();
                            }
                            Err(error) => panic!("contention payload operation: {error}"),
                        }
                    }
                }
                latencies.push(operation.elapsed().as_nanos() as u64);
            }
            WorkerSample { latencies, busy_retries }
        }));
    }
    // Start before releasing workers: some may finish their first operation
    // before the main thread returns from the barrier.
    let started = Instant::now();
    barrier.wait();
    let mut latencies = Vec::with_capacity(entries);
    let mut busy_retries = 0u64;
    for handle in handles {
        let worker = handle.join().expect("contention worker");
        latencies.extend(worker.latencies);
        busy_retries = busy_retries.checked_add(worker.busy_retries).expect("retry count");
    }
    let elapsed = started.elapsed();
    assert_eq!(latencies.len(), entries);
    let expected_items = match mode {
        ContentionMode::MetadataStatsHeavy => 1,
        ContentionMode::WriteHeavy => entries,
        ContentionMode::MixedStatsPut => {
            1 + (0..threads)
                .map(|thread_id| {
                    let count = entries / threads + usize::from(thread_id < entries % threads);
                    count / 2
                })
                .sum::<usize>()
        }
    };
    assert_eq!(
        store.stats().expect("contention stats").item_count,
        expected_items,
        "contention workload must retain every unique write exactly once"
    );
    let semantic_peak = store.charged_bytes().expect("charged bytes");
    assert!(semantic_peak > 0, "contention workload must retain semantic bytes");
    store.close().expect("close payload store");
    drop(store);
    drop(barrier);
    let semantic_after =
        platform.broker().ledger_snapshot().expect("ledger after close").total_owner_bytes();
    drop(platform);
    let latency = summarize_latencies(latencies);
    let allocation = end_allocation_epoch(allocations);
    emit(
        "payload_contention",
        mode.label(),
        entries,
        threads,
        repetition,
        elapsed,
        latency,
        semantic_peak,
        semantic_after,
        allocation,
        OperationCounts::all_successful(entries),
        busy_retries,
    );
}
