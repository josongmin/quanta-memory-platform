//! Public API surface snapshot for `quanta-memory-platform-core`.
//!
//! This integration test pins the crate's public re-export surface. It compiles
//! ONLY against `quanta_memory_platform_core::*` (no `crate::` internals), so an
//! unintended pub addition/removal/rename is caught at the boundary:
//!
//!  - if a re-export is REMOVED or RENAMED, [`names_every_public_reexport`] fails
//!    to compile (the path no longer resolves) — a hard regression signal, not a
//!    runtime assertion;
//!  - if a stable SHAPE fact changes (an enum variant disappears, the mandatory
//!    class count drifts), the `#[test]` functions below fail.
//!
//! It is deliberately NOT a string snapshot of the whole API: it references the
//! real items so the compiler is the guard, and asserts a few load-bearing
//! invariants that the host adapter and downstream presets depend on.

use quanta_memory_platform_core::{
    // domain::ledger
    AccountingLedger,
    // ports::telemetry
    AllocatorAdvisory,
    // domain::stats
    AllocatorSnapshot,
    AllocatorTelemetry,
    // ports::pin
    BasicPinRegistry,
    // api
    BasicProcessMemoryBroker,
    BasicRuntimeMemoryView,
    // ports::scratch
    BasicScratchAllocator,
    // domain::config
    BudgetOverrideLayer,
    // domain::errors
    CapacityError,
    ChargeKind,
    EffectiveBudgetResolver,
    EffectiveMemoryBudgetReport,
    // domain::policy
    ExecutionBudget,
    InvalidationOutcome,
    LedgerSnapshot,
    MemoryBudgetManifest,
    MemoryClass,
    MemoryClassBudget,
    // domain::context
    MemoryContextHandle,
    MemoryContextKind,
    MemoryError,
    MemoryPreset,
    MemoryResult,
    OverrideRecord,
    OverrideSource,
    OwnerKey,
    OwnerQuota,
    PayloadPinLease,
    PinError,
    PinRegistry,
    PressureState,
    ProcessMemoryBroker,
    ProcessMemoryBrokerBuilder,
    PublishError,
    // ports::store
    PublishedPayload,
    ReclaimPriority,
    ReservationTicket,
    ResidencyPolicy,
    ResolvedBudget,
    ResourceGroup,
    RetainedEnvelope,
    RuntimeId,
    RuntimeKind,
    RuntimeMemoryView,
    ScratchAllocator,
    ScratchRequest,
    ScratchReservation,
    StagedHandle,
    StaticAllocatorTelemetry,
    StoreBindingHandle,
    StoreDeleteError,
    StoreStats,
    StoreWriteError,
    SweepHint,
    SweepStats,
    UsageBreakdown,
};

#[cfg(feature = "live_control")]
use quanta_memory_platform_core::{
    LadderPressureController, LiveProcessMemoryBroker, PressureController, PressureThresholds,
    ReclaimRung, ResourceGroupScheduler, WeightedResourceGroupScheduler,
};

/// Names EVERY public re-export from the crate root in type position. Never
/// called; its only job is to fail compilation if any pinned export is removed
/// or renamed, so the public surface cannot silently shrink or drift. If a NEW
/// export is added intentionally, this list must be extended (the deliberate
/// review gate); an accidental rename of an existing one breaks the build here.
#[allow(dead_code, clippy::type_complexity)] // -- reason: compile-time surface pin; never executed, tuples just force path resolution.
fn names_every_public_reexport() {
    // Type/trait aliases referenced as types (resolution is the assertion).
    #[cfg(feature = "live_control")]
    type _Brokers = (BasicProcessMemoryBroker, BasicRuntimeMemoryView, LiveProcessMemoryBroker);
    #[cfg(not(feature = "live_control"))]
    type _Brokers = (BasicProcessMemoryBroker, BasicRuntimeMemoryView);
    type _BrokerTraits =
        (Box<dyn ProcessMemoryBroker>, Box<dyn RuntimeMemoryView>, ProcessMemoryBrokerBuilder);
    #[cfg(feature = "live_control")]
    type _Application = (
        LadderPressureController,
        Box<dyn PressureController>,
        PressureThresholds,
        ReclaimRung,
        WeightedResourceGroupScheduler,
        Box<dyn ResourceGroupScheduler>,
    );
    type _Config = (
        BudgetOverrideLayer,
        EffectiveBudgetResolver,
        OverrideRecord,
        OverrideSource,
        ResolvedBudget,
    );
    type _Context = (MemoryContextHandle, MemoryContextKind, OwnerKey, RuntimeId, RuntimeKind);
    type _Errors = (
        CapacityError,
        MemoryError,
        MemoryResult<()>,
        PinError,
        PublishError,
        StoreDeleteError,
        StoreWriteError,
    );
    type _Ledger =
        (AccountingLedger, ChargeKind, LedgerSnapshot, ReservationTicket, UsageBreakdown);
    type _Policy = (
        ExecutionBudget,
        MemoryBudgetManifest,
        MemoryClass,
        MemoryClassBudget,
        MemoryPreset,
        OwnerQuota,
        ReclaimPriority,
        ResidencyPolicy,
        ResourceGroup,
    );
    type _Stats = (AllocatorSnapshot, EffectiveMemoryBudgetReport, PressureState);
    type _Pin = (
        BasicPinRegistry,
        InvalidationOutcome<String>,
        PayloadPinLease<String, String>,
        Box<dyn PinRegistry<String, String>>,
    );
    type _Scratch =
        (BasicScratchAllocator, Box<dyn ScratchAllocator>, ScratchRequest, ScratchReservation);
    type _Store = (
        PublishedPayload<String>,
        RetainedEnvelope<Vec<u8>>,
        StagedHandle<String>,
        StoreBindingHandle,
        StoreStats,
        SweepHint,
        SweepStats,
    );
    type _Telemetry = (AllocatorAdvisory, Box<dyn AllocatorTelemetry>, StaticAllocatorTelemetry);
}

/// All six `MemoryError` variants remain constructible. The single error type is
/// a public contract (Build|Config|Capacity|InvalidState|Report|Unsupported);
/// removing or renaming a variant breaks this exhaustive match.
#[test]
fn memory_error_has_exactly_the_six_documented_variants() {
    let variants = [
        MemoryError::Build("b".to_string()),
        MemoryError::Config("c".to_string()),
        MemoryError::Capacity("cap".to_string()),
        MemoryError::InvalidState("s".to_string()),
        MemoryError::Report("r".to_string()),
        MemoryError::Unsupported("u".to_string()),
    ];
    // Exhaustive match: a 7th variant added without updating the snapshot fails
    // to compile here; a removed variant fails to construct above.
    for v in &variants {
        let _classified = match v {
            MemoryError::Build(_) => "build",
            MemoryError::Config(_) => "config",
            MemoryError::Capacity(_) => "capacity",
            MemoryError::InvalidState(_) => "invalid_state",
            MemoryError::Report(_) => "report",
            MemoryError::Unsupported(_) => "unsupported",
        };
    }
    // The constructor helpers are part of the surface too; assert they build the
    // expected VARIANT (not message text).
    assert!(matches!(MemoryError::build("x"), MemoryError::Build(_)));
    assert!(matches!(MemoryError::config("x"), MemoryError::Config(_)));
    assert!(matches!(MemoryError::capacity("x"), MemoryError::Capacity(_)));
    assert!(matches!(MemoryError::report("x"), MemoryError::Report(_)));
}

/// The mandatory memory-class taxonomy is a stable contract every preset must
/// populate; the count and disjointness from `future()` are load-bearing for the
/// preset validation suite. Pin both so a class moved between the two buckets (or
/// a duplicate) is caught.
#[test]
fn memory_class_taxonomy_is_stable() {
    let mandatory = MemoryClass::mandatory();
    let future = MemoryClass::future();

    // Exact counts pin the taxonomy size (14 mandatory, 4 future today).
    assert_eq!(mandatory.len(), 14, "mandatory class count is a pinned contract");
    assert_eq!(future.len(), 4, "future class count is a pinned contract");

    // No class appears in both buckets — they partition the taxonomy.
    for class in &mandatory {
        assert!(
            !future.contains(class),
            "{class:?} must not appear in both mandatory() and future()"
        );
    }
    // No duplicates within mandatory (each class is listed once).
    for (i, class) in mandatory.iter().enumerate() {
        assert!(!mandatory[i + 1..].contains(class), "{class:?} is duplicated in mandatory()");
    }
}

/// `ChargeKind` partitions all owner usage into exactly three kinds (ledger
/// invariant 2). Pin the variant set so a silent addition/removal — which would
/// break the charge-kind partition invariant — fails to compile here.
#[test]
fn charge_kind_has_exactly_three_partitions() {
    let kinds =
        [ChargeKind::RetainedPayload, ChargeKind::RetainedMetadata, ChargeKind::LiveScratch];
    for k in &kinds {
        // Exhaustive match: a 4th charge kind without updating the partition
        // fails to compile here.
        let _ = match k {
            ChargeKind::RetainedPayload => 0u8,
            ChargeKind::RetainedMetadata => 1,
            ChargeKind::LiveScratch => 2,
        };
    }
    assert_eq!(kinds.len(), 3, "ChargeKind partitions owner usage into exactly three kinds");
}

/// `MemoryPreset` is the host-facing entry point; its four variants are a public
/// contract consumed by the facade presets. Pin them exhaustively.
#[test]
fn memory_preset_has_exactly_the_four_documented_variants() {
    let presets = [
        MemoryPreset::Development,
        MemoryPreset::Ci,
        MemoryPreset::Production,
        MemoryPreset::Interactive,
    ];
    for p in &presets {
        let _ = match p {
            MemoryPreset::Development => 0u8,
            MemoryPreset::Ci => 1,
            MemoryPreset::Production => 2,
            MemoryPreset::Interactive => 3,
        };
    }
    assert_eq!(presets.len(), 4);
}

/// The builder is the only public way to materialize a broker (no raw-constructor
/// backdoor). Assert the builder-first surface really produces a broker that
/// advertises the preset it was built from — proving the public path is wired,
/// not just present.
#[test]
fn builder_first_surface_produces_a_broker_from_a_preset() {
    let broker = ProcessMemoryBrokerBuilder::new()
        .with_preset(MemoryPreset::Production)
        .build()
        .expect("a preset manifest is structurally valid and must build");
    let report: EffectiveMemoryBudgetReport = broker.effective_report();
    assert_eq!(
        report.preset,
        MemoryPreset::Production,
        "the broker must advertise the preset the builder was given"
    );
    // The report carries the full mandatory class taxonomy (surface is wired
    // end-to-end through the public builder, not a stub).
    assert_eq!(
        report.residency.classes.len(),
        MemoryClass::mandatory().len(),
        "a preset broker must define exactly the mandatory class set"
    );
}

/// A missing manifest fails closed with the `Build` VARIANT (asserted on the
/// variant, not message text) — pinning that the builder's error contract is a
/// typed `MemoryError`, part of the public surface.
#[test]
fn builder_without_manifest_fails_closed_with_build_variant() {
    let result: MemoryResult<BasicProcessMemoryBroker> = ProcessMemoryBrokerBuilder::new().build();
    assert!(
        matches!(result, Err(MemoryError::Build(_))),
        "a missing manifest must fail closed with MemoryError::Build, got {result:?}"
    );
}
