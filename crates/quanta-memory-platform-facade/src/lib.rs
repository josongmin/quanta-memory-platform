// Slice-agnostic glue is always available: the broker builder, presets, the
// accounting seam, and byte-measurement. Each store/pipeline slice is gated
// behind its feature so `--no-default-features` carries no slice surface and a
// feature combination that does not enable a slice cannot reach its (or its
// backing's) code — the feature-slice contract is real, not nominal.
pub mod presets;
pub mod shared;

#[cfg(feature = "payload_store")]
pub mod payload_store;
#[cfg(feature = "publish_pipeline")]
pub mod publish_pipeline;
#[cfg(feature = "retained_map")]
pub mod retained_map;
#[cfg(feature = "revocable_store")]
pub mod revocable_store;
#[cfg(feature = "payload_store")]
pub mod scratch;
#[cfg(feature = "spill")]
pub mod spill;
#[cfg(feature = "transcript")]
pub mod transcript_store;

pub use presets::PortableMemoryPresets;
pub use shared::accounting::{AccountingSink, SharedAccountingSink};
#[cfg(feature = "core-live-accounting")]
pub use shared::builder::LiveMemoryPlatform;
pub use shared::builder::MemoryPlatformBuilder;
pub use shared::measure::{ByteSized, CanonicalKey, MeasuredValue};

#[cfg(feature = "payload_store")]
pub use payload_store::{PayloadStore, PayloadStoreBuilder, SimplePayloadStore};
#[cfg(feature = "publish_pipeline")]
pub use publish_pipeline::{
    CommitTransition, DefaultCommitTransition, PublishPipeline, PublishPipelineBuilder,
    SimplePublishPipeline,
};
#[cfg(feature = "retained_map")]
pub use retained_map::{RetainedMapStore, RetainedMapStoreBuilder, SimpleRetainedMapStore};
#[cfg(feature = "revocable_store")]
pub use revocable_store::{RevocableStore, RevocableStoreBuilder, SimpleRevocableStore};
#[cfg(feature = "transcript")]
pub use shared::builder::SessionTranscriptStoreBuilder;
#[cfg(feature = "spill")]
pub use spill::{
    InMemorySpillTarget, NoopSpillController, RetainingSpillTarget, RoutedSpillController,
    SpillController, SpillTarget,
};
#[cfg(feature = "transcript")]
pub use transcript_store::{SessionTranscriptStore, SimpleSessionTranscriptStore};
