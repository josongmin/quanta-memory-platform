pub mod backings;
pub mod service;

use quanta_memory_platform_core::MemoryResult;

pub use service::{
    InMemorySpillTarget, NoopSpillController, RetainingSpillTarget, RoutedSpillController,
    SpillFailure, SpillTarget,
};

pub trait SpillController<Handle> {
    /// Offload a handle to the spill tier. On failure the caller regains the
    /// original handle and remains responsible for its accounting authority.
    fn spill(&self, handle: Handle) -> Result<(), SpillFailure<Handle>>;

    /// Restore a previously spilled handle (oldest first), or `None` if the tier
    /// holds nothing to bring back. Default: nothing to restore (a write-only /
    /// no-op controller).
    fn restore(&self) -> MemoryResult<Option<Handle>> {
        Ok(None)
    }
}
