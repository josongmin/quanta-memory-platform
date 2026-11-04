#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::let_underscore_must_use, clippy::allow_attributes_without_reason)
)]

//! Builder-first portable memory platform.
//!
//! Report-only resolution returns data, not runtime authority:
//!
//! ```compile_fail
//! # use quanta_memory_platform_facade::{MemoryPlatformBuilder, PortableMemoryPresets};
//! # use quanta_memory_platform_core::RuntimeKind;
//! # fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let report = MemoryPlatformBuilder::new()
//!     .with_manifest(PortableMemoryPresets::development())
//!     .build_report_only()?;
//! report.attach_runtime("runtime".to_string(), RuntimeKind::Runtime)?;
//! # Ok(()) }
//! ```
//!
//! The former unrestricted bumpalo scratch surface is not public:
//!
//! ```compile_fail
//! use quanta_memory_platform_facade::scratch::bumpalo::BumpaloScratchArena;
//! ```
//!
//! Accounting authority remains internal to the facade:
//!
//! ```compile_fail
//! use quanta_memory_platform_facade::AccountingSink;
//! ```
//!
//! ```compile_fail
//! use quanta_memory_platform_facade::SharedAccountingSink;
//! ```

// Slice-agnostic glue is always available: the broker builder, presets, the
// accounting seam, and byte-measurement. Each store/pipeline slice is gated
// behind its feature so `--no-default-features` carries no slice surface and a
// feature combination that does not enable a slice cannot reach its (or its
// backing's) code — the feature-slice contract is real, not nominal.
pub mod presets;
pub mod shared;

#[cfg(all(
    test,
    any(
        feature = "payload_store",
        feature = "retained_map",
        feature = "revocable_store",
        feature = "transcript",
        feature = "publish_pipeline"
    )
))]
mod test_support;

#[cfg(feature = "payload_store")]
pub mod payload_store;
#[cfg(feature = "publish_pipeline")]
pub mod publish_pipeline;
#[cfg(feature = "retained_map")]
pub mod retained_map;
#[cfg(feature = "revocable_store")]
pub mod revocable_store;
#[cfg(feature = "spill")]
pub mod spill;
#[cfg(feature = "transcript")]
pub mod transcript_store;

pub use presets::PortableMemoryPresets;
pub use shared::accounting::AccountingUnset;
#[cfg(any(
    feature = "payload_store",
    feature = "retained_map",
    feature = "revocable_store",
    feature = "transcript",
    feature = "publish_pipeline"
))]
pub use shared::accounting::Budgeted;
#[cfg(all(
    feature = "unbudgeted-stores",
    any(
        feature = "payload_store",
        feature = "retained_map",
        feature = "revocable_store",
        feature = "transcript",
        feature = "publish_pipeline"
    )
))]
pub use shared::accounting::Unbudgeted;
#[cfg(feature = "core-live-accounting")]
pub use shared::builder::LiveMemoryPlatform;
pub use shared::builder::MemoryPlatformBuilder;
pub use shared::measure::{
    semantic_cost_v1, ByteSized, CanonicalKey, MeasuredValue, RetainedCostV1,
};

#[cfg(feature = "payload_store")]
pub use payload_store::{PayloadReference, PayloadStore, PayloadStoreBuilder, SimplePayloadStore};
#[cfg(feature = "publish_pipeline")]
pub use publish_pipeline::{
    AbortDispositionV1, PrepareDispositionV1, PreparedPublicationErrorV1,
    PreparedPublicationIdentityV1, PreparedPublicationReceiptV1, PreparedPublicationRequestV1,
    PreparedPublicationService, PreparedPublicationStatsV1, PreparedPublicationTokenV1,
};
#[cfg(feature = "retained_map")]
pub use retained_map::{
    BoundRetainedMapStoreBuilder, RetainedMapInsertOrExistingV1, RetainedMapInsertOutcomeV1,
    RetainedMapReadRefusalV73, RetainedMapStore, RetainedMapStoreBuilder, SimpleRetainedMapStore,
};
#[cfg(all(feature = "revocable_store", feature = "spill"))]
pub use revocable_store::RevocableSpillHandle;
#[cfg(feature = "revocable_store")]
pub use revocable_store::{RevocableStore, RevocableStoreBuilder, SimpleRevocableStore};
#[cfg(feature = "publish_pipeline")]
pub use shared::builder::PreparedPublicationServiceBuilder;
#[cfg(feature = "transcript")]
pub use shared::builder::SessionTranscriptStoreBuilder;
#[cfg(feature = "spill-redb")]
pub use spill::backings::redb::{RedbSpillBacking, RedbSpillLimits};
#[cfg(feature = "spill-durable")]
pub use spill::{
    DurableCommitFailure, DurableRecordId, DurableRecordResolution, DurableRestoreCandidate,
    DurableRestoreClaim, DurableRestoreController, DurableRestoreOutcome, DurableRestorePhase,
    DurableRestoreRecovery, DurableRestoreTarget, DurableSpillController, DurableSpillTarget,
    DurableWriteFailure, ReconciledDurableSpillController,
};
#[cfg(feature = "spill")]
pub use spill::{
    InMemorySpillTarget, InProcessSpillController, InProcessSpillTarget, NoopSpillController,
    RetainingSpillTarget, RoutedSpillController, SpillController, SpillFailure, SpillInventory,
    SpillReceipt, SpillTarget,
};
#[cfg(feature = "transcript")]
pub use transcript_store::{
    SessionHandle, SessionTranscriptStore, SimpleSessionTranscriptStore,
    TranscriptAppendDispositionV1, TranscriptAppendErrorV1, TranscriptAppendIdentityV1,
    TranscriptAppendRequestV1, TranscriptCursor, TranscriptRecordV1, TranscriptRetainDispositionV1,
    TranscriptRetentionPolicyV2, TranscriptRetentionReconfigureErrorV2, TranscriptRetentionStatsV2,
    TranscriptSnapshotV1,
};
