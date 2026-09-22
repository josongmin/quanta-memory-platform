pub mod service;

use std::fmt::{Display, Formatter};

use crate::shared::measure::ByteSized;
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
    PayloadConflict {
        sequence: u64,
    },
    SequenceConflict {
        sequence: u64,
    },
    MessageIdConflict,
<<<<<<< HEAD
    RetentionSaturated {
        evidence_count: usize,
        max_evidence_count: usize,
        evidence_bytes: u64,
        requested_evidence_bytes: u64,
        max_evidence_bytes: u64,
=======
    ExpiredIdentity {
        sequence: u64,
    },
    EvidenceSaturated {
        retained_count: usize,
        retained_bytes: u64,
        max_count: usize,
        max_bytes: u64,
>>>>>>> codex/qmp130-upstream-hardening
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
<<<<<<< HEAD
            Self::RetentionSaturated {
                evidence_count,
                max_evidence_count,
                evidence_bytes,
                requested_evidence_bytes,
                max_evidence_bytes,
            } => write!(
                f,
                "transcript identity retention saturated: count {evidence_count}/{max_evidence_count}, bytes {evidence_bytes}+{requested_evidence_bytes}/{max_evidence_bytes}"
=======
            Self::ExpiredIdentity { sequence } => {
                write!(f, "transcript identity evidence expired at sequence {sequence}")
            }
            Self::EvidenceSaturated {
                retained_count,
                retained_bytes,
                max_count,
                max_bytes,
            } => write!(
                f,
                "transcript identity evidence saturated: count {retained_count}/{max_count}, bytes {retained_bytes}/{max_bytes}"
>>>>>>> codex/qmp130-upstream-hardening
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

<<<<<<< HEAD
/// Bounded, fail-closed dedupe evidence policy for the in-memory backing.
///
/// Active message bodies may be truncated, but admitted sequence/message-id/body
/// evidence remains until `close`. Exact retries and conflicts are still checked
/// at saturation; only a new identity is rejected. This proves process-lifetime
/// idempotency inside the configured horizon, never restart durability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranscriptRetentionPolicyV2 {
    max_evidence_count: usize,
    max_evidence_bytes: u64,
}

impl TranscriptRetentionPolicyV2 {
    pub const DEFAULT_MAX_EVIDENCE_COUNT: usize = 65_536;
    pub const DEFAULT_MAX_EVIDENCE_BYTES: u64 = 64 * 1024 * 1024;

    #[must_use]
    pub const fn fail_closed(max_evidence_count: usize, max_evidence_bytes: u64) -> Self {
        Self { max_evidence_count, max_evidence_bytes }
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
=======
/// Bounded process-lifetime dedupe-evidence policy for the in-memory backing.
///
/// Active bodies may be truncated, but identity evidence remains until the
/// store closes. Once either limit would be exceeded, a new identity is
/// rejected without mutation. Existing exact retries and conflicts remain
/// observable at saturation. This policy makes no restart-durability claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranscriptRetentionPolicyV2 {
    max_tombstone_count: usize,
    max_tombstone_bytes: u64,
}

impl TranscriptRetentionPolicyV2 {
    pub fn fail_closed(max_tombstone_count: usize, max_tombstone_bytes: u64) -> MemoryResult<Self> {
        if max_tombstone_count == 0 {
            return Err(MemoryError::capacity(
                "transcript tombstone count limit must be greater than zero",
            ));
        }
        if max_tombstone_bytes == 0 {
            return Err(MemoryError::capacity(
                "transcript tombstone byte limit must be greater than zero",
            ));
        }
        Ok(Self { max_tombstone_count, max_tombstone_bytes })
    }

    #[must_use]
    pub fn max_tombstone_count(self) -> usize {
        self.max_tombstone_count
    }

    #[must_use]
    pub fn max_tombstone_bytes(self) -> u64 {
        self.max_tombstone_bytes
    }

    pub(crate) fn admits(self, next_count: usize, next_bytes: u64) -> bool {
        next_count <= self.max_tombstone_count && next_bytes <= self.max_tombstone_bytes
    }
}

/// Split observability for active payload and retained idempotency evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranscriptEvidenceStatsV1 {
    pub tombstone_count: usize,
    pub tombstone_bytes: u64,
    pub active_row_count: usize,
    pub active_row_bytes: u64,
}

impl Default for TranscriptRetentionPolicyV2 {
    fn default() -> Self {
        Self { max_tombstone_count: 65_536, max_tombstone_bytes: 64 * 1024 * 1024 }
>>>>>>> codex/qmp130-upstream-hardening
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
        }
    }
}

impl std::error::Error for TranscriptRetentionReconfigureErrorV2 {}

/// Transcript admission charges active rows, persistent idempotency evidence,
/// and the first retained session key. Both identity types must report their
/// semantic retained byte length; truncating active rows does not erase or
/// refund admitted identity evidence. `close` explicitly terminates the replay
/// horizon and releases every charge.
pub trait SessionTranscriptStore<SessionId, MessageId, Message>
where
    SessionId: ByteSized,
    MessageId: ByteSized,
{
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
    fn retention_stats(&self) -> MemoryResult<TranscriptRetentionStatsV2>;
    /// Terminal, idempotent release of all active rows and managed charges.
    fn close(&self) -> MemoryResult<()>;
}
