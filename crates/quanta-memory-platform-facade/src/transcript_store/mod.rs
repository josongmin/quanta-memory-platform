pub mod service;

use std::fmt::{Display, Formatter};

use quanta_memory_platform_core::{MemoryError, MemoryResult, StoreBindingHandle, StoreStats};

pub use crate::shared::builder::SessionTranscriptStoreBuilder;
pub use service::SimpleSessionTranscriptStore;

/// Caller-owned idempotency identity for one append operation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TranscriptAppendIdentityV1<SessionId, MessageId> {
    session_id: SessionId,
    sequence: u64,
    message_id: MessageId,
}

impl<SessionId, MessageId> TranscriptAppendIdentityV1<SessionId, MessageId> {
    #[must_use]
    pub fn new(session_id: SessionId, sequence: u64, message_id: MessageId) -> Self {
        Self { session_id, sequence, message_id }
    }

    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptAppendRequestV1<SessionId, MessageId, Message> {
    identity: TranscriptAppendIdentityV1<SessionId, MessageId>,
    body: Message,
}

impl<SessionId, MessageId, Message> TranscriptAppendRequestV1<SessionId, MessageId, Message> {
    #[must_use]
    pub fn new(identity: TranscriptAppendIdentityV1<SessionId, MessageId>, body: Message) -> Self {
        Self { identity, body }
    }

    #[must_use]
    pub fn identity(&self) -> &TranscriptAppendIdentityV1<SessionId, MessageId> {
        &self.identity
    }

    #[must_use]
    pub fn body(&self) -> &Message {
        &self.body
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptAppendDispositionV1 {
    Applied,
    Replayed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptAppendErrorV1 {
    PayloadConflict { sequence: u64 },
    SequenceConflict { sequence: u64 },
    MessageIdConflict,
    ExpiredIdentity { sequence: u64 },
    Accounting(MemoryError),
    Storage(MemoryError),
}

impl Display for TranscriptAppendErrorV1 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PayloadConflict { sequence } => {
                write!(f, "transcript payload conflict at sequence {sequence}")
            }
            Self::SequenceConflict { sequence } => {
                write!(f, "transcript sequence conflict at sequence {sequence}")
            }
            Self::MessageIdConflict => f.write_str("transcript message-id conflict"),
            Self::ExpiredIdentity { sequence } => {
                write!(f, "transcript identity evidence expired at sequence {sequence}")
            }
            Self::Accounting(error) => write!(f, "transcript accounting error: {error}"),
            Self::Storage(error) => write!(f, "transcript storage error: {error}"),
        }
    }
}

impl std::error::Error for TranscriptAppendErrorV1 {}

impl From<MemoryError> for TranscriptAppendErrorV1 {
    fn from(error: MemoryError) -> Self {
        Self::Storage(error)
    }
}

/// Ordered transcript row retaining the caller's complete append identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptRecordV1<MessageId, Message> {
    sequence: u64,
    message_id: MessageId,
    body: Message,
}

/// Immutable active-row snapshot paired with the session mutation version.
///
/// Callers may derive a retention selector without holding the store mutex,
/// then submit it through `retain_sequences_if_version`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptSnapshotV1<MessageId, Message> {
    version: u64,
    records: Vec<TranscriptRecordV1<MessageId, Message>>,
}

impl<MessageId, Message> TranscriptSnapshotV1<MessageId, Message> {
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    #[must_use]
    pub fn records(&self) -> &[TranscriptRecordV1<MessageId, Message>] {
        &self.records
    }

    #[must_use]
    pub fn into_records(self) -> Vec<TranscriptRecordV1<MessageId, Message>> {
        self.records
    }
}

/// Result of a conditional retention mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptRetainDispositionV1 {
    Applied { version: u64 },
    StaleSnapshot { expected_version: u64, actual_version: u64 },
}

impl<MessageId, Message> TranscriptRecordV1<MessageId, Message> {
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }

    #[must_use]
    pub fn body(&self) -> &Message {
        &self.body
    }

    #[must_use]
    pub fn into_body(self) -> Message {
        self.body
    }
}

/// Dedupe evidence policy for the in-memory backing.
///
/// Active message bodies may be truncated, but the sequence/message-id/body
/// digest tombstone remains for the process lifetime. This proves only
/// process-lifetime idempotency, never restart durability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranscriptRetentionPolicyV1;

impl TranscriptRetentionPolicyV1 {
    #[must_use]
    pub fn process_lifetime() -> Self {
        Self
    }

    #[must_use]
    pub fn retains_process_lifetime_tombstones(self) -> bool {
        true
    }
}

impl Default for TranscriptRetentionPolicyV1 {
    fn default() -> Self {
        Self::process_lifetime()
    }
}

pub trait SessionTranscriptStore<SessionId, MessageId, Message> {
    fn binding(&self) -> &StoreBindingHandle;
    fn append(
        &self,
        request: TranscriptAppendRequestV1<SessionId, MessageId, Message>,
    ) -> Result<TranscriptAppendDispositionV1, TranscriptAppendErrorV1>;
    /// Load active rows ordered by caller-supplied sequence. Sparse sequences
    /// are valid; duplicate sequences are rejected during append.
    fn load(
        &self,
        session_id: &SessionId,
    ) -> MemoryResult<Vec<TranscriptRecordV1<MessageId, Message>>>
    where
        MessageId: Clone,
        Message: Clone;
    /// Retain exactly the named active sequences and evict every other active
    /// row. Identity tombstones survive eviction, so late retries remain
    /// idempotent. Duplicate or unknown sequence selectors fail closed before
    /// accounting or row state changes.
    fn retain_sequences(
        &self,
        session_id: &SessionId,
        retained_sequences: &[u64],
    ) -> MemoryResult<()>;
    fn truncate_to(&self, session_id: &SessionId, max_messages: usize) -> MemoryResult<()>;
    /// Checked statistics: a poisoned backing cannot appear as an empty store.
    fn stats(&self) -> MemoryResult<StoreStats>;
    /// Terminal, idempotent release of all active rows and managed charges.
    fn close(&self) -> MemoryResult<()>;
}
