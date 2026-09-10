use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use quanta_memory_platform_core::{
    ChargeKind, MemoryContextKind, MemoryError, MemoryResult, ReservationTicket,
    StoreBindingHandle, StoreStats,
};

use crate::shared::accounting::SharedAccountingSink;
use crate::shared::measure::{ByteSized, CanonicalKey};
use crate::transcript_store::{
    SessionTranscriptStore, TranscriptAppendDispositionV1, TranscriptAppendErrorV1,
    TranscriptAppendRequestV1, TranscriptRecordV1, TranscriptRetainDispositionV1,
    TranscriptRetentionPolicyV1, TranscriptSnapshotV1,
};

const BODY_DIGEST_DOMAIN: &[u8] = b"qmp.transcript-body.v1\0";

#[derive(Debug)]
struct TranscriptEntry<MessageId, Message> {
    sequence: u64,
    message_id: MessageId,
    body: Message,
    charged_bytes: u64,
    ticket: Option<ReservationTicket>,
}

#[derive(Clone, Debug)]
struct SequenceEvidence<MessageId> {
    message_id: MessageId,
    body_digest: [u8; 32],
}

#[derive(Clone, Debug)]
struct MessageEvidence {
    sequence: u64,
    body_digest: [u8; 32],
}

#[derive(Debug)]
struct SessionState<MessageId, Message> {
    version: u64,
    rows: BTreeMap<u64, TranscriptEntry<MessageId, Message>>,
    sequence_evidence: BTreeMap<u64, SequenceEvidence<MessageId>>,
    message_evidence: BTreeMap<MessageId, MessageEvidence>,
}

impl<MessageId, Message> Default for SessionState<MessageId, Message> {
    fn default() -> Self {
        Self {
            version: 0,
            rows: BTreeMap::new(),
            sequence_evidence: BTreeMap::new(),
            message_evidence: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
struct TranscriptState<SessionId, MessageId, Message> {
    sessions: BTreeMap<SessionId, SessionState<MessageId, Message>>,
    retained_bytes: u64,
}

/// Portable process-lifetime transcript store with caller-owned append identity.
///
/// One mutex owns rows, both identity indexes, body evidence, retained-byte
/// accounting, and reservation receipts. Every append either commits all of
/// them or none. Truncation evicts active bodies and refunds their tickets while
/// retaining process-lifetime tombstones; this in-memory implementation makes no
/// restart-durability claim.
#[derive(Debug)]
pub struct SimpleSessionTranscriptStore<SessionId, MessageId, Message> {
    binding: StoreBindingHandle,
    charger: Option<SharedAccountingSink>,
    retention: TranscriptRetentionPolicyV1,
    state: Mutex<TranscriptState<SessionId, MessageId, Message>>,
}

impl<SessionId, MessageId, Message> SimpleSessionTranscriptStore<SessionId, MessageId, Message> {
    pub fn new(binding: StoreBindingHandle, retention: TranscriptRetentionPolicyV1) -> Self {
        debug_assert!(retention.retains_process_lifetime_tombstones());
        Self {
            binding,
            charger: None,
            retention,
            state: Mutex::new(TranscriptState { sessions: BTreeMap::new(), retained_bytes: 0 }),
        }
    }

    #[must_use]
    pub fn charged_against(mut self, sink: SharedAccountingSink) -> Self {
        self.charger = Some(sink);
        self
    }

    #[must_use]
    pub fn retention_policy(&self) -> TranscriptRetentionPolicyV1 {
        self.retention
    }

    fn lock(
        &self,
    ) -> MemoryResult<std::sync::MutexGuard<'_, TranscriptState<SessionId, MessageId, Message>>>
    {
        self.state
            .lock()
            .map_err(|_| MemoryError::InvalidState("transcript store lock poisoned".to_string()))
    }

    pub fn retained_bytes(&self) -> MemoryResult<u64> {
        Ok(self.lock()?.retained_bytes)
    }
}

impl<SessionId, MessageId, Message> SimpleSessionTranscriptStore<SessionId, MessageId, Message>
where
    SessionId: Clone + Ord,
{
    pub fn session_bytes(&self, session_id: &SessionId) -> MemoryResult<u64> {
        Ok(self
            .lock()?
            .sessions
            .get(session_id)
            .map_or(0, |session| session.rows.values().map(|entry| entry.charged_bytes).sum()))
    }

    pub fn checked_stats(&self) -> MemoryResult<StoreStats> {
        let state = self.lock()?;
        Ok(StoreStats {
            item_count: state.sessions.values().map(|session| session.rows.len()).sum(),
            estimated_bytes: state.retained_bytes,
        })
    }

    fn retain_sequences_in_state(
        &self,
        state: &mut TranscriptState<SessionId, MessageId, Message>,
        session_id: &SessionId,
        retained_sequences: &[u64],
    ) -> MemoryResult<()> {
        let retained: BTreeSet<_> = retained_sequences.iter().copied().collect();
        if retained.len() != retained_sequences.len() {
            return Err(MemoryError::InvalidState(
                "transcript retain selector contains duplicate sequences".to_string(),
            ));
        }

        let Some(session) = state.sessions.get(session_id) else {
            return if retained.is_empty() {
                Ok(())
            } else {
                Err(MemoryError::InvalidState(
                    "transcript retain selector references a missing session".to_string(),
                ))
            };
        };
        if let Some(sequence) =
            retained.iter().find(|sequence| !session.rows.contains_key(sequence))
        {
            return Err(MemoryError::InvalidState(format!(
                "transcript retain selector references inactive sequence {sequence}"
            )));
        }

        let drop_sequences: Vec<_> =
            session.rows.keys().copied().filter(|sequence| !retained.contains(sequence)).collect();
        if drop_sequences.is_empty() {
            return Ok(());
        }
        let next_version = session.version.checked_add(1).ok_or_else(|| {
            MemoryError::InvalidState("transcript session version overflow".to_string())
        })?;
        let mut dropped_bytes = 0u64;
        let mut dropped_tickets = Vec::new();
        for sequence in &drop_sequences {
            let entry = session.rows.get(sequence).ok_or_else(|| {
                MemoryError::InvalidState("transcript row disappeared during retain".to_string())
            })?;
            dropped_bytes = dropped_bytes.checked_add(entry.charged_bytes).ok_or_else(|| {
                MemoryError::InvalidState("transcript dropped-byte counter overflow".to_string())
            })?;
            if let Some(ticket) = &entry.ticket {
                dropped_tickets.push(ticket.clone());
            }
        }
        let next_retained_bytes =
            state.retained_bytes.checked_sub(dropped_bytes).ok_or_else(|| {
                MemoryError::InvalidState("transcript retained-byte underflow".to_string())
            })?;

        if !dropped_tickets.is_empty() {
            let sink = self.charger.as_ref().ok_or_else(|| {
                MemoryError::InvalidState(
                    "ticketed transcript row has no accounting sink".to_string(),
                )
            })?;
            sink.refund_many(&dropped_tickets)?;
        }

        let session = state.sessions.get_mut(session_id).ok_or_else(|| {
            MemoryError::InvalidState("transcript session disappeared during retain".to_string())
        })?;
        for sequence in drop_sequences {
            session.rows.remove(&sequence).ok_or_else(|| {
                MemoryError::InvalidState("transcript row disappeared during retain".to_string())
            })?;
        }
        session.version = next_version;
        // sequence_evidence and message_evidence intentionally survive as
        // process-lifetime tombstones. They are the late-retry safety owner.
        state.retained_bytes = next_retained_bytes;
        Ok(())
    }
}

impl<SessionId, MessageId, Message> SimpleSessionTranscriptStore<SessionId, MessageId, Message>
where
    SessionId: Clone + Ord,
    MessageId: Clone + Ord,
    Message: Clone + ByteSized + CanonicalKey,
{
    /// Copy active rows and their mutation version under the store mutex.
    /// Caller policy executes after this method releases the mutex.
    pub fn load_versioned_snapshot(
        &self,
        session_id: &SessionId,
    ) -> MemoryResult<TranscriptSnapshotV1<MessageId, Message>> {
        let state = self.lock()?;
        let version = state.sessions.get(session_id).map_or(0, |session| session.version);
        let records = state.sessions.get(session_id).map_or_else(Vec::new, |session| {
            session
                .rows
                .values()
                .map(|entry| TranscriptRecordV1 {
                    sequence: entry.sequence,
                    message_id: entry.message_id.clone(),
                    body: entry.body.clone(),
                })
                .collect()
        });
        Ok(TranscriptSnapshotV1 { version, records })
    }

    /// Retain rows only if no append or retention mutation occurred after the
    /// supplied snapshot. Stale snapshots are reported without mutation.
    pub fn retain_sequences_if_version(
        &self,
        session_id: &SessionId,
        expected_version: u64,
        retained_sequences: &[u64],
    ) -> MemoryResult<TranscriptRetainDispositionV1> {
        let mut state = self.lock()?;
        let actual_version = state.sessions.get(session_id).map_or(0, |session| session.version);
        if actual_version != expected_version {
            return Ok(TranscriptRetainDispositionV1::StaleSnapshot {
                expected_version,
                actual_version,
            });
        }
        self.retain_sequences_in_state(&mut state, session_id, retained_sequences)?;
        let version = state.sessions.get(session_id).map_or(0, |session| session.version);
        Ok(TranscriptRetainDispositionV1::Applied { version })
    }
}

impl<SessionId, MessageId, Message> SessionTranscriptStore<SessionId, MessageId, Message>
    for SimpleSessionTranscriptStore<SessionId, MessageId, Message>
where
    SessionId: Clone + Ord,
    MessageId: Clone + Ord,
    Message: Clone + ByteSized + CanonicalKey,
{
    fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    fn append(
        &self,
        request: TranscriptAppendRequestV1<SessionId, MessageId, Message>,
    ) -> Result<TranscriptAppendDispositionV1, TranscriptAppendErrorV1> {
        let body_key = request.body.canonical_key();
        let body_digest = canonical_body_digest(&body_key);
        let session_id = request.identity.session_id;
        let sequence = request.identity.sequence;
        let message_id = request.identity.message_id;
        let charged_bytes = request.body.byte_size();
        let mut state = self.lock().map_err(TranscriptAppendErrorV1::Storage)?;

        if let Some(session) = state.sessions.get(&session_id) {
            if let Some(evidence) = session.sequence_evidence.get(&sequence) {
                if evidence.message_id != message_id {
                    return Err(TranscriptAppendErrorV1::SequenceConflict { sequence });
                }
                let exact_replay = session.rows.get(&sequence).map_or_else(
                    || evidence.body_digest == body_digest,
                    |row| row.body.canonical_key() == body_key,
                );
                if exact_replay {
                    return Ok(TranscriptAppendDispositionV1::Replayed);
                }
                return Err(TranscriptAppendErrorV1::PayloadConflict { sequence });
            }
            if let Some(evidence) = session.message_evidence.get(&message_id) {
                if evidence.sequence != sequence {
                    return Err(TranscriptAppendErrorV1::MessageIdConflict);
                }
                if evidence.body_digest != body_digest {
                    return Err(TranscriptAppendErrorV1::PayloadConflict { sequence });
                }
                return Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                    "message-id index exists without sequence evidence".to_string(),
                )));
            }
        }

        let next_total = state.retained_bytes.checked_add(charged_bytes).ok_or_else(|| {
            TranscriptAppendErrorV1::Accounting(MemoryError::capacity(
                "transcript retained-byte counter overflow",
            ))
        })?;
        let next_version = state.sessions.get(&session_id).map_or(Ok(1), |session| {
            session.version.checked_add(1).ok_or_else(|| {
                TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                    "transcript session version overflow".to_string(),
                ))
            })
        })?;
        let ticket = self
            .charger
            .as_ref()
            .map(|sink| {
                sink.charge(
                    self.binding.class.clone(),
                    self.binding.owner.clone(),
                    MemoryContextKind::Session,
                    ChargeKind::RetainedPayload,
                    charged_bytes,
                )
            })
            .transpose()
            .map_err(TranscriptAppendErrorV1::Accounting)?;

        let session = state.sessions.entry(session_id).or_default();
        session.version = next_version;
        session.rows.insert(
            sequence,
            TranscriptEntry {
                sequence,
                message_id: message_id.clone(),
                body: request.body,
                charged_bytes,
                ticket,
            },
        );
        session
            .sequence_evidence
            .insert(sequence, SequenceEvidence { message_id: message_id.clone(), body_digest });
        session.message_evidence.insert(message_id, MessageEvidence { sequence, body_digest });
        state.retained_bytes = next_total;
        Ok(TranscriptAppendDispositionV1::Applied)
    }

    fn load(
        &self,
        session_id: &SessionId,
    ) -> MemoryResult<Vec<TranscriptRecordV1<MessageId, Message>>>
    where
        MessageId: Clone,
        Message: Clone,
    {
        let state = self.lock()?;
        Ok(state.sessions.get(session_id).map_or_else(Vec::new, |session| {
            session
                .rows
                .values()
                .map(|entry| TranscriptRecordV1 {
                    sequence: entry.sequence,
                    message_id: entry.message_id.clone(),
                    body: entry.body.clone(),
                })
                .collect()
        }))
    }

    fn retain_sequences(
        &self,
        session_id: &SessionId,
        retained_sequences: &[u64],
    ) -> MemoryResult<()> {
        let mut state = self.lock()?;
        self.retain_sequences_in_state(&mut state, session_id, retained_sequences)
    }

    fn truncate_to(&self, session_id: &SessionId, max_messages: usize) -> MemoryResult<()> {
        let mut state = self.lock()?;
        let retained_sequences = state.sessions.get(session_id).map_or_else(Vec::new, |session| {
            session.rows.keys().copied().take(max_messages).collect()
        });
        self.retain_sequences_in_state(&mut state, session_id, &retained_sequences)
    }

    fn stats(&self) -> StoreStats {
        self.checked_stats().unwrap_or_default()
    }
}

fn canonical_body_digest(key: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(BODY_DIGEST_DOMAIN);
    hasher.update(&(key.len() as u64).to_le_bytes());
    hasher.update(key);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript_store::{
        TranscriptAppendIdentityV1, TranscriptAppendRequestV1, TranscriptRetentionPolicyV1,
    };
    use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
    use std::sync::{Arc, Barrier};
    use std::thread;

    type Store = SimpleSessionTranscriptStore<String, String, String>;

    fn store() -> Store {
        Store::new(
            StoreBindingHandle::new(
                MemoryClass::SessionTranscript,
                "transcript-idempotency-tests".to_string(),
                None,
            ),
            TranscriptRetentionPolicyV1::process_lifetime(),
        )
    }

    fn request(
        session_id: &str,
        sequence: u64,
        message_id: &str,
        body: &str,
    ) -> TranscriptAppendRequestV1<String, String, String> {
        TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new(
                session_id.to_string(),
                sequence,
                message_id.to_string(),
            ),
            body.to_string(),
        )
    }

    #[test]
    fn exact_request_retried_n_times_applies_once() {
        let store = store();
        let outcomes: Vec<_> =
            (0..8).map(|_| store.append(request("s", 0, "m0", "body")).expect("append")).collect();
        assert_eq!(
            outcomes.iter().filter(|d| **d == TranscriptAppendDispositionV1::Applied).count(),
            1
        );
        assert_eq!(
            outcomes.iter().filter(|d| **d == TranscriptAppendDispositionV1::Replayed).count(),
            7
        );
        assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
        assert_eq!(store.retained_bytes().expect("bytes"), 4);
    }

    #[test]
    fn same_identity_with_body_drift_is_payload_conflict() {
        let store = store();
        store.append(request("s", 0, "m0", "old")).expect("seed");
        let err = store.append(request("s", 0, "m0", "new")).expect_err("conflict");
        assert!(matches!(err, TranscriptAppendErrorV1::PayloadConflict { sequence: 0 }));
        assert_eq!(store.load(&"s".to_string()).expect("load")[0].body(), "old");
    }

    #[test]
    fn reused_sequence_with_different_message_id_is_typed_conflict() {
        let store = store();
        store.append(request("s", 4, "m0", "body")).expect("seed");
        let err = store.append(request("s", 4, "m1", "body")).expect_err("conflict");
        assert!(matches!(err, TranscriptAppendErrorV1::SequenceConflict { sequence: 4 }));
    }

    #[test]
    fn reused_message_id_at_different_sequence_is_typed_conflict() {
        let store = store();
        store.append(request("s", 4, "m0", "body")).expect("seed");
        let err = store.append(request("s", 5, "m0", "body")).expect_err("conflict");
        assert!(matches!(err, TranscriptAppendErrorV1::MessageIdConflict));
    }

    #[test]
    fn append_identity_is_partitioned_by_session() {
        let store = store();
        assert_eq!(
            store.append(request("left", 4, "m0", "left-body")).expect("left"),
            TranscriptAppendDispositionV1::Applied
        );
        assert_eq!(
            store.append(request("right", 4, "m0", "right-body")).expect("right"),
            TranscriptAppendDispositionV1::Applied
        );
        assert_eq!(store.load(&"left".to_string()).expect("left load")[0].body(), "left-body");
        assert_eq!(store.load(&"right".to_string()).expect("right load")[0].body(), "right-body");
    }

    #[test]
    fn concurrent_identical_appends_have_one_applied_winner() {
        const N: usize = 8;
        let store = Arc::new(store());
        let barrier = Arc::new(Barrier::new(N));
        let handles: Vec<_> = (0..N)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    store.append(request("s", 0, "m0", "body")).expect("append")
                })
            })
            .collect();
        let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().expect("join")).collect();
        assert_eq!(
            outcomes.iter().filter(|d| **d == TranscriptAppendDispositionV1::Applied).count(),
            1
        );
        assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
        assert_eq!(store.retained_bytes().expect("bytes"), 4);
    }

    #[test]
    fn concurrent_conflicting_appends_have_one_applied_winner() {
        let store = Arc::new(store());
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = ["left", "right"]
            .into_iter()
            .map(|body| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    store.append(request("s", 0, "m0", body))
                })
            })
            .collect();
        let outcomes: Vec<_> =
            handles.into_iter().map(|handle| handle.join().expect("join")).collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(TranscriptAppendDispositionV1::Applied)))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(
                    outcome,
                    Err(TranscriptAppendErrorV1::PayloadConflict { sequence: 0 })
                ))
                .count(),
            1
        );
        assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
    }

    #[test]
    fn truncate_retains_tombstone_so_late_retry_cannot_reappend() {
        let store = store();
        store.append(request("s", 0, "m0", "old")).expect("first");
        store.append(request("s", 1, "m1", "drop")).expect("second");
        store.truncate_to(&"s".to_string(), 1).expect("truncate");
        assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
        assert_eq!(
            store.append(request("s", 1, "m1", "drop")).expect("late retry"),
            TranscriptAppendDispositionV1::Replayed
        );
        assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
        assert!(matches!(
            store.append(request("s", 1, "m1", "drift")),
            Err(TranscriptAppendErrorV1::PayloadConflict { sequence: 1 })
        ));
    }

    #[test]
    fn truncate_to_zero_retains_all_identity_tombstones() {
        let store = store();
        store.append(request("s", 0, "m0", "zero")).expect("zero");
        store.append(request("s", 2, "m2", "two")).expect("two");
        store.truncate_to(&"s".to_string(), 0).expect("truncate all");

        assert!(store.load(&"s".to_string()).expect("load").is_empty());
        assert_eq!(store.retained_bytes().expect("bytes"), 0);
        assert_eq!(
            store.append(request("s", 0, "m0", "zero")).expect("zero replay"),
            TranscriptAppendDispositionV1::Replayed
        );
        assert_eq!(
            store.append(request("s", 2, "m2", "two")).expect("two replay"),
            TranscriptAppendDispositionV1::Replayed
        );
        assert!(store.load(&"s".to_string()).expect("load after retries").is_empty());
    }

    #[test]
    fn selective_retain_preserves_sparse_rows_and_evicted_identity_tombstones() {
        let store = store();
        store.append(request("s", 0, "m0", "zero")).expect("zero");
        store.append(request("s", 1, "m1", "one")).expect("one");
        store.append(request("s", 2, "m2", "two")).expect("two");

        store.retain_sequences(&"s".to_string(), &[0, 2]).expect("retain sparse set");

        let rows = store.load(&"s".to_string()).expect("load");
        assert_eq!(rows.iter().map(TranscriptRecordV1::sequence).collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(store.retained_bytes().expect("bytes"), 7);
        assert_eq!(
            store.append(request("s", 1, "m1", "one")).expect("evicted replay"),
            TranscriptAppendDispositionV1::Replayed
        );
        assert_eq!(store.load(&"s".to_string()).expect("load after replay").len(), 2);
    }

    #[test]
    fn selective_retain_rejects_invalid_selector_without_mutation() {
        let store = store();
        store.append(request("s", 0, "m0", "zero")).expect("zero");
        store.append(request("s", 1, "m1", "one")).expect("one");
        let before = store.retained_bytes().expect("before");

        assert!(matches!(
            store.retain_sequences(&"s".to_string(), &[0, 0]),
            Err(MemoryError::InvalidState(_))
        ));
        assert!(matches!(
            store.retain_sequences(&"s".to_string(), &[0, 9]),
            Err(MemoryError::InvalidState(_))
        ));
        assert_eq!(store.load(&"s".to_string()).expect("load").len(), 2);
        assert_eq!(store.retained_bytes().expect("after"), before);
    }

    #[test]
    fn versioned_snapshot_retain_rejects_concurrent_append_without_dropping_rows() {
        let store = store();
        store.append(request("s", 0, "m0", "zero")).expect("zero");
        store.append(request("s", 1, "m1", "one")).expect("one");
        let stale = store.load_versioned_snapshot(&"s".to_string()).expect("snapshot");
        assert_eq!(stale.version(), 2);
        store.append(request("s", 2, "m2", "two")).expect("concurrent append");
        assert_eq!(
            store
                .retain_sequences_if_version(&"s".to_string(), stale.version(), &[0])
                .expect("conditional retain"),
            TranscriptRetainDispositionV1::StaleSnapshot { expected_version: 2, actual_version: 3 }
        );
        assert_eq!(
            store
                .load(&"s".to_string())
                .expect("load")
                .iter()
                .map(TranscriptRecordV1::sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );

        let current = store.load_versioned_snapshot(&"s".to_string()).expect("current snapshot");
        assert_eq!(
            store
                .retain_sequences_if_version(&"s".to_string(), current.version(), &[0, 2])
                .expect("current retain"),
            TranscriptRetainDispositionV1::Applied { version: 4 }
        );
        assert_eq!(
            store
                .load(&"s".to_string())
                .expect("load after retain")
                .iter()
                .map(TranscriptRecordV1::sequence)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn checked_load_reports_poisoned_storage_instead_of_empty_success() {
        let store = Arc::new(store());
        let poisoner = Arc::clone(&store);
        let _ = thread::spawn(move || {
            let _guard = poisoner.state.lock().expect("lock");
            panic!("deterministic poison");
        })
        .join();
        assert!(matches!(store.load(&"s".to_string()), Err(MemoryError::InvalidState(_))));
    }

    #[test]
    fn ordered_load_sorts_by_caller_sequence_and_allows_declared_gaps() {
        let store = store();
        store.append(request("s", 7, "m7", "seven")).expect("seven");
        store.append(request("s", 2, "m2", "two")).expect("two");
        let rows = store.load(&"s".to_string()).expect("load");
        assert_eq!(rows.iter().map(TranscriptRecordV1::sequence).collect::<Vec<_>>(), vec![2, 7]);
    }

    #[cfg(feature = "core-live-accounting")]
    mod accounting {
        use super::*;
        use crate::SharedAccountingSink;
        use quanta_memory_platform_core::{
            EffectiveMemoryBudgetReport, ExecutionBudget, LiveProcessMemoryBroker,
            MemoryBudgetManifest, MemoryClassBudget, MemoryPreset, ReclaimPriority,
            ResidencyPolicy,
        };

        fn broker(class_hard: u64) -> LiveProcessMemoryBroker {
            let residency = ResidencyPolicy::new(1_000_000).with_class_budget(
                MemoryClass::SessionTranscript,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: class_hard,
                    hard_cap_bytes: class_hard,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            );
            let report = EffectiveMemoryBudgetReport::from_manifest(
                MemoryBudgetManifest::new(
                    MemoryPreset::Production,
                    ExecutionBudget::default(),
                    residency,
                ),
                vec!["transcript-idempotency-test".to_string()],
            );
            LiveProcessMemoryBroker::from_report(report).expect("valid broker")
        }

        fn charged(broker: &LiveProcessMemoryBroker) -> Store {
            let sink: SharedAccountingSink = Arc::new(broker.clone());
            store().charged_against(sink)
        }

        #[test]
        fn quota_denial_leaves_row_indexes_and_charge_unchanged() {
            let broker = broker(3);
            let store = charged(&broker);
            let denied = store.append(request("s", 0, "m0", "four"));
            assert!(matches!(
                denied,
                Err(TranscriptAppendErrorV1::Accounting(MemoryError::Capacity(_)))
            ));
            assert!(store.load(&"s".to_string()).expect("load").is_empty());
            assert_eq!(store.retained_bytes().expect("bytes"), 0);
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 0);
            assert_eq!(
                store.append(request("s", 0, "m0", "ok")).expect("no index residue"),
                TranscriptAppendDispositionV1::Applied
            );
        }

        #[test]
        fn every_conflict_axis_preserves_rows_indexes_and_accounting() {
            let broker = broker(1_000_000);
            let store = charged(&broker);
            store.append(request("s", 4, "m0", "body")).expect("seed");

            let conflicts = [
                store.append(request("s", 4, "m0", "drift")),
                store.append(request("s", 4, "m1", "body")),
                store.append(request("s", 5, "m0", "body")),
            ];
            assert!(matches!(
                conflicts[0],
                Err(TranscriptAppendErrorV1::PayloadConflict { sequence: 4 })
            ));
            assert!(matches!(
                conflicts[1],
                Err(TranscriptAppendErrorV1::SequenceConflict { sequence: 4 })
            ));
            assert!(matches!(conflicts[2], Err(TranscriptAppendErrorV1::MessageIdConflict)));
            assert_eq!(store.load(&"s".to_string()).expect("load").len(), 1);
            assert_eq!(store.retained_bytes().expect("bytes"), 4);
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 4);
        }

        #[test]
        fn exact_retry_charges_once_and_truncate_refunds_only_active_body() {
            let broker = broker(1_000_000);
            let store = charged(&broker);
            store.append(request("s", 0, "m0", "one")).expect("first");
            store.append(request("s", 0, "m0", "one")).expect("replay");
            store.append(request("s", 1, "m1", "four")).expect("second");
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 7);
            store.truncate_to(&"s".to_string(), 1).expect("truncate");
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 3);
            assert_eq!(
                store.append(request("s", 1, "m1", "four")).expect("late replay"),
                TranscriptAppendDispositionV1::Replayed
            );
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 3);
        }

        #[test]
        fn truncate_refund_failure_preserves_rows_indexes_and_accounting() {
            let broker = broker(1_000_000);
            let store = charged(&broker);
            store.append(request("s", 0, "m0", "one")).expect("first");
            store.append(request("s", 1, "m1", "four")).expect("second");
            broker.begin_rebuild().expect("begin rebuild");
            let denied = store.truncate_to(&"s".to_string(), 1);
            assert!(matches!(denied, Err(MemoryError::InvalidState(_))));
            assert_eq!(store.load(&"s".to_string()).expect("load").len(), 2);
            assert_eq!(store.retained_bytes().expect("bytes"), 7);
            assert_eq!(broker.ledger_snapshot().expect("snapshot").total_owner_bytes(), 7);
            broker.finish_rebuild().expect("finish rebuild");
        }
    }
}
