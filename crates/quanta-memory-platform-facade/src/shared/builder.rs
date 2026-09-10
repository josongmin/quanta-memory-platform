use quanta_memory_platform_core::{
    BasicProcessMemoryBroker, MemoryBudgetManifest, MemoryPreset, MemoryResult,
    ProcessMemoryBrokerBuilder,
};

// Imports shared by the store/pipeline builders; present only when at least one
// such slice is enabled, so a slice-free build carries no unused imports.
#[cfg(any(
    feature = "payload_store",
    feature = "revocable_store",
    feature = "publish_pipeline",
    feature = "transcript"
))]
use quanta_memory_platform_core::MemoryError;
#[cfg(any(
    feature = "payload_store",
    feature = "publish_pipeline",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript"
))]
use quanta_memory_platform_core::StoreBindingHandle;
#[cfg(any(
    feature = "payload_store",
    feature = "publish_pipeline",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript"
))]
use std::marker::PhantomData;

#[cfg(feature = "payload_store")]
use crate::payload_store::service::SimplePayloadStore;
#[cfg(feature = "publish_pipeline")]
use crate::publish_pipeline::{
    prepared_publication::PreparedPublicationService, service::SimplePublishPipeline,
};
#[cfg(feature = "retained_map")]
use crate::retained_map::service::SimpleRetainedMapStore;
#[cfg(feature = "revocable_store")]
use crate::revocable_store::service::SimpleRevocableStore;
#[cfg(any(
    feature = "payload_store",
    feature = "publish_pipeline",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript"
))]
use crate::shared::accounting::SharedAccountingSink;
#[cfg(feature = "transcript")]
use crate::transcript_store::{service::SimpleSessionTranscriptStore, TranscriptRetentionPolicyV1};
#[cfg(feature = "payload_store")]
use quanta_memory_platform_core::MemoryContextHandle;

#[derive(Clone, Debug, Default)]
pub struct MemoryPlatformBuilder {
    inner: ProcessMemoryBrokerBuilder,
}

impl MemoryPlatformBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_manifest(mut self, manifest: MemoryBudgetManifest) -> Self {
        self.inner = self.inner.with_manifest(manifest);
        self
    }

    pub fn with_preset(mut self, preset: MemoryPreset) -> Self {
        self.inner = self.inner.with_preset(preset);
        self
    }

    pub fn build(self) -> MemoryResult<BasicProcessMemoryBroker> {
        self.inner.build()
    }

    /// Build a live, accounting-backed platform — the builder-first surface for a
    /// CHARGED deployment. Consumers never name the concrete `LiveProcessMemoryBroker`
    /// nor hand-roll `Arc::new(broker.clone())`: the returned [`LiveMemoryPlatform`]
    /// encapsulates the accounting sink and hands out pre-charged store builders.
    #[cfg(feature = "core-live-accounting")]
    pub fn build_live(self) -> MemoryResult<LiveMemoryPlatform> {
        Ok(LiveMemoryPlatform::new(self.inner.build_live()?))
    }
}

/// A live, accounting-backed memory platform built via [`MemoryPlatformBuilder::build_live`].
///
/// It owns the live broker (the accounting SSOT) and the single shared accounting
/// sink derived from it, and exposes pre-charged store builders so every store is
/// wired to the ledger builder-first — closing the DIP gap where consumers
/// previously had to name the concrete broker and compose `Arc::new(broker.clone())`
/// by hand.
#[cfg(feature = "core-live-accounting")]
#[derive(Clone)]
pub struct LiveMemoryPlatform {
    broker: quanta_memory_platform_core::LiveProcessMemoryBroker,
    sink: crate::shared::accounting::SharedAccountingSink,
}

#[cfg(feature = "core-live-accounting")]
impl std::fmt::Debug for LiveMemoryPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveMemoryPlatform").finish_non_exhaustive()
    }
}

#[cfg(feature = "core-live-accounting")]
impl LiveMemoryPlatform {
    fn new(broker: quanta_memory_platform_core::LiveProcessMemoryBroker) -> Self {
        let sink: crate::shared::accounting::SharedAccountingSink =
            std::sync::Arc::new(broker.clone());
        Self { broker, sink }
    }

    /// The live broker (accounting SSOT) for snapshots, reconfigure, rebuild, etc.
    #[must_use]
    pub fn broker(&self) -> &quanta_memory_platform_core::LiveProcessMemoryBroker {
        &self.broker
    }

    /// A cloneable handle to this platform's accounting sink. Encapsulates the
    /// `Arc::new(broker.clone())` glue so consumers never write it.
    #[must_use]
    pub fn sink(&self) -> crate::shared::accounting::SharedAccountingSink {
        std::sync::Arc::clone(&self.sink)
    }

    /// A payload-store builder already charged against this platform's ledger.
    #[cfg(feature = "payload_store")]
    #[must_use]
    pub fn payload_store_builder<PayloadRef, Bytes>(
        &self,
    ) -> PayloadStoreBuilder<PayloadRef, Bytes> {
        PayloadStoreBuilder::new().charged_against(self.sink())
    }

    /// A retained-map-store builder already charged against this platform's ledger.
    #[cfg(feature = "retained_map")]
    #[must_use]
    pub fn retained_map_store_builder<Key, Value>(&self) -> RetainedMapStoreBuilder<Key, Value> {
        RetainedMapStoreBuilder::new().charged_against(self.sink())
    }

    /// A revocable-store builder already charged against this platform's ledger.
    #[cfg(feature = "revocable_store")]
    #[must_use]
    pub fn revocable_store_builder<Key, Value>(&self) -> RevocableStoreBuilder<Key, Value> {
        RevocableStoreBuilder::new().charged_against(self.sink())
    }

    /// A session-transcript-store builder already charged against this platform's ledger.
    #[cfg(feature = "transcript")]
    #[must_use]
    pub fn transcript_store_builder<SessionId, MessageId, Message>(
        &self,
    ) -> SessionTranscriptStoreBuilder<SessionId, MessageId, Message> {
        SessionTranscriptStoreBuilder::new().charged_against(self.sink())
    }

    /// A prepared-publication builder already charged against this platform's ledger.
    #[cfg(feature = "publish_pipeline")]
    #[must_use]
    pub fn prepared_publication_builder<Payload>(
        &self,
    ) -> PreparedPublicationServiceBuilder<Payload> {
        PreparedPublicationServiceBuilder::new().charged_against(self.sink())
    }
}

#[cfg(feature = "payload_store")]
#[derive(Clone, Debug)]
pub struct PayloadStoreBuilder<PayloadRef, Bytes> {
    binding: Option<StoreBindingHandle>,
    context: Option<MemoryContextHandle>,
    charger: Option<SharedAccountingSink>,
    _marker: PhantomData<(PayloadRef, Bytes)>,
}

#[cfg(feature = "payload_store")]
impl<PayloadRef, Bytes> Default for PayloadStoreBuilder<PayloadRef, Bytes> {
    fn default() -> Self {
        Self { binding: None, context: None, charger: None, _marker: PhantomData }
    }
}

#[cfg(feature = "payload_store")]
impl<PayloadRef, Bytes> PayloadStoreBuilder<PayloadRef, Bytes> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(mut self, binding: StoreBindingHandle) -> Self {
        self.binding = Some(binding);
        self
    }

    pub fn with_context(mut self, context: MemoryContextHandle) -> Self {
        self.context = Some(context);
        self
    }

    /// Charge the built store's bytes against a broker ledger (accounting SSOT).
    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }

    pub fn build(self) -> MemoryResult<SimplePayloadStore<PayloadRef, Bytes>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        let mut store = SimplePayloadStore::new(binding, self.context);
        if let Some(sink) = self.charger {
            store = store.charged_against(sink);
        }
        Ok(store)
    }
}

/// Typed builder error for the required-but-missing store binding. Builders are
/// `-> Result` paths, so a missing binding is a typed `Err`, never a panic.
#[cfg(any(
    feature = "payload_store",
    feature = "revocable_store",
    feature = "publish_pipeline",
    feature = "transcript"
))]
fn missing_binding() -> MemoryError {
    MemoryError::build("store binding is required before build")
}

#[cfg(feature = "retained_map")]
/// Unbound retained-map builder state.
///
/// Binding is a compile-time construction prerequisite: this state deliberately
/// has no `build` method.
///
/// ```compile_fail
/// use quanta_memory_platform_facade::RetainedMapStoreBuilder;
///
/// let _ = RetainedMapStoreBuilder::<String, Vec<u8>>::new().build();
/// ```
#[derive(Clone, Debug)]
pub struct RetainedMapStoreBuilder<Key, Value> {
    charger: Option<SharedAccountingSink>,
    _marker: PhantomData<(Key, Value)>,
}

/// Bound retained-map builder state. Only this state can build a store, and the
/// build is infallible because its required binding is owned, not optional.
#[cfg(feature = "retained_map")]
#[derive(Clone, Debug)]
pub struct BoundRetainedMapStoreBuilder<Key, Value> {
    binding: StoreBindingHandle,
    charger: Option<SharedAccountingSink>,
    _marker: PhantomData<(Key, Value)>,
}

#[cfg(feature = "retained_map")]
impl<Key, Value> Default for RetainedMapStoreBuilder<Key, Value> {
    fn default() -> Self {
        Self { charger: None, _marker: PhantomData }
    }
}

#[cfg(feature = "retained_map")]
impl<Key, Value> RetainedMapStoreBuilder<Key, Value> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(self, binding: StoreBindingHandle) -> BoundRetainedMapStoreBuilder<Key, Value> {
        BoundRetainedMapStoreBuilder { binding, charger: self.charger, _marker: PhantomData }
    }

    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }
}

#[cfg(feature = "retained_map")]
impl<Key, Value> BoundRetainedMapStoreBuilder<Key, Value> {
    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }

    pub fn build(self) -> SimpleRetainedMapStore<Key, Value> {
        match self.charger {
            Some(sink) => SimpleRetainedMapStore::new_charged(self.binding, sink),
            None => SimpleRetainedMapStore::new(self.binding),
        }
    }
}

#[cfg(feature = "revocable_store")]
#[derive(Clone, Debug)]
pub struct RevocableStoreBuilder<Key, Value> {
    binding: Option<StoreBindingHandle>,
    charger: Option<SharedAccountingSink>,
    _marker: PhantomData<(Key, Value)>,
}

#[cfg(feature = "revocable_store")]
impl<Key, Value> Default for RevocableStoreBuilder<Key, Value> {
    fn default() -> Self {
        Self { binding: None, charger: None, _marker: PhantomData }
    }
}

#[cfg(feature = "revocable_store")]
impl<Key, Value> RevocableStoreBuilder<Key, Value> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(mut self, binding: StoreBindingHandle) -> Self {
        self.binding = Some(binding);
        self
    }

    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }

    pub fn build(self) -> MemoryResult<SimpleRevocableStore<Key, Value>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        let mut store = SimpleRevocableStore::new(binding);
        if let Some(sink) = self.charger {
            store = store.charged_against(sink);
        }
        Ok(store)
    }
}

#[cfg(feature = "publish_pipeline")]
#[derive(Clone, Debug)]
pub struct PublishPipelineBuilder<Stage, Commit, Published> {
    binding: Option<StoreBindingHandle>,
    _marker: PhantomData<(Stage, Commit, Published)>,
}

#[cfg(feature = "publish_pipeline")]
impl<Stage, Commit, Published> Default for PublishPipelineBuilder<Stage, Commit, Published> {
    fn default() -> Self {
        Self { binding: None, _marker: PhantomData }
    }
}

#[cfg(feature = "publish_pipeline")]
impl<Stage, Commit, Published> PublishPipelineBuilder<Stage, Commit, Published> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(mut self, binding: StoreBindingHandle) -> Self {
        self.binding = Some(binding);
        self
    }

    /// Build the builder-first happy-path pipeline, which uses the infallible
    /// identity commit transition (`Published: From<Stage>`). Callers that need a
    /// fallible commit transition construct the pipeline via
    /// [`SimplePublishPipeline::with_transition`] directly.
    pub fn build(self) -> MemoryResult<SimplePublishPipeline<Stage, Commit, Published>>
    where
        Published: From<Stage>,
    {
        let binding = self.binding.ok_or_else(missing_binding)?;
        Ok(SimplePublishPipeline::new(binding))
    }
}

#[cfg(feature = "publish_pipeline")]
#[derive(Clone, Debug)]
pub struct PreparedPublicationServiceBuilder<Payload> {
    binding: Option<StoreBindingHandle>,
    capacity_bytes: Option<u64>,
    tombstone_limits: Option<(usize, u64)>,
    charger: Option<SharedAccountingSink>,
    _marker: PhantomData<Payload>,
}

#[cfg(feature = "publish_pipeline")]
impl<Payload> Default for PreparedPublicationServiceBuilder<Payload> {
    fn default() -> Self {
        Self {
            binding: None,
            capacity_bytes: None,
            tombstone_limits: None,
            charger: None,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "publish_pipeline")]
impl<Payload> PreparedPublicationServiceBuilder<Payload> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(mut self, binding: StoreBindingHandle) -> Self {
        self.binding = Some(binding);
        self
    }

    #[must_use]
    pub fn with_capacity_bytes(mut self, capacity_bytes: u64) -> Self {
        self.capacity_bytes = Some(capacity_bytes);
        self
    }

    /// Bound process-lifetime retry tombstones retained after abort or orphan
    /// collection. Saturation fails the transition before any payload change.
    #[must_use]
    pub fn with_tombstone_limits(mut self, max_count: usize, max_bytes: u64) -> Self {
        self.tombstone_limits = Some((max_count, max_bytes));
        self
    }

    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }

    pub fn build(self) -> MemoryResult<PreparedPublicationService<Payload>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        let mut service = PreparedPublicationService::new(binding, self.capacity_bytes);
        if let Some((max_count, max_bytes)) = self.tombstone_limits {
            service = service.with_tombstone_limits(max_count, max_bytes);
        }
        if let Some(sink) = self.charger {
            service = service.charged_against(sink);
        }
        Ok(service)
    }
}

#[cfg(feature = "transcript")]
#[derive(Clone, Debug)]
pub struct SessionTranscriptStoreBuilder<SessionId, MessageId, Message> {
    binding: Option<StoreBindingHandle>,
    charger: Option<SharedAccountingSink>,
    retention: TranscriptRetentionPolicyV1,
    _marker: PhantomData<(SessionId, MessageId, Message)>,
}

#[cfg(feature = "transcript")]
impl<SessionId, MessageId, Message> Default
    for SessionTranscriptStoreBuilder<SessionId, MessageId, Message>
{
    fn default() -> Self {
        Self {
            binding: None,
            charger: None,
            retention: TranscriptRetentionPolicyV1::process_lifetime(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "transcript")]
impl<SessionId, MessageId, Message> SessionTranscriptStoreBuilder<SessionId, MessageId, Message> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(mut self, binding: StoreBindingHandle) -> Self {
        self.binding = Some(binding);
        self
    }

    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }

    #[must_use]
    pub fn with_retention_policy(mut self, retention: TranscriptRetentionPolicyV1) -> Self {
        self.retention = retention;
        self
    }

    pub fn build(
        self,
    ) -> MemoryResult<SimpleSessionTranscriptStore<SessionId, MessageId, Message>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        let mut store = SimpleSessionTranscriptStore::new(binding, self.retention);
        if let Some(sink) = self.charger {
            store = store.charged_against(sink);
        }
        Ok(store)
    }
}

#[cfg(all(test, feature = "full"))]
mod tests {
    use super::*;
    use quanta_memory_platform_core::{
        EffectiveMemoryBudgetReport, ExecutionBudget, LiveProcessMemoryBroker,
        MemoryBudgetManifest, MemoryClass, ResidencyPolicy,
    };

    fn binding() -> StoreBindingHandle {
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner".to_string(), None)
    }

    fn live_broker() -> LiveProcessMemoryBroker {
        let report = EffectiveMemoryBudgetReport::from_manifest(
            MemoryBudgetManifest::new(
                MemoryPreset::Development,
                ExecutionBudget::default(),
                ResidencyPolicy::new(1_000_000),
            ),
            vec!["builder-tests".to_string()],
        );
        LiveProcessMemoryBroker::from_report(report).expect("valid broker")
    }

    #[test]
    fn builder_first_happy_path_builds_a_usable_payload_store() {
        use crate::payload_store::PayloadStore;
        use quanta_memory_platform_core::RetainedEnvelope;

        let store = PayloadStoreBuilder::<String, Vec<u8>>::new()
            .bind(binding())
            .build()
            .expect("builder-first happy path must succeed");
        // The built store actually works.
        let published =
            store.put_canonical(RetainedEnvelope::new(b"x".to_vec())).expect("put via built store");
        assert_eq!(store.stats().item_count, 1);
        assert_eq!(published.payload_ref(), &"payload-0".to_string());
    }

    #[test]
    fn payload_builder_without_binding_fails_with_build_error() {
        // NEGATIVE-PAIR: missing required binding is a typed Build error, not a panic.
        let err = PayloadStoreBuilder::<String, Vec<u8>>::new()
            .build()
            .expect_err("missing binding must fail");
        assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");
    }

    #[test]
    fn retained_map_builder_charged_against_routes_bytes_through_the_ledger() {
        use crate::retained_map::RetainedMapStore;

        let broker = live_broker();
        let sink: SharedAccountingSink = std::sync::Arc::new(broker.clone());
        let store = RetainedMapStoreBuilder::<String, Vec<u8>>::new()
            .bind(binding())
            .charged_against(sink)
            .build();

        store.insert_checked("k".to_string(), vec![0u8; 64]).expect("insert");
        assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 64);
    }

    #[test]
    fn publish_pipeline_builder_without_binding_fails_with_build_error() {
        let err = PublishPipelineBuilder::<String, (), String>::new()
            .build()
            .expect_err("missing binding");
        assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");
    }

    #[test]
    fn transcript_builder_without_binding_fails_with_build_error() {
        let err = SessionTranscriptStoreBuilder::<String, String, String>::new()
            .build()
            .expect_err("missing binding");
        assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");
    }

    #[test]
    fn transcript_builder_charged_against_routes_bytes_through_the_ledger() {
        use crate::transcript_store::{
            SessionTranscriptStore, TranscriptAppendIdentityV1, TranscriptAppendRequestV1,
        };

        let broker = live_broker();
        let sink: SharedAccountingSink = std::sync::Arc::new(broker.clone());
        let store = SessionTranscriptStoreBuilder::<String, String, String>::new()
            .bind(binding())
            .charged_against(sink)
            .build()
            .expect("builder charged transcript store");

        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new("s".to_string(), 0, "m0".to_string()),
                "hello".to_string(),
            ))
            .expect("append");
        assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 5);
    }

    #[test]
    fn revocable_builder_without_binding_fails_with_build_error() {
        let err =
            RevocableStoreBuilder::<String, Vec<u8>>::new().build().expect_err("missing binding");
        assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");
    }

    #[test]
    fn revocable_builder_charged_against_routes_bytes_through_the_ledger() {
        use crate::revocable_store::RevocableStore;

        let broker = live_broker();
        let sink: SharedAccountingSink = std::sync::Arc::new(broker.clone());
        let store = RevocableStoreBuilder::<String, Vec<u8>>::new()
            .bind(binding())
            .charged_against(sink)
            .build()
            .expect("builder charged revocable store");

        store.insert_checked("k".to_string(), vec![0u8; 16]).expect("insert");
        assert_eq!(broker.ledger_snapshot().expect("snap").total_owner_bytes(), 16);
    }

    #[test]
    fn build_live_platform_composes_charged_stores_without_manual_glue() {
        use crate::retained_map::RetainedMapStore;
        // Builder-first CHARGED path: no LiveProcessMemoryBroker named, no
        // Arc::new(broker.clone()) written — the platform encapsulates both.
        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build_live()
            .expect("live platform builds from preset");
        let store =
            platform.retained_map_store_builder::<String, Vec<u8>>().bind(binding()).build();

        store.insert_checked("k".to_string(), vec![0u8; 48]).expect("insert");
        assert_eq!(
            platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(),
            48,
            "a store built from the platform routes its bytes through the platform ledger"
        );
        store.remove(&"k".to_string()).expect("remove");
        assert_eq!(platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(), 0);
    }

    #[test]
    fn build_live_platform_sink_is_the_same_ledger_as_the_broker() {
        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build_live()
            .expect("live platform");
        // TWO distinct stores from the SAME platform must share ONE ledger via
        // platform.sink(): the broker sees the SUM of both. A regression where
        // sink() handed out independent sinks would make the total 25, not 35.
        use crate::retained_map::RetainedMapStore;
        let a = platform
            .retained_map_store_builder::<String, Vec<u8>>()
            .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-a".to_string(), None))
            .build();
        let b = platform
            .retained_map_store_builder::<String, Vec<u8>>()
            .bind(StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner-b".to_string(), None))
            .build();
        a.insert_checked("x".to_string(), vec![0u8; 10]).expect("insert a");
        b.insert_checked("y".to_string(), vec![0u8; 25]).expect("insert b");
        assert_eq!(
            platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(),
            35,
            "both stores' bytes land in the one shared ledger (10 + 25)"
        );
    }

    #[test]
    fn memory_platform_builder_builds_broker_from_preset() {
        let broker = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker build from preset");
        // The broker is usable: it can attach a runtime.
        use quanta_memory_platform_core::{ProcessMemoryBroker, RuntimeKind};
        let view = broker
            .attach_runtime("rt-1".to_string(), RuntimeKind::Runtime)
            .expect("attach runtime");
        assert_eq!(view.runtime_kind(), RuntimeKind::Runtime);
    }
}
