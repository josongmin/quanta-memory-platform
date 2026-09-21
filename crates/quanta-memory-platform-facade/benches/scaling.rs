use std::alloc::System;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use quanta_memory_platform_core::{
    ChargeKind, MemoryClass, MemoryContextKind, MemoryError, RetainedEnvelope, StoreBindingHandle,
};
use quanta_memory_platform_facade::{
    MemoryPlatformBuilder, PayloadStore, PortableMemoryPresets, RevocableStore,
};
use tracking_allocator::{AllocationGroupId, AllocationRegistry, AllocationTracker, Allocator};

#[global_allocator]
static GLOBAL: Allocator<System> = Allocator::system();

#[derive(Debug, Default)]
struct AllocationCounters {
    allocated: AtomicU64,
    deallocated: AtomicU64,
    live: AtomicU64,
    high_water: AtomicU64,
}

#[derive(Clone, Debug)]
struct BenchAllocationTracker(Arc<AllocationCounters>);

impl AllocationTracker for BenchAllocationTracker {
    fn allocated(
        &self,
        _addr: usize,
        object_size: usize,
        _wrapped_size: usize,
        _group_id: AllocationGroupId,
    ) {
        let bytes = object_size as u64;
        self.0.allocated.fetch_add(bytes, Ordering::Relaxed);
        let live = self.0.live.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.0.high_water.fetch_max(live, Ordering::Relaxed);
    }

    fn deallocated(
        &self,
        _addr: usize,
        object_size: usize,
        _wrapped_size: usize,
        _source_group_id: AllocationGroupId,
        _current_group_id: AllocationGroupId,
    ) {
        let bytes = object_size as u64;
        self.0.deallocated.fetch_add(bytes, Ordering::Relaxed);
        self.0.live.fetch_sub(bytes, Ordering::Relaxed);
    }
}

#[derive(Clone, Debug)]
struct Options {
    sizes: Vec<usize>,
    threads: Vec<usize>,
    repetitions: usize,
}

#[derive(Clone, Copy, Debug)]
enum ContentionMode {
    ReadHeavy,
    WriteHeavy,
    Mixed,
}

impl ContentionMode {
    fn label(self) -> &'static str {
        match self {
            Self::ReadHeavy => "read-heavy",
            Self::WriteHeavy => "write-heavy",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct AllocationSample {
    allocated: u64,
    deallocated: u64,
    live: u64,
    high_water: u64,
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

fn options() -> Options {
    let mut options =
        Options { sizes: vec![1_000, 10_000], threads: vec![1, 2, 8, 32], repetitions: 5 };
    for argument in std::env::args().skip(1) {
        if argument == "--smoke" {
            options.sizes = vec![1_000];
            options.threads = vec![1, 2];
            options.repetitions = 3;
        } else if let Some(raw) = argument.strip_prefix("--sizes=") {
            options.sizes = parse_usize_list(raw, "--sizes");
        } else if let Some(raw) = argument.strip_prefix("--threads=") {
            options.threads = parse_usize_list(raw, "--threads");
            assert!(options.threads.iter().all(|threads| *threads > 0), "threads must be > 0");
        } else if let Some(raw) = argument.strip_prefix("--repetitions=") {
            options.repetitions = raw.parse().expect("--repetitions must be a usize");
            assert!(options.repetitions >= 3, "at least three repetitions are required");
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

fn install_tracker() -> Arc<AllocationCounters> {
    let counters = Arc::new(AllocationCounters::default());
    AllocationRegistry::set_global_tracker(BenchAllocationTracker(counters.clone()))
        .expect("benchmark installs the process tracker exactly once");
    counters
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
    AllocationRegistry::enable_tracking();
}

fn end_allocation_epoch(counters: &AllocationCounters) -> AllocationSample {
    AllocationRegistry::disable_tracking();
    AllocationSample {
        allocated: counters.allocated.load(Ordering::Relaxed),
        deallocated: counters.deallocated.load(Ordering::Relaxed),
        live: counters.live.load(Ordering::Relaxed),
        high_water: counters.high_water.load(Ordering::Relaxed),
    }
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    let index = ((sorted.len() - 1) * percentile).div_ceil(100);
    sorted[index]
}

fn summarize_latencies(mut latencies: Vec<u64>) -> LatencySample {
    latencies.sort_unstable();
    LatencySample {
        p50: percentile(&latencies, 50),
        p95: percentile(&latencies, 95),
        p99: percentile(&latencies, 99),
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
    busy_retries: u64,
) {
    let elapsed_ns = elapsed.as_nanos();
    let throughput = (entries as f64 / elapsed.as_secs_f64()).round() as u64;
    println!(
        concat!(
            "{{\"schema\":\"qmp-scaling-sample-v2\",",
            "\"operation\":\"{operation}\",\"mode\":\"{mode}\",",
            "\"entries\":{entries},\"threads\":{threads},\"repetition\":{repetition},",
            "\"elapsed_ns\":{elapsed_ns},\"throughput_ops_per_sec\":{throughput},",
            "\"p50_ns\":{p50},\"p95_ns\":{p95},\"p99_ns\":{p99},",
            "\"semantic_peak_bytes\":{semantic_peak_bytes},",
            "\"semantic_after_close_bytes\":{semantic_after_close_bytes},",
            "\"allocator_allocated_bytes\":{allocated},",
            "\"allocator_deallocated_bytes\":{deallocated},",
            "\"allocator_live_bytes\":{live},",
            "\"allocator_high_water_bytes\":{high_water},",
            "\"busy_retries\":{busy_retries}}}"
        ),
        operation = operation,
        mode = mode,
        entries = entries,
        threads = threads,
        repetition = repetition,
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
        busy_retries = busy_retries,
    );
}

fn main() {
    let options = options();
    let allocations = install_tracker();
    for entries in options.sizes {
        for repetition in 0..options.repetitions {
            payload_unique_fill_and_lookup(entries, repetition, &allocations);
            revocable_fill_and_drain(entries, repetition, &allocations);
            broker_release_owner_cardinality(entries, repetition, &allocations);
        }
        for threads in &options.threads {
            for mode in
                [ContentionMode::ReadHeavy, ContentionMode::WriteHeavy, ContentionMode::Mixed]
            {
                for repetition in 0..options.repetitions {
                    payload_contention(entries, *threads, repetition, mode, &allocations);
                }
            }
        }
    }
}

fn new_platform() -> quanta_memory_platform_facade::LiveMemoryPlatform {
    MemoryPlatformBuilder::new()
        .with_manifest(PortableMemoryPresets::production())
        .build()
        .expect("production platform")
}

fn payload_unique_fill_and_lookup(
    entries: usize,
    repetition: usize,
    allocations: &AllocationCounters,
) {
    let platform = new_platform();
    begin_allocation_epoch(allocations);
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
        0,
    );

    let platform = new_platform();
    begin_allocation_epoch(allocations);
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
        "payload_dedup_lookup",
        "single-thread",
        entries,
        1,
        repetition,
        elapsed,
        latency,
        semantic_peak,
        semantic_after,
        allocation,
        0,
    );
}

fn revocable_fill_and_drain(entries: usize, repetition: usize, allocations: &AllocationCounters) {
    let platform = new_platform();
    begin_allocation_epoch(allocations);
    let store = platform
        .revocable_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "bench:revocable".into(), None))
        .build()
        .expect("revocable store");
    let mut latencies = Vec::with_capacity(entries);
    let started = Instant::now();
    for value in 0..entries {
        let operation = Instant::now();
        store.insert_checked(format!("key-{value}"), vec![0; 8]).expect("revocable insert");
        latencies.push(operation.elapsed().as_nanos() as u64);
    }
    let elapsed = started.elapsed();
    let semantic_peak = store.stats().expect("stats").estimated_bytes;
    for _ in 0..entries {
        black_box(store.reclaim_one().expect("reclaim").expect("entry"));
    }
    store.close().expect("close revocable store");
    drop(store);
    let semantic_after =
        platform.broker().ledger_snapshot().expect("ledger after close").total_owner_bytes();
    drop(platform);
    let latency = summarize_latencies(latencies);
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
        0,
    );
}

fn broker_release_owner_cardinality(
    entries: usize,
    repetition: usize,
    allocations: &AllocationCounters,
) {
    let platform = new_platform();
    begin_allocation_epoch(allocations);
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
    let platform = new_platform();
    begin_allocation_epoch(allocations);
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
    if matches!(mode, ContentionMode::ReadHeavy | ContentionMode::Mixed) {
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
                let unique = ((thread_id as u64) << 32) | operation_id as u64 | 1;
                let is_read = matches!(mode, ContentionMode::ReadHeavy)
                    || matches!(mode, ContentionMode::Mixed) && operation_id % 2 == 0;
                if is_read {
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
                            Err(error) => panic!("contention read: {error}"),
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
    barrier.wait();
    let started = Instant::now();
    let mut latencies = Vec::with_capacity(entries);
    let mut busy_retries = 0u64;
    for handle in handles {
        let worker = handle.join().expect("contention worker");
        latencies.extend(worker.latencies);
        busy_retries = busy_retries.checked_add(worker.busy_retries).expect("retry count");
    }
    let elapsed = started.elapsed();
    assert_eq!(latencies.len(), entries);
    let semantic_peak = store.charged_bytes().expect("charged bytes");
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
        busy_retries,
    );
}
