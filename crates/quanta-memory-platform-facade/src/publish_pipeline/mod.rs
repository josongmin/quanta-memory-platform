//! Retaining prepared publications with caller-owned canonical-root visibility.
//!
//! The legacy non-retaining transition wrapper has been removed. Callers that
//! only transform values execute their own transition; prepared publication
//! retains payload and identity evidence under the platform ledger.
//!
//! ```compile_fail
//! use quanta_memory_platform_facade::{PublishPipeline, SimplePublishPipeline};
//! ```
//!
//! ```compile_fail
//! use quanta_memory_platform_facade::publish_pipeline::service::SimplePublishPipeline;
//! ```
//!
//! The evidence limit has one public builder name. The old tombstone-only
//! spelling must not remain as an alias for a narrower limit:
//!
//! ```compile_fail
//! use quanta_memory_platform_core::MemoryPreset;
//! use quanta_memory_platform_facade::MemoryPlatformBuilder;
//! let platform = MemoryPlatformBuilder::new().with_preset(MemoryPreset::Development).build().unwrap();
//! let _ = platform.prepared_publication_builder::<Vec<u8>>().with_tombstone_limits(1, 1024);
//! ```
//!
//! The old error variant must also be absent, independently of the builder:
//!
//! ```compile_fail
//! use quanta_memory_platform_facade::PreparedPublicationErrorV1;
//! let error: Option<PreparedPublicationErrorV1> = None;
//! if let Some(PreparedPublicationErrorV1::TombstoneCapacity { .. }) = error {}
//! ```

pub(crate) mod prepared_publication;

pub use crate::shared::builder::PreparedPublicationServiceBuilder;
pub use prepared_publication::{
    AbortDispositionV1, PrepareDispositionV1, PreparedPublicationErrorV1,
    PreparedPublicationIdentityV1, PreparedPublicationReceiptV1, PreparedPublicationRequestV1,
    PreparedPublicationService, PreparedPublicationStatsV1, PreparedPublicationTokenV1,
};
