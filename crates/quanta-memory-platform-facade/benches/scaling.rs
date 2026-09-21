use std::hint::black_box;
use std::time::Instant;

use quanta_memory_platform_core::{MemoryClass, RetainedEnvelope, StoreBindingHandle};
use quanta_memory_platform_facade::{
    MemoryPlatformBuilder, PayloadStore, PortableMemoryPresets, RevocableStore,
};

fn selected_sizes() -> Vec<usize> {
    let mut sizes = vec![1_000, 10_000];
    for argument in std::env::args().skip(1) {
        if argument == "--smoke" {
            sizes = vec![1_000];
        } else if let Some(raw) = argument.strip_prefix("--sizes=") {
            sizes = raw
                .split(',')
                .map(|value| value.parse::<usize>().expect("--sizes must contain usize values"))
                .collect();
        }
    }
    sizes
}

fn emit(operation: &str, entries: usize, elapsed: std::time::Duration) {
    let nanos = elapsed.as_nanos();
    let per_operation = nanos / entries.max(1) as u128;
    println!(
        "{{\"schema\":\"qmp-scaling-sample-v1\",\"operation\":\"{operation}\",\"entries\":{entries},\"elapsed_ns\":{nanos},\"ns_per_op\":{per_operation}}}"
    );
}

fn main() {
    for entries in selected_sizes() {
        payload_unique_fill_and_lookup(entries);
        revocable_fill_and_drain(entries);
    }
}

fn payload_unique_fill_and_lookup(entries: usize) {
    let platform = MemoryPlatformBuilder::new()
        .with_manifest(PortableMemoryPresets::production())
        .build()
        .expect("production platform");
    let store = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "bench:payload".into(), None))
        .build()
        .expect("payload store");

    let started = Instant::now();
    for value in 0..entries {
        let bytes = (value as u64).to_be_bytes().to_vec();
        black_box(store.put_canonical(RetainedEnvelope::new(bytes)).expect("unique payload"));
    }
    emit("payload_unique_fill", entries, started.elapsed());

    let started = Instant::now();
    for value in 0..entries {
        let bytes = (value as u64).to_be_bytes().to_vec();
        black_box(store.put_canonical(RetainedEnvelope::new(bytes)).expect("dedup payload"));
    }
    emit("payload_dedup_lookup", entries, started.elapsed());
    store.close().expect("close payload store");
}

fn revocable_fill_and_drain(entries: usize) {
    let platform = MemoryPlatformBuilder::new()
        .with_manifest(PortableMemoryPresets::production())
        .build()
        .expect("production platform");
    let store = platform
        .revocable_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "bench:revocable".into(), None))
        .build()
        .expect("revocable store");

    let started = Instant::now();
    for value in 0..entries {
        store.insert_checked(format!("key-{value}"), vec![0; 8]).expect("revocable insert");
    }
    emit("revocable_fill", entries, started.elapsed());

    let started = Instant::now();
    for _ in 0..entries {
        black_box(store.reclaim_one().expect("reclaim").expect("entry"));
    }
    emit("revocable_full_drain", entries, started.elapsed());
    store.close().expect("close revocable store");
}
