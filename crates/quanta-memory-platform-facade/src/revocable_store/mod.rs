pub mod service;

use quanta_memory_platform_core::{MemoryResult, StoreBindingHandle, StoreStats, StoreWriteError};

pub use crate::shared::builder::RevocableStoreBuilder;
<<<<<<< HEAD
#[cfg(feature = "spill")]
pub use service::RevocableSpillHandle;
pub use service::SimpleRevocableStore;
=======
pub use service::{
    ReclaimFinalizeError, ReclaimedEntry, RestoreReclaimedError, SimpleRevocableStore,
};
>>>>>>> codex/qmp130-upstream-hardening

pub trait RevocableStore<Key, Value> {
    fn binding(&self) -> &StoreBindingHandle;
    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError>;
    /// Detach the oldest entry while preserving its live accounting authority.
    /// The returned entry can be restored to this source or transferred to an
    /// in-process tier without creating uncharged resident bytes.
    fn prepare_reclaim_one(&self) -> MemoryResult<Option<ReclaimedEntry<Key, Value>>> {
        Err(quanta_memory_platform_core::MemoryError::InvalidState(
            "this revocable store does not expose recoverable transfer authority".to_string(),
        ))
    }
    /// Restore a prepared entry to its exact source binding.
    fn restore_reclaimed(
        &self,
        entry: ReclaimedEntry<Key, Value>,
    ) -> Result<(), RestoreReclaimedError<Key, Value>> {
        Err(RestoreReclaimedError::new(
            quanta_memory_platform_core::MemoryError::InvalidState(
                "this revocable store cannot restore transfer authority".to_string(),
            ),
            entry,
        ))
    }
    /// Explicit destructive reclaim. Prefer `prepare_reclaim_one` when the value
    /// will be transferred to another tier.
    fn discard_one(&self) -> MemoryResult<Option<(Key, Value)>>;
    /// Checked statistics. Poison and lifecycle failures are never projected as
    /// an empty store.
    fn stats(&self) -> MemoryResult<StoreStats>;
    /// Terminal, idempotent release of every live entry and accounting charge.
    fn close(&self) -> MemoryResult<()>;
}
