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
pub trait InProcessSpillController<Handle>: SpillController<Handle> {}

/// Marker contract for controllers that commit an encoded handle to a durable
/// tier before returning [`SpillReceipt::Durable`].
pub trait DurableSpillController<Handle>: SpillController<Handle> {}

/// Non-destructive durable restore candidate. The destination remains
/// authoritative until [`DurableRestoreController::commit_restore`] succeeds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableRestoreCandidate {
    pub sequence: u64,
    pub bytes: Vec<u8>,
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
}
