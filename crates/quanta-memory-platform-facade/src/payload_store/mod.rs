pub mod service;

use quanta_memory_platform_core::{
    PublishedPayload, RetainedEnvelope, StoreBindingHandle, StoreDeleteError, StoreStats,
    StoreWriteError,
};

pub use crate::shared::builder::PayloadStoreBuilder;
pub use service::SimplePayloadStore;

pub trait PayloadStore<PayloadRef, Bytes> {
    fn binding(&self) -> &StoreBindingHandle;
    fn put_canonical(
        &self,
        value: RetainedEnvelope<Bytes>,
    ) -> Result<PublishedPayload<PayloadRef>, StoreWriteError>;
    fn delete_if_unpinned(&self, payload_ref: &PayloadRef) -> Result<(), StoreDeleteError>;
    fn stats(&self) -> StoreStats;
}
