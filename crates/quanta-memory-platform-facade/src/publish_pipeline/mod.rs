pub(crate) mod prepared_publication;
pub mod service;

use quanta_memory_platform_core::{PublishError, StagedHandle, StoreBindingHandle};

pub use crate::shared::builder::{PreparedPublicationServiceBuilder, PublishPipelineBuilder};
pub use prepared_publication::{
    AbortDispositionV1, PrepareDispositionV1, PreparedPublicationErrorV1,
    PreparedPublicationIdentityV1, PreparedPublicationReceiptV1, PreparedPublicationRequestV1,
    PreparedPublicationService, PreparedPublicationStatsV1, PreparedPublicationTokenV1,
};
pub use service::{CommitTransition, DefaultCommitTransition, SimplePublishPipeline};

/// Legacy non-retaining transition surface.
///
/// # Deprecated contract
///
/// New retaining publication integrations must use [`PreparedPublicationService`].
/// This transition remains temporarily for existing non-retaining callers and
/// is owned for removal by the QMP publish-pipeline migration. It is not a
/// publication visibility authority and exposes no reader/store.
///
/// New callers cannot admit the legacy transition under a deny-deprecated gate:
///
/// ```compile_fail
/// #![deny(deprecated)]
/// use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
/// use quanta_memory_platform_facade::{PublishPipeline, SimplePublishPipeline};
///
/// let binding = StoreBindingHandle::new(MemoryClass::ManifestResidency, "legacy".into(), None);
/// let pipeline: SimplePublishPipeline<String, (), String> = SimplePublishPipeline::new(binding);
/// let _ = pipeline.stage("payload".to_string());
/// ```
///
/// ```compile_fail
/// #![deny(deprecated)]
/// use quanta_memory_platform_core::{MemoryClass, StagedHandle, StoreBindingHandle};
/// use quanta_memory_platform_facade::{PublishPipeline, SimplePublishPipeline};
///
/// let binding = StoreBindingHandle::new(MemoryClass::ManifestResidency, "legacy".into(), None);
/// let pipeline: SimplePublishPipeline<String, (), String> = SimplePublishPipeline::new(binding);
/// let _ = pipeline.commit(StagedHandle::new("payload".to_string()), ());
/// ```
pub trait PublishPipeline<Stage, Commit, Published> {
    fn binding(&self) -> &StoreBindingHandle;
    #[deprecated(
        since = "0.1.0",
        note = "use PreparedPublicationService::prepare; removal owner: QMP publish-pipeline migration"
    )]
    fn stage(&self, input: Stage) -> Result<StagedHandle<Stage>, PublishError>;
    #[deprecated(
        since = "0.1.0",
        note = "use PreparedPublicationService root-referenced resolve/abort; removal owner: QMP publish-pipeline migration"
    )]
    fn commit(
        &self,
        staged: StagedHandle<Stage>,
        commit: Commit,
    ) -> Result<Published, PublishError>;
}
