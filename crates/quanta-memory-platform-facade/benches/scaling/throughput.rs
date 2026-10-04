use super::*;

pub(super) fn payload_unique_fill_and_dedup_put(
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

pub(super) fn revocable_fill_and_drain(
    entries: usize,
    repetition: usize,
    allocations: &AllocationCounters,
) {
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

pub(super) fn broker_release_owner_cardinality(
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
pub(super) fn broker_release_pairwise(
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
pub(super) fn retained_map_point_read(
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

pub(super) fn broker_group_admission_cardinality(
    entries: usize,
    repetition: usize,
    allocations: &AllocationCounters,
) {
    // The broker owns the scheduler's usage. Every timed admission passes
    // through its ticket and ledger path, with construction outside the epoch.
    let group_count = broker_group_cardinality(entries);
    let shared_pool = u64::try_from(group_count).expect("group cardinality fits u64");
    let global_limit = shared_pool.checked_add(2_000).expect("benchmark budget fits u64");
    let owner = "bench:group-admission".to_string();
    let target = format!("group-{:08}", group_count / 2);
    let mut policy = ResidencyPolicy::new(global_limit)
        .with_shared_burst_pool_bytes(shared_pool)
        .with_class_budget(
            MemoryClass::ArtifactCache,
            MemoryClassBudget {
                reserved_bytes: 0,
                soft_cap_bytes: global_limit,
                hard_cap_bytes: global_limit,
                borrow_from_shared_pool: false,
                reclaim_priority: ReclaimPriority::Normal,
            },
        )
        .with_owner_group(owner.clone(), target);
    for index in 0..group_count {
        policy = policy.with_resource_group(
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
    let platform = MemoryPlatformBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            policy,
        ))
        .build()
        .expect("group admission platform");
    let broker = platform.broker();
    let mut seed = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            owner.clone(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1_000,
        )
        .expect("seed target group hard cap");
    let mut probe = broker
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            owner.clone(),
            MemoryContextKind::Request,
            ChargeKind::RetainedPayload,
            1,
        )
        .expect("one-byte weighted share admits");
    let semantic_peak = broker.ledger_snapshot().expect("probe peak").total_owner_bytes();
    assert_eq!(semantic_peak, 1_001);
    probe.release().expect("return probe capacity");
    let mut latencies = Vec::with_capacity(entries.checked_mul(2).expect("operation count"));

    // Start the allocation epoch after policy, broker, seed, and samples exist.
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

    // An admitted ticket is released before the next probe, so each operation
    // starts at the same 1,000-byte group usage. A two-byte request exceeds
    // that group's one-byte weighted share.
    let started = Instant::now();
    for _ in 0..entries {
        let operation = Instant::now();
        let mut admitted = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                owner.clone(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                1,
            )
            .expect("one-byte group admission");
        admitted.release().expect("return one-byte group capacity");
        let latency = operation.elapsed().as_nanos() as u64;
        latencies.push(latency);
        let operation = Instant::now();
        assert!(matches!(
            broker.reserve_managed_charge(
                MemoryClass::ArtifactCache,
                owner.clone(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                2,
            ),
            Err(MemoryError::Capacity(message)) if message.contains("resource group")
        ));
        let latency = operation.elapsed().as_nanos() as u64;
        latencies.push(latency);
    }
    let elapsed = started.elapsed();

    // Stop tracking before dropping the seed that predates the measured epoch.
    let allocation = epoch.finish(allocations);
    assert!(
        !ALLOCATION_TRACKING_ENABLED.load(Ordering::Relaxed),
        "tracker must be inactive after the admission measurement"
    );

    assert!(allocation.allocated > 0, "broker admission must record allocator activity");
    assert_eq!(allocation.allocated, allocation.deallocated);
    assert_eq!(allocation.live, 0);
    assert!(!allocation.underflowed);
    assert_eq!(broker.ledger_snapshot().expect("seed remains").total_owner_bytes(), 1_000);

    // Split the alternating samples only after the timed loop.
    let admitted_latencies: Vec<u64> = latencies.iter().step_by(2).copied().collect();
    let denied_latencies: Vec<u64> = latencies.iter().skip(1).step_by(2).copied().collect();
    let latency = summarize_latencies(latencies);
    let outcome_latencies =
        (summarize_latencies(admitted_latencies), summarize_latencies(denied_latencies));
    seed.release().expect("release seed charge");
    let semantic_after = broker.ledger_snapshot().expect("drained broker").total_owner_bytes();
    drop(platform);
    emit(
        "broker_group_admission_cardinality",
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
            successful: entries,
            denied: entries,
            outcome_latencies: Some(outcome_latencies),
            detail_latencies: DetailLatencies::None,
        },
        0,
    );
}

pub(super) fn payload_contention(
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
