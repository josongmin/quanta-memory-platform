use super::*;

/// One live byte exactly fills the global/class cap. Each iteration measures
/// one typed denial and one admission after an untimed refund; wall throughput
/// includes the refund needed to make the next admission possible.
pub(super) fn broker_pressure_cycle(
    entries: usize,
    repetition: usize,
    allocations: &AllocationCounters,
) {
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
pub(super) fn dual_store_write_contention(
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
pub(super) fn contended_pressure_reclaim(
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
