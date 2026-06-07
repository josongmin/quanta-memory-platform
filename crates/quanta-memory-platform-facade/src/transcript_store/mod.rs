pub mod service;

use quanta_memory_platform_core::{MemoryResult, StoreBindingHandle, StoreStats};

pub use crate::shared::builder::SessionTranscriptStoreBuilder;
pub use service::SimpleSessionTranscriptStore;

pub trait SessionTranscriptStore<SessionId, Message> {
    fn binding(&self) -> &StoreBindingHandle;
    fn append(&self, session_id: SessionId, message: Message) -> MemoryResult<()>;
    fn load(&self, session_id: &SessionId) -> Vec<Message>
    where
        Message: Clone;
    fn truncate_to(&self, session_id: &SessionId, max_messages: usize) -> MemoryResult<()>;
    fn stats(&self) -> StoreStats;
}
