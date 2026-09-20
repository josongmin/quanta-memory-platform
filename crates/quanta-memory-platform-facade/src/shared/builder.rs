use quanta_memory_platform_core::{
    BudgetOverrideLayer, EffectiveMemoryBudgetReport, MemoryBudgetManifest, MemoryPreset,
    MemoryResult, OverrideSource, ProcessMemoryBrokerBuilder,
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
#[cfg(all(
    feature = "unbudgeted-stores",
    any(
        feature = "payload_store",
        feature = "publish_pipeline",
        feature = "retained_map",
        feature = "revocable_store",
        feature = "transcript"
    )
))]
use crate::shared::accounting::Unbudgeted;
#[cfg(any(
    feature = "payload_store",
    feature = "publish_pipeline",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript"
))]
use crate::shared::accounting::{AccountingUnset, Budgeted};
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

    #[must_use]
    pub fn with_override_layer(
        mut self,
        source: OverrideSource,
        layer: BudgetOverrideLayer,
    ) -> Self {
        self.inner = self.inner.with_override_layer(source, layer);
        self
    }

    /// Build the enforced, accounting-backed default platform.
    #[cfg(feature = "core-live-accounting")]
    pub fn build(self) -> MemoryResult<LiveMemoryPlatform> {
        Ok(LiveMemoryPlatform::new(self.inner.build()?))
    }

    /// Compatibility alias for consumers migrating to the enforced default.
    #[cfg(feature = "core-live-accounting")]
    pub fn build_live(self) -> MemoryResult<LiveMemoryPlatform> {
        self.build()
    }

    /// Resolve configuration without creating runtime or accounting authority.
    pub fn build_report_only(self) -> MemoryResult<EffectiveMemoryBudgetReport> {
        self.inner.build_report_only()
    }
}

/// A live, accounting-backed memory platform built via [`MemoryPlatformBuilder::build`].
///
/// It owns the live broker (the accounting SSOT) and is the only public factory
/// for budgeted store builders. No public callback seam or post-construction
/// accounting attachment exists.
#[cfg(feature = "core-live-accounting")]
#[derive(Clone)]
pub struct LiveMemoryPlatform {
    broker: quanta_memory_platform_core::LiveProcessMemoryBroker,
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
        Self { broker }
    }

    /// The live broker (accounting SSOT) for snapshots, reconfigure, rebuild, etc.
    #[must_use]
    pub fn broker(&self) -> &quanta_memory_platform_core::LiveProcessMemoryBroker {
        &self.broker
    }

    /// A payload-store builder already charged against this platform's ledger.
    #[cfg(feature = "payload_store")]
    #[must_use]
    pub fn payload_store_builder<PayloadRef, Bytes>(
        &self,
    ) -> PayloadStoreBuilder<PayloadRef, Bytes, Budgeted> {
        PayloadStoreBuilder::new_budgeted(self.broker.clone())
    }

    /// A retained-map-store builder already charged against this platform's ledger.
    #[cfg(feature = "retained_map")]
    #[must_use]
    pub fn retained_map_store_builder<Key, Value>(
        &self,
    ) -> RetainedMapStoreBuilder<Key, Value, Budgeted> {
        RetainedMapStoreBuilder::new_budgeted(self.broker.clone())
    }

    /// A revocable-store builder already charged against this platform's ledger.
    #[cfg(feature = "revocable_store")]
    #[must_use]
    pub fn revocable_store_builder<Key, Value>(
        &self,
    ) -> RevocableStoreBuilder<Key, Value, Budgeted> {
        RevocableStoreBuilder::new_budgeted(self.broker.clone())
    }

    /// A session-transcript-store builder already charged against this platform's ledger.
    #[cfg(feature = "transcript")]
    #[must_use]
    pub fn transcript_store_builder<SessionId, MessageId, Message>(
        &self,
    ) -> SessionTranscriptStoreBuilder<SessionId, MessageId, Message, Budgeted> {
        SessionTranscriptStoreBuilder::new_budgeted(self.broker.clone())
    }

    /// A prepared-publication builder already charged against this platform's ledger.
    #[cfg(feature = "publish_pipeline")]
    #[must_use]
    pub fn prepared_publication_builder<Payload>(
        &self,
    ) -> PreparedPublicationServiceBuilder<Payload, Budgeted> {
        PreparedPublicationServiceBuilder::new_budgeted(self.broker.clone())
    }
}

#[cfg(feature = "core-live-accounting")]
impl quanta_memory_platform_core::ProcessMemoryBroker for LiveMemoryPlatform {
    fn attach_runtime(
        &self,
        runtime_id: quanta_memory_platform_core::RuntimeId,
        runtime_kind: quanta_memory_platform_core::RuntimeKind,
    ) -> MemoryResult<std::sync::Arc<dyn quanta_memory_platform_core::RuntimeMemoryView>> {
        quanta_memory_platform_core::ProcessMemoryBroker::attach_runtime(
            &self.broker,
            runtime_id,
            runtime_kind,
        )
    }

    fn effective_report(&self) -> EffectiveMemoryBudgetReport {
        quanta_memory_platform_core::ProcessMemoryBroker::effective_report(&self.broker)
    }
}

#[cfg(feature = "payload_store")]
#[derive(Clone, Debug)]
/// Payload-store builder branded by its accounting state.
///
/// The unset state deliberately has no `build` method:
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{AccountingUnset, PayloadStoreBuilder};
///
/// let _ = PayloadStoreBuilder::<String, Vec<u8>, AccountingUnset>::new().build();
/// ```
pub struct PayloadStoreBuilder<PayloadRef, Bytes, Mode> {
    binding: Option<StoreBindingHandle>,
    context: Option<MemoryContextHandle>,
    accounting: Mode,
    _marker: PhantomData<fn() -> (PayloadRef, Bytes)>,
}

#[cfg(feature = "payload_store")]
impl<PayloadRef, Bytes> Default for PayloadStoreBuilder<PayloadRef, Bytes, AccountingUnset> {
    fn default() -> Self {
        Self {
            binding: None,
            context: None,
            accounting: AccountingUnset::default(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "payload_store")]
impl<PayloadRef, Bytes> PayloadStoreBuilder<PayloadRef, Bytes, AccountingUnset> {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "unbudgeted-stores")]
    pub fn unbudgeted(self) -> PayloadStoreBuilder<PayloadRef, Bytes, Unbudgeted> {
        PayloadStoreBuilder {
            binding: self.binding,
            context: self.context,
            accounting: Unbudgeted::new(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "payload_store")]
impl<PayloadRef, Bytes> PayloadStoreBuilder<PayloadRef, Bytes, Budgeted> {
    fn new_budgeted(broker: quanta_memory_platform_core::LiveProcessMemoryBroker) -> Self {
        Self {
            binding: None,
            context: None,
            accounting: Budgeted::new(broker),
            _marker: PhantomData,
        }
    }

    pub fn build(self) -> MemoryResult<SimplePayloadStore<PayloadRef, Bytes, Budgeted>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        Ok(SimplePayloadStore::new_budgeted(binding, self.context, self.accounting.into_broker()))
    }
}

#[cfg(all(feature = "payload_store", feature = "unbudgeted-stores"))]
impl<PayloadRef, Bytes> PayloadStoreBuilder<PayloadRef, Bytes, Unbudgeted> {
    pub fn build(self) -> MemoryResult<SimplePayloadStore<PayloadRef, Bytes, Unbudgeted>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        Ok(SimplePayloadStore::new_unbudgeted(binding, self.context))
    }
}

#[cfg(feature = "payload_store")]
impl<PayloadRef, Bytes, Mode> PayloadStoreBuilder<PayloadRef, Bytes, Mode> {
    pub fn bind(mut self, binding: StoreBindingHandle) -> Self {
        self.binding = Some(binding);
        self
    }

    pub fn with_context(mut self, context: MemoryContextHandle) -> Self {
        self.context = Some(context);
        self
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
/// use quanta_memory_platform_facade::{AccountingUnset, RetainedMapStoreBuilder};
///
/// let _ = RetainedMapStoreBuilder::<String, Vec<u8>, AccountingUnset>::new().build();
/// ```
#[derive(Clone, Debug)]
pub struct RetainedMapStoreBuilder<Key, Value, Mode> {
    accounting: Mode,
    _marker: PhantomData<fn() -> (Key, Value)>,
}

/// Bound retained-map builder state. Only this state can build a store, and the
/// build is infallible because its required binding is owned, not optional.
#[cfg(feature = "retained_map")]
#[derive(Clone, Debug)]
pub struct BoundRetainedMapStoreBuilder<Key, Value, Mode> {
    binding: StoreBindingHandle,
    accounting: Mode,
    _marker: PhantomData<fn() -> (Key, Value)>,
}

#[cfg(feature = "retained_map")]
impl<Key, Value> Default for RetainedMapStoreBuilder<Key, Value, AccountingUnset> {
    fn default() -> Self {
        Self { accounting: AccountingUnset::default(), _marker: PhantomData }
    }
}

#[cfg(feature = "retained_map")]
impl<Key, Value> RetainedMapStoreBuilder<Key, Value, AccountingUnset> {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "unbudgeted-stores")]
    pub fn unbudgeted(self) -> RetainedMapStoreBuilder<Key, Value, Unbudgeted> {
        RetainedMapStoreBuilder { accounting: Unbudgeted::new(), _marker: PhantomData }
    }
}

#[cfg(feature = "retained_map")]
impl<Key, Value> RetainedMapStoreBuilder<Key, Value, Budgeted> {
    fn new_budgeted(broker: quanta_memory_platform_core::LiveProcessMemoryBroker) -> Self {
        Self { accounting: Budgeted::new(broker), _marker: PhantomData }
    }
}

#[cfg(feature = "retained_map")]
impl<Key, Value, Mode> RetainedMapStoreBuilder<Key, Value, Mode> {
    pub fn bind(
        self,
        binding: StoreBindingHandle,
    ) -> BoundRetainedMapStoreBuilder<Key, Value, Mode> {
        BoundRetainedMapStoreBuilder { binding, accounting: self.accounting, _marker: PhantomData }
    }
}

#[cfg(feature = "retained_map")]
impl<Key, Value> BoundRetainedMapStoreBuilder<Key, Value, Budgeted> {
    pub fn build(self) -> SimpleRetainedMapStore<Key, Value, Budgeted> {
        SimpleRetainedMapStore::new_budgeted(self.binding, self.accounting.into_broker())
    }
}

#[cfg(all(feature = "retained_map", feature = "unbudgeted-stores"))]
impl<Key, Value> BoundRetainedMapStoreBuilder<Key, Value, Unbudgeted> {
    pub fn build(self) -> SimpleRetainedMapStore<Key, Value, Unbudgeted> {
        SimpleRetainedMapStore::new_unbudgeted(self.binding)
    }
}

#[cfg(feature = "revocable_store")]
#[derive(Clone, Debug)]
pub struct RevocableStoreBuilder<Key, Value, Mode> {
    binding: Option<StoreBindingHandle>,
    accounting: Mode,
    _marker: PhantomData<fn() -> (Key, Value)>,
}

#[cfg(feature = "revocable_store")]
impl<Key, Value> Default for RevocableStoreBuilder<Key, Value, AccountingUnset> {
    fn default() -> Self {
        Self { binding: None, accounting: AccountingUnset::default(), _marker: PhantomData }
    }
}

#[cfg(feature = "revocable_store")]
impl<Key, Value> RevocableStoreBuilder<Key, Value, AccountingUnset> {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "unbudgeted-stores")]
    pub fn unbudgeted(self) -> RevocableStoreBuilder<Key, Value, Unbudgeted> {
        RevocableStoreBuilder {
            binding: self.binding,
            accounting: Unbudgeted::new(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "revocable_store")]
impl<Key, Value> RevocableStoreBuilder<Key, Value, Budgeted> {
    fn new_budgeted(broker: quanta_memory_platform_core::LiveProcessMemoryBroker) -> Self {
        Self { binding: None, accounting: Budgeted::new(broker), _marker: PhantomData }
    }

    pub fn build(self) -> MemoryResult<SimpleRevocableStore<Key, Value, Budgeted>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        Ok(SimpleRevocableStore::new_budgeted(binding, self.accounting.into_broker()))
    }
}

#[cfg(all(feature = "revocable_store", feature = "unbudgeted-stores"))]
impl<Key, Value> RevocableStoreBuilder<Key, Value, Unbudgeted> {
    pub fn build(self) -> MemoryResult<SimpleRevocableStore<Key, Value, Unbudgeted>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        Ok(SimpleRevocableStore::new_unbudgeted(binding))
    }
}

#[cfg(feature = "revocable_store")]
impl<Key, Value, Mode> RevocableStoreBuilder<Key, Value, Mode> {
    pub fn bind(mut self, binding: StoreBindingHandle) -> Self {
        self.binding = Some(binding);
        self
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
pub struct PreparedPublicationServiceBuilder<Payload, Mode> {
    binding: Option<StoreBindingHandle>,
    capacity_bytes: Option<u64>,
    tombstone_limits: Option<(usize, u64)>,
    accounting: Mode,
    _marker: PhantomData<fn() -> Payload>,
}

#[cfg(feature = "publish_pipeline")]
impl<Payload> Default for PreparedPublicationServiceBuilder<Payload, AccountingUnset> {
    fn default() -> Self {
        Self {
            binding: None,
            capacity_bytes: None,
            tombstone_limits: None,
            accounting: AccountingUnset::default(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "publish_pipeline")]
impl<Payload> PreparedPublicationServiceBuilder<Payload, AccountingUnset> {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "unbudgeted-stores")]
    pub fn unbudgeted(self) -> PreparedPublicationServiceBuilder<Payload, Unbudgeted> {
        PreparedPublicationServiceBuilder {
            binding: self.binding,
            capacity_bytes: self.capacity_bytes,
            tombstone_limits: self.tombstone_limits,
            accounting: Unbudgeted::new(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "publish_pipeline")]
impl<Payload> PreparedPublicationServiceBuilder<Payload, Budgeted> {
    fn new_budgeted(broker: quanta_memory_platform_core::LiveProcessMemoryBroker) -> Self {
        Self {
            binding: None,
            capacity_bytes: None,
            tombstone_limits: None,
            accounting: Budgeted::new(broker),
            _marker: PhantomData,
        }
    }

    pub fn build(self) -> MemoryResult<PreparedPublicationService<Payload, Budgeted>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        Ok(PreparedPublicationService::new_budgeted(
            binding,
            self.capacity_bytes,
            self.tombstone_limits,
            self.accounting.into_broker(),
        ))
    }
}

#[cfg(all(feature = "publish_pipeline", feature = "unbudgeted-stores"))]
impl<Payload> PreparedPublicationServiceBuilder<Payload, Unbudgeted> {
    pub fn build(self) -> MemoryResult<PreparedPublicationService<Payload, Unbudgeted>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        Ok(PreparedPublicationService::new_unbudgeted(
            binding,
            self.capacity_bytes,
            self.tombstone_limits,
        ))
    }
}

#[cfg(feature = "publish_pipeline")]
impl<Payload, Mode> PreparedPublicationServiceBuilder<Payload, Mode> {
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
}

#[cfg(feature = "transcript")]
#[derive(Clone, Debug)]
pub struct SessionTranscriptStoreBuilder<SessionId, MessageId, Message, Mode> {
    binding: Option<StoreBindingHandle>,
    accounting: Mode,
    retention: TranscriptRetentionPolicyV1,
    _marker: PhantomData<fn() -> (SessionId, MessageId, Message)>,
}

#[cfg(feature = "transcript")]
impl<SessionId, MessageId, Message> Default
    for SessionTranscriptStoreBuilder<SessionId, MessageId, Message, AccountingUnset>
{
    fn default() -> Self {
        Self {
            binding: None,
            accounting: AccountingUnset::default(),
            retention: TranscriptRetentionPolicyV1::process_lifetime(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "transcript")]
impl<SessionId, MessageId, Message>
    SessionTranscriptStoreBuilder<SessionId, MessageId, Message, AccountingUnset>
{
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "unbudgeted-stores")]
    pub fn unbudgeted(
        self,
    ) -> SessionTranscriptStoreBuilder<SessionId, MessageId, Message, Unbudgeted> {
        SessionTranscriptStoreBuilder {
            binding: self.binding,
            accounting: Unbudgeted::new(),
            retention: self.retention,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "transcript")]
impl<SessionId, MessageId, Message>
    SessionTranscriptStoreBuilder<SessionId, MessageId, Message, Budgeted>
{
    fn new_budgeted(broker: quanta_memory_platform_core::LiveProcessMemoryBroker) -> Self {
        Self {
            binding: None,
            accounting: Budgeted::new(broker),
            retention: TranscriptRetentionPolicyV1::process_lifetime(),
            _marker: PhantomData,
        }
    }

    pub fn build(
        self,
    ) -> MemoryResult<SimpleSessionTranscriptStore<SessionId, MessageId, Message, Budgeted>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        Ok(SimpleSessionTranscriptStore::new_budgeted(
            binding,
            self.retention,
            self.accounting.into_broker(),
        ))
    }
}

#[cfg(all(feature = "transcript", feature = "unbudgeted-stores"))]
impl<SessionId, MessageId, Message>
    SessionTranscriptStoreBuilder<SessionId, MessageId, Message, Unbudgeted>
{
    pub fn build(
        self,
    ) -> MemoryResult<SimpleSessionTranscriptStore<SessionId, MessageId, Message, Unbudgeted>> {
        let binding = self.binding.ok_or_else(missing_binding)?;
        Ok(SimpleSessionTranscriptStore::new_unbudgeted(binding, self.retention))
    }
}

#[cfg(feature = "transcript")]
impl<SessionId, MessageId, Message, Mode>
    SessionTranscriptStoreBuilder<SessionId, MessageId, Message, Mode>
{
    pub fn bind(mut self, binding: StoreBindingHandle) -> Self {
        self.binding = Some(binding);
        self
    }

    #[must_use]
    pub fn with_retention_policy(mut self, retention: TranscriptRetentionPolicyV1) -> Self {
        self.retention = retention;
        self
    }
}

#[cfg(all(test, feature = "full"))]
mod tests {
    use super::*;
    use quanta_memory_platform_core::MemoryClass;

    fn binding() -> StoreBindingHandle {
        StoreBindingHandle::new(MemoryClass::ArtifactCache, "owner".to_string(), None)
    }

    fn platform() -> LiveMemoryPlatform {
        MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("valid live platform")
    }

    #[test]
    fn builder_first_happy_path_builds_a_usable_payload_store() {
        use crate::payload_store::PayloadStore;
        use quanta_memory_platform_core::RetainedEnvelope;

        let platform = platform();
        let store = platform
            .payload_store_builder::<String, Vec<u8>>()
            .bind(binding())
            .build()
            .expect("builder-first happy path must succeed");
        // The built store actually works.
        let published =
            store.put_canonical(RetainedEnvelope::new(b"x".to_vec())).expect("put via built store");
        assert_eq!(store.stats().expect("stats").item_count, 1);
        assert_eq!(published.payload_ref(), &"payload-0".to_string());
    }

    #[test]
    fn payload_builder_without_binding_fails_with_build_error() {
        // NEGATIVE-PAIR: missing required binding is a typed Build error, not a panic.
        let err = platform()
            .payload_store_builder::<String, Vec<u8>>()
            .build()
            .expect_err("missing binding must fail");
        assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");
    }

    #[test]
    fn retained_map_platform_builder_routes_bytes_through_the_ledger() {
        use crate::retained_map::RetainedMapStore;

        let platform = platform();
        let store =
            platform.retained_map_store_builder::<String, Vec<u8>>().bind(binding()).build();

        store.insert_checked("k".to_string(), vec![0u8; 64]).expect("insert");
        assert_eq!(platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(), 64);
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
        let err = platform()
            .transcript_store_builder::<String, String, String>()
            .build()
            .expect_err("missing binding");
        assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");
    }

    #[test]
    fn transcript_platform_builder_routes_bytes_through_the_ledger() {
        use crate::transcript_store::{
            SessionTranscriptStore, TranscriptAppendIdentityV1, TranscriptAppendRequestV1,
        };

        let platform = platform();
        let store = platform
            .transcript_store_builder::<String, String, String>()
            .bind(binding())
            .build()
            .expect("builder charged transcript store");

        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new("s".to_string(), 0, "m0".to_string()),
                "hello".to_string(),
            ))
            .expect("append");
        assert_eq!(platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(), 5);
    }

    #[test]
    fn revocable_builder_without_binding_fails_with_build_error() {
        let err = platform()
            .revocable_store_builder::<String, Vec<u8>>()
            .build()
            .expect_err("missing binding");
        assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");
    }

    #[test]
    fn revocable_platform_builder_routes_bytes_through_the_ledger() {
        use crate::revocable_store::RevocableStore;

        let platform = platform();
        let store = platform
            .revocable_store_builder::<String, Vec<u8>>()
            .bind(binding())
            .build()
            .expect("builder charged revocable store");

        store.insert_checked("k".to_string(), vec![0u8; 16]).expect("insert");
        assert_eq!(platform.broker().ledger_snapshot().expect("snap").total_owner_bytes(), 16);
    }

    #[test]
    fn default_build_composes_charged_stores_without_manual_glue() {
        use crate::retained_map::RetainedMapStore;
        // Builder-first CHARGED path: no LiveProcessMemoryBroker named, no
        // Arc::new(broker.clone()) written — the platform encapsulates both.
        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
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
    fn default_build_platform_sink_is_the_same_ledger_as_the_broker() {
        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("live platform");
        // TWO distinct stores from the SAME platform share ONE concrete broker
        // authority. The broker sees the SUM of both.
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
