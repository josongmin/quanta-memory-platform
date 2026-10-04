pub mod service;

use std::hash::Hash;

use quanta_memory_platform_core::{
    MemoryResult, PublishedPayload, RetainedEnvelope, StoreBindingHandle, StoreDeleteError,
    StoreStats, StoreWriteError,
};

pub use crate::shared::builder::PayloadStoreBuilder;
use crate::shared::measure::ByteSized;
pub use service::SimplePayloadStore;

mod private {
    pub trait Sealed {}
    #[cfg(not(test))]
    impl Sealed for String {}
    // Unit tests exercise hostile caller traits. Dependency and integration
    // builds never see this blanket implementation.
    #[cfg(test)]
    impl<T> Sealed for T where
        T: Clone + From<String> + Eq + std::hash::Hash + crate::shared::measure::ByteSized
    {
    }
}

/// Payload reference representation issued by this in-memory store.
///
/// The generated reference is process-scoped and includes a unique store nonce
/// and a monotonically increasing entry sequence. Only `String` is supported
/// in production: an arbitrary `From<String>` conversion can erase the store
/// nonce or reuse a deleted reference, allowing it to address a different
/// canonical payload. References are not authorization tokens or durable IDs.
///
/// A caller-defined conversion cannot opt into this authority:
///
/// ```compile_fail
/// use quanta_memory_platform_facade::{ByteSized, PayloadReference};
/// #[derive(Clone, PartialEq, Eq, Hash)]
/// struct CollapsedRef;
/// impl From<String> for CollapsedRef { fn from(_: String) -> Self { Self } }
/// impl ByteSized for CollapsedRef { fn byte_size(&self) -> u64 { 0 } }
/// impl PayloadReference for CollapsedRef {}
/// ```
pub trait PayloadReference: private::Sealed + Clone + From<String> + Eq + Hash + ByteSized {}

#[cfg(not(test))]
impl PayloadReference for String {}

#[cfg(test)]
impl<T> PayloadReference for T where T: Clone + From<String> + Eq + Hash + ByteSized {}

pub trait PayloadStore<PayloadRef, Bytes> {
    fn binding(&self) -> &StoreBindingHandle;
    fn put_canonical(
        &self,
        value: RetainedEnvelope<Bytes>,
    ) -> Result<PublishedPayload<PayloadRef>, StoreWriteError>;
    fn delete_if_unpinned(&self, payload_ref: &PayloadRef) -> Result<(), StoreDeleteError>;
    fn stats(&self) -> MemoryResult<StoreStats>;
    fn close(&self) -> MemoryResult<()>;
}
