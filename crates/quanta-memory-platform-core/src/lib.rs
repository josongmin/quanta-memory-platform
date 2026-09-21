//! Portable memory-budget authority.
//!
//! Raw ledger construction is intentionally not part of the public API:
//!
//! ```compile_fail
//! use quanta_memory_platform_core::AccountingLedger;
//! ```
//!
//! A live broker is materialized only through [`ProcessMemoryBrokerBuilder`]:
//!
//! ```compile_fail
//! # use quanta_memory_platform_core::{EffectiveMemoryBudgetReport, LiveProcessMemoryBroker};
//! # fn raw(report: EffectiveMemoryBudgetReport) {
//! let broker = LiveProcessMemoryBroker::from_report(report);
//! # }
//! ```
//!
//! Removed basic/unbudgeted constructors and nested raw authority are also
//! pinned out of the safe surface:
//!
//! ```compile_fail
//! use quanta_memory_platform_core::domain::ledger::AccountingLedger;
//! ```
//!
//! ```compile_fail
//! use quanta_memory_platform_core::api::broker::BasicProcessMemoryBroker;
//! ```
//!
//! ```compile_fail
//! use quanta_memory_platform_core::api::broker::BasicRuntimeMemoryView;
//! ```
//!
//! ```compile_fail
//! use quanta_memory_platform_core::adapters::scratch::BasicScratchAllocator;
//! ```
//!
//! ```compile_fail
//! use quanta_memory_platform_core::BasicPinRegistry;
//! ```
//!
//! Raw ledger tickets and discardable committed authorities are not part of the
//! external safe API:
//!
//! ```compile_fail
//! use quanta_memory_platform_core::ReservationTicket;
//! ```
//!
//! ```compile_fail
//! use quanta_memory_platform_core::CommittedCharge;
//! ```
//!
//! ```compile_fail
//! # use quanta_memory_platform_core::{ChargeKind, LiveProcessMemoryBroker, MemoryClass, MemoryContextKind};
//! # fn raw_reserve(broker: &LiveProcessMemoryBroker) {
//! let ticket = broker.reserve(
//!     MemoryClass::ArtifactCache,
//!     "owner".to_string(),
//!     MemoryContextKind::Request,
//!     ChargeKind::RetainedPayload,
//!     1,
//! );
//! # }
//! ```
//!
//! Resource-group usage is derived only from owned ticket transitions; there is
//! no public side channel that can mutate scheduler counters:
//!
//! ```compile_fail
//! # use quanta_memory_platform_core::LiveProcessMemoryBroker;
//! # fn side_channel(broker: &LiveProcessMemoryBroker) {
//! broker.record_group_usage("group", 1);
//! # }
//! ```

pub mod adapters;
pub mod api;
pub mod application;
pub mod domain;
pub mod ports;

#[cfg(feature = "unbudgeted_pin")]
pub use adapters::pin::UnbudgetedPinRegistry;
pub use api::broker::{ProcessMemoryBroker, RuntimeMemoryView};
pub use api::builder::ProcessMemoryBrokerBuilder;
#[cfg(feature = "live_control")]
pub use api::control::{
    LiveProcessMemoryBroker, ManagedCharge, ManagedChargeRechargeError,
    ManagedChargeRechargeRecovery, PendingReservation,
};
#[cfg(feature = "live_control")]
pub use application::controller::{
    LadderPressureController, PressureController, PressureThresholds, ReclaimRung,
};
#[cfg(feature = "live_control")]
pub use application::scheduler::{ResourceGroupScheduler, WeightedResourceGroupScheduler};
pub use domain::config::{
    validate_residency, BudgetOverrideLayer, EffectiveBudgetResolver, OverrideRecord,
    OverrideSource, ResolvedBudget, ValidatedResidencyPolicy,
};
pub use domain::context::{
    MemoryContextHandle, MemoryContextKind, OwnerKey, RuntimeId, RuntimeKind,
};
pub use domain::errors::{
    CapacityError, MemoryError, MemoryResult, PinError, PublishError, StoreDeleteError,
    StoreWriteError,
};
pub use domain::ledger::{ChargeKind, LedgerSnapshot, UsageBreakdown};
pub use domain::policy::{
    ExecutionBudget, MemoryBudgetManifest, MemoryClass, MemoryClassBudget, MemoryPreset,
    OwnerQuota, ReclaimPriority, ResidencyPolicy, ResourceGroup,
};
pub use domain::stats::{
    AllocatorSnapshot, EffectiveMemoryBudgetReport, PressureState, ReclaimRuntimeStats,
};
#[cfg(feature = "unbudgeted_pin")]
pub use ports::pin::{InvalidationOutcome, PayloadPinLease, PinRegistry};
#[cfg(feature = "live_control")]
pub use ports::reclaim::{
    MemoryReclaimer, ReclaimCandidate, ReclaimGroupCandidate, ReclaimOutcome, ReclaimRequest,
    ReclaimTrigger,
};
pub use ports::scratch::{ScratchAllocator, ScratchRequest, ScratchReservation};
pub use ports::store::{
    PublishedPayload, RetainedEnvelope, StagedHandle, StoreBindingHandle, StoreStats, SweepHint,
    SweepStats,
};
pub use ports::telemetry::{AllocatorAdvisory, AllocatorTelemetry, StaticAllocatorTelemetry};
