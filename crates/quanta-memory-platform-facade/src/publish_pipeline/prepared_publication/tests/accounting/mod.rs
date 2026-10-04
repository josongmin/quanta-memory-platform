use super::*;
use quanta_memory_platform_core::{
    ExecutionBudget, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClassBudget,
    MemoryPreset, ReclaimPriority, ResidencyPolicy,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{OnceLock, Weak};

mod tombstone_reservation;

#[derive(Clone, Debug)]
struct PanicOnDropPayload {
    bytes: Vec<u8>,
    drops: Arc<AtomicUsize>,
}

impl ByteSized for PanicOnDropPayload {
    fn byte_size(&self) -> u64 {
        self.bytes.len() as u64
    }
}

impl CanonicalKey for PanicOnDropPayload {
    fn canonical_key(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

impl Drop for PanicOnDropPayload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        panic!("client payload destructor panic");
    }
}

struct ReentrantClonePayload {
    bytes: Vec<u8>,
    service: Arc<OnceLock<Weak<PreparedPublicationService<Self, Budgeted>>>>,
    clone_hits: Arc<AtomicUsize>,
}

impl Clone for ReentrantClonePayload {
    fn clone(&self) -> Self {
        if let Some(service) = self.service.get().and_then(Weak::upgrade) {
            assert!(matches!(
                service.stats(),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(resource)))
                    if resource == "prepared publication service"
            ));
            self.clone_hits.fetch_add(1, Ordering::SeqCst);
        }
        Self {
            bytes: self.bytes.clone(),
            service: Arc::clone(&self.service),
            clone_hits: Arc::clone(&self.clone_hits),
        }
    }
}

impl ByteSized for ReentrantClonePayload {
    fn byte_size(&self) -> u64 {
        self.bytes.len() as u64
    }
}

impl CanonicalKey for ReentrantClonePayload {
    fn canonical_key(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

#[derive(Clone)]
struct ReentrantMeasurePayload {
    bytes: Vec<u8>,
    service: Arc<OnceLock<Weak<PreparedPublicationService<Self, Budgeted>>>>,
    measure_hits: Arc<AtomicUsize>,
}

impl ByteSized for ReentrantMeasurePayload {
    fn byte_size(&self) -> u64 {
        let service = self.service.get().and_then(Weak::upgrade).expect("bound service");
        assert!(matches!(
            service.stats(),
            Err(PreparedPublicationErrorV1::Storage(
                MemoryError::OperationInProgress(resource)
            )) if resource == "prepared publication service"
        ));
        self.measure_hits.fetch_add(1, Ordering::SeqCst);
        self.bytes.len() as u64
    }
}

impl CanonicalKey for ReentrantMeasurePayload {
    fn canonical_key(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

#[derive(Debug)]
struct OneShotPanicPayload {
    bytes: Vec<u8>,
    panic_measure: Arc<AtomicBool>,
    panic_clone: Arc<AtomicBool>,
}

impl Clone for OneShotPanicPayload {
    fn clone(&self) -> Self {
        if self.panic_clone.swap(false, Ordering::SeqCst) {
            panic!("client clone panic");
        }
        Self {
            bytes: self.bytes.clone(),
            panic_measure: Arc::clone(&self.panic_measure),
            panic_clone: Arc::clone(&self.panic_clone),
        }
    }
}

impl ByteSized for OneShotPanicPayload {
    fn byte_size(&self) -> u64 {
        if self.panic_measure.swap(false, Ordering::SeqCst) {
            panic!("client measurement panic");
        }
        self.bytes.len() as u64
    }
}

impl CanonicalKey for OneShotPanicPayload {
    fn canonical_key(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

fn one_shot_request(
    identity: &str,
    panic_measure: &Arc<AtomicBool>,
    panic_clone: &Arc<AtomicBool>,
) -> PreparedPublicationRequestV1<OneShotPanicPayload> {
    PreparedPublicationRequestV1::new(
        PreparedPublicationIdentityV1::new(identity.as_bytes().to_vec()).expect("identity"),
        OneShotPanicPayload {
            bytes: b"owned".to_vec(),
            panic_measure: Arc::clone(panic_measure),
            panic_clone: Arc::clone(panic_clone),
        },
    )
}

fn panic_payload_request(
    identity: &str,
    bytes: &[u8],
    drops: &Arc<AtomicUsize>,
) -> PreparedPublicationRequestV1<PanicOnDropPayload> {
    PreparedPublicationRequestV1::new(
        PreparedPublicationIdentityV1::new(identity.as_bytes().to_vec()).expect("identity"),
        PanicOnDropPayload { bytes: bytes.to_vec(), drops: Arc::clone(drops) },
    )
}

struct RetireDuringUnwind<'a> {
    service: &'a PreparedPublicationService<PanicOnDropPayload, Budgeted>,
    gc: bool,
}

impl Drop for RetireDuringUnwind<'_> {
    fn drop(&mut self) {
        if self.gc {
            self.service.gc_orphans(&[]).expect("orphan GC during unwind");
        } else {
            self.service.close().expect("close during unwind");
        }
    }
}

fn broker(class_hard: u64) -> LiveProcessMemoryBroker {
    let residency = ResidencyPolicy::new(1_000_000).with_class_budget(
        MemoryClass::ManifestResidency,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: class_hard,
            hard_cap_bytes: class_hard,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        },
    );
    quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            residency,
        ))
        .build()
        .expect("valid broker")
}

fn charged(broker: &LiveProcessMemoryBroker) -> PreparedPublicationService<Vec<u8>, Budgeted> {
    PreparedPublicationService::new_budgeted(binding(), None, None, broker.clone())
}

#[test]
fn large_identity_with_zero_payload_is_denied_before_retention() {
    let broker = broker(100);
    let service = charged(&broker);
    let identity = PreparedPublicationIdentityV1::new(vec![7; 101]).expect("identity");
    let request = PreparedPublicationRequestV1::new(identity, Vec::new());
    assert!(matches!(
        service.prepare(request),
        Err(PreparedPublicationErrorV1::Accounting(MemoryError::Capacity(_)))
    ));
    assert_eq!(service.stats().expect("unchanged stats").prepared_count, 0);
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
}

#[test]
fn prepared_identity_index_and_tombstone_keep_exact_independent_cost() {
    use quanta_memory_platform_core::UsageBreakdown;
    use std::collections::BTreeMap;

    // Independent numeric oracle: identity `[b'i']` with an empty payload.
    // The numbers are pinned literally and never computed through production
    // cost helpers. The active lease charges 17 semantic bytes (1 identity +
    // 16 entry state) in the RetainedPayload bucket; the persistent
    // identity/index/tombstone lease charges 129 semantic bytes (1 identity +
    // 48 ordered-index + 80 token/fixed overhead) in the RetainedMetadata
    // bucket.
    const ACTIVE_PAYLOAD_BUCKET: u64 = 17;
    const PERSISTENT_METADATA_BUCKET: u64 = 129;
    const LIVE_TOTAL: u64 = ACTIVE_PAYLOAD_BUCKET + PERSISTENT_METADATA_BUCKET;

    fn kind_axes(broker: &LiveProcessMemoryBroker) -> (u64, u64, u64, u64, usize) {
        let snap = broker.ledger_snapshot().expect("snapshot");
        (
            snap.total_retained_payload_bytes(),
            snap.total_retained_metadata_bytes(),
            snap.total_live_scratch_bytes(),
            snap.total_owner_bytes(),
            snap.live_ticket_count,
        )
    }

    fn sole_axis<K: Ord>(axis: &BTreeMap<K, UsageBreakdown>) -> (u64, u64, u64) {
        assert_eq!(axis.len(), 1, "fixture owns exactly one axis entry");
        let usage = axis.values().next().expect("sole axis entry");
        (usage.retained_payload_bytes, usage.retained_metadata_bytes, usage.live_scratch_bytes)
    }

    fn assert_axes(broker: &LiveProcessMemoryBroker, payload: u64, metadata: u64) {
        let snap = broker.ledger_snapshot().expect("snapshot");
        assert_eq!(sole_axis(&snap.class_usage), (payload, metadata, 0));
        assert_eq!(sole_axis(&snap.owner_usage), (payload, metadata, 0));
        assert_eq!(sole_axis(&snap.context_usage), (payload, metadata, 0));
    }

    let broker = broker(1_000_000);
    let service = charged(&broker);

    // Empty preimage: no charge on any axis.
    assert_eq!(kind_axes(&broker), (0, 0, 0, 0, 0));

    let receipt = service.prepare(request("i", b"")).expect("prepare");
    // Two owned lifetime components, two exact leases, two kind buckets.
    assert_eq!(
        kind_axes(&broker),
        (ACTIVE_PAYLOAD_BUCKET, PERSISTENT_METADATA_BUCKET, 0, LIVE_TOTAL, 2)
    );
    assert_axes(&broker, ACTIVE_PAYLOAD_BUCKET, PERSISTENT_METADATA_BUCKET);

    // Identical replay returns the same token without another lease pair.
    let replay = service.prepare(request("i", b"")).expect("replay");
    assert_eq!(replay.token(), receipt.token());
    assert_eq!(
        kind_axes(&broker),
        (ACTIVE_PAYLOAD_BUCKET, PERSISTENT_METADATA_BUCKET, 0, LIVE_TOTAL, 2)
    );

    // Failed prepare (broker rebuild fence denies the reservation) charges
    // nothing and leaves the live entry untouched.
    let rebuild_fence1 = broker.begin_rebuild().expect("begin rebuild");
    assert!(matches!(
        service.prepare(request("j", b"")),
        Err(PreparedPublicationErrorV1::Accounting(_))
    ));
    assert_eq!(
        kind_axes(&broker),
        (ACTIVE_PAYLOAD_BUCKET, PERSISTENT_METADATA_BUCKET, 0, LIVE_TOTAL, 2)
    );

    // Failed release retry under the same fence: the abort is denied and both
    // leases keep their original kinds, bytes, and data.
    assert!(matches!(
        service.abort(receipt.token()),
        Err(PreparedPublicationErrorV1::Accounting(_))
    ));
    assert_eq!(
        kind_axes(&broker),
        (ACTIVE_PAYLOAD_BUCKET, PERSISTENT_METADATA_BUCKET, 0, LIVE_TOTAL, 2)
    );
    assert_eq!(service.resolve(receipt.token()).expect("data intact"), Vec::new());
    assert!(rebuild_fence1.finish().expect("finish rebuild").drain_error.is_none());

    // Orphan GC refunds only the active payload component of the collected
    // entry; its identity evidence stays behind as a metadata tombstone.
    let collected = service.prepare(request("j", b"")).expect("second entry");
    assert_eq!(
        kind_axes(&broker),
        (2 * ACTIVE_PAYLOAD_BUCKET, 2 * PERSISTENT_METADATA_BUCKET, 0, 2 * LIVE_TOTAL, 4)
    );
    assert_axes(&broker, 2 * ACTIVE_PAYLOAD_BUCKET, 2 * PERSISTENT_METADATA_BUCKET);
    assert_eq!(service.gc_orphans(&[receipt.token().clone()]).expect("gc"), 1);
    assert!(matches!(
        service.resolve(collected.token()),
        Err(PreparedPublicationErrorV1::AbortedToken { .. })
    ));
    assert_eq!(
        kind_axes(&broker),
        (
            ACTIVE_PAYLOAD_BUCKET,
            2 * PERSISTENT_METADATA_BUCKET,
            0,
            ACTIVE_PAYLOAD_BUCKET + 2 * PERSISTENT_METADATA_BUCKET,
            3
        )
    );
    assert_axes(&broker, ACTIVE_PAYLOAD_BUCKET, 2 * PERSISTENT_METADATA_BUCKET);

    // Abort of the remaining live entry refunds its active lease; both
    // identity tombstones stay in the metadata bucket until close.
    assert_eq!(service.abort(receipt.token()).expect("abort"), AbortDispositionV1::Aborted);
    let stats = service.stats().expect("tombstone stats");
    assert_eq!(stats.prepared_count, 0);
    assert_eq!(stats.tombstone_count, 2);
    assert_eq!(stats.tombstone_bytes, 2 * PERSISTENT_METADATA_BUCKET);
    assert_eq!(stats.charged_bytes, 2 * PERSISTENT_METADATA_BUCKET);
    assert_eq!(
        kind_axes(&broker),
        (0, 2 * PERSISTENT_METADATA_BUCKET, 0, 2 * PERSISTENT_METADATA_BUCKET, 2)
    );
    assert_axes(&broker, 0, 2 * PERSISTENT_METADATA_BUCKET);

    service.close().expect("close evidence");
    assert_eq!(kind_axes(&broker), (0, 0, 0, 0, 0));
}

#[test]
fn orphan_roots_bound_and_fail_closed() {
    let broker = broker(1_000_000);
    let service = charged(&broker);
    let receipt = service.prepare(request("id", b"payload")).expect("prepare");
    let token = receipt.token().clone();
    let unknown = PreparedPublicationTokenV1::from_bytes([0xA5; 32]);
    let before_stats = service.stats().expect("stats");
    let before_ledger = broker.ledger_snapshot().expect("ledger");

    // At the cap (raw duplicates count toward the bound): the input is
    // normalized and the call behaves normally.
    let at_cap = vec![token.clone(); MAX_REFERENCED_ROOT_TOKENS];
    assert!(service.orphan_tokens(&at_cap).expect("at-cap read").is_empty());
    assert_eq!(service.gc_orphans(&at_cap).expect("at-cap gc"), 0);

    // One token above the cap: typed storage capacity error before any
    // state lock or mutation, leaving payload, stats, and the ledger
    // byte-identical for both public entry points.
    let above_cap = vec![token.clone(); MAX_REFERENCED_ROOT_TOKENS + 1];
    assert!(matches!(
        service.orphan_tokens(&above_cap),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::Capacity(_)))
    ));
    assert!(matches!(
        service.gc_orphans(&above_cap),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::Capacity(_)))
    ));
    assert_eq!(service.stats().expect("stats unchanged"), before_stats);
    assert_eq!(broker.ledger_snapshot().expect("ledger unchanged"), before_ledger);
    assert_eq!(service.resolve(&token).expect("payload unchanged"), b"payload".to_vec());

    // A valid call after the denial still succeeds.
    assert!(service
        .orphan_tokens(std::slice::from_ref(&token))
        .expect("valid after denial")
        .is_empty());

    // An unknown token mixed with valid references is a typed prior
    // error that never partially collects the store.
    let mixed = vec![token.clone(), unknown];
    assert!(matches!(
        service.orphan_tokens(&mixed),
        Err(PreparedPublicationErrorV1::UnknownToken { .. })
    ));
    assert!(matches!(
        service.gc_orphans(&mixed),
        Err(PreparedPublicationErrorV1::UnknownToken { .. })
    ));
    assert_eq!(service.stats().expect("no partial gc"), before_stats);
    assert_eq!(broker.ledger_snapshot().expect("ledger unchanged"), before_ledger);

    // A same-thread read while the GC mutation lane is held is typed
    // Busy, never a blocking lock or an empty success.
    let permit = service.lifecycle.begin_mutation().expect("own lane");
    assert!(matches!(
        service.orphan_tokens(std::slice::from_ref(&token)),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
    ));
    drop(permit);
    assert!(service.orphan_tokens(std::slice::from_ref(&token)).expect("reopened").is_empty());

    // An aborted reference is likewise a typed prior error.
    assert_eq!(service.abort(&token).expect("abort"), AbortDispositionV1::Aborted);
    assert!(matches!(
        service.orphan_tokens(std::slice::from_ref(&token)),
        Err(PreparedPublicationErrorV1::AbortedToken { .. })
    ));
    assert!(matches!(
        service.gc_orphans(std::slice::from_ref(&token)),
        Err(PreparedPublicationErrorV1::AbortedToken { .. })
    ));

    service.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}

#[derive(Clone)]
struct OverflowPayload;

impl ByteSized for OverflowPayload {
    fn byte_size(&self) -> u64 {
        u64::MAX
    }
}

impl CanonicalKey for OverflowPayload {
    fn canonical_key(&self) -> Vec<u8> {
        Vec::new()
    }
}

#[test]
fn prepared_component_sum_overflow_is_fail_closed_before_reservation() {
    let broker = broker(1_000_000);
    let service = PreparedPublicationService::<OverflowPayload, Budgeted>::new_budgeted(
        binding(),
        None,
        None,
        broker.clone(),
    );
    let request = PreparedPublicationRequestV1::new(
        PreparedPublicationIdentityV1::new(b"overflow".to_vec()).expect("identity"),
        OverflowPayload,
    );
    assert!(matches!(
        service.prepare(request),
        Err(PreparedPublicationErrorV1::Accounting(MemoryError::Capacity(_)))
    ));
    assert_eq!(service.stats().expect("unchanged stats"), PreparedPublicationStatsV1::default());
    assert_eq!(broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(), 0);
}

#[test]
fn resolve_clone_reentry_under_read_lock_is_typed_busy_and_preserves_charge() {
    let broker = broker(1_000_000);
    let service =
        Arc::new(PreparedPublicationService::<ReentrantClonePayload, Budgeted>::new_budgeted(
            binding(),
            None,
            None,
            broker.clone(),
        ));
    let probe = Arc::new(OnceLock::new());
    assert!(probe.set(Arc::downgrade(&service)).is_ok());
    let clone_hits = Arc::new(AtomicUsize::new(0));
    let receipt = service
        .prepare(PreparedPublicationRequestV1::new(
            PreparedPublicationIdentityV1::new(b"clone-reentry".to_vec()).expect("identity"),
            ReentrantClonePayload {
                bytes: b"owned".to_vec(),
                service: Arc::clone(&probe),
                clone_hits: Arc::clone(&clone_hits),
            },
        ))
        .expect("prepare");

    let resolved = service.resolve(receipt.token()).expect("resolve without deadlock");
    assert_eq!(resolved.bytes, b"owned");
    assert_eq!(clone_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        service.stats().expect("complete state").charged_bytes,
        live_cost("clone-reentry", b"owned")
    );
    assert_eq!(
        broker.ledger_snapshot().expect("unchanged charge").total_owner_bytes(),
        live_cost("clone-reentry", b"owned")
    );
    service.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
}

#[test]
fn prepare_measurement_reentry_is_typed_busy_and_replay_skips_measurement() {
    let broker = broker(1_000_000);
    let service =
        Arc::new(PreparedPublicationService::<ReentrantMeasurePayload, Budgeted>::new_budgeted(
            binding(),
            None,
            None,
            broker.clone(),
        ));
    let probe = Arc::new(OnceLock::new());
    assert!(probe.set(Arc::downgrade(&service)).is_ok());
    let measure_hits = Arc::new(AtomicUsize::new(0));
    let request = || {
        PreparedPublicationRequestV1::new(
            PreparedPublicationIdentityV1::new(b"measure-reentry".to_vec()).expect("identity"),
            ReentrantMeasurePayload {
                bytes: b"owned".to_vec(),
                service: Arc::clone(&probe),
                measure_hits: Arc::clone(&measure_hits),
            },
        )
    };

    let first = service.prepare(request()).expect("first prepare");
    assert_eq!(first.disposition(), PrepareDispositionV1::Applied);
    assert_eq!(measure_hits.load(Ordering::SeqCst), 1);
    let replay = service.prepare(request()).expect("replay");
    assert_eq!(replay.disposition(), PrepareDispositionV1::Replayed);
    assert_eq!(first.token(), replay.token());
    assert_eq!(measure_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        broker.ledger_snapshot().expect("exact charge").total_owner_bytes(),
        live_cost("measure-reentry", b"owned")
    );
    service.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
}

#[test]
fn prepare_measurement_panic_preserves_readability_and_allows_retry() {
    let broker = broker(1_000_000);
    let service = PreparedPublicationService::<OneShotPanicPayload, Budgeted>::new_budgeted(
        binding(),
        None,
        None,
        broker.clone(),
    );
    let panic_measure = Arc::new(AtomicBool::new(true));
    let panic_clone = Arc::new(AtomicBool::new(false));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = service.prepare(one_shot_request("measure-panic", &panic_measure, &panic_clone));
    }))
    .is_err());
    assert_eq!(
        service.stats().expect("state remains readable"),
        PreparedPublicationStatsV1::default()
    );
    assert_eq!(broker.ledger_snapshot().expect("no charge").total_owner_bytes(), 0);

    let receipt = service
        .prepare(one_shot_request("measure-panic", &panic_measure, &panic_clone))
        .expect("retry after client panic");
    assert_eq!(service.resolve(receipt.token()).expect("retry payload").bytes, b"owned");
    panic_measure.store(true, Ordering::SeqCst);
    let replay = service
        .prepare(one_shot_request("measure-panic", &panic_measure, &panic_clone))
        .expect("replay needs no measurement");
    assert_eq!(replay.disposition(), PrepareDispositionV1::Replayed);
    assert!(panic_measure.load(Ordering::SeqCst), "replay must not call ByteSized");
    assert_eq!(
        broker.ledger_snapshot().expect("exact charge").total_owner_bytes(),
        live_cost("measure-panic", b"owned")
    );
    service.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
}

#[test]
fn resolve_clone_panic_preserves_readability_and_charge() {
    let broker = broker(1_000_000);
    let service = PreparedPublicationService::<OneShotPanicPayload, Budgeted>::new_budgeted(
        binding(),
        None,
        None,
        broker.clone(),
    );
    let panic_measure = Arc::new(AtomicBool::new(false));
    let panic_clone = Arc::new(AtomicBool::new(false));
    let receipt = service
        .prepare(one_shot_request("clone-panic", &panic_measure, &panic_clone))
        .expect("prepare");
    let before = service.stats().expect("before stats");
    let charged = broker.ledger_snapshot().expect("before ledger").total_owner_bytes();
    panic_clone.store(true, Ordering::SeqCst);

    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = service.resolve(receipt.token());
    }))
    .is_err());
    assert_eq!(service.stats().expect("state remains readable"), before);
    assert_eq!(broker.ledger_snapshot().expect("charge intact").total_owner_bytes(), charged);
    assert_eq!(service.resolve(receipt.token()).expect("retry resolve").bytes, b"owned");
    service.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
}

#[test]
fn replay_charges_once_and_abort_preserves_identity_evidence_once() {
    let broker = broker(1_000_000);
    let service = charged(&broker);
    let first = service.prepare(request("id", b"payload")).expect("first");
    let replay = service.prepare(request("id", b"payload")).expect("replay");
    assert_eq!(first.token(), replay.token());
    assert_eq!(
        broker.ledger_snapshot().expect("snapshot").total_owner_bytes(),
        live_cost("id", b"payload")
    );
    assert_eq!(service.abort(first.token()).expect("abort"), AbortDispositionV1::Aborted);
    assert_eq!(
        broker.ledger_snapshot().expect("snapshot").total_owner_bytes(),
        evidence_cost("id")
    );
    assert_eq!(service.abort(first.token()).expect("replay"), AbortDispositionV1::Replayed);
    assert_eq!(
        broker.ledger_snapshot().expect("snapshot").total_owner_bytes(),
        evidence_cost("id")
    );
    service.close().expect("close evidence");
    assert_eq!(broker.ledger_snapshot().expect("close").total_owner_bytes(), 0);
}

#[test]
fn failed_abort_keeps_payload_index_and_charge_coherent() {
    let broker = broker(1_000_000);
    let service = charged(&broker);
    let receipt = service.prepare(request("id", b"payload")).expect("prepare");
    let rebuild_fence2 = broker.begin_rebuild().expect("begin rebuild");

    let denied = service.abort(receipt.token());
    assert!(matches!(denied, Err(PreparedPublicationErrorV1::Accounting(_))));
    assert_eq!(service.resolve(receipt.token()).expect("still prepared"), b"payload");
    assert_eq!(service.stats().expect("stats").charged_bytes, live_cost("id", b"payload"));
    assert_eq!(
        broker.ledger_snapshot().expect("snapshot").total_owner_bytes(),
        live_cost("id", b"payload")
    );

    assert!(rebuild_fence2.finish().expect("finish rebuild").drain_error.is_none());
    assert_eq!(service.abort(receipt.token()).expect("abort"), AbortDispositionV1::Aborted);
}

#[test]
fn broker_capacity_denial_leaves_no_local_or_ledger_state() {
    let broker = broker(4);
    let service = charged(&broker);
    let denied = service.prepare(request("id", b"payload"));
    assert!(matches!(
        denied,
        Err(PreparedPublicationErrorV1::Accounting(MemoryError::Capacity(_)))
    ));
    assert_eq!(service.stats().expect("stats"), PreparedPublicationStatsV1::default());
    assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
}

#[test]
fn failed_orphan_gc_preserves_every_payload_index_and_charge() {
    let broker = broker(1_000_000);
    let service = charged(&broker);
    let first = service.prepare(request("id-1", b"one")).expect("first");
    let second = service.prepare(request("id-2", b"two")).expect("second");
    let rebuild_fence3 = broker.begin_rebuild().expect("begin rebuild");

    let denied = service.gc_orphans(&[]);
    assert!(matches!(denied, Err(PreparedPublicationErrorV1::Accounting(_))));
    assert_eq!(service.resolve(first.token()).expect("first intact"), b"one".to_vec());
    assert_eq!(service.resolve(second.token()).expect("second intact"), b"two".to_vec());
    assert_eq!(
        service.stats().expect("stats"),
        PreparedPublicationStatsV1 {
            prepared_count: 2,
            charged_bytes: live_cost("id-1", b"one") + live_cost("id-2", b"two"),
            identity_evidence_count: 2,
            identity_evidence_bytes: evidence_cost("id-1") + evidence_cost("id-2"),
            ..PreparedPublicationStatsV1::default()
        }
    );
    assert_eq!(
        broker.ledger_snapshot().expect("snapshot").total_owner_bytes(),
        live_cost("id-1", b"one") + live_cost("id-2", b"two")
    );

    assert!(rebuild_fence3.finish().expect("finish rebuild").drain_error.is_none());
    assert_eq!(service.gc_orphans(&[]).expect("retry GC"), 2);
    assert_eq!(
        broker.ledger_snapshot().expect("snapshot").total_owner_bytes(),
        evidence_cost("id-1") + evidence_cost("id-2")
    );
    service.close().expect("close evidence");
    assert_eq!(broker.ledger_snapshot().expect("close").total_owner_bytes(), 0);
}

#[test]
fn close_refunds_all_entries_and_is_terminal_and_idempotent() {
    let broker = broker(1_000_000);
    let service = charged(&broker);
    let first = service.prepare(request("id-1", b"one")).expect("first");
    service.prepare(request("id-2", b"two")).expect("second");
    assert_eq!(
        broker.ledger_snapshot().expect("before").total_owner_bytes(),
        live_cost("id-1", b"one") + live_cost("id-2", b"two")
    );

    service.close().expect("close");
    assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
    assert_eq!(service.stats().expect("terminal stats"), PreparedPublicationStatsV1::default());
    assert!(matches!(
        service.resolve(first.token()),
        Err(PreparedPublicationErrorV1::UnknownToken { .. })
    ));
    assert!(matches!(
        service.prepare(request("id-3", b"three")),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::Closed(_)))
    ));
    assert!(matches!(
        service.abort(first.token()),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::Closed(_)))
    ));
    assert!(matches!(
        service.gc_orphans(&[]),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::Closed(_)))
    ));
    service.close().expect("double close");
    assert_eq!(broker.ledger_snapshot().expect("double close ledger").total_owner_bytes(), 0);
}

#[test]
fn failed_close_keeps_exact_payloads_tokens_and_charges_for_retry() {
    let broker = broker(1_000_000);
    let service = charged(&broker);
    let first = service.prepare(request("id-1", b"one")).expect("first");
    let second = service.prepare(request("id-2", b"two")).expect("second");
    let before = service.stats().expect("before");
    let rebuild_fence4 = broker.begin_rebuild().expect("begin rebuild");

    assert!(matches!(service.close(), Err(PreparedPublicationErrorV1::Accounting(_))));
    assert_eq!(service.stats().expect("after failed close"), before);
    assert_eq!(service.resolve(first.token()).expect("first intact"), b"one");
    assert_eq!(service.resolve(second.token()).expect("second intact"), b"two");
    assert_eq!(
        broker.ledger_snapshot().expect("still charged").total_owner_bytes(),
        live_cost("id-1", b"one") + live_cost("id-2", b"two")
    );

    assert!(rebuild_fence4.finish().expect("finish rebuild").drain_error.is_none());
    service.close().expect("retry close");
    assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
}

#[test]
fn drop_refunds_live_entries_and_rebuild_drop_defers_exact_release() {
    let broker = broker(1_000_000);
    {
        let service = charged(&broker);
        service.prepare(request("id-1", b"one")).expect("first");
        service.prepare(request("id-2", b"two")).expect("second");
        assert_eq!(
            broker.ledger_snapshot().expect("live").total_owner_bytes(),
            live_cost("id-1", b"one") + live_cost("id-2", b"two")
        );
    }
    assert_eq!(broker.ledger_snapshot().expect("healthy drop").total_owner_bytes(), 0);

    // The fence must outlive the dropped service: bind it outside
    // the inner scope (initialized after the last prepare) so the
    // deferred drain below owns it.
    let rebuild_fence5;
    {
        let service = charged(&broker);
        service.prepare(request("id-3", b"three")).expect("third");
        rebuild_fence5 = broker.begin_rebuild().expect("begin rebuild");
        assert_eq!(
            broker.ledger_snapshot().expect("before drop").total_owner_bytes(),
            live_cost("id-3", b"three")
        );
    }
    assert_eq!(
        broker.ledger_snapshot().expect("deferred").total_owner_bytes(),
        live_cost("id-3", b"three")
    );
    assert!(rebuild_fence5.finish().expect("drain deferred").drain_error.is_none());
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}

#[test]
fn multiple_panicking_client_drops_during_outer_unwind_drain_exact_charges() {
    for operation in ["drop", "close", "gc"] {
        let broker = broker(1_000_000);
        let drops = Arc::new(AtomicUsize::new(0));
        let service = PreparedPublicationService::<PanicOnDropPayload, Budgeted>::new_budgeted(
            binding(),
            None,
            None,
            broker.clone(),
        );
        for (identity, bytes) in [("one", b"a".as_slice()), ("two", b"bb"), ("three", b"ccc")] {
            service
                .prepare(panic_payload_request(identity, bytes, &drops))
                .expect("prepare panicking payload");
        }
        assert_eq!(
            broker.ledger_snapshot().expect("before unwind").total_owner_bytes(),
            live_cost("one", b"a") + live_cost("two", b"bb") + live_cost("three", b"ccc")
        );

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match operation {
                "drop" => {
                    // The owned service itself must be destroyed while
                    // the outer panic is already unwinding.
                    let _owned = service;
                    panic!("outer unwind");
                }
                "close" | "gc" => {
                    let _retire = RetireDuringUnwind { service: &service, gc: operation == "gc" };
                    panic!("outer unwind");
                }
                _ => unreachable!(),
            }
        }));
        assert!(result.is_err(), "the outer panic must remain observable");
        assert_eq!(drops.load(Ordering::SeqCst), 3, "{operation} must retire all payloads");
        assert_eq!(broker.ledger_snapshot().expect("after unwind").total_owner_bytes(), 0);
    }
}

#[test]
fn mutation_gate_returns_typed_busy_and_exposes_no_partial_plan() {
    let broker = broker(1_000_000);
    let service = charged(&broker);
    let receipt = service.prepare(request("id", b"payload")).expect("prepare");
    let guard = service.lifecycle.begin_mutation().expect("own lane");
    assert!(matches!(
        service.prepare(request("other", b"x")),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
    ));
    assert!(matches!(
        service.abort(receipt.token()),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
    ));
    assert!(matches!(
        service.gc_orphans(&[]),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
    ));
    assert!(matches!(
        service.close(),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
    ));
    assert!(matches!(
        service.stats(),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::OperationInProgress(_)))
    ));
    drop(guard);
    assert_eq!(service.stats().expect("reopened").charged_bytes, live_cost("id", b"payload"));
    // Closed image: the terminal empty stats stay readable and consistent.
    service.close().expect("close");
    let closed = service.stats().expect("closed stats");
    assert_eq!(closed.prepared_count, 0);
    assert_eq!(closed.charged_bytes, 0);
    assert_eq!(closed.tombstone_count, 0);
    assert_eq!(closed.tombstone_bytes, 0);
    assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}

#[test]
fn poisoned_store_reports_error_but_drop_still_releases_every_charge() {
    let broker = broker(1_000_000);
    let service = charged(&broker);
    service.prepare(request("id", b"payload")).expect("prepare");
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = service.state.lock().expect("healthy lock");
        panic!("poison prepared publication state");
    }));
    assert!(matches!(service.stats(), Err(PreparedPublicationErrorV1::Storage(_))));
    assert!(matches!(service.close(), Err(PreparedPublicationErrorV1::Storage(_))));
    assert_eq!(
        broker.ledger_snapshot().expect("still charged").total_owner_bytes(),
        live_cost("id", b"payload")
    );
    drop(service);
    assert_eq!(broker.ledger_snapshot().expect("drop refunds").total_owner_bytes(), 0);
}

#[test]
fn checked_overflow_and_underflow_reject_before_broker_or_index_mutation() {
    let broker = broker(1_000_000);
    let service = charged(&broker);
    {
        let mut state = service.state.lock().expect("state");
        state.charged_bytes = u64::MAX;
    }
    assert!(matches!(
        service.prepare(request("overflow", b"x")),
        Err(PreparedPublicationErrorV1::Capacity { .. })
    ));
    assert_eq!(broker.ledger_snapshot().expect("no charge").total_owner_bytes(), 0);
    {
        let mut state = service.state.lock().expect("state");
        state.charged_bytes = 0;
    }
    let receipt = service.prepare(request("id", b"payload")).expect("prepare");
    let expected_live = live_cost("id", b"payload");
    {
        let mut state = service.state.lock().expect("state");
        state.charged_bytes = 0;
    }
    assert!(matches!(
        service.abort(receipt.token()),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(_)))
    ));
    assert_eq!(service.resolve(receipt.token()).expect("index intact"), b"payload");
    assert_eq!(broker.ledger_snapshot().expect("charge intact").total_owner_bytes(), expected_live);
    {
        let mut state = service.state.lock().expect("state");
        state.charged_bytes = expected_live;
    }
    assert_eq!(service.abort(receipt.token()).expect("retry"), AbortDispositionV1::Aborted);
    assert_eq!(
        broker.ledger_snapshot().expect("persistent evidence").total_owner_bytes(),
        evidence_cost("id")
    );
    service.close().expect("close evidence");
    assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
}

#[test]
fn orphan_gc_rejects_corrupt_active_lease_cost_without_partial_refund() {
    let broker = broker(1_000_000);
    let service = charged(&broker);
    let first = service.prepare(request("id-1", b"a")).expect("first");
    let second = service.prepare(request("id-2", b"b")).expect("second");
    let original_active = {
        let state = service.state.lock().expect("state");
        state.entries.values().map(|entry| entry.active_bytes).collect::<Vec<_>>()
    };
    {
        let mut state = service.state.lock().expect("state");
        for entry in state.entries.values_mut() {
            entry.active_bytes = u64::MAX;
        }
    }

    assert!(matches!(
        service.gc_orphans(&[]),
        Err(PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(_)))
    ));
    assert_eq!(service.resolve(first.token()).expect("first intact"), b"a");
    assert_eq!(service.resolve(second.token()).expect("second intact"), b"b");
    assert_eq!(service.stats().expect("stats").tombstone_count, 0);
    assert_eq!(
        broker.ledger_snapshot().expect("charges intact").total_owner_bytes(),
        live_cost("id-1", b"a") + live_cost("id-2", b"b")
    );

    {
        let mut state = service.state.lock().expect("state");
        for (entry, bytes) in state.entries.values_mut().zip(original_active) {
            entry.active_bytes = bytes;
        }
    }
    assert_eq!(service.gc_orphans(&[]).expect("retry"), 2);
    assert_eq!(
        broker.ledger_snapshot().expect("persistent evidence").total_owner_bytes(),
        evidence_cost("id-1") + evidence_cost("id-2")
    );
    service.close().expect("close evidence");
    assert_eq!(broker.ledger_snapshot().expect("refunded").total_owner_bytes(), 0);
}
