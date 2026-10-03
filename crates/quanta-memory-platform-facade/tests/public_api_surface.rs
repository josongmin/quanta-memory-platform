//! Public API surface snapshot for `quanta-memory-platform-facade`.
//!
//! Mirrors `quanta-memory-platform-core`'s surface guard: it compiles ONLY
//! against `quanta_memory_platform_facade::*` (no `crate::` internals), so an
//! unintended pub addition/removal/rename is caught at the boundary:
//!
//!  - if a re-export is REMOVED or RENAMED, the corresponding never-called
//!    `names_*` fn fails to COMPILE (the path no longer resolves) — a hard
//!    regression signal, not a runtime assertion;
//!  - each item is gated by the `#[cfg(feature = …)]` that owns it, so the guard
//!    is exact under any feature slice and `--no-default-features` still compiles;
//!  - if a stable SHAPE fact changes (a platform factory no longer yields a
//!    budgeted store, an unset builder gains `build`, or a spill target stops
//!    round-tripping), the tests below fail.
//!
//! It is deliberately NOT a string snapshot: it references the real items so the
//! compiler is the guard, and exercises a few load-bearing facts the host adapter
//! and downstream presets depend on.

// ---------------------------------------------------------------------------
// Slice-agnostic surface — always present (no feature gate).
// ---------------------------------------------------------------------------

use quanta_memory_platform_facade::{
    AccountingUnset, ByteSized, CanonicalKey, MeasuredValue, MemoryPlatformBuilder,
    PortableMemoryPresets,
};

// Root re-exports are checked below. These imports additionally pin the
// public module paths that a store-file split could accidentally remove.
#[allow(unused_imports)] // -- reason: this module compiles public paths without runtime use.
mod legacy_store_paths {
    #[cfg(feature = "payload_store")]
    use quanta_memory_platform_facade::payload_store::service::SimplePayloadStore;
    #[cfg(feature = "publish_pipeline")]
    use quanta_memory_platform_facade::publish_pipeline::service::{
        CommitTransition, DefaultCommitTransition, SimplePublishPipeline,
    };
    #[cfg(feature = "retained_map")]
    use quanta_memory_platform_facade::retained_map::service::SimpleRetainedMapStore;
    #[cfg(all(feature = "revocable_store", feature = "spill"))]
    use quanta_memory_platform_facade::revocable_store::service::RevocableSpillHandle;
    #[cfg(feature = "revocable_store")]
    use quanta_memory_platform_facade::revocable_store::service::SimpleRevocableStore;
    #[cfg(feature = "spill-redb")]
    use quanta_memory_platform_facade::spill::backings::redb::{RedbSpillBacking, RedbSpillLimits};
    #[cfg(feature = "spill")]
    use quanta_memory_platform_facade::spill::service::{
        InMemorySpillTarget, NoopSpillController, RoutedSpillController, SpillTarget,
    };
    #[cfg(feature = "transcript")]
    use quanta_memory_platform_facade::transcript_store::service::SimpleSessionTranscriptStore;
}

#[cfg(any(
    feature = "payload_store",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript",
    feature = "publish_pipeline"
))]
use quanta_memory_platform_facade::Budgeted;
#[cfg(feature = "core-live-accounting")]
use quanta_memory_platform_facade::LiveMemoryPlatform;

// Core types the facade surface is expressed in terms of (used by the shape
// assertions below). These are core re-exports consumed through the facade's
// public dependency, not facade internals.
use quanta_memory_platform_core::{MemoryBudgetManifest, MemoryClass, MemoryPreset};

/// Names EVERY always-on facade re-export in type position. Never called; its only
/// job is to fail compilation if any pinned export is removed or renamed. A NEW
/// export must be added here deliberately (the review gate).
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed, the tuple only forces path resolution.
fn names_always_on_reexports() {
    type _Glue = (MemoryPlatformBuilder, PortableMemoryPresets);
    type _Unset = AccountingUnset;
    #[cfg(feature = "core-live-accounting")]
    type _Live = LiveMemoryPlatform;
    #[cfg(any(
        feature = "payload_store",
        feature = "retained_map",
        feature = "revocable_store",
        feature = "transcript",
        feature = "publish_pipeline"
    ))]
    type _Budgeted = Budgeted;
    // `ByteSized` and `CanonicalKey` are object-safe-by-name traits; reference
    // them as trait bounds via a dyn box / fn so a rename breaks compilation.
    type _Measure = (MeasuredValue<u8>, Box<dyn ByteSized>);
    let _canonical_key: fn(&dyn CanonicalKey) = |_k| {};
}

/// Live accounting exposes one sealed platform factory and its broker view, not
/// a user-implemented accounting callback or a detachable sink. The separate
/// reclaim port may request eviction but cannot mutate counters directly.
#[cfg(feature = "core-live-accounting")]
#[test]
fn live_broker_bridge_is_present_only_with_core_live_accounting() {
    let platform = MemoryPlatformBuilder::new()
        .with_preset(MemoryPreset::Production)
        .build()
        .expect("valid platform");
    let _broker = platform.broker();
}

#[cfg(feature = "core-live-accounting")]
#[allow(dead_code)] // some isolated feature slices enable the bridge without a store test consumer.
fn public_test_platform() -> LiveMemoryPlatform {
    MemoryPlatformBuilder::new()
        .with_preset(MemoryPreset::Production)
        .build()
        .expect("valid platform")
}

#[cfg(feature = "payload_store")]
use quanta_memory_platform_facade::{PayloadStore, PayloadStoreBuilder, SimplePayloadStore};

/// Pins the `payload_store` slice's public exports.
#[cfg(feature = "payload_store")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed.
fn names_payload_store_reexports() {
    type _Trait = Box<dyn PayloadStore<String, Vec<u8>>>;
    type _Impl = SimplePayloadStore<String, Vec<u8>, Budgeted>;
    type _Builder = PayloadStoreBuilder<String, Vec<u8>, AccountingUnset>;
}

#[cfg(feature = "retained_map")]
use quanta_memory_platform_facade::{
    BoundRetainedMapStoreBuilder, RetainedMapInsertOrExistingV1, RetainedMapInsertOutcomeV1,
    RetainedMapStore, RetainedMapStoreBuilder, SimpleRetainedMapStore,
};

/// Pins the `retained_map` slice's public exports.
#[cfg(feature = "retained_map")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed.
fn names_retained_map_reexports() {
    type _Trait = Box<dyn RetainedMapStore<String, Vec<u8>>>;
    type _Impl = SimpleRetainedMapStore<String, Vec<u8>, Budgeted>;
    type _Builder = RetainedMapStoreBuilder<String, Vec<u8>, AccountingUnset>;
    type _BoundBuilder = BoundRetainedMapStoreBuilder<String, Vec<u8>, Budgeted>;
    type _AtomicInsertOutcome = RetainedMapInsertOutcomeV1;
    type _AtomicInsertOrExisting = RetainedMapInsertOrExistingV1<Vec<u8>>;
}

#[cfg(feature = "revocable_store")]
use quanta_memory_platform_facade::{RevocableStore, RevocableStoreBuilder, SimpleRevocableStore};

/// Pins the `revocable_store` slice's public exports.
#[cfg(feature = "revocable_store")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed.
fn names_revocable_store_reexports() {
    type _Trait = Box<dyn RevocableStore<String, Vec<u8>>>;
    type _Impl = SimpleRevocableStore<String, Vec<u8>, Budgeted>;
    type _Builder = RevocableStoreBuilder<String, Vec<u8>, AccountingUnset>;
}

#[cfg(feature = "transcript")]
use quanta_memory_platform_facade::{
    SessionTranscriptStore, SessionTranscriptStoreBuilder, SimpleSessionTranscriptStore,
    TranscriptAppendDispositionV1, TranscriptAppendErrorV1, TranscriptAppendIdentityV1,
    TranscriptAppendRequestV1, TranscriptRecordV1, TranscriptRetentionPolicyV2,
    TranscriptRetentionReconfigureErrorV2, TranscriptRetentionStatsV2,
};

/// Pins the `transcript` slice's public exports.
#[cfg(feature = "transcript")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed.
fn names_transcript_reexports() {
    type _Trait = Box<dyn SessionTranscriptStore<String, String, String>>;
    type _Impl = SimpleSessionTranscriptStore<String, String, String, Budgeted>;
    type _Builder = SessionTranscriptStoreBuilder<String, String, String, AccountingUnset>;
    type _Identity = TranscriptAppendIdentityV1<String, String>;
    type _Request = TranscriptAppendRequestV1<String, String, String>;
    type _Record = TranscriptRecordV1<String, String>;
    type _Error = TranscriptAppendErrorV1;
    type _Disposition = TranscriptAppendDispositionV1;
    type _Retention = TranscriptRetentionPolicyV2;
    type _RetentionStats = TranscriptRetentionStatsV2;
    type _RetentionReconfigureError = TranscriptRetentionReconfigureErrorV2;
}

#[cfg(feature = "publish_pipeline")]
use quanta_memory_platform_facade::{
    AbortDispositionV1, CommitTransition, DefaultCommitTransition, PrepareDispositionV1,
    PreparedPublicationErrorV1, PreparedPublicationIdentityV1, PreparedPublicationRequestV1,
    PreparedPublicationService, PreparedPublicationServiceBuilder, PreparedPublicationStatsV1,
    PreparedPublicationTokenV1, PublishPipeline, PublishPipelineBuilder, SimplePublishPipeline,
};

/// Pins the `publish_pipeline` slice's public exports.
#[cfg(feature = "publish_pipeline")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed.
fn names_publish_pipeline_reexports() {
    type _Trait = Box<dyn PublishPipeline<String, String, String>>;
    type _Transition = Box<dyn CommitTransition<String, String, String>>;
    type _DefaultTransition = DefaultCommitTransition;
    type _Impl = SimplePublishPipeline<String, String, String>;
    type _Builder = PublishPipelineBuilder<String, String, String>;
    type _Prepared = PreparedPublicationService<Vec<u8>, Budgeted>;
    type _PreparedBuilder = PreparedPublicationServiceBuilder<Vec<u8>, AccountingUnset>;
    type _PreparedError = PreparedPublicationErrorV1;
    type _PreparedStats = PreparedPublicationStatsV1;
    type _PreparedToken = PreparedPublicationTokenV1;
    type _PreparedDispositions = (PrepareDispositionV1, AbortDispositionV1);
}

#[cfg(feature = "publish_pipeline")]
#[test]
fn prepared_publication_builder_exposes_root_bound_resolution_contract() {
    use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};

    let platform = public_test_platform();
    let service = platform
        .prepared_publication_builder::<Vec<u8>>()
        .bind(StoreBindingHandle::new(
            MemoryClass::ManifestResidency,
            "public-prepared-publication".to_string(),
            None,
        ))
        .with_capacity_bytes(1024)
        .with_tombstone_limits(16, 4096)
        .build()
        .expect("prepared publication builder");
    let identity = PreparedPublicationIdentityV1::new(b"root-owned-id".to_vec()).expect("identity");
    let receipt = service
        .prepare(PreparedPublicationRequestV1::new(identity, b"payload".to_vec()))
        .expect("prepare");

    assert_eq!(receipt.disposition(), PrepareDispositionV1::Applied);
    assert_eq!(service.resolve(receipt.token()).expect("root-bound resolve"), b"payload".to_vec());
    assert!(
        service
            .orphan_tokens(&[receipt.token().clone()])
            .expect("root snapshot validation")
            .is_empty(),
        "a canonical-root reference must exclude its token from orphan enumeration"
    );
    let serialized_token = *receipt.token().as_bytes();
    assert_eq!(PreparedPublicationTokenV1::from_bytes(serialized_token), *receipt.token());
}

#[cfg(feature = "spill")]
use quanta_memory_platform_core::MemoryResult;
#[cfg(feature = "spill")]
use quanta_memory_platform_facade::{
    DurableCommitFailure, DurableRestoreCandidate, DurableRestoreController, DurableRestoreOutcome,
    DurableRestoreTarget, DurableSpillController, DurableSpillTarget, InMemorySpillTarget,
    InProcessSpillController, InProcessSpillTarget, NoopSpillController, RetainingSpillTarget,
    RoutedSpillController, SpillController, SpillInventory, SpillReceipt, SpillTarget,
};

/// Pins the `spill` slice's public exports, including the controller and target
/// traits and every concrete controller/target.
#[cfg(feature = "spill")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed.
fn names_spill_reexports() {
    type _Controllers = (
        Box<dyn SpillController<u64>>,
        NoopSpillController,
        RoutedSpillController<InMemorySpillTarget<u64>>,
    );
    type _Targets =
        (Box<dyn SpillTarget<u64>>, InMemorySpillTarget<u64>, RetainingSpillTarget<u64>);
    type _Inventory = Box<dyn SpillInventory>;
    type _InProcess = (Box<dyn InProcessSpillController<u64>>, Box<dyn InProcessSpillTarget<u64>>);
    type _Durable = (
        Box<dyn DurableSpillController<Vec<u8>>>,
        Box<dyn DurableRestoreController>,
        Box<dyn DurableSpillTarget<Vec<u8>>>,
        Box<dyn DurableRestoreTarget>,
        SpillReceipt,
        DurableRestoreCandidate,
        DurableRestoreOutcome,
        DurableCommitFailure,
    );
}

// ---------------------------------------------------------------------------
// Shape assertions — behavior the public surface promises, not just presence.
// ---------------------------------------------------------------------------

/// `MeasuredValue` preserves the (value, size) pair and exposes it through the
/// public accessors — the measurement seam the accounting path is wired through.
#[test]
fn measured_value_round_trips_value_and_size() {
    let mv = MeasuredValue::new("payload".to_string(), 7);
    assert_eq!(
        mv.value().as_str(),
        "payload",
        "MeasuredValue::value must return the wrapped value"
    );
    assert_eq!(mv.size(), 7, "MeasuredValue::size must return the recorded size");
    // `ByteSized` charges the caller-supplied size verbatim, not the inner length.
    assert_eq!(mv.byte_size(), 7, "MeasuredValue charges the declared size via ByteSized");
    assert_eq!(mv.into_inner().as_str(), "payload", "into_inner must yield the original value");
}

/// `PortableMemoryPresets` exposes all four host-facing presets, and each returns
/// a manifest carrying the full mandatory class taxonomy — proving the facade
/// preset path is wired end-to-end, not a stub.
#[test]
fn portable_presets_cover_every_preset_with_full_class_taxonomy() {
    let manifests: [(MemoryPreset, MemoryBudgetManifest); 4] = [
        (MemoryPreset::Development, PortableMemoryPresets::development()),
        (MemoryPreset::Ci, PortableMemoryPresets::ci()),
        (MemoryPreset::Production, PortableMemoryPresets::production()),
        (MemoryPreset::Interactive, PortableMemoryPresets::interactive()),
    ];
    let expected_classes = MemoryClass::mandatory().len();
    for (preset, manifest) in &manifests {
        assert_eq!(
            manifest.residency.classes.len(),
            expected_classes,
            "{preset:?} preset manifest must define exactly the mandatory class set"
        );
    }
}

/// The builder-first surface materializes a broker from a preset through the
/// portable facade entry point (no raw broker constructor). Pins that
/// `MemoryPlatformBuilder` is wired to the core builder, not merely re-exported.
#[test]
#[cfg(feature = "core-live-accounting")]
fn memory_platform_builder_builds_a_broker_from_a_preset() {
    let result = MemoryPlatformBuilder::new().with_preset(MemoryPreset::Production).build();
    assert!(result.is_ok(), "a preset manifest is structurally valid and must build: {result:?}");
}

/// A `MemoryPlatformBuilder` with neither manifest nor preset fails closed with a
/// typed error (never panics, never a default broker) — pinning the builder's
/// fail-closed error contract.
#[test]
#[cfg(feature = "core-live-accounting")]
fn memory_platform_builder_without_config_fails_closed() {
    let result = MemoryPlatformBuilder::new().build();
    assert!(result.is_err(), "an unconfigured builder must fail closed, got Ok");
}

#[test]
fn report_only_builder_resolves_without_runtime_authority() {
    let report = MemoryPlatformBuilder::new()
        .with_preset(MemoryPreset::Production)
        .build_report_only()
        .expect("preset resolves");
    assert_eq!(report.preset, MemoryPreset::Production);
}

#[test]
fn report_only_builder_without_config_fails_closed() {
    let result = MemoryPlatformBuilder::new().build_report_only();
    assert!(result.is_err(), "an unconfigured report builder must fail closed");
}

#[cfg(all(feature = "core-live-accounting", feature = "retained_map"))]
#[test]
fn default_builder_over_cap_store_mutation_is_denied() {
    use quanta_memory_platform_core::{
        ExecutionBudget, MemoryClassBudget, ReclaimPriority, ResidencyPolicy, StoreBindingHandle,
    };

    let residency = ResidencyPolicy::new(100).with_class_budget(
        MemoryClass::ArtifactCache,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: 64,
            hard_cap_bytes: 64,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        },
    );
    let manifest =
        MemoryBudgetManifest::new(MemoryPreset::Ci, ExecutionBudget::default(), residency);
    let platform =
        MemoryPlatformBuilder::new().with_manifest(manifest).build().expect("enforced platform");
    let binding = StoreBindingHandle::new(
        MemoryClass::ArtifactCache,
        "default-builder-owner".to_string(),
        None,
    );
    let store = platform.retained_map_store_builder::<String, Vec<u8>>().bind(binding).build();
    // 20 payload + 4 key + 24 hash slot + 16 state = class cap 64.
    store.insert_checked("fits".to_string(), vec![0; 20]).expect("at cap");
    let before = platform.broker().ledger_snapshot().expect("before denial");

    let denied = store.insert_checked("over".to_string(), vec![1]);
    assert!(
        matches!(denied, Err(quanta_memory_platform_core::MemoryError::Capacity(_))),
        "cap+1 store mutation must return typed Capacity denial"
    );
    assert_eq!(platform.broker().ledger_snapshot().expect("after denial"), before);
    assert!(
        store.get_checked(&"over".to_string()).expect("checked read").is_none(),
        "denied value was not retained"
    );
}

#[cfg(feature = "core-live-accounting")]
#[test]
fn default_builder_and_report_only_share_override_resolution() {
    use quanta_memory_platform_core::{
        BudgetOverrideLayer, ExecutionBudget, MemoryClassBudget, OverrideSource, ReclaimPriority,
        ResidencyPolicy,
    };

    let residency = ResidencyPolicy::new(100).with_class_budget(
        MemoryClass::ArtifactCache,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: 80,
            hard_cap_bytes: 80,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Normal,
        },
    );
    let builder = MemoryPlatformBuilder::new()
        .with_manifest(MemoryBudgetManifest::new(
            MemoryPreset::Ci,
            ExecutionBudget::default(),
            residency,
        ))
        .with_override_layer(
            OverrideSource::TestFixture,
            BudgetOverrideLayer::empty().with_global_hard_limit_bytes(80),
        );
    let expected = builder.clone().build_report_only().expect("report");
    let platform = builder.build().expect("live platform");
    let actual = platform.broker().effective_report().expect("live report");

    assert_eq!(actual, expected);
    assert_eq!(actual.residency.global_hard_limit_bytes, 80);
    assert_eq!(
        actual.override_provenance.last().map(|record| record.source),
        Some(OverrideSource::TestFixture)
    );
}

#[cfg(feature = "core-live-accounting")]
#[test]
fn platform_builder_installs_reclaimer_before_live_construction() {
    use quanta_memory_platform_core::{
        MemoryReclaimer, MemoryResult, ReclaimOutcome, ReclaimRequest,
    };

    struct NoopReclaimer;
    impl MemoryReclaimer for NoopReclaimer {
        fn reclaim(&self, _request: &ReclaimRequest) -> MemoryResult<ReclaimOutcome> {
            Ok(ReclaimOutcome::default())
        }
    }

    let platform = MemoryPlatformBuilder::new()
        .with_preset(MemoryPreset::Ci)
        .with_reclaimer(std::sync::Arc::new(NoopReclaimer))
        .build()
        .expect("builder accepts the reclaim port before construction");
    assert_eq!(platform.broker().reclaim_runtime_stats().expect("stats").attempts, 0);
}

/// The `payload_store` builder yields a `SimplePayloadStore`, and a fresh store
/// reports zero charged bytes — pinning that the builder is wired to a real,
/// queryable store (not a presence-only re-export).
#[cfg(feature = "payload_store")]
#[test]
fn payload_store_builder_yields_a_queryable_store() {
    use quanta_memory_platform_core::StoreBindingHandle;

    let platform = public_test_platform();
    let binding = StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-a".to_string(), None);
    let store: SimplePayloadStore<String, Vec<u8>, Budgeted> = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(binding)
        .build()
        .expect("store builds");
    assert_eq!(
        store.charged_bytes().expect("charged_bytes is queryable"),
        0,
        "a fresh payload store has charged zero bytes"
    );
    assert_eq!(store.stats().expect("checked payload stats").item_count, 0);
}

#[cfg(all(feature = "core-live-accounting", feature = "retained_map"))]
#[test]
fn retained_map_builder_exposes_public_charged_path() {
    use quanta_memory_platform_core::StoreBindingHandle;

    let platform = public_test_platform();
    let binding =
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-retained".to_string(), None);
    let store: SimpleRetainedMapStore<String, Vec<u8>, Budgeted> =
        platform.retained_map_store_builder::<String, Vec<u8>>().bind(binding).build();

    store.insert_checked("k".to_string(), vec![0u8; 32]).expect("insert");
    assert_eq!(platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(), 73);
}

#[cfg(all(feature = "retained_map", feature = "unbudgeted-stores"))]
#[test]
fn explicit_unbudgeted_store_is_a_distinct_product_and_does_not_charge_the_platform() {
    use quanta_memory_platform_core::StoreBindingHandle;
    use quanta_memory_platform_facade::Unbudgeted;

    fn only_budgeted(_: &SimpleRetainedMapStore<String, Vec<u8>, Budgeted>) {}
    fn only_unbudgeted(_: &SimpleRetainedMapStore<String, Vec<u8>, Unbudgeted>) {}

    let platform = public_test_platform();
    let binding =
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "mode-owner".to_string(), None);
    let budgeted =
        platform.retained_map_store_builder::<String, Vec<u8>>().bind(binding.clone()).build();
    let unbudgeted = RetainedMapStoreBuilder::<String, Vec<u8>, AccountingUnset>::new()
        .unbudgeted()
        .bind(binding)
        .build();
    only_budgeted(&budgeted);
    only_unbudgeted(&unbudgeted);

    unbudgeted.insert_checked("fixture".to_string(), vec![0; 7]).expect("explicit fixture write");
    assert_eq!(unbudgeted.stats_checked().expect("fixture stats").estimated_bytes, 54);
    assert_eq!(platform.broker().ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
    budgeted.insert_checked("production".to_string(), vec![0; 11]).expect("charged write");
    assert_eq!(platform.broker().ledger_snapshot().expect("ledger").total_owner_bytes(), 61);
}

#[cfg(feature = "retained_map")]
#[test]
fn retained_map_atomic_insert_returns_occupied_value_without_overwrite() {
    use std::sync::Arc;

    use quanta_memory_platform_core::StoreBindingHandle;

    let platform = public_test_platform();
    let binding =
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-occupied".to_string(), None);
    let store: SimpleRetainedMapStore<String, Arc<Vec<u8>>, Budgeted> =
        platform.retained_map_store_builder::<String, Arc<Vec<u8>>>().bind(binding).build();
    let retained = Arc::new(vec![1, 2, 3]);

    let first = store
        .insert_if_absent_or_get_checked("k".to_string(), Arc::clone(&retained))
        .expect("first atomic insert");
    assert_eq!(first, RetainedMapInsertOrExistingV1::Inserted);

    let duplicate = store
        .insert_if_absent_or_get_checked("k".to_string(), Arc::new(vec![9, 9, 9, 9]))
        .expect("occupied atomic insert");
    let RetainedMapInsertOrExistingV1::AlreadyPresent(existing) = duplicate else {
        panic!("occupied insert must return the existing value");
    };
    assert!(
        Arc::ptr_eq(&existing, &retained),
        "occupied result must share the retained payload allocation"
    );
    let stored = store.get_checked(&"k".to_string()).expect("checked read").expect("stored value");
    assert!(
        Arc::ptr_eq(&stored, &retained),
        "occupied insert must preserve the original allocation"
    );
    assert_eq!(store.stats_checked().expect("stats").estimated_bytes, 44);
}

#[cfg(all(feature = "core-live-accounting", feature = "revocable_store"))]
#[test]
fn revocable_builder_exposes_public_charged_path() {
    use quanta_memory_platform_core::StoreBindingHandle;

    let platform = public_test_platform();
    let binding =
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-revocable".to_string(), None);
    let store: SimpleRevocableStore<String, Vec<u8>, Budgeted> = platform
        .revocable_store_builder::<String, Vec<u8>>()
        .bind(binding)
        .build()
        .expect("budgeted revocable builder must build");

    store.insert_checked("k".to_string(), vec![0u8; 24]).expect("insert");
    assert_eq!(store.stats().expect("checked revocable stats").estimated_bytes, 57);
    assert_eq!(platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(), 57);
}

#[cfg(all(feature = "core-live-accounting", feature = "transcript"))]
#[test]
fn transcript_builder_exposes_public_charged_path() {
    use quanta_memory_platform_core::StoreBindingHandle;

    let platform = public_test_platform();
    let binding =
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-transcript".to_string(), None);
    let store: SimpleSessionTranscriptStore<String, String, String, Budgeted> = platform
        .transcript_store_builder::<String, String, String>()
        .bind(binding)
        .with_retention_policy(TranscriptRetentionPolicyV2::default())
        .build()
        .expect("budgeted transcript builder must build");

    let disposition = store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new("s".to_string(), 0, "m0".to_string()),
            "hello".to_string(),
        ))
        .expect("append");
    assert_eq!(disposition, TranscriptAppendDispositionV1::Applied);
    assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
    assert_eq!(store.stats().expect("checked transcript stats").estimated_bytes, 236);
    let retention = store.retention_stats().expect("retention stats");
    assert_eq!(retention.active_row_count, 1);
    assert_eq!(retention.evidence_count, 1);
    assert_eq!(retention.total_retained_bytes, 236);
    assert_eq!(platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(), 236);
}

/// The `spill` slice is a genuine two-way tier: a routed controller over a
/// retaining in-memory target ACCEPTS a handle, counts it, then RESTORES it (FIFO)
/// and decrements the count. Pins both `spill` and `restore` round-trip behavior.
#[cfg(feature = "spill")]
#[test]
fn spill_controller_round_trips_a_handle_via_restore() {
    let controller = RoutedSpillController::new(InMemorySpillTarget::<u64>::available());

    let spilled = controller.spill(42);
    assert!(spilled.is_ok(), "an available target must accept the spill: {spilled:?}");
    assert_eq!(controller.spilled_count(), Ok(1), "a successful spill increments inventory");

    let restored: MemoryResult<Option<u64>> = controller.restore();
    assert_eq!(
        restored.expect("restore succeeds on a retaining target"),
        Some(42),
        "a retaining target restores the spilled handle"
    );
    assert_eq!(controller.spilled_count(), Ok(0), "restoring a handle decrements inventory");
}

/// A routed controller over an UNAVAILABLE target fails closed on spill (it never
/// silently drops the handle) — pinning the negative path of the spill contract.
#[cfg(feature = "spill")]
#[test]
fn spill_controller_fails_closed_when_target_unavailable() {
    let controller = RoutedSpillController::new(InMemorySpillTarget::<u64>::unavailable());
    let result = controller.spill(7);
    let failure = result.expect_err("an unavailable spill target must fail closed");
    assert!(matches!(failure.error(), quanta_memory_platform_core::MemoryError::Capacity(_)));
    assert_eq!(failure.into_parts().1, 7, "failed spill returns the only handle");
    assert_eq!(controller.spilled_count(), Ok(0), "a failed spill must not increment inventory");
}

/// No configured target must not certify a destructive discard as persistence.
#[cfg(feature = "spill")]
#[test]
fn noop_spill_controller_rejects_and_restores_nothing() {
    let controller = NoopSpillController;
    let failure = SpillController::<u64>::spill(&controller, 1).expect_err("no target");
    assert_eq!(failure.into_parts().1, 1);
    assert_eq!(
        SpillController::<u64>::restore(&controller).expect("noop restore succeeds"),
        None,
        "the noop controller retains nothing to restore"
    );
}
