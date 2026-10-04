pub mod service;

use std::fmt::{Display, Formatter};

use crate::shared::measure::ByteSized;
use quanta_memory_platform_core::{MemoryError, MemoryResult, StoreBindingHandle, StoreStats};

pub use crate::shared::builder::SessionTranscriptStoreBuilder;
pub use service::SimpleSessionTranscriptStore;

/// Process-local incarnation of one opened transcript session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionHandle {
    store_nonce: u64,
    sequence: u64,
}

impl SessionHandle {
    pub(crate) const fn new(store_nonce: u64, sequence: u64) -> Self {
        Self { store_nonce, sequence }
    }
}

/// Idempotency identity for one append: a store-issued session handle plus
/// caller-selected sequence and message id.
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
    PayloadConflict {
        sequence: u64,
    },
    SequenceConflict {
        sequence: u64,
    },
    MessageIdConflict,
    RetentionSaturated {
        evidence_count: usize,
        max_evidence_count: usize,
        evidence_bytes: u64,
        requested_evidence_bytes: u64,
        max_evidence_bytes: u64,
    },
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
            Self::RetentionSaturated {
                evidence_count,
                max_evidence_count,
                evidence_bytes,
                requested_evidence_bytes,
                max_evidence_bytes,
            } => write!(
                f,
                "transcript identity retention saturated: count {evidence_count}/{max_evidence_count}, bytes {evidence_bytes}+{requested_evidence_bytes}/{max_evidence_bytes}"
            ),
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
    handle: SessionHandle,
    version: u64,
    records: Vec<TranscriptRecordV1<MessageId, Message>>,
    next_cursor: Option<TranscriptCursor>,
}

/// Store-issued position in one session version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranscriptCursor {
    handle: SessionHandle,
    version: u64,
    last_sequence: u64,
}

impl TranscriptCursor {
    #[must_use]
    pub const fn handle(&self) -> SessionHandle {
        self.handle
    }
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }
    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }
}

impl<MessageId, Message> TranscriptSnapshotV1<MessageId, Message> {
    #[must_use]
    pub const fn handle(&self) -> SessionHandle {
        self.handle
    }
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    #[must_use]
    pub fn records(&self) -> &[TranscriptRecordV1<MessageId, Message>] {
        &self.records
    }

    #[must_use]
    pub const fn next_cursor(&self) -> Option<TranscriptCursor> {
        self.next_cursor
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

/// Bounded, fail-closed dedupe evidence policy for the in-memory backing.
///
/// Active message bodies may be truncated, but admitted sequence/message-id/body
/// evidence remains until that handle's `retire_session` or store `close`.
/// Exact retries and conflicts are checked only while its handle is open.
/// This is a process-local horizon, never restart durability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranscriptRetentionPolicyV2 {
    max_evidence_count: usize,
    max_evidence_bytes: u64,
    max_session_count: usize,
    max_session_key_bytes: u64,
}

impl TranscriptRetentionPolicyV2 {
    pub const DEFAULT_MAX_EVIDENCE_COUNT: usize = 65_536;
    pub const DEFAULT_MAX_EVIDENCE_BYTES: u64 = 64 * 1024 * 1024;
    pub const DEFAULT_MAX_SESSION_COUNT: usize = 65_536;
    pub const DEFAULT_MAX_SESSION_KEY_BYTES: u64 = 64 * 1024 * 1024;

    #[must_use]
    pub const fn fail_closed(max_evidence_count: usize, max_evidence_bytes: u64) -> Self {
        Self {
            max_evidence_count,
            max_evidence_bytes,
            max_session_count: Self::DEFAULT_MAX_SESSION_COUNT,
            max_session_key_bytes: Self::DEFAULT_MAX_SESSION_KEY_BYTES,
        }
    }

    #[must_use]
    pub const fn with_session_limits(
        mut self,
        max_session_count: usize,
        max_session_key_bytes: u64,
    ) -> Self {
        self.max_session_count = max_session_count;
        self.max_session_key_bytes = max_session_key_bytes;
        self
    }

    #[must_use]
    pub const fn max_session_count(self) -> usize {
        self.max_session_count
    }
    #[must_use]
    pub const fn max_session_key_bytes(self) -> u64 {
        self.max_session_key_bytes
    }

    #[must_use]
    pub const fn max_evidence_count(self) -> usize {
        self.max_evidence_count
    }

    #[must_use]
    pub const fn max_evidence_bytes(self) -> u64 {
        self.max_evidence_bytes
    }
}

impl Default for TranscriptRetentionPolicyV2 {
    fn default() -> Self {
        Self::fail_closed(Self::DEFAULT_MAX_EVIDENCE_COUNT, Self::DEFAULT_MAX_EVIDENCE_BYTES)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranscriptRetentionStatsV2 {
    pub active_row_count: usize,
    pub active_bytes: u64,
    pub evidence_count: usize,
    pub evidence_bytes: u64,
    pub session_count: usize,
    pub session_key_bytes: u64,
    pub total_retained_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptRetentionReconfigureErrorV2 {
    BelowLiveEvidence {
        live_evidence_count: usize,
        requested_max_evidence_count: usize,
        live_evidence_bytes: u64,
        requested_max_evidence_bytes: u64,
    },
    BelowLiveSessions {
        live_session_count: usize,
        requested_max_session_count: usize,
        live_session_key_bytes: u64,
        requested_max_session_key_bytes: u64,
    },
    Storage(MemoryError),
}

impl Display for TranscriptRetentionReconfigureErrorV2 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BelowLiveEvidence {
                live_evidence_count,
                requested_max_evidence_count,
                live_evidence_bytes,
                requested_max_evidence_bytes,
            } => write!(
                f,
                "transcript retention limit is below live evidence: count {live_evidence_count}/{requested_max_evidence_count}, bytes {live_evidence_bytes}/{requested_max_evidence_bytes}"
            ),
            Self::Storage(error) => write!(f, "transcript retention reconfigure failed: {error}"),
            Self::BelowLiveSessions { live_session_count, requested_max_session_count, live_session_key_bytes, requested_max_session_key_bytes } => write!(f,
                "transcript session limit is below live sessions: count {live_session_count}/{requested_max_session_count}, bytes {live_session_key_bytes}/{requested_max_session_key_bytes}"),
        }
    }
}

impl std::error::Error for TranscriptRetentionReconfigureErrorV2 {}

/// Opening a session charges its caller metadata, even with zero rows. Appends
/// charge active rows and identity evidence. Truncation preserves evidence;
/// `retire_session` ends one handle's replay horizon and `close` ends all of them.
/// Caller metadata is not a routing key: equal metadata may have concurrent
/// independent handles, and a retired handle never resolves to a newer one.
pub trait SessionTranscriptStore<SessionId, MessageId, Message>
where
    SessionId: ByteSized,
    MessageId: ByteSized,
{
    fn binding(&self) -> &StoreBindingHandle;
    fn open_session(&self, session_id: SessionId) -> MemoryResult<SessionHandle>;
    fn append(
        &self,
        request: TranscriptAppendRequestV1<SessionHandle, MessageId, Message>,
    ) -> Result<TranscriptAppendDispositionV1, TranscriptAppendErrorV1>;
    /// Load active rows ordered by caller-supplied sequence. Sparse sequences
    /// are valid; duplicate sequences are rejected during append.
    fn load(
        &self,
        handle: &SessionHandle,
    ) -> MemoryResult<Vec<TranscriptRecordV1<MessageId, Message>>>
    where
        MessageId: Clone,
        Message: Clone;
    /// Page rows from one session version. The semantic byte limit sums each
    /// selected row's active retained cost (body bytes, message-id bytes,
    /// ordered-index bytes, entry overhead). Returned caller-owned copies are
    /// separate from store accounting. Zero limits and a first oversized row
    /// return Capacity without advancing a cursor. A page is only a subset:
    /// its rows are not a complete retention selector for the session.
    fn read_page(
        &self,
        handle: &SessionHandle,
        cursor: Option<&TranscriptCursor>,
        max_rows: usize,
        max_semantic_bytes: u64,
    ) -> MemoryResult<TranscriptSnapshotV1<MessageId, Message>>
    where
        MessageId: Clone,
        Message: Clone;
    /// Retain exactly the named active sequences and evict every other active
    /// row. Identity tombstones survive eviction, so late retries remain
    /// idempotent. Duplicate or unknown sequence selectors fail closed before
    /// accounting or row state changes.
    fn retain_sequences(
        &self,
        handle: &SessionHandle,
        retained_sequences: &[u64],
    ) -> MemoryResult<()>;
    /// The `(handle, expected_version)` pair conditions one open session's
    /// state; it is not a provenance or completeness token. Version checks
    /// concurrent append or row eviction. Passing one page's sequences here can evict
    /// rows on other pages, because retention applies to the entire session.
    fn retain_sequences_if_version(
        &self,
        handle: &SessionHandle,
        expected_version: u64,
        retained_sequences: &[u64],
    ) -> MemoryResult<TranscriptRetainDispositionV1>;
    fn truncate_to(&self, handle: &SessionHandle, max_messages: usize) -> MemoryResult<()>;
    fn retire_session(&self, handle: &SessionHandle) -> MemoryResult<()>;
    /// Checked statistics: a poisoned backing cannot appear as an empty store.
    fn stats(&self) -> MemoryResult<StoreStats>;
    fn retention_stats(&self) -> MemoryResult<TranscriptRetentionStatsV2>;
    /// Terminal, idempotent release of all active rows and managed charges.
    fn close(&self) -> MemoryResult<()>;
}
