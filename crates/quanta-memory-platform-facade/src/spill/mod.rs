pub mod backings;
pub mod service;

use quanta_memory_platform_core::MemoryResult;

pub use service::{
    InMemorySpillTarget, NoopSpillController, RetainingSpillTarget, RoutedSpillController,
    SpillTarget,
};

pub trait SpillController<Handle> {
    /// Offload a handle to the spill tier.
    fn spill(&self, handle: Handle) -> MemoryResult<()>;

    /// Restore a previously spilled handle (oldest first), or `None` if the tier
    /// holds nothing to bring back. Default: nothing to restore (a write-only /
    /// no-op controller).
    fn restore(&self) -> MemoryResult<Option<Handle>> {
        Ok(None)
    }
}
