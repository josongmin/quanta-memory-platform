pub mod adapters;
pub mod api;
pub mod application;
pub mod domain;
pub mod ports;

pub use adapters::pin::BasicPinRegistry;
pub use adapters::scratch::BasicScratchAllocator;
pub use api::broker::{
    BasicProcessMemoryBroker, BasicRuntimeMemoryView, ProcessMemoryBroker, RuntimeMemoryView,
};
pub use api::builder::ProcessMemoryBrokerBuilder;
#[cfg(feature = "live_control")]
pub use api::control::LiveProcessMemoryBroker;
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
pub use domain::ledger::{
    AccountingLedger, ChargeKind, LedgerSnapshot, ReservationTicket, UsageBreakdown,
};
pub use domain::policy::{
    ExecutionBudget, MemoryBudgetManifest, MemoryClass, MemoryClassBudget, MemoryPreset,
    OwnerQuota, ReclaimPriority, ResidencyPolicy, ResourceGroup,
};
pub use domain::recharge::{RechargeError, RechargeRecoveryV1};
pub use domain::stats::{AllocatorSnapshot, EffectiveMemoryBudgetReport, PressureState};
pub use ports::pin::{InvalidationOutcome, PayloadPinLease, PinRegistry};
pub use ports::scratch::{ScratchAllocator, ScratchRequest, ScratchReservation};
pub use ports::store::{
    PublishedPayload, RetainedEnvelope, StagedHandle, StoreBindingHandle, StoreStats, SweepHint,
    SweepStats,
};
pub use ports::telemetry::{AllocatorAdvisory, AllocatorTelemetry, StaticAllocatorTelemetry};
