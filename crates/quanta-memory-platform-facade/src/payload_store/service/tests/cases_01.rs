use super::*;

#[test]
fn public_budgeted_stores_issue_distinct_refs_and_reject_foreign_mutations() {
    use crate::MemoryPlatformBuilder;

    let platform = MemoryPlatformBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("live platform");
    let a = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-a".to_string(), None))
        .build()
        .expect("first store");
    let b = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-b".to_string(), None))
        .build()
        .expect("second store");
    let same_binding = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-a".to_string(), None))
        .build()
        .expect("third store with first binding");

    let a_ref = a.put_canonical(envelope(b"a")).expect("a put").payload_ref().clone();
    let b_ref = b.put_canonical(envelope(b"b")).expect("b put").payload_ref().clone();
    let same_binding_ref =
        same_binding.put_canonical(envelope(b"c")).expect("same binding put").payload_ref().clone();
    assert_ne!(a_ref, b_ref, "distinct owners must not share reference identity");
    assert_ne!(a_ref, same_binding_ref, "the store instance owns reference identity");
    assert_ne!(b_ref, same_binding_ref);

    let before_ledger = platform.broker().ledger_snapshot().expect("ledger before probes");
    let before_b = b.stats().expect("b stats before probes");
    let before_same_binding = same_binding.stats().expect("same binding stats before probes");
    for foreign in [&b, &same_binding] {
        assert!(matches!(foreign.pin(&a_ref), Err(MemoryError::InvalidState(_))));
        assert!(matches!(foreign.unpin(&a_ref), Err(MemoryError::InvalidState(_))));
        assert!(matches!(foreign.delete_if_unpinned(&a_ref), Err(MemoryError::InvalidState(_))));
        assert_eq!(foreign.pin_count(&a_ref).expect("absent ref pin count"), 0);
    }
    assert_eq!(b.stats().expect("b stats after probes"), before_b);
    assert_eq!(same_binding.stats().expect("same binding stats after probes"), before_same_binding);
    assert_eq!(platform.broker().ledger_snapshot().expect("ledger after probes"), before_ledger);

    a.delete_if_unpinned(&a_ref).expect("a delete");
    b.delete_if_unpinned(&b_ref).expect("b delete");
    same_binding.delete_if_unpinned(&same_binding_ref).expect("third delete");
    assert_eq!(platform.broker().ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}

#[test]
fn deleted_public_reference_never_addresses_later_payload() {
    use crate::MemoryPlatformBuilder;

    let platform = MemoryPlatformBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("live platform");
    let store = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(binding())
        .build()
        .expect("payload store");
    let stale = store.put_canonical(envelope(b"old")).expect("old put").payload_ref().clone();
    store.delete_if_unpinned(&stale).expect("old delete");
    let current = store.put_canonical(envelope(b"new")).expect("new put").payload_ref().clone();
    assert_ne!(stale, current);
    let before_ledger = platform.broker().ledger_snapshot().expect("before stale probes");
    let before_stats = store.stats().expect("before stats");
    assert!(matches!(store.pin(&stale), Err(MemoryError::InvalidState(_))));
    assert!(matches!(store.unpin(&stale), Err(MemoryError::InvalidState(_))));
    assert!(matches!(store.delete_if_unpinned(&stale), Err(MemoryError::InvalidState(_))));
    assert_eq!(store.pin_count(&stale).expect("absent ref pin count"), 0);
    assert_eq!(store.stats().expect("after stats"), before_stats);
    assert_eq!(platform.broker().ledger_snapshot().expect("after stale probes"), before_ledger);
    store.delete_if_unpinned(&current).expect("current delete");
    assert_eq!(platform.broker().ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}

#[test]
fn store_identity_counter_exhaustion_is_fail_closed() {
    let counter = AtomicU64::new(u64::MAX - 1);
    assert_eq!(allocate_store_nonce(&counter).expect("last available nonce"), u64::MAX - 1);
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    assert!(matches!(allocate_store_nonce(&counter), Err(MemoryError::Capacity(_))));
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn payload_sequence_exhaustion_preserves_store_and_ledger() {
    let (broker, store) = store();
    store.state.lock().expect("state").next_ref = u64::MAX;
    let before = broker.ledger_snapshot().expect("before ledger");
    assert!(matches!(store.put_canonical(envelope(b"new")), Err(MemoryError::Capacity(_))));
    assert_eq!(store.stats().expect("stats"), StoreStats::default());
    assert_eq!(broker.ledger_snapshot().expect("after ledger"), before);
    assert_eq!(store.state.lock().expect("state after").next_ref, u64::MAX);
}

#[test]
fn delete_counter_corruption_preserves_entry_and_charge_authority() {
    let (broker, store) = store();
    let published = store.put_canonical(envelope(b"payload")).expect("publish");
    let payload_ref = published.payload_ref().clone();
    let charged = store.charged_bytes().expect("charged bytes");
    let before_ledger = broker.ledger_snapshot().expect("ledger before corrupt delete");
    let before_entry = {
        let state = store.state.lock().expect("state before corrupt delete");
        let entry = state.entries.iter().flatten().next().expect("one entry");
        (
            entry.payload_ref.clone(),
            entry.bytes.clone(),
            entry.pin_count,
            entry.charge.as_ref().and_then(ManagedCharge::id),
        )
    };

    store.state.lock().expect("inject counter corruption").charged_total = 0;
    assert!(matches!(store.delete_if_unpinned(&payload_ref), Err(MemoryError::InvalidState(_))));
    let state = store.state.lock().expect("state after rejection");
    let entry = state.entries.iter().flatten().next().expect("entry survives");
    assert_eq!(state.live_entries, 1);
    assert_eq!(state.canonical_index.values().map(Vec::len).sum::<usize>(), 1);
    assert_eq!(state.reference_index.values().map(Vec::len).sum::<usize>(), 1);
    assert_eq!(
        (
            &entry.payload_ref,
            &entry.bytes,
            entry.pin_count,
            entry.charge.as_ref().and_then(ManagedCharge::id)
        ),
        (&before_entry.0, &before_entry.1, before_entry.2, before_entry.3)
    );
    assert!(entry.charge.as_ref().is_some_and(ManagedCharge::is_active));
    drop(state);
    assert_eq!(broker.ledger_snapshot().expect("ledger after rejection"), before_ledger);

    store.state.lock().expect("restore test corruption").charged_total = charged;
    store.delete_if_unpinned(&payload_ref).expect("retry after counter repair");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}

#[test]
fn canonical_put_deduplicates_and_charges_once() {
    let (broker, store) = store();
    let first = store.put_canonical(envelope(b"same")).expect("first");
    let second = store.put_canonical(envelope(b"same")).expect("dedup");
    assert_eq!(first.payload_ref(), second.payload_ref());
    assert_eq!(store.stats().expect("stats").item_count, 1);
    assert_eq!(store.charged_bytes().expect("bytes"), retained_cost(4, 0, store.store_nonce));
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        retained_cost(4, 0, store.store_nonce)
    );
}
#[test]
fn payload_lookup_probe_count_is_n_independent() {
    #[derive(Clone)]
    struct ProbeBytes {
        bytes: Vec<u8>,
        probes: Arc<AtomicUsize>,
    }

    impl ByteSized for ProbeBytes {
        fn byte_size(&self) -> u64 {
            self.bytes.len() as u64
        }
    }

    impl CanonicalKey for ProbeBytes {
        fn canonical_key(&self) -> Vec<u8> {
            self.probes.fetch_add(1, Ordering::SeqCst);
            self.bytes.clone()
        }
    }

    let broker = live_broker(1_000_000, 600_000);
    let store =
        SimplePayloadStore::<String, ProbeBytes, Budgeted>::new_budgeted(binding(), None, broker)
            .expect("payload store nonce");
    let probes = Arc::new(AtomicUsize::new(0));
    for value in 0u32..2_048 {
        store
            .put_canonical(RetainedEnvelope::new(ProbeBytes {
                bytes: value.to_be_bytes().to_vec(),
                probes: Arc::clone(&probes),
            }))
            .expect("unique payload insert");
    }

    probes.store(0, Ordering::SeqCst);
    let duplicate = store
        .put_canonical(RetainedEnvelope::new(ProbeBytes {
            bytes: 1_024u32.to_be_bytes().to_vec(),
            probes: Arc::clone(&probes),
        }))
        .expect("indexed duplicate lookup");
    assert_eq!(duplicate.payload_ref(), &format!("payload-{}-1024", store.store_nonce));
    assert_eq!(
        probes.load(Ordering::SeqCst),
        2,
        "lookup evaluates only the query and its digest-bucket candidate, independent of N"
    );
    assert_eq!(store.stats().expect("stats").item_count, 2_048);
}
#[test]
fn payload_delete_reuses_vacant_slots_without_stale_index_alias() {
    let (broker, store) = store();
    let mut first_generation = Vec::new();
    for value in 0u32..128 {
        first_generation.push(
            store
                .put_canonical(RetainedEnvelope::new(value.to_be_bytes().to_vec()))
                .expect("first generation")
                .payload_ref()
                .clone(),
        );
    }
    for payload_ref in &first_generation {
        store.delete_if_unpinned(payload_ref).expect("delete first generation");
    }
    {
        let state = store.state.lock().expect("state");
        assert_eq!(state.entries.len(), 128);
        assert_eq!(state.free_slots.len(), 128);
        assert!(state.canonical_index.is_empty() && state.reference_index.is_empty());
    }

    for value in 128u32..256 {
        store
            .put_canonical(RetainedEnvelope::new(value.to_be_bytes().to_vec()))
            .expect("second generation");
    }
    let state = store.state.lock().expect("state");
    assert_eq!(state.entries.len(), 128, "churn must reuse stable vacant slots");
    assert!(state.free_slots.is_empty());
    assert_eq!(state.live_entries, 128);
    drop(state);
    drop(store);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn transient_canonical_key_is_not_charged_but_retained_identity_is() {
    let broker = live_broker(100, 100);
    let store = SimplePayloadStore::<String, ZeroMeasuredHugeCanonicalKey, Budgeted>::new_budgeted(
        binding(),
        None,
        broker.clone(),
    )
    .expect("payload store nonce");
    let first = store
        .put_canonical(RetainedEnvelope::new(ZeroMeasuredHugeCanonicalKey))
        .expect("semantic cost fits");
    let second =
        store.put_canonical(RetainedEnvelope::new(ZeroMeasuredHugeCanonicalKey)).expect("dedup");
    assert_eq!(first.payload_ref(), second.payload_ref());
    assert_eq!(store.charged_bytes().expect("bytes"), retained_cost(0, 0, store.store_nonce));
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        retained_cost(0, 0, store.store_nonce)
    );
}
#[test]
fn cost_component_sum_overflow_denies_without_publishing_or_charging() {
    let broker = live_broker(1_000, 1_000);
    let store = SimplePayloadStore::<String, MaxMeasuredPayload, Budgeted>::new_budgeted(
        binding(),
        None,
        broker.clone(),
    )
    .expect("payload store nonce");
    assert!(matches!(
        store.put_canonical(RetainedEnvelope::new(MaxMeasuredPayload)),
        Err(MemoryError::Capacity(_))
    ));
    assert_eq!(store.stats().expect("unchanged stats").item_count, 0);
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
}
#[test]
fn composite_entry_charges_one_payload_bucket() {
    let broker = live_broker(1_000_000, 600_000);
    let store = SimplePayloadStore::<String, Vec<u8>, Budgeted>::new_budgeted(
        binding(),
        None,
        broker.clone(),
    )
    .expect("payload store nonce");
    store.put_canonical(envelope(b"payload")).expect("put");
    let charged = store.charged_bytes().expect("charged total");
    assert!(charged > 0, "a published payload holds a live charge");
    // One owned lifetime component, one bucket: the composite
    // payload+key/index/entry cost is charged entirely as RetainedPayload.
    let snap = broker.ledger_snapshot().expect("snapshot");
    assert_eq!(snap.total_retained_payload_bytes(), charged);
    assert_eq!(snap.total_retained_metadata_bytes(), 0);
    assert_eq!(snap.total_live_scratch_bytes(), 0);
    assert_eq!(snap.total_owner_bytes(), charged);
    assert_eq!(snap.live_ticket_count, 1);
    let usage = snap.owner_usage.values().next().expect("sole owner");
    assert_eq!(
        (usage.retained_payload_bytes, usage.retained_metadata_bytes, usage.live_scratch_bytes),
        (charged, 0, 0)
    );
    store.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn canonical_digests_are_randomized_per_store_seed() {
    // Fixed-key hashing would let an attacker grind canonical keys into
    // one index bucket offline; per-store random seeds must not reproduce
    // a deterministic digest (collision probability 2^-64).
    let (_, store_a) = store();
    let (_, store_b) = store();
    let a = &store_a.digest_seed;
    let b = &store_b.digest_seed;
    let key = b"adversarial-canonical-key".as_slice();
    assert_ne!(
        canonical_digest(a, key),
        canonical_digest(b, key),
        "per-store random seeds must not reproduce a grindable fixed-key digest"
    );
    assert_eq!(canonical_digest(a, key), canonical_digest(a, key));
}
#[test]
fn counter_reads_show_open_busy_and_closed_images() {
    let broker = live_broker(1_000_000, 600_000);
    let store = SimplePayloadStore::<String, Vec<u8>, Budgeted>::new_budgeted(
        binding(),
        None,
        broker.clone(),
    )
    .expect("payload store nonce");
    store.put_canonical(envelope(b"payload")).expect("put");
    // Open image: the plain counters mirror the authoritative ledger charge.
    let charged = broker.ledger_snapshot().expect("open ledger").total_owner_bytes();
    assert!(charged > 0, "seed payload must hold a live charge");
    assert_eq!(
        store.stats().expect("open stats"),
        StoreStats { item_count: 1, estimated_bytes: charged }
    );
    assert_eq!(store.charged_bytes().expect("open bytes"), charged);

    // Busy image: while the mutation lane is held, counter reads are typed
    // contention — never an empty success masking an in-flight plan.
    let permit = store.lifecycle.begin_mutation().expect("own lane");
    assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(store.charged_bytes(), Err(MemoryError::OperationInProgress(_))));
    drop(permit);

    // Closed image: the terminal empty image stays readable and consistent.
    store.close().expect("close");
    assert_eq!(store.stats().expect("closed stats"), StoreStats::default());
    assert_eq!(store.charged_bytes().expect("closed bytes"), 0);
    assert_eq!(broker.ledger_snapshot().expect("drained ledger").total_owner_bytes(), 0);
}
#[test]
fn measurement_panic_precedes_accounting_and_does_not_poison_state() {
    let broker = live_broker(1_000, 1_000);
    let store = SimplePayloadStore::<String, PanicMeasuredPayload, Budgeted>::new_budgeted(
        binding(),
        None,
        broker.clone(),
    )
    .expect("payload store nonce");
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.put_canonical(RetainedEnvelope::new(PanicMeasuredPayload));
    }));
    assert!(caught.is_err());
    assert_eq!(store.stats().expect("state not poisoned"), StoreStats::default());
    assert_eq!(broker.ledger_snapshot().expect("no charge").total_owner_bytes(), 0);
}
#[test]
fn metadata_equality_reentry_under_read_lock_is_typed_busy_and_preserves_charge() {
    let broker = live_broker(1_000_000, 600_000);
    let store = Arc::new(
        SimplePayloadStore::<ReentrantPayloadRef, Vec<u8>, Budgeted>::new_budgeted(
            binding(),
            None,
            broker.clone(),
        )
        .expect("payload store nonce"),
    );
    let published = store.put_canonical(envelope(b"metadata")).expect("put");
    REF_REENTRY_STORE.with(|slot| *slot.borrow_mut() = Some(Arc::downgrade(&store)));
    REF_REENTRY_HITS.with(|hits| hits.set(0));

    assert_eq!(store.pin_count(published.payload_ref()).expect("read metadata"), 0);
    REF_REENTRY_HITS.with(|hits| assert_eq!(hits.get(), 1));
    REF_REENTRY_STORE.with(|slot| *slot.borrow_mut() = None);
    assert_eq!(store.stats().expect("complete state").item_count, 1);
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        retained_cost(8, 0, store.store_nonce)
    );
    store.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
}
#[test]
fn duplicate_canonical_callback_reentry_under_lock_is_typed_busy_and_charges_once() {
    let broker = live_broker(1_000_000, 600_000);
    let store = Arc::new(
        SimplePayloadStore::<String, ReentrantCanonicalBytes, Budgeted>::new_budgeted(
            binding(),
            None,
            broker.clone(),
        )
        .expect("payload store nonce"),
    );
    let probe = Arc::new(OnceLock::new());
    let hits = Arc::new(AtomicUsize::new(0));
    let first = store
        .put_canonical(RetainedEnvelope::new(ReentrantCanonicalBytes {
            bytes: b"same".to_vec(),
            probe: Some(Arc::clone(&probe)),
            hits: Arc::clone(&hits),
        }))
        .expect("first");
    assert!(probe.set(Arc::downgrade(&store)).is_ok());

    let replay = store
        .put_canonical(RetainedEnvelope::new(ReentrantCanonicalBytes {
            bytes: b"same".to_vec(),
            probe: None,
            hits: Arc::clone(&hits),
        }))
        .expect("deduplicated replay");
    assert_eq!(first.payload_ref(), replay.payload_ref());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(store.stats().expect("complete state").item_count, 1);
    assert_eq!(
        broker.ledger_snapshot().expect("charged once").total_owner_bytes(),
        retained_cost(4, 0, store.store_nonce)
    );
    store.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
}
#[test]
fn dedup_hit_preserves_first_context_and_stores_no_second_entry() {
    let (broker, store) = store();
    let first = store
        .put_canonical(
            envelope(b"same-bytes").with_context(context_handle(MemoryContextKind::Session)),
        )
        .expect("first");
    let bytes_before = store.charged_bytes().expect("bytes before");
    let second = store
        .put_canonical(
            envelope(b"same-bytes").with_context(context_handle(MemoryContextKind::Request)),
        )
        .expect("dedup");

    assert_eq!(second.payload_ref(), first.payload_ref());
    assert_eq!(store.stats().expect("stats").item_count, 1);
    assert_eq!(store.charged_bytes().expect("bytes after"), bytes_before);
    let snapshot = broker.ledger_snapshot().expect("ledger");
    assert_eq!(snapshot.total_owner_bytes(), bytes_before);
    assert_eq!(
        snapshot.context_usage.get(&MemoryContextKind::Session).map_or(0, |usage| usage.total()),
        bytes_before
    );
    assert_eq!(
        snapshot.context_usage.get(&MemoryContextKind::Request).map_or(0, |usage| usage.total()),
        0
    );
}
#[test]
fn distinct_payloads_each_charge_once() {
    let (broker, store) = store();
    store.put_canonical(envelope(b"aa")).expect("a");
    store.put_canonical(envelope(b"bbbb")).expect("b");
    assert_eq!(
        store.charged_bytes().expect("bytes"),
        retained_cost(2, 0, store.store_nonce) + retained_cost(4, 1, store.store_nonce)
    );
    assert_eq!(store.stats().expect("stats").item_count, 2);
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        retained_cost(2, 0, store.store_nonce) + retained_cost(4, 1, store.store_nonce)
    );
}
#[test]
fn non_injective_test_reference_conversion_is_rejected_before_charge() {
    let broker = live_broker(1_000_000, 600_000);
    let store = SimplePayloadStore::<ConstantRef, Vec<u8>, Budgeted>::new_budgeted(
        binding(),
        None,
        broker.clone(),
    )
    .expect("payload store nonce");
    let first = store.put_canonical(envelope(b"one")).expect("first identity");
    let before_stats = store.stats().expect("before stats");
    let before_ledger = broker.ledger_snapshot().expect("before ledger");
    assert!(matches!(store.put_canonical(envelope(b"two")), Err(MemoryError::InvalidState(_))));
    assert_eq!(store.stats().expect("unchanged stats"), before_stats);
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger"), before_ledger);
    store.pin(first.payload_ref()).expect("first remains addressable");
    store.unpin(first.payload_ref()).expect("unpin first");
    store.delete_if_unpinned(first.payload_ref()).expect("delete first");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn delete_unpinned_releases_bytes_exactly_once() {
    let (broker, store) = store();
    let published = store.put_canonical(envelope(b"deleteme")).expect("put");
    store.delete_if_unpinned(published.payload_ref()).expect("delete");
    assert_eq!(store.charged_bytes().expect("bytes"), 0);
    assert_eq!(store.stats().expect("stats").item_count, 0);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    assert!(matches!(
        store.delete_if_unpinned(published.payload_ref()),
        Err(MemoryError::InvalidState(_))
    ));
}
#[test]
fn over_cap_put_is_fail_closed_and_does_not_consume_ref() {
    let broker = live_broker(1_000, 140);
    let store = store_with(&broker);
    assert!(matches!(store.put_canonical(envelope(&[0; 100])), Err(MemoryError::Capacity(_))));
    assert_eq!(store.stats().expect("stats").item_count, 0);
    let published = store.put_canonical(envelope(&[0; 40])).expect("within cap");
    assert_eq!(published.payload_ref(), &format!("payload-{}-0", store.store_nonce));
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        retained_cost(40, 0, store.store_nonce)
    );
}
#[test]
fn pin_is_metadata_only_and_pinned_delete_is_rejected() {
    let (broker, store) = store();
    let published = store.put_canonical(envelope(b"payload")).expect("put");
    store.pin(published.payload_ref()).expect("pin");
    store.pin(published.payload_ref()).expect("pin again");
    assert_eq!(store.pin_count(published.payload_ref()).expect("count"), 2);
    assert!(matches!(
        store.delete_if_unpinned(published.payload_ref()),
        Err(MemoryError::InvalidState(_))
    ));
    assert_eq!(
        broker.ledger_snapshot().expect("ledger").total_owner_bytes(),
        retained_cost(7, 0, store.store_nonce)
    );
    store.unpin(published.payload_ref()).expect("unpin");
    store.unpin(published.payload_ref()).expect("unpin again");
    store.delete_if_unpinned(published.payload_ref()).expect("delete");
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn reference_eq_panic_before_mutation_keeps_public_store_available() {
    use std::sync::atomic::AtomicBool;

    static PANIC_NEXT_EQ: AtomicBool = AtomicBool::new(false);

    #[derive(Clone, Debug)]
    struct Ref(String);
    impl From<String> for Ref {
        fn from(value: String) -> Self {
            Self(value)
        }
    }
    impl ByteSized for Ref {
        fn byte_size(&self) -> u64 {
            self.0.len() as u64
        }
    }
    impl Hash for Ref {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.0.hash(state);
        }
    }
    impl PartialEq for Ref {
        fn eq(&self, other: &Self) -> bool {
            if PANIC_NEXT_EQ.swap(false, Ordering::SeqCst) {
                panic!("injected reference Eq panic");
            }
            self.0 == other.0
        }
    }
    impl Eq for Ref {}

    for operation in ["pin", "unpin", "pin_count", "delete"] {
        let broker = live_broker(1_000_000, 600_000);
        let store = SimplePayloadStore::<Ref, Vec<u8>, Budgeted>::new_budgeted(
            binding(),
            None,
            broker.clone(),
        )
        .expect("payload store nonce");
        let published = store.put_canonical(envelope(b"payload")).expect("put");
        if operation == "unpin" {
            store.pin(published.payload_ref()).expect("seed pin");
        }
        let before = broker.ledger_snapshot().expect("before");
        let pins_before = store.pin_count(published.payload_ref()).expect("pins before");
        PANIC_NEXT_EQ.store(true, Ordering::SeqCst);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match operation {
            "pin" => {
                let _ = store.pin(published.payload_ref());
            }
            "unpin" => {
                let _ = store.unpin(published.payload_ref());
            }
            "pin_count" => {
                let _ = store.pin_count(published.payload_ref());
            }
            "delete" => {
                let _ = store.delete_if_unpinned(published.payload_ref());
            }
            _ => unreachable!(),
        }));
        assert!(panicked.is_err(), "{operation}: Eq trap must fire");
        assert_eq!(broker.ledger_snapshot().expect("after"), before);
        assert_eq!(store.stats().expect("public stats after panic").item_count, 1);
        assert_eq!(
            store.pin_count(published.payload_ref()).expect("public pin count"),
            pins_before
        );
        if pins_before > 0 {
            store.unpin(published.payload_ref()).expect("unpin after panic");
        }
        store.delete_if_unpinned(published.payload_ref()).expect("delete after panic");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }
}
#[test]
fn reference_clone_panic_before_publication_keeps_public_store_available() {
    use std::sync::atomic::AtomicBool;

    static PANIC_NEXT_CLONE: AtomicBool = AtomicBool::new(false);

    #[derive(Debug, PartialEq, Eq, Hash)]
    struct Ref(String);
    impl From<String> for Ref {
        fn from(value: String) -> Self {
            Self(value)
        }
    }
    impl Clone for Ref {
        fn clone(&self) -> Self {
            if PANIC_NEXT_CLONE.swap(false, Ordering::SeqCst) {
                panic!("injected reference Clone panic");
            }
            Self(self.0.clone())
        }
    }
    impl ByteSized for Ref {
        fn byte_size(&self) -> u64 {
            self.0.len() as u64
        }
    }

    let broker = live_broker(1_000_000, 600_000);
    let store =
        SimplePayloadStore::<Ref, Vec<u8>, Budgeted>::new_budgeted(binding(), None, broker.clone())
            .expect("payload store nonce");
    PANIC_NEXT_CLONE.store(true, Ordering::SeqCst);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.put_canonical(envelope(b"payload"));
    }));
    assert!(panicked.is_err(), "Clone trap must fire before publication");
    assert_eq!(broker.ledger_snapshot().expect("after panic").total_owner_bytes(), 0);
    assert_eq!(store.stats().expect("public stats after panic").item_count, 0);

    let published = store.put_canonical(envelope(b"payload")).expect("retry after panic");
    assert_eq!(store.stats().expect("published stats").item_count, 1);
    store.delete_if_unpinned(published.payload_ref()).expect("cleanup");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn missing_delete_and_unpin_without_pin_are_invalid_state() {
    let (_broker, store) = store();
    assert!(matches!(
        store.delete_if_unpinned(&"payload-404".to_string()),
        Err(MemoryError::InvalidState(_))
    ));
    let published = store.put_canonical(envelope(b"x")).expect("put");
    assert!(matches!(store.unpin(published.payload_ref()), Err(MemoryError::InvalidState(_))));
}
#[test]
fn put_and_delete_during_rebuild_are_fail_closed_then_resume() {
    let (broker, store) = store();
    let published = store.put_canonical(envelope(&[0; 80])).expect("seed");
    let rebuild_fence1 = broker.begin_rebuild().expect("begin rebuild");

    assert!(matches!(store.put_canonical(envelope(&[1; 40])), Err(MemoryError::InvalidState(_))));
    assert!(matches!(
        store.delete_if_unpinned(published.payload_ref()),
        Err(MemoryError::InvalidState(_))
    ));
    assert_eq!(store.stats().expect("preserved stats").item_count, 1);
    assert_eq!(
        store.charged_bytes().expect("preserved bytes"),
        retained_cost(80, 0, store.store_nonce)
    );
    assert_eq!(
        broker.ledger_snapshot().expect("preserved ledger").total_owner_bytes(),
        retained_cost(80, 0, store.store_nonce)
    );

    assert!(rebuild_fence1.finish().expect("finish rebuild").drain_error.is_none());
    store.delete_if_unpinned(published.payload_ref()).expect("delete resumes");
    assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
}
#[cfg(feature = "unbudgeted-stores")]
#[test]
fn explicit_unbudgeted_store_is_a_distinct_non_ledger_mode() {
    let store = SimplePayloadStore::<String, Vec<u8>, Unbudgeted>::new_unbudgeted(binding(), None)
        .expect("payload store nonce");
    store.put_canonical(envelope(b"free")).expect("put");
    assert_eq!(store.charged_bytes().expect("bytes"), retained_cost(4, 0, store.store_nonce));
    assert_eq!(store.stats().expect("stats").item_count, 1);
    store.close().expect("close unbudgeted");
}
#[test]
fn close_refunds_every_live_entry_exactly_once() {
    let (broker, store) = store();
    store.put_canonical(envelope(b"aa")).expect("a");
    store.put_canonical(envelope(b"bbbb")).expect("b");
    assert_eq!(
        broker.ledger_snapshot().expect("before").total_owner_bytes(),
        retained_cost(2, 0, store.store_nonce) + retained_cost(4, 1, store.store_nonce)
    );
    store.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
    assert_eq!(store.stats().expect("stats").item_count, 0);
}
#[test]
fn close_refund_failure_preserves_entries_and_charge_ownership() {
    let (broker, store) = store();
    store.put_canonical(envelope(b"owned")).expect("put");
    let rebuild_fence2 = broker.begin_rebuild().expect("begin rebuild");
    assert!(matches!(store.close(), Err(MemoryError::InvalidState(_))));
    assert_eq!(store.stats().expect("preserved stats").item_count, 1);
    assert_eq!(
        store.charged_bytes().expect("preserved bytes"),
        retained_cost(5, 0, store.store_nonce)
    );
    assert_eq!(
        broker.ledger_snapshot().expect("preserved ledger").total_owner_bytes(),
        retained_cost(5, 0, store.store_nonce)
    );
    assert!(rebuild_fence2.finish().expect("finish rebuild").drain_error.is_none());
    store.close().expect("retry close");
    assert_eq!(broker.ledger_snapshot().expect("released").total_owner_bytes(), 0);
}
#[test]
fn drop_during_rebuild_defers_then_finish_drains_capacity() {
    let (broker, store) = store();
    store.put_canonical(envelope(b"owned")).expect("put");
    let expected_retained = retained_cost(5, 0, store.store_nonce);
    let rebuild_fence3 = broker.begin_rebuild().expect("begin rebuild");
    drop(store);
    assert_eq!(broker.ledger_snapshot().expect("deferred").total_owner_bytes(), expected_retained);
    assert!(rebuild_fence3.finish().expect("finish drains").drain_error.is_none());
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
#[test]
fn drop_during_outer_unwind_contains_two_client_panics_and_refunds_all_charges() {
    let broker = live_broker(1_000_000, 600_000);
    let drops = Arc::new(AtomicUsize::new(0));
    let store = SimplePayloadStore::<String, PanickingBytes, Budgeted>::new_budgeted(
        binding(),
        None,
        broker.clone(),
    )
    .expect("payload store nonce");
    for bytes in [b"four".as_slice(), b"sixsix".as_slice()] {
        store
            .put_canonical(RetainedEnvelope::new(PanickingBytes {
                bytes: bytes.to_vec(),
                drops: Arc::clone(&drops),
            }))
            .expect("put before unwind");
    }
    assert_eq!(
        broker.ledger_snapshot().expect("before").total_owner_bytes(),
        retained_cost(4, 0, store.store_nonce) + retained_cost(6, 1, store.store_nonce)
    );

    let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _store = store;
        panic!("outer unwind");
    }))
    .expect_err("outer panic must propagate");

    assert_eq!(outer.downcast_ref::<&str>(), Some(&"outer unwind"));
    assert_eq!(drops.load(Ordering::SeqCst), 2, "both client values must be retired");
    assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
}
#[test]
fn mutation_after_close_is_typed_closed() {
    let (_broker, store) = store();
    store.close().expect("close");
    assert!(matches!(
        store.put_canonical(envelope(b"late")),
        Err(MemoryError::Closed(resource)) if resource == "payload store"
    ));
    assert!(matches!(
        store.pin(&"payload-0".to_string()),
        Err(MemoryError::Closed(resource)) if resource == "payload store"
    ));
}
#[test]
fn double_close_is_idempotent_without_double_refund() {
    let (broker, store) = store();
    store.put_canonical(envelope(b"once")).expect("put");
    store.close().expect("first close");
    store.close().expect("second close");
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn concurrent_mutation_and_close_has_one_typed_linearized_outcome() {
    let broker = live_broker(1_000_000, 600_000);
    let store = Arc::new(store_with(&broker));
    let barrier = Arc::new(Barrier::new(3));
    let close_store = Arc::clone(&store);
    let close_barrier = Arc::clone(&barrier);
    let close = std::thread::spawn(move || {
        close_barrier.wait();
        close_store.close()
    });
    let put_store = Arc::clone(&store);
    let put_barrier = Arc::clone(&barrier);
    let put = std::thread::spawn(move || {
        put_barrier.wait();
        put_store.put_canonical(envelope(b"race"))
    });
    barrier.wait();

    let close = close.join().expect("close thread");
    let put = put.join().expect("put thread");
    assert!(matches!(&close, Ok(()) | Err(MemoryError::OperationInProgress(_))));
    assert!(matches!(
        put,
        Ok(_) | Err(MemoryError::Closed(_)) | Err(MemoryError::OperationInProgress(_))
    ));
    if close.is_err() {
        store.close().expect("retry close after winning mutation");
    }
    assert_eq!(broker.ledger_snapshot().expect("linearized ledger").total_owner_bytes(), 0);
    assert_eq!(store.stats().expect("linearized stats").item_count, 0);
}
#[test]
fn counter_overflow_is_rejected_without_accounting_or_entry_mutation() {
    let (broker, store) = store();
    store.state.lock().expect("state").charged_total = u64::MAX;
    assert!(matches!(store.put_canonical(envelope(b"x")), Err(MemoryError::Capacity(_))));
    let state = store.state.lock().expect("state");
    assert_eq!(state.charged_total, u64::MAX);
    assert!(state.entries.is_empty());
    drop(state);
    assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
}
#[test]
fn poisoned_stats_is_error_not_empty() {
    let (_broker, store) = store();
    let store = Arc::new(store);
    let poison = Arc::clone(&store);
    assert!(std::thread::spawn(move || {
        let _guard = poison.state.lock().expect("lock");
        panic!("poison payload state");
    })
    .join()
    .is_err());
    assert!(matches!(store.stats(), Err(MemoryError::Poisoned(_))));
}
