//! Public cross-owner accounting scenarios that require multiple facade slices.
#![cfg(all(
    feature = "core-live-accounting",
    feature = "retained_map",
    feature = "revocable_store"
))]

use std::sync::{Arc, Barrier, Mutex, OnceLock, Weak};

use quanta_memory_platform_core::{
    ChargeKind, ExecutionBudget, LedgerSnapshot, ManagedCharge, MemoryBudgetManifest, MemoryClass,
    MemoryClassBudget, MemoryContextKind, MemoryError, MemoryPreset, MemoryReclaimer, MemoryResult,
    ProcessMemoryBroker, ReclaimOutcome, ReclaimPriority, ReclaimRequest, ResidencyPolicy,
    ResourceGroup, RuntimeKind, ScratchRequest, StoreBindingHandle, SweepStats, UsageBreakdown,
};
use quanta_memory_platform_facade::{
    Budgeted, LiveMemoryPlatform, MemoryPlatformBuilder, RetainedMapStore, RevocableStore,
    SimpleRetainedMapStore, SimpleRevocableStore,
};

const MAP_OWNER: &str = "owner:cross-map";
const REVOCABLE_OWNER: &str = "owner:cross-revocable";
const SCRATCH_OWNER: &str = "owner:cross-scratch";
const ZERO_OWNER: &str = "owner:cross-zero";

// Independent semantic-cost oracle; these constants intentionally do not call
// production cost helpers, so a cost-formula drift changes the observed ledger.
fn map_cost(key: &str, value_len: usize) -> u64 {
    value_len as u64 + key.len() as u64 + 24 + 16
}

fn revocable_cost(key: &str, value_len: usize) -> u64 {
    value_len as u64 + key.len() as u64 + 16 + 16
}

fn class_budget(hard_cap_bytes: u64) -> MemoryClassBudget {
    MemoryClassBudget {
        reserved_bytes: 0,
        soft_cap_bytes: hard_cap_bytes,
        hard_cap_bytes,
        borrow_from_shared_pool: false,
        reclaim_priority: ReclaimPriority::Normal,
    }
}

fn group(hard_cap_bytes: u64) -> ResourceGroup {
    ResourceGroup {
        reserved_bytes: 0,
        soft_cap_bytes: hard_cap_bytes,
        hard_cap_bytes,
        burst_percent: 0,
        reclaim_priority: ReclaimPriority::Normal,
        admission_weight: 1,
    }
}

fn policy(global: u64, map_cap: u64) -> ResidencyPolicy {
    ResidencyPolicy::new(global)
        .with_class_budget(MemoryClass::MemoResidency, class_budget(map_cap))
        .with_class_budget(MemoryClass::ExecutionScratchRevocable, class_budget(global))
        .with_class_budget(MemoryClass::ExecutionScratch, class_budget(global))
        .with_class_budget(MemoryClass::ArtifactCache, class_budget(global))
        .with_resource_group("g-map", group(global))
        .with_resource_group("g-revocable", group(global))
        .with_resource_group("g-scratch", group(global))
        .with_resource_group("g-zero", group(global))
        .with_resource_group("g-other", group(global))
        .with_owner_group(MAP_OWNER, "g-map")
        .with_owner_group(REVOCABLE_OWNER, "g-revocable")
        .with_owner_group(SCRATCH_OWNER, "g-scratch")
        .with_owner_group(ZERO_OWNER, "g-zero")
}

fn platform(global: u64, map_cap: u64) -> LiveMemoryPlatform {
    MemoryPlatformBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            policy(global, map_cap),
        ))
        .build()
        .expect("valid grouped platform")
}

fn map_store(platform: &LiveMemoryPlatform) -> SimpleRetainedMapStore<String, Vec<u8>, Budgeted> {
    platform
        .retained_map_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::MemoResidency, MAP_OWNER.into(), None))
        .build()
}

fn revocable_store(
    platform: &LiveMemoryPlatform,
) -> SimpleRevocableStore<String, Vec<u8>, Budgeted> {
    platform
        .revocable_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(
            MemoryClass::ExecutionScratchRevocable,
            REVOCABLE_OWNER.into(),
            None,
        ))
        .build()
        .expect("budgeted revocable store")
}

fn assert_balanced(snapshot: &LedgerSnapshot) {
    assert!(snapshot.class_owner_balanced(), "class/owner divergence: {snapshot:?}");
    assert!(snapshot.context_owner_balanced(), "context/owner divergence: {snapshot:?}");
    assert!(snapshot.owner_partition_balanced(), "kind partition divergence: {snapshot:?}");
}

fn assert_only_charge(
    snapshot: &LedgerSnapshot,
    class: MemoryClass,
    owner: &str,
    context: MemoryContextKind,
    kind: ChargeKind,
    bytes: u64,
) {
    assert_balanced(snapshot);
    assert_eq!(snapshot.total_class_bytes(), bytes);
    assert_eq!(snapshot.class_usage.len(), 1);
    assert_eq!(snapshot.owner_usage.len(), 1);
    assert_eq!(snapshot.context_usage.len(), 1);
    assert_eq!(snapshot.live_ticket_count, 1);
    let expected = match kind {
        ChargeKind::RetainedPayload => {
            UsageBreakdown { retained_payload_bytes: bytes, ..UsageBreakdown::default() }
        }
        ChargeKind::RetainedMetadata => {
            UsageBreakdown { retained_metadata_bytes: bytes, ..UsageBreakdown::default() }
        }
        ChargeKind::LiveScratch => {
            UsageBreakdown { live_scratch_bytes: bytes, ..UsageBreakdown::default() }
        }
    };
    assert_eq!(snapshot.class_usage.get(&class), Some(&expected));
    assert_eq!(snapshot.owner_usage.get(owner), Some(&expected));
    assert_eq!(snapshot.context_usage.get(&context), Some(&expected));
}

fn assert_drained(snapshot: &LedgerSnapshot) {
    assert_balanced(snapshot);
    assert_eq!(snapshot.total_owner_bytes(), 0);
    assert_eq!(snapshot.live_ticket_count, 0);
    assert!(snapshot.class_usage.is_empty(), "class residue: {snapshot:?}");
    assert!(snapshot.owner_usage.is_empty(), "owner residue: {snapshot:?}");
    assert!(snapshot.context_usage.is_empty(), "context residue: {snapshot:?}");
}

#[test]
fn distinct_grouped_stores_compete_for_one_global_slot_and_handoff_after_release() {
    let map_bytes = map_cost("map", 200);
    let revocable_bytes = revocable_cost("rev", 200);
    assert!(map_bytes + revocable_bytes > map_bytes);
    let platform = platform(map_bytes, map_bytes);
    let map = Arc::new(map_store(&platform));
    let revocable = Arc::new(revocable_store(&platform));
    let start = Arc::new(Barrier::new(3));

    let map_thread = {
        let map = Arc::clone(&map);
        let start = Arc::clone(&start);
        std::thread::spawn(move || {
            start.wait();
            map.insert_checked("map".into(), vec![7; 200])
        })
    };
    let revocable_thread = {
        let revocable = Arc::clone(&revocable);
        let start = Arc::clone(&start);
        std::thread::spawn(move || {
            start.wait();
            revocable.insert_checked("rev".into(), vec![9; 200])
        })
    };
    start.wait();
    let map_result = map_thread.join().expect("map worker panicked");
    let revocable_result = revocable_thread.join().expect("revocable worker panicked");
    assert_eq!(usize::from(map_result.is_ok()) + usize::from(revocable_result.is_ok()), 1);
    assert!(
        map_result.is_ok() || matches!(map_result, Err(MemoryError::Capacity(_))),
        "unexpected map result: {map_result:?}"
    );
    assert!(
        revocable_result.is_ok() || matches!(revocable_result, Err(MemoryError::Capacity(_))),
        "unexpected revocable result: {revocable_result:?}"
    );

    let snapshot = platform.broker().ledger_snapshot().expect("quiescent snapshot");
    if map_result.is_ok() {
        assert_only_charge(
            &snapshot,
            MemoryClass::MemoResidency,
            MAP_OWNER,
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata,
            map_bytes,
        );
        assert_eq!(map.committed_bytes().expect("map counter"), map_bytes);
        assert_eq!(revocable.committed_bytes().expect("revocable counter"), 0);
        assert_eq!(map.remove(&"map".to_string()).expect("map release"), Some(vec![7; 200]));
        assert_drained(&platform.broker().ledger_snapshot().expect("released snapshot"));
        revocable.insert_checked("rev".into(), vec![9; 200]).expect("peer now fits");
        assert_only_charge(
            &platform.broker().ledger_snapshot().expect("peer snapshot"),
            MemoryClass::ExecutionScratchRevocable,
            REVOCABLE_OWNER,
            MemoryContextKind::Runtime,
            ChargeKind::LiveScratch,
            revocable_bytes,
        );
        assert_eq!(
            revocable.reclaim_one().expect("peer release"),
            Some(("rev".into(), vec![9; 200]))
        );
    } else {
        assert_only_charge(
            &snapshot,
            MemoryClass::ExecutionScratchRevocable,
            REVOCABLE_OWNER,
            MemoryContextKind::Runtime,
            ChargeKind::LiveScratch,
            revocable_bytes,
        );
        assert_eq!(map.committed_bytes().expect("map counter"), 0);
        assert_eq!(revocable.committed_bytes().expect("revocable counter"), revocable_bytes);
        assert_eq!(
            revocable.reclaim_one().expect("revocable release"),
            Some(("rev".into(), vec![9; 200]))
        );
        assert_drained(&platform.broker().ledger_snapshot().expect("released snapshot"));
        map.insert_checked("map".into(), vec![7; 200]).expect("peer now fits");
        assert_only_charge(
            &platform.broker().ledger_snapshot().expect("peer snapshot"),
            MemoryClass::MemoResidency,
            MAP_OWNER,
            MemoryContextKind::Request,
            ChargeKind::RetainedMetadata,
            map_bytes,
        );
        assert_eq!(map.remove(&"map".to_string()).expect("peer release"), Some(vec![7; 200]));
    }
    assert_drained(&platform.broker().ledger_snapshot().expect("final snapshot"));
}

#[test]
fn store_scratch_and_zero_byte_authorities_survive_denied_remap_and_legal_raise() {
    let platform = platform(1_000, 200);
    let map = map_store(&platform);
    let first_cost = map_cost("a", 100);
    map.insert_checked("a".into(), vec![1; 100]).expect("first map entry");

    let runtime = platform
        .attach_runtime("runtime:cross".into(), RuntimeKind::Runtime)
        .expect("attach runtime");
    let context = runtime.context(MemoryContextKind::Session, SCRATCH_OWNER.into());
    let mut scratch = runtime
        .scratch()
        .reserve_scratch_scope(&context, ScratchRequest::new("cross-store scratch", 70))
        .expect("reserve runtime scratch");
    let mut zero: ManagedCharge = platform
        .broker()
        .reserve_managed_charge(
            MemoryClass::ArtifactCache,
            ZERO_OWNER.into(),
            MemoryContextKind::Background,
            ChargeKind::RetainedMetadata,
            0,
        )
        .expect("zero-byte authority");

    let before = platform.broker().ledger_snapshot().expect("before remap");
    assert_balanced(&before);
    assert_eq!(before.total_owner_bytes(), first_cost + 70);
    assert_eq!(before.live_ticket_count, 3, "zero bytes still own a live ticket");
    assert_eq!(before.owner_usage[MAP_OWNER].retained_metadata_bytes, first_cost);
    assert_eq!(before.owner_usage[SCRATCH_OWNER].live_scratch_bytes, 70);
    assert!(!before.owner_usage.contains_key(ZERO_OWNER));
    assert_eq!(
        before.context_usage[&MemoryContextKind::Request].retained_metadata_bytes,
        first_cost
    );
    assert_eq!(before.context_usage[&MemoryContextKind::Session].live_scratch_bytes, 70);

    let denied_policy = policy(1_000, 500).with_owner_group(ZERO_OWNER, "g-other");
    let denied = platform.broker().try_live_reconfigure(denied_policy);
    assert!(
        matches!(denied, Err(MemoryError::InvalidState(_))),
        "live zero-byte remap: {denied:?}"
    );
    assert_eq!(platform.broker().ledger_snapshot().expect("after denial"), before);
    let old_cap_denial = map.insert_checked("b".into(), vec![2; 100]);
    assert!(
        matches!(old_cap_denial, Err(MemoryError::Capacity(_))),
        "old cap still applies: {old_cap_denial:?}"
    );
    assert_eq!(platform.broker().ledger_snapshot().expect("after old-cap denial"), before);

    platform.broker().try_live_reconfigure(policy(1_000, 500)).expect("same mapping, raised cap");
    assert_eq!(platform.broker().ledger_snapshot().expect("after accepted raise"), before);
    map.insert_checked("b".into(), vec![2; 100]).expect("raised cap admits second entry");
    let after = platform.broker().ledger_snapshot().expect("after second entry");
    assert_balanced(&after);
    assert_eq!(after.total_owner_bytes(), first_cost + map_cost("b", 100) + 70);
    assert_eq!(after.live_ticket_count, 4);
    assert_eq!(
        after.owner_usage[MAP_OWNER].retained_metadata_bytes,
        first_cost + map_cost("b", 100)
    );
    assert_eq!(after.owner_usage[SCRATCH_OWNER].live_scratch_bytes, 70);
    assert_eq!(
        after.context_usage[&MemoryContextKind::Request].retained_metadata_bytes,
        first_cost + map_cost("b", 100)
    );
    assert_eq!(after.context_usage[&MemoryContextKind::Session].live_scratch_bytes, 70);

    map.close().expect("close charged store");
    scratch.release().expect("release runtime scratch");
    zero.release().expect("release zero-byte authority");
    assert_drained(&platform.broker().ledger_snapshot().expect("all authorities drained"));
}

/// A host reclaimer evicting from one revocable store unblocks a hard-cap
/// denied insert into a *different* revocable store on the same broker. The
/// callback deliberately under-reports its reclaimed bytes: admission must
/// succeed from the broker's own remeasurement of the ledger, never from the
/// advisory [`ReclaimOutcome`] number.
#[test]
fn other_store_reclaim_frees_admission_and_drains_every_ticket() {
    const VICTIM_OWNER: &str = "owner:cross-revict";

    struct OtherStoreReclaimer {
        victim: OnceLock<Weak<SimpleRevocableStore<String, Vec<u8>, Budgeted>>>,
        inner_results: Mutex<Vec<MemoryResult<SweepStats>>>,
    }

    impl MemoryReclaimer for OtherStoreReclaimer {
        fn reclaim(&self, request: &ReclaimRequest) -> MemoryResult<ReclaimOutcome> {
            let victim = self
                .victim
                .get()
                .expect("victim store registered before use")
                .upgrade()
                .expect("victim remains live during callback");
            let swept = victim.reclaim_until(request.requested_bytes)?;
            self.inner_results.lock().expect("results").push(Ok(swept));
            // Advisory lie: report zero reclaimed bytes while the victim store
            // actually released its FIFO head. The broker must remeasure.
            Ok(ReclaimOutcome { reclaimed_bytes: 0 })
        }
    }

    fn revocable_store_for(
        platform: &LiveMemoryPlatform,
        owner: &str,
    ) -> SimpleRevocableStore<String, Vec<u8>, Budgeted> {
        platform
            .revocable_store_builder::<String, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ExecutionScratchRevocable,
                owner.into(),
                None,
            ))
            .build()
            .expect("budgeted revocable store")
    }

    // The revocable class cap is shared by both stores' owners.
    let victim_entry = revocable_cost("v1", 4);
    let admitted_entry = revocable_cost("a", 4);
    assert_eq!(victim_entry, 38);
    assert_eq!(admitted_entry, 37);
    let class_cap = 2 * victim_entry; // 76: two victims fit, the third insert does not.

    let reclaimer = Arc::new(OtherStoreReclaimer {
        victim: OnceLock::new(),
        inner_results: Mutex::new(Vec::new()),
    });
    let platform = MemoryPlatformBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            ResidencyPolicy::new(10_000)
                .with_class_budget(MemoryClass::ExecutionScratchRevocable, class_budget(class_cap))
                .with_class_budget(MemoryClass::MemoResidency, class_budget(10_000)),
        ))
        .with_reclaimer(Arc::clone(&reclaimer) as Arc<dyn MemoryReclaimer>)
        .build()
        .expect("platform with registered reclaimer");

    let victim = Arc::new(revocable_store_for(&platform, VICTIM_OWNER));
    let admitting = revocable_store_for(&platform, REVOCABLE_OWNER);
    assert!(reclaimer.victim.set(Arc::downgrade(&victim)).is_ok());

    // Two FIFO victims fill the shared class budget.
    victim.insert_checked("v1".into(), vec![0; 4]).expect("first victim");
    victim.insert_checked("v2".into(), vec![0; 4]).expect("second victim");
    let before = platform.broker().ledger_snapshot().expect("before denial");
    assert_balanced(&before);
    assert_eq!(before.total_owner_bytes(), 2 * victim_entry);
    assert_eq!(before.live_ticket_count, 2);
    assert_eq!(before.owner_usage[VICTIM_OWNER].live_scratch_bytes, 2 * victim_entry);

    // The other store's insert is hard-cap denied, the callback evicts the
    // victim FIFO head, and the broker's remeasurement admits the insert.
    admitting.insert_checked("a".into(), vec![0; 4]).expect("admitted after cross-store reclaim");

    let inner = reclaimer.inner_results.lock().expect("results").clone();
    assert_eq!(inner.len(), 1, "exactly one reclaim callback");
    let swept = inner[0].as_ref().expect("victim reclaim succeeds");
    assert_eq!(swept.reclaimed_items, 1);
    assert_eq!(swept.reclaimed_bytes, victim_entry);

    let after = platform.broker().ledger_snapshot().expect("after admission");
    assert_balanced(&after);
    assert_eq!(after.total_owner_bytes(), victim_entry + admitted_entry);
    assert_eq!(after.live_ticket_count, 2);
    assert_eq!(after.owner_usage[VICTIM_OWNER].live_scratch_bytes, victim_entry);
    assert_eq!(after.owner_usage[REVOCABLE_OWNER].live_scratch_bytes, admitted_entry);
    assert_eq!(victim.committed_bytes().expect("victim counter"), victim_entry);
    assert_eq!(admitting.committed_bytes().expect("admitting counter"), admitted_entry);

    // The victim's FIFO lost its head, not its tail.
    assert_eq!(victim.reclaim_one().expect("victim tail"), Some(("v2".into(), vec![0; 4])));
    victim.close().expect("close victim");
    admitting.close().expect("close admitting store");
    assert_drained(&platform.broker().ledger_snapshot().expect("final drain"));
    let weak_victim = Arc::downgrade(&victim);
    drop(victim);
    assert!(weak_victim.upgrade().is_none(), "reclaimer must not pin a closed victim store");
}

/// One mixed-store lifecycle pins every charge-kind axis with independent
/// numeric constants (no production cost helpers): a retained-map entry owns
/// one metadata bucket, a revocable entry owns one scratch bucket, and a
/// prepared publication splits its payload and persistent identity evidence
/// across the payload and metadata buckets. Grand totals alone cannot pass:
/// each owner/class/context entry is asserted separately so a wrong
/// attribution with the right sum fails.
#[test]
#[cfg(feature = "publish_pipeline")]
fn mixed_store_kind_lifetimes_balance_every_axis() {
    use quanta_memory_platform_facade::{
        PreparedPublicationIdentityV1, PreparedPublicationRequestV1,
    };

    const PREPARED_OWNER: &str = "owner:cross-prepared";

    // Independent constants for one-byte keys/values (see the shared oracle
    // helpers above): map = 1+1+24+16 = 42 metadata; revocable = 1+1+16+16
    // = 34 scratch; prepared identity [b'i'] with an empty payload = 17
    // payload + 129 metadata.
    const MAP_ENTRY_BYTES: u64 = 42;
    const REVOCABLE_ENTRY_BYTES: u64 = 34;
    const PREPARED_ACTIVE_BYTES: u64 = 17;
    const PREPARED_PERSISTENT_BYTES: u64 = 129;

    let platform = MemoryPlatformBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            ResidencyPolicy::new(10_000)
                .with_class_budget(MemoryClass::MemoResidency, class_budget(10_000))
                .with_class_budget(MemoryClass::ExecutionScratchRevocable, class_budget(10_000))
                .with_class_budget(MemoryClass::ArtifactCache, class_budget(10_000)),
        ))
        .build()
        .expect("mixed-store platform");

    let map = map_store(&platform);
    let revocable = revocable_store(&platform);
    let prepared = platform
        .prepared_publication_builder::<Vec<u8>>()
        .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, PREPARED_OWNER.into(), None))
        .build()
        .expect("budgeted prepared publication service");

    // One entry per store.
    map.insert_checked("m".into(), vec![9]).expect("map entry");
    revocable.insert_checked("r".into(), vec![8]).expect("revocable entry");
    let receipt = prepared
        .prepare(PreparedPublicationRequestV1::new(
            PreparedPublicationIdentityV1::new(vec![b'i']).expect("identity"),
            Vec::new(),
        ))
        .expect("prepared entry");

    let expected = |payload: u64, metadata: u64, scratch: u64, total: u64, tickets: usize| {
        let snap = platform.broker().ledger_snapshot().expect("snapshot");
        assert_balanced(&snap);
        assert_eq!(snap.total_retained_payload_bytes(), payload, "payload bucket");
        assert_eq!(snap.total_retained_metadata_bytes(), metadata, "metadata bucket");
        assert_eq!(snap.total_live_scratch_bytes(), scratch, "scratch bucket");
        assert_eq!(snap.total_owner_bytes(), total, "grand total");
        assert_eq!(snap.live_ticket_count, tickets, "live tickets");
        // Per-owner attribution: each owner holds exactly its own bucket.
        assert_eq!(
            (
                snap.owner_usage[MAP_OWNER].retained_payload_bytes,
                snap.owner_usage[MAP_OWNER].retained_metadata_bytes,
                snap.owner_usage[MAP_OWNER].live_scratch_bytes
            ),
            (0, MAP_ENTRY_BYTES, 0),
            "map owner holds one metadata bucket"
        );
        assert_eq!(
            (
                snap.owner_usage[REVOCABLE_OWNER].retained_payload_bytes,
                snap.owner_usage[REVOCABLE_OWNER].retained_metadata_bytes,
                snap.owner_usage[REVOCABLE_OWNER].live_scratch_bytes
            ),
            (0, 0, REVOCABLE_ENTRY_BYTES),
            "revocable owner holds one scratch bucket"
        );
        assert_eq!(
            (
                snap.owner_usage[PREPARED_OWNER].retained_payload_bytes,
                snap.owner_usage[PREPARED_OWNER].retained_metadata_bytes,
                snap.owner_usage[PREPARED_OWNER].live_scratch_bytes
            ),
            (payload, metadata - MAP_ENTRY_BYTES, 0),
            "prepared owner splits payload and identity evidence buckets"
        );
        // Per-class attribution detects a correct grand total with the wrong
        // class bucket.
        assert_eq!(
            (
                snap.class_usage[&MemoryClass::MemoResidency].retained_payload_bytes,
                snap.class_usage[&MemoryClass::MemoResidency].retained_metadata_bytes,
                snap.class_usage[&MemoryClass::MemoResidency].live_scratch_bytes
            ),
            (0, MAP_ENTRY_BYTES, 0)
        );
        assert_eq!(
            (
                snap.class_usage[&MemoryClass::ExecutionScratchRevocable].retained_payload_bytes,
                snap.class_usage[&MemoryClass::ExecutionScratchRevocable].retained_metadata_bytes,
                snap.class_usage[&MemoryClass::ExecutionScratchRevocable].live_scratch_bytes
            ),
            (0, 0, REVOCABLE_ENTRY_BYTES)
        );
        assert_eq!(
            (
                snap.class_usage[&MemoryClass::ArtifactCache].retained_payload_bytes,
                snap.class_usage[&MemoryClass::ArtifactCache].retained_metadata_bytes,
                snap.class_usage[&MemoryClass::ArtifactCache].live_scratch_bytes
            ),
            (payload, metadata - MAP_ENTRY_BYTES, 0)
        );
        // Per-context attribution: map and prepared bind Request; the
        // revocable store charges its entries under Runtime.
        assert_eq!(
            (
                snap.context_usage[&MemoryContextKind::Request].retained_payload_bytes,
                snap.context_usage[&MemoryContextKind::Request].retained_metadata_bytes,
                snap.context_usage[&MemoryContextKind::Request].live_scratch_bytes
            ),
            (payload, metadata, 0)
        );
        assert_eq!(
            snap.context_usage[&MemoryContextKind::Runtime].live_scratch_bytes,
            REVOCABLE_ENTRY_BYTES
        );
        assert_eq!(snap.context_usage[&MemoryContextKind::Runtime].retained_payload_bytes, 0);
        assert_eq!(snap.context_usage[&MemoryContextKind::Runtime].retained_metadata_bytes, 0);
    };

    // Live postimage: payload 17, metadata 42+129=171, scratch 34, total 222,
    // four tickets (one map, one revocable, two prepared leases).
    expected(
        PREPARED_ACTIVE_BYTES,
        MAP_ENTRY_BYTES + PREPARED_PERSISTENT_BYTES,
        REVOCABLE_ENTRY_BYTES,
        PREPARED_ACTIVE_BYTES + MAP_ENTRY_BYTES + PREPARED_PERSISTENT_BYTES + REVOCABLE_ENTRY_BYTES,
        4,
    );

    // Abort refunds only the prepared payload lease; the identity tombstone
    // stays in the metadata bucket.
    use quanta_memory_platform_facade::AbortDispositionV1;
    assert_eq!(prepared.abort(receipt.token()).expect("abort"), AbortDispositionV1::Aborted);
    expected(
        0,
        MAP_ENTRY_BYTES + PREPARED_PERSISTENT_BYTES,
        REVOCABLE_ENTRY_BYTES,
        MAP_ENTRY_BYTES + PREPARED_PERSISTENT_BYTES + REVOCABLE_ENTRY_BYTES,
        3,
    );

    // Closing every store drains every axis.
    map.close().expect("close map");
    revocable.close().expect("close revocable");
    prepared.close().expect("close prepared");
    assert_drained(&platform.broker().ledger_snapshot().expect("drained"));
}
