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
//!  - if a stable SHAPE fact changes (a builder no longer yields its store, a
//!    spill target stops round-tripping, `charged_against` is dropped from a
//!    builder), the `#[test]` functions below fail.
//!
//! It is deliberately NOT a string snapshot: it references the real items so the
//! compiler is the guard, and exercises a few load-bearing facts the host adapter
//! and downstream presets depend on.

// ---------------------------------------------------------------------------
// Slice-agnostic surface — always present (no feature gate).
// ---------------------------------------------------------------------------

use quanta_memory_platform_facade::{
    AccountingSink, ByteSized, CanonicalKey, MeasuredValue, MemoryPlatformBuilder,
    PortableMemoryPresets, SharedAccountingSink,
};

// Core types the facade surface is expressed in terms of (used by the shape
// assertions below). These are core re-exports consumed through the facade's
// public dependency, not facade internals.
#[cfg(feature = "core-live-accounting")]
use quanta_memory_platform_core::{
    EffectiveMemoryBudgetReport, ExecutionBudget, LiveProcessMemoryBroker, ResidencyPolicy,
};
use quanta_memory_platform_core::{MemoryBudgetManifest, MemoryClass, MemoryPreset};

/// Names EVERY always-on facade re-export in type position. Never called; its only
/// job is to fail compilation if any pinned export is removed or renamed. A NEW
/// export must be added here deliberately (the review gate).
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed, the tuple only forces path resolution.
fn names_always_on_reexports() {
    type _Glue = (MemoryPlatformBuilder, PortableMemoryPresets);
    type _Accounting = (Box<dyn AccountingSink>, SharedAccountingSink);
    // `ByteSized` and `CanonicalKey` are object-safe-by-name traits; reference
    // them as trait bounds via a dyn box / fn so a rename breaks compilation.
    type _Measure = (MeasuredValue<u8>, Box<dyn ByteSized>);
    let _canonical_key: fn(&dyn CanonicalKey) = |_k| {};
}

/// The accounting seam is always exported, but the facade-to-live-broker bridge
/// exists only when `core-live-accounting` forwards core's `live_control`
/// surface. This pins that feature-specific coercion in public API terms.
#[cfg(feature = "core-live-accounting")]
#[test]
fn live_broker_bridge_is_present_only_with_core_live_accounting() {
    let report = EffectiveMemoryBudgetReport::from_manifest(
        MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            ResidencyPolicy::new(1_000_000),
        ),
        vec!["public-api-surface".to_string()],
    );
    let broker = LiveProcessMemoryBroker::from_report(report).expect("valid broker");
    let _sink: SharedAccountingSink = std::sync::Arc::new(broker);
}

#[cfg(feature = "core-live-accounting")]
fn public_test_broker() -> LiveProcessMemoryBroker {
    let report = EffectiveMemoryBudgetReport::from_manifest(
        MemoryBudgetManifest::new(
            MemoryPreset::Production,
            ExecutionBudget::default(),
            ResidencyPolicy::new(1_000_000),
        ),
        vec!["public-api-surface".to_string()],
    );
    LiveProcessMemoryBroker::from_report(report).expect("valid broker")
}

#[cfg(feature = "payload_store")]
use quanta_memory_platform_facade::{PayloadStore, PayloadStoreBuilder, SimplePayloadStore};

/// Pins the `payload_store` slice's public exports.
#[cfg(feature = "payload_store")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed.
fn names_payload_store_reexports() {
    type _Trait = Box<dyn PayloadStore<String, Vec<u8>>>;
    type _Impl = SimplePayloadStore<String, Vec<u8>>;
    type _Builder = PayloadStoreBuilder<String, Vec<u8>>;
}

#[cfg(feature = "retained_map")]
use quanta_memory_platform_facade::{
    RetainedMapStore, RetainedMapStoreBuilder, SimpleRetainedMapStore,
};

/// Pins the `retained_map` slice's public exports.
#[cfg(feature = "retained_map")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed.
fn names_retained_map_reexports() {
    type _Trait = Box<dyn RetainedMapStore<String, Vec<u8>>>;
    type _Impl = SimpleRetainedMapStore<String, Vec<u8>>;
    type _Builder = RetainedMapStoreBuilder<String, Vec<u8>>;
}

#[cfg(feature = "revocable_store")]
use quanta_memory_platform_facade::{RevocableStore, RevocableStoreBuilder, SimpleRevocableStore};

/// Pins the `revocable_store` slice's public exports.
#[cfg(feature = "revocable_store")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed.
fn names_revocable_store_reexports() {
    type _Trait = Box<dyn RevocableStore<String, Vec<u8>>>;
    type _Impl = SimpleRevocableStore<String, Vec<u8>>;
    type _Builder = RevocableStoreBuilder<String, Vec<u8>>;
}

#[cfg(feature = "transcript")]
use quanta_memory_platform_facade::{
    SessionTranscriptStore, SessionTranscriptStoreBuilder, SimpleSessionTranscriptStore,
};

/// Pins the `transcript` slice's public exports.
#[cfg(feature = "transcript")]
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed.
fn names_transcript_reexports() {
    type _Trait = Box<dyn SessionTranscriptStore<String, String>>;
    type _Impl = SimpleSessionTranscriptStore<String, String>;
    type _Builder = SessionTranscriptStoreBuilder<String, String>;
}

#[cfg(feature = "publish_pipeline")]
use quanta_memory_platform_facade::{
    CommitTransition, DefaultCommitTransition, PublishPipeline, PublishPipelineBuilder,
    SimplePublishPipeline,
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
}

#[cfg(feature = "spill")]
use quanta_memory_platform_core::MemoryResult;
#[cfg(feature = "spill")]
use quanta_memory_platform_facade::{
    InMemorySpillTarget, NoopSpillController, RetainingSpillTarget, RoutedSpillController,
    SpillController, SpillTarget,
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
fn memory_platform_builder_builds_a_broker_from_a_preset() {
    let result = MemoryPlatformBuilder::new().with_preset(MemoryPreset::Production).build();
    assert!(result.is_ok(), "a preset manifest is structurally valid and must build: {result:?}");
}

/// A `MemoryPlatformBuilder` with neither manifest nor preset fails closed with a
/// typed error (never panics, never a default broker) — pinning the builder's
/// fail-closed error contract.
#[test]
fn memory_platform_builder_without_config_fails_closed() {
    let result = MemoryPlatformBuilder::new().build();
    assert!(result.is_err(), "an unconfigured builder must fail closed, got Ok");
}

/// The `payload_store` builder yields a `SimplePayloadStore`, and a fresh store
/// reports zero charged bytes — pinning that the builder is wired to a real,
/// queryable store (not a presence-only re-export).
#[cfg(feature = "payload_store")]
#[test]
fn payload_store_builder_yields_a_queryable_store() {
    use quanta_memory_platform_core::StoreBindingHandle;

    let binding = StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-a".to_string(), None);
    let store: SimplePayloadStore<String, Vec<u8>> =
        PayloadStoreBuilder::<String, Vec<u8>>::new().bind(binding).build().expect("store builds");
    assert_eq!(
        store.charged_bytes().expect("charged_bytes is queryable"),
        0,
        "a fresh payload store has charged zero bytes"
    );
}

#[cfg(all(feature = "core-live-accounting", feature = "retained_map"))]
#[test]
fn retained_map_builder_exposes_public_charged_path() {
    use quanta_memory_platform_core::StoreBindingHandle;

    let broker = public_test_broker();
    let sink: SharedAccountingSink = std::sync::Arc::new(broker.clone());
    let binding =
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-retained".to_string(), None);
    let store: SimpleRetainedMapStore<String, Vec<u8>> =
        RetainedMapStoreBuilder::<String, Vec<u8>>::new()
            .bind(binding)
            .charged_against(sink)
            .build()
            .expect("charged retained map builder must build");

    store.insert_checked("k".to_string(), vec![0u8; 32]).expect("insert");
    assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 32);
}

#[cfg(all(feature = "core-live-accounting", feature = "revocable_store"))]
#[test]
fn revocable_builder_exposes_public_charged_path() {
    use quanta_memory_platform_core::StoreBindingHandle;

    let broker = public_test_broker();
    let sink: SharedAccountingSink = std::sync::Arc::new(broker.clone());
    let binding =
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-revocable".to_string(), None);
    let store: SimpleRevocableStore<String, Vec<u8>> =
        RevocableStoreBuilder::<String, Vec<u8>>::new()
            .bind(binding)
            .charged_against(sink)
            .build()
            .expect("charged revocable builder must build");

    store.insert_checked("k".to_string(), vec![0u8; 24]).expect("insert");
    assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 24);
}

#[cfg(all(feature = "core-live-accounting", feature = "transcript"))]
#[test]
fn transcript_builder_exposes_public_charged_path() {
    use quanta_memory_platform_core::StoreBindingHandle;

    let broker = public_test_broker();
    let sink: SharedAccountingSink = std::sync::Arc::new(broker.clone());
    let binding =
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-transcript".to_string(), None);
    let store: SimpleSessionTranscriptStore<String, String> =
        SessionTranscriptStoreBuilder::<String, String>::new()
            .bind(binding)
            .charged_against(sink)
            .build()
            .expect("charged transcript builder must build");

    store.append("s".to_string(), "hello".to_string()).expect("append");
    assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 5);
}

/// The `spill` slice is a genuine two-way tier: a routed controller over a
/// retaining in-memory target ACCEPTS a handle, counts it, then RESTORES it (FIFO)
/// and decrements the count. Pins both `spill` and `restore` round-trip behavior.
#[cfg(feature = "spill")]
#[test]
fn spill_controller_round_trips_a_handle_via_restore() {
    let controller = RoutedSpillController::new(InMemorySpillTarget::<u64>::available());

    let spilled: MemoryResult<()> = controller.spill(42);
    assert!(spilled.is_ok(), "an available target must accept the spill: {spilled:?}");
    assert_eq!(controller.spilled_count(), 1, "a successful spill increments the count");

    let restored: MemoryResult<Option<u64>> = controller.restore();
    assert_eq!(
        restored.expect("restore succeeds on a retaining target"),
        Some(42),
        "a retaining target restores the spilled handle"
    );
    assert_eq!(controller.spilled_count(), 0, "restoring a handle decrements the spilled count");
}

/// A routed controller over an UNAVAILABLE target fails closed on spill (it never
/// silently drops the handle) — pinning the negative path of the spill contract.
#[cfg(feature = "spill")]
#[test]
fn spill_controller_fails_closed_when_target_unavailable() {
    let controller = RoutedSpillController::new(InMemorySpillTarget::<u64>::unavailable());
    let result: MemoryResult<()> = controller.spill(7);
    assert!(result.is_err(), "an unavailable spill target must fail closed, got Ok");
    assert_eq!(controller.spilled_count(), 0, "a failed spill must not increment the count");
}

/// The `NoopSpillController` always succeeds and never retains — the documented
/// no-op contract for hosts without a configured spill backend.
#[cfg(feature = "spill")]
#[test]
fn noop_spill_controller_succeeds_and_restores_nothing() {
    let controller = NoopSpillController;
    assert!(SpillController::<u64>::spill(&controller, 1).is_ok(), "noop spill always succeeds");
    assert_eq!(
        SpillController::<u64>::restore(&controller).expect("noop restore succeeds"),
        None,
        "the noop controller retains nothing to restore"
    );
}
