pub mod backings;
pub mod service;

use quanta_memory_platform_core::{MemoryResult, StoreBindingHandle, StoreStats, StoreWriteError};

pub use crate::shared::builder::RevocableStoreBuilder;
pub use service::SimpleRevocableStore;

pub trait RevocableStore<Key, Value> {
    fn binding(&self) -> &StoreBindingHandle;
    fn insert_checked(&self, key: Key, value: Value) -> Result<(), StoreWriteError>;
    fn reclaim_one(&self) -> MemoryResult<Option<(Key, Value)>>;
    fn stats(&self) -> StoreStats;
}
