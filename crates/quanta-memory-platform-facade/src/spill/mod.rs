pub mod backings;
pub mod service;

use quanta_memory_platform_core::{MemoryError, MemoryResult};

#[cfg(feature = "revocable_store")]
pub use service::DurableSpillTransferError;
pub use service::{
    DiscardSpillController, DurableSpillTarget, InMemorySpillTarget, RetainingSpillTarget,
    RoutedSpillController, SpillTarget, UnavailableSpillController,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpillOwnership {
    Discarded,
    InProcessRetained,
    DurableCommitted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpillReceiptV1 {
    ownership: SpillOwnership,
}

impl SpillReceiptV1 {
    #[must_use]
    pub fn new(ownership: SpillOwnership) -> Self {
        Self { ownership }
    }

    #[must_use]
    pub fn ownership(self) -> SpillOwnership {
        self.ownership
    }
}

#[derive(Debug)]
pub struct SpillTransferError<Handle> {
    error: MemoryError,
    handle: Handle,
}

impl<Handle> SpillTransferError<Handle> {
    #[must_use]
    pub fn new(error: MemoryError, handle: Handle) -> Self {
        Self { error, handle }
    }

    #[must_use]
    pub fn error(&self) -> &MemoryError {
        &self.error
    }

    #[must_use]
    pub fn into_parts(self) -> (MemoryError, Handle) {
        (self.error, self.handle)
    }
}

impl<Handle> std::fmt::Display for SpillTransferError<Handle> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "spill transfer failed before commit: {}", self.error)
    }
}

impl<Handle: std::fmt::Debug> std::error::Error for SpillTransferError<Handle> {}

pub trait SpillController<Handle> {
    /// Transfer a handle to an explicitly classified tier. Every pre-commit
    /// failure returns the original handle so source ownership can be restored.
    fn spill(&self, handle: Handle) -> Result<SpillReceiptV1, SpillTransferError<Handle>>;

    /// Restore a previously spilled handle (oldest first), or `None` if the tier
    /// holds nothing to bring back. Default: nothing to restore (a write-only /
    /// no-op controller).
    fn restore(&self) -> MemoryResult<Option<Handle>> {
        Ok(None)
    }
}
