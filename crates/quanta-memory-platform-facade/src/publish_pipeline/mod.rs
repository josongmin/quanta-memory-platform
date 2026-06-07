pub mod service;

use quanta_memory_platform_core::{PublishError, StagedHandle, StoreBindingHandle};

pub use crate::shared::builder::PublishPipelineBuilder;
pub use service::{CommitTransition, DefaultCommitTransition, SimplePublishPipeline};

pub trait PublishPipeline<Stage, Commit, Published> {
    fn binding(&self) -> &StoreBindingHandle;
    fn stage(&self, input: Stage) -> Result<StagedHandle<Stage>, PublishError>;
    fn commit(
        &self,
        staged: StagedHandle<Stage>,
        commit: Commit,
    ) -> Result<Published, PublishError>;
}
