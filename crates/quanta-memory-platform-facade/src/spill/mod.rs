pub mod backings;
pub mod service;

use quanta_memory_platform_core::MemoryResult;

pub use service::{
    DurableRestoreTarget, DurableSpillTarget, InMemorySpillTarget, InProcessSpillTarget,
    NoopSpillController, RetainingSpillTarget, RoutedSpillController, SpillFailure, SpillInventory,
    SpillReceipt, SpillTarget,
};

pub trait SpillController<Handle> {
    /// Offload a handle to the spill tier. On failure the caller regains the
    /// original handle and remains responsible for its accounting authority.
    fn spill(&self, handle: Handle) -> Result<SpillReceipt, SpillFailure<Handle>>;

    /// Restore a previously spilled handle (oldest first), or `None` if the tier
    /// holds nothing to bring back. Default: nothing to restore (a write-only /
    /// no-op controller).
    fn restore(&self) -> MemoryResult<Option<Handle>> {
        Ok(None)
    }
}

/// Marker contract for controllers whose successful receipt keeps the exact
/// handle resident in this process. It is the only controller class accepted
/// by charge-preserving revocable-store transfer.
///
/// External controllers cannot claim this accounting-bearing capability:
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{InProcessSpillController, SpillController};
/// struct Foreign;
/// impl SpillController<u8> for Foreign {
///     fn spill(&self, handle: u8) -> Result<quanta_memory_platform_facade::SpillReceipt, quanta_memory_platform_facade::SpillFailure<u8>> {
///         unimplemented!()
///     }
/// }
/// impl InProcessSpillController<u8> for Foreign {}
/// ```
pub trait InProcessSpillController<Handle>:
    SpillController<Handle> + service::sealed::InProcessSpillController<Handle>
{
}

/// Marker contract for controllers that commit an encoded handle to a durable
/// tier before returning [`SpillReceipt::Durable`].
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{DurableSpillController, SpillController};
/// struct Foreign;
/// impl SpillController<Vec<u8>> for Foreign {
///     fn spill(&self, handle: Vec<u8>) -> Result<quanta_memory_platform_facade::SpillReceipt, quanta_memory_platform_facade::SpillFailure<Vec<u8>>> {
///         unimplemented!()
///     }
/// }
/// impl DurableSpillController<Vec<u8>> for Foreign {}
/// ```
pub trait DurableSpillController<Handle>:
    SpillController<Handle> + service::sealed::DurableSpillController<Handle>
{
}

/// Persisted, never-reused identity for one durable write attempt.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DurableRecordId(u64);

impl DurableRecordId {
    #[must_use]
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Read-side resolution of an ambiguous durable write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableRecordResolution {
    Present {
        inventory_after: u64,
    },
    /// The record existed but was durably consumed. A stale write must not
    /// recreate it; a restore owned by the matching claim may publish locally.
    Finalized,
    /// The reserved record never became durable.
    Absent,
}

/// Durable write failure with an explicit authority disposition.
#[derive(Debug)]
pub enum DurableWriteFailure {
    /// The record is known not to have committed; source authority remains live.
    NotCommitted(Box<quanta_memory_platform_core::MemoryError>),
    /// Commit acknowledgement and the follow-up probe both failed. The source
    /// must be fenced until [`ReconciledDurableSpillController::resolve_record`]
    /// determines the persisted postimage.
    RecoveryRequired {
        record_id: DurableRecordId,
        commit_error: Box<quanta_memory_platform_core::MemoryError>,
        probe_error: Box<quanta_memory_platform_core::MemoryError>,
    },
}

/// Durable write protocol used by accounting-bearing stores. Bytes are borrowed,
/// so a controller panic or precommit error cannot consume the source's only
/// recovery copy.
pub trait ReconciledDurableSpillController:
    service::sealed::DurableSpillController<Vec<u8>>
{
    fn reserve_record_id(&self) -> MemoryResult<DurableRecordId>;
    fn commit_spill(
        &self,
        record_id: DurableRecordId,
        bytes: &[u8],
    ) -> Result<SpillReceipt, DurableWriteFailure>;
    fn resolve_record(&self, record_id: DurableRecordId) -> MemoryResult<DurableRecordResolution>;
}

/// Non-destructive durable restore candidate protected by a persisted claim.
/// The destination remains authoritative until
/// [`DurableRestoreController::commit_restore`] succeeds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableRestoreCandidate {
    pub sequence: u64,
    pub bytes: Vec<u8>,
}

/// Persisted phase of an interrupted durable restore.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableRestorePhase {
    /// The record is still in the ordinary durable queue under an active claim.
    Prepared,
    /// The record was removed from the queue but its bytes remain in the
    /// durable recovery slot until resident publication is acknowledged.
    Committed,
}

/// Restart recovery material for the single persisted restore claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableRestoreRecovery {
    pub candidate: DurableRestoreCandidate,
    pub phase: DurableRestorePhase,
}

/// Classification of a failed durable-delete commit.
#[derive(Debug)]
pub enum DurableCommitFailure {
    /// The durable record is known to remain authoritative.
    Retained(Box<quanta_memory_platform_core::MemoryError>),
    /// The backend cannot prove whether the exact record was deleted.
    Ambiguous(Box<quanta_memory_platform_core::MemoryError>),
}

impl DurableCommitFailure {
    pub fn retained(error: quanta_memory_platform_core::MemoryError) -> Self {
        Self::Retained(Box::new(error))
    }

    pub fn ambiguous(error: quanta_memory_platform_core::MemoryError) -> Self {
        Self::Ambiguous(Box::new(error))
    }
}

/// Result of a durable-to-source transfer.
#[derive(Debug)]
#[must_use = "ambiguous restore outcomes require sequence reconciliation"]
pub enum DurableRestoreOutcome {
    Empty,
    Restored { sequence: u64 },
    RecoveryRequired { sequence: u64, commit_error: Box<quanta_memory_platform_core::MemoryError> },
}

pub trait DurableRestoreController: DurableSpillController<Vec<u8>> {
    fn prepare_restore(&self) -> MemoryResult<Option<DurableRestoreCandidate>>;
    fn commit_restore(&self, sequence: u64) -> Result<(), DurableCommitFailure>;
    /// Delete the durable recovery payload only after the resident copy has a
    /// live charge and is ready for publication.
    fn finalize_restore(&self, sequence: u64) -> MemoryResult<()>;
    /// Release a prepared claim when decode or destination admission fails.
    fn abort_restore(&self, sequence: u64) -> MemoryResult<()>;
    /// Resolve an ambiguous exact-delete outcome and release a retained claim.
    fn reconcile_restore(&self, sequence: u64) -> MemoryResult<DurableRecordResolution>;
    /// Persisted claim left by an interrupted or ambiguous restore.
    fn active_restore(&self) -> MemoryResult<Option<DurableRecordId>>;
    /// Read restart recovery material without changing its persisted phase.
    fn recover_active_restore(&self) -> MemoryResult<Option<DurableRestoreRecovery>>;
}
