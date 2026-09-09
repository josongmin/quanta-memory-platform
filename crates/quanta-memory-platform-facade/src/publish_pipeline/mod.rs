pub mod prepared_publication;
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
pub trait PublishPipeline<Stage, Commit, Published> {
    fn binding(&self) -> &StoreBindingHandle;
    fn stage(&self, input: Stage) -> Result<StagedHandle<Stage>, PublishError>;
    fn commit(
        &self,
        staged: StagedHandle<Stage>,
        commit: Commit,
    ) -> Result<Published, PublishError>;
}
