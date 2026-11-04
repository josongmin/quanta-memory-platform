pub mod service;

use std::hash::Hash;

use quanta_memory_platform_core::{
    MemoryResult, StoreBindingHandle, StoreStats, StoreWriteError, SweepHint, SweepStats,
};

pub use crate::shared::builder::{BoundRetainedMapStoreBuilder, RetainedMapStoreBuilder};
pub use crate::shared::lifecycle::RetainedMapReadRefusalV73;
pub use service::SimpleRetainedMapStore;

/// Result of an atomic insert-if-absent publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedMapInsertOutcomeV1 {
    Inserted,
    AlreadyPresent,
}

/// Result of an atomic insert-if-absent operation that returns the occupied
/// value when the key already exists.
///
/// The occupied value is cloned under the store's exclusive mutation lane
/// with the state detached from the container mutex. This lets callers
/// validate immutable value metadata without a split point-read and insert
/// race.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetainedMapInsertOrExistingV1<Value> {
    Inserted,
    AlreadyPresent(Value),
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
    /// Publish only when `key` is absent; when occupied, clone and return the
    /// existing value under the same exclusive mutation lane (state detached
    /// from the container mutex).
    fn insert_if_absent_or_get_checked(
        &self,
        key: Key,
        value: Value,
    ) -> Result<RetainedMapInsertOrExistingV1<Value>, StoreWriteError>
    where
        Value: Clone;
    /// Publish only when `key` is absent, deciding presence and mutation under
    /// the store's exclusive mutation lane. An `AlreadyPresent` result leaves the
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
    fn remove(&self, key: &Key) -> MemoryResult<Option<Value>>
    where
        Value: Clone;
    fn sweep(&self, hint: SweepHint) -> MemoryResult<SweepStats>;
    /// Checked statistics read. A poisoned store is not an empty store.
    fn stats_checked(&self) -> MemoryResult<StoreStats>;

    /// Checked enumeration of every live key in deterministic key order.
    ///
    /// Enumerations return owned vectors so callers never retain the state lock.
    /// The order is stable across runs and process boundaries (sorted by `Key`),
    /// so callers that need to iterate, GC, or list-all over the retained set get
    /// a reproducible result rather than insertion order.
    fn keys_checked(&self) -> MemoryResult<Vec<Key>>
    where
        Key: Clone + Ord;

    /// Checked enumeration of every live `(key, value)` pair.
    fn entries_checked(&self) -> MemoryResult<Vec<(Key, Value)>>
    where
        Key: Clone + Ord,
        Value: Clone;

    /// Atomically release every live accounting authority and terminally close
    /// this store. Repeated close is idempotent; later mutation is rejected.
    fn close(&self) -> MemoryResult<()>;
}
