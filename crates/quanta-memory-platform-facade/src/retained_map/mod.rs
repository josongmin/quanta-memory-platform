pub mod service;

use std::hash::Hash;

use quanta_memory_platform_core::{
    MemoryResult, StoreBindingHandle, StoreStats, StoreWriteError, SweepHint, SweepStats,
};

pub use crate::shared::builder::{BoundRetainedMapStoreBuilder, RetainedMapStoreBuilder};
pub use service::SimpleRetainedMapStore;

/// A retained key/value store with exact, hash-indexed point operations.
///
/// `Key` equality and hashing are part of the public identity contract: keys
/// that compare equal MUST produce the same hash. Implementations may rely on
/// that law for exact lookup, overwrite, and removal without scanning the
/// retained set.
pub trait RetainedMapStore<Key, Value>
where
    Key: Eq + Hash,
{
    fn binding(&self) -> &StoreBindingHandle;
    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError>;
    fn get(&self, key: &Key) -> Option<Value>
    where
        Value: Clone;
    fn remove(&self, key: &Key) -> MemoryResult<Option<Value>>
    where
        Value: Clone;
    fn sweep(&self, hint: SweepHint) -> MemoryResult<SweepStats>;
    fn stats(&self) -> StoreStats;

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
