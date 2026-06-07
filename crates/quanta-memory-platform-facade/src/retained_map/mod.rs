pub mod service;

use quanta_memory_platform_core::{
    MemoryResult, StoreBindingHandle, StoreStats, StoreWriteError, SweepHint, SweepStats,
};

pub use crate::shared::builder::RetainedMapStoreBuilder;
pub use service::SimpleRetainedMapStore;

pub trait RetainedMapStore<Key, Value> {
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
