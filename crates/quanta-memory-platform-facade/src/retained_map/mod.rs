pub mod service;

use std::hash::Hash;

use quanta_memory_platform_core::{
    MemoryResult, StoreBindingHandle, StoreStats, StoreWriteError, SweepHint, SweepStats,
};

pub use crate::shared::builder::{BoundRetainedMapStoreBuilder, RetainedMapStoreBuilder};
pub use service::SimpleRetainedMapStore;

/// Result of an atomic insert-if-absent publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedMapInsertOutcomeV1 {
    Inserted,
    AlreadyPresent,
}

/// A retained key/value store with exact, hash-indexed point operations.
///
/// `Key` equality and hashing are part of the public identity contract: keys
/// that compare equal MUST produce the same hash. Implementations may rely on
/// that law for exact lookup, overwrite, and removal without scanning the
/// retained set.
///
/// A key with equality but no lawful hash identity is rejected at the public
/// trait boundary:
///
/// ```compile_fail
/// use quanta_memory_platform_facade::RetainedMapStore;
///
/// #[derive(Clone, PartialEq)]
/// struct PartialEqOnly(String);
///
/// fn require_retained_store<S>()
/// where
///     S: RetainedMapStore<PartialEqOnly, Vec<u8>>,
/// {
/// }
/// ```
pub trait RetainedMapStore<Key, Value>
where
    Key: Eq + Hash,
{
    fn binding(&self) -> &StoreBindingHandle;
    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError>;
    /// Publish only when `key` is absent, deciding presence and mutation under
    /// the store's canonical state lock. An `AlreadyPresent` result leaves the
    /// existing payload, charge, and insertion order unchanged.
    fn insert_if_absent_checked(
        &self,
        key: Key,
        value: Value,
    ) -> Result<RetainedMapInsertOutcomeV1, StoreWriteError>;
    /// Checked point read. Lock/storage failure is distinct from absence.
    fn get_checked(&self, key: &Key) -> MemoryResult<Option<Value>>
    where
        Value: Clone;
    /// Compatibility projection for advisory callers that cannot carry a
    /// storage error. Correctness-sensitive paths MUST use `get_checked`.
    fn get(&self, key: &Key) -> Option<Value>
    where
        Value: Clone,
    {
        self.get_checked(key).ok().flatten()
    }
    fn remove(&self, key: &Key) -> MemoryResult<Option<Value>>
    where
        Value: Clone;
    fn sweep(&self, hint: SweepHint) -> MemoryResult<SweepStats>;
    /// Checked statistics read. A poisoned store is not an empty store.
    fn stats_checked(&self) -> MemoryResult<StoreStats>;
    /// Advisory compatibility projection. Admission and consistency checks
    /// MUST use `stats_checked`.
    fn stats(&self) -> StoreStats {
        self.stats_checked().unwrap_or_default()
    }

    /// Enumerate every live key in a deterministic, key-sorted order.
    ///
    /// The order is stable across runs and process boundaries (sorted by `Key`),
    /// so callers that need to iterate, GC, or list-all over the retained set get
    /// a reproducible result rather than insertion order. Returns an owned `Vec`
    /// so callers never hold the store's internal lock.
    fn keys(&self) -> Vec<Key>
    where
        Key: Clone + Ord;

    /// Enumerate every live `(key, value)` pair in the same deterministic,
    /// key-sorted order as [`RetainedMapStore::keys`].
    fn entries(&self) -> Vec<(Key, Value)>
    where
        Key: Clone + Ord,
        Value: Clone;
}
