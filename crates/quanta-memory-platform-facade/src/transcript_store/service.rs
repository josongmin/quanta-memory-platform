use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::sync::{Mutex, MutexGuard, PoisonError};

use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, ManagedCharge, MemoryContextKind, MemoryError,
    MemoryResult, StoreBindingHandle, StoreStats,
};

#[cfg(feature = "unbudgeted-stores")]
use crate::shared::accounting::Unbudgeted;
use crate::shared::accounting::{Budgeted, StoreAccounting};
use crate::shared::lifecycle::{retire_client_value, BeginClose, DropTransition, StoreLifecycle};
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
    charge: Option<ManagedCharge>,
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
/// accounting, and non-clone charge leases. Every append either commits all of
/// them or none. Truncation evicts active bodies and refunds their charges while
/// retaining process-lifetime tombstones; this in-memory implementation makes no
/// restart-durability claim.
///
/// The accounting mode is fixed before construction and has no default. Raw
/// construction and late attachment cannot bypass the platform builder:
///
/// ```compile_fail
/// # use quanta_memory_platform_facade::SimpleSessionTranscriptStore;
/// let _: SimpleSessionTranscriptStore<String, String, String> = todo!();
/// ```
///
/// ```compile_fail
/// # use quanta_memory_platform_facade::{Budgeted, SimpleSessionTranscriptStore, TranscriptRetentionPolicyV1};
/// # use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
/// let binding = StoreBindingHandle::new(MemoryClass::SessionTranscript, "owner".into(), None);
/// let _ = SimpleSessionTranscriptStore::<String, String, String, Budgeted>::new(
///     binding, TranscriptRetentionPolicyV1::process_lifetime()
/// );
/// ```
///
/// ```compile_fail
/// # use quanta_memory_platform_facade::{Budgeted, SimpleSessionTranscriptStore};
/// # fn cannot_rebind(store: SimpleSessionTranscriptStore<String, String, String, Budgeted>) {
/// let _ = store.charged_against;
/// # }
/// ```
#[derive(Debug)]
pub struct SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode> {
    binding: StoreBindingHandle,
    accounting: StoreAccounting,
    lifecycle: StoreLifecycle,
    retention: TranscriptRetentionPolicyV1,
    state: Mutex<TranscriptState<SessionId, MessageId, Message>>,
    _mode: PhantomData<fn() -> Mode>,
}

impl<SessionId, MessageId, Message>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, Budgeted>
{
    pub(crate) fn new_budgeted(
        binding: StoreBindingHandle,
        retention: TranscriptRetentionPolicyV1,
        broker: LiveProcessMemoryBroker,
    ) -> Self {
        Self::with_accounting(binding, retention, StoreAccounting::Budgeted(broker))
    }
}

#[cfg(feature = "unbudgeted-stores")]
impl<SessionId, MessageId, Message>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, Unbudgeted>
{
    pub(crate) fn new_unbudgeted(
        binding: StoreBindingHandle,
        retention: TranscriptRetentionPolicyV1,
    ) -> Self {
        Self::with_accounting(binding, retention, StoreAccounting::Unbudgeted)
    }
}

impl<SessionId, MessageId, Message, Mode>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode>
{
    fn with_accounting(
        binding: StoreBindingHandle,
        retention: TranscriptRetentionPolicyV1,
        accounting: StoreAccounting,
    ) -> Self {
        debug_assert!(retention.retains_process_lifetime_tombstones());
        Self {
            binding,
            accounting,
            lifecycle: StoreLifecycle::new("transcript store"),
            retention,
            state: Mutex::new(TranscriptState { sessions: BTreeMap::new(), retained_bytes: 0 }),
            _mode: PhantomData,
        }
    }

    #[must_use]
    pub fn retention_policy(&self) -> TranscriptRetentionPolicyV1 {
        self.retention
    }

    fn lock_state(
        &self,
    ) -> MemoryResult<MutexGuard<'_, TranscriptState<SessionId, MessageId, Message>>> {
        self.lifecycle.try_lock_state(&self.state)
    }

    /// Checked reads cannot observe the temporary preimage extracted by a
    /// two-phase accounting mutation.
    fn lock_read(
        &self,
    ) -> MemoryResult<MutexGuard<'_, TranscriptState<SessionId, MessageId, Message>>> {
        self.lifecycle.ensure_readable()?;
        let state = self.lock_state()?;
        self.lifecycle.ensure_readable()?;
        Ok(state)
    }

    pub fn retained_bytes(&self) -> MemoryResult<u64> {
        Ok(self.lock_read()?.retained_bytes)
    }

    fn require_charge_shape(
        &self,
        entry: &TranscriptEntry<MessageId, Message>,
        operation: &str,
    ) -> MemoryResult<()> {
        match (self.accounting.broker().is_some(), entry.charge.is_some()) {
            (true, false) => Err(MemoryError::InvalidState(format!(
                "budgeted transcript row has no managed charge during {operation}"
            ))),
            (false, true) => Err(MemoryError::InvalidState(format!(
                "unbudgeted transcript row owns a managed charge during {operation}"
            ))),
            _ => Ok(()),
        }
    }

    /// Release every active row and enter the terminal closed state. Tombstones
    /// are discarded only after every ledger release succeeds.
    pub fn close(&self) -> MemoryResult<()> {
        let close = match self.lifecycle.begin_close()? {
            BeginClose::AlreadyClosed => return Ok(()),
            BeginClose::Acquired(close) => close,
        };

        let mut sessions = {
            let mut state = self.lock_state()?;
            for session in state.sessions.values() {
                for entry in session.rows.values() {
                    self.require_charge_shape(entry, "close")?;
                }
            }
            std::mem::take(&mut state.sessions)
        };
        let (slots, mut charges) = take_session_charges(&mut sessions);
        if let Err(error) = self.accounting.release_many(&mut charges) {
            restore_session_charges(&mut sessions, &slots, charges);
            self.state.lock().unwrap_or_else(PoisonError::into_inner).sessions = sessions;
            return Err(error);
        }

        self.state.lock().unwrap_or_else(PoisonError::into_inner).retained_bytes = 0;
        close.commit();
        // User message destructors run after both store and ledger reach the
        // terminal postimage.
        retire_sessions(sessions);
        Ok(())
    }
}

#[cfg(test)]
#[derive(Debug)]
struct TestUnbudgeted;

#[cfg(test)]
impl<SessionId, MessageId, Message>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, TestUnbudgeted>
{
    fn new_test_unbudgeted(
        binding: StoreBindingHandle,
        retention: TranscriptRetentionPolicyV1,
    ) -> Self {
        Self::with_accounting(binding, retention, StoreAccounting::Unbudgeted)
    }
}

impl<SessionId, MessageId, Message, Mode>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode>
where
    SessionId: Clone + Ord,
{
    pub fn session_bytes(&self, session_id: &SessionId) -> MemoryResult<u64> {
        self.lock_read()?.sessions.get(session_id).map_or(Ok(0), |session| {
            session.rows.values().try_fold(0u64, |total, entry| {
                total.checked_add(entry.charged_bytes).ok_or_else(|| {
                    MemoryError::capacity("transcript session byte counter overflow")
                })
            })
        })
    }

    pub fn checked_stats(&self) -> MemoryResult<StoreStats> {
        let state = self.lock_read()?;
        let item_count = state.sessions.values().try_fold(0usize, |total, session| {
            total
                .checked_add(session.rows.len())
                .ok_or_else(|| MemoryError::capacity("transcript item counter overflow"))
        })?;
        Ok(StoreStats { item_count, estimated_bytes: state.retained_bytes })
    }

    fn retain_sequences_owned(
        &self,
        session_id: &SessionId,
        retained_sequences: &[u64],
    ) -> MemoryResult<()> {
        let retained: BTreeSet<_> = retained_sequences.iter().copied().collect();
        if retained.len() != retained_sequences.len() {
            return Err(MemoryError::InvalidState(
                "transcript retain selector contains duplicate sequences".to_string(),
            ));
        }

        let (mut dropped, session_ordinal, next_version, next_retained_bytes) =
            {
                let mut state = self.lock_state()?;
                // Find the canonical session before detaching any row or charge.
                // Later recovery/publication must not compare a client-supplied
                // SessionId: Ord may panic and strand detached accounting authority.
                let Some(session_ordinal) =
                    state.sessions.keys().position(|candidate| candidate.cmp(session_id).is_eq())
                else {
                    return if retained.is_empty() {
                        Ok(())
                    } else {
                        Err(MemoryError::InvalidState(
                            "transcript retain selector references a missing session".to_string(),
                        ))
                    };
                };
                let session =
                    state.sessions.values().nth(session_ordinal).expect(
                        "validated transcript session ordinal is present under state mutex",
                    );
                if let Some(sequence) =
                    retained.iter().find(|sequence| !session.rows.contains_key(sequence))
                {
                    return Err(MemoryError::InvalidState(format!(
                        "transcript retain selector references inactive sequence {sequence}"
                    )));
                }

                let drop_sequences: Vec<_> = session
                    .rows
                    .keys()
                    .copied()
                    .filter(|sequence| !retained.contains(sequence))
                    .collect();
                if drop_sequences.is_empty() {
                    return Ok(());
                }
                let next_version = session
                    .version
                    .checked_add(1)
                    .ok_or_else(|| MemoryError::capacity("transcript session version overflow"))?;
                let mut dropped_bytes = 0u64;
                for sequence in &drop_sequences {
                    let entry = session.rows.get(sequence).ok_or_else(|| {
                        MemoryError::InvalidState(
                            "transcript row disappeared during retain planning".to_string(),
                        )
                    })?;
                    self.require_charge_shape(entry, "retain")?;
                    dropped_bytes =
                        dropped_bytes.checked_add(entry.charged_bytes).ok_or_else(|| {
                            MemoryError::capacity("transcript dropped-byte counter overflow")
                        })?;
                }
                let next_retained_bytes =
                    state.retained_bytes.checked_sub(dropped_bytes).ok_or_else(|| {
                        MemoryError::InvalidState("transcript retained-byte underflow".to_string())
                    })?;
                let session = state.sessions.values_mut().nth(session_ordinal).expect(
                    "validated transcript session ordinal remains present under state mutex",
                );
                let dropped = drop_sequences
                    .into_iter()
                    .map(|sequence| {
                        let entry = session
                            .rows
                            .remove(&sequence)
                            .expect("validated transcript row remains present under state mutex");
                        (sequence, entry)
                    })
                    .collect::<Vec<_>>();
                (dropped, session_ordinal, next_version, next_retained_bytes)
            };

        let (charge_slots, mut charges) = take_entry_charges(&mut dropped);
        if let Err(error) = self.accounting.release_many(&mut charges) {
            restore_entry_charges(&mut dropped, &charge_slots, charges);
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let session = state.sessions.values_mut().nth(session_ordinal).expect(
                "mutation lane preserves transcript session while retain accounting is in flight",
            );
            session.rows.extend(dropped);
            return Err(error);
        }

        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let session = state.sessions.values_mut().nth(session_ordinal).expect(
            "mutation lane preserves transcript session while retain accounting is in flight",
        );
        session.version = next_version;
        // sequence_evidence and message_evidence intentionally survive as
        // process-lifetime tombstones. They are the late-retry safety owner.
        state.retained_bytes = next_retained_bytes;
        drop(state);
        retire_rows(dropped.into_iter().map(|(_, row)| row));
        Ok(())
    }
}

impl<SessionId, MessageId, Message, Mode>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode>
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
        let state = self.lock_read()?;
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
        let _mutation = self.lifecycle.begin_mutation()?;
        let actual_version =
            self.lock_state()?.sessions.get(session_id).map_or(0, |session| session.version);
        if actual_version != expected_version {
            return Ok(TranscriptRetainDispositionV1::StaleSnapshot {
                expected_version,
                actual_version,
            });
        }
        self.retain_sequences_owned(session_id, retained_sequences)?;
        let version =
            self.lock_state()?.sessions.get(session_id).map_or(0, |session| session.version);
        Ok(TranscriptRetainDispositionV1::Applied { version })
    }
}

impl<SessionId, MessageId, Message, Mode> SessionTranscriptStore<SessionId, MessageId, Message>
    for SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode>
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
        let _mutation =
            self.lifecycle.begin_mutation().map_err(TranscriptAppendErrorV1::Storage)?;
        let body_key = request.body.canonical_key();
        let body_digest = canonical_body_digest(&body_key);
        let session_id = request.identity.session_id;
        let sequence = request.identity.sequence;
        let message_id = request.identity.message_id;
        let charged_bytes = request.body.byte_size();
        let (next_total, next_version) = {
            let state = self.lock_state().map_err(TranscriptAppendErrorV1::Storage)?;

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
                    TranscriptAppendErrorV1::Storage(MemoryError::capacity(
                        "transcript session version overflow",
                    ))
                })
            })?;
            (next_total, next_version)
        };

        // Client Clone may panic or reenter. Complete both copies before
        // acquiring a charge or mutating any store-owned state.
        let row_message_id = message_id.clone();
        let evidence_message_id = message_id.clone();

        let charge = self
            .accounting
            .charge(
                self.binding.class.clone(),
                self.binding.owner.clone(),
                MemoryContextKind::Session,
                ChargeKind::RetainedPayload,
                charged_bytes,
            )
            .map_err(TranscriptAppendErrorV1::Accounting)?;

        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(error) => {
                drop(charge);
                return Err(TranscriptAppendErrorV1::Storage(error));
            }
        };
        let mut row = Some(TranscriptEntry {
            sequence,
            message_id: row_message_id,
            body: request.body,
            charged_bytes,
            charge,
        });
        let mut sequence_evidence =
            Some(SequenceEvidence { message_id: evidence_message_id, body_digest });

        // Both BTreeMap::entry lookups may invoke client Ord. Resolve their
        // insertion positions before publishing a row, evidence, version, or
        // retained bytes. VacantEntry::insert and the u64-keyed maps need no
        // client callbacks. Catch an Ord panic while the guard is held, then
        // resume it only after releasing the guard so the state stays readable.
        let publication = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match state.sessions.entry(session_id) {
                Entry::Occupied(mut occupied) => {
                    let session = occupied.get_mut();
                    if session.rows.contains_key(&sequence)
                        || session.sequence_evidence.contains_key(&sequence)
                    {
                        return Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                            "transcript sequence index changed during append publication"
                                .to_string(),
                        )));
                    }
                    let Entry::Vacant(message_slot) = session.message_evidence.entry(message_id)
                    else {
                        return Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                            "transcript message-id index changed during append publication"
                                .to_string(),
                        )));
                    };
                    session.rows.insert(sequence, row.take().expect("planned row"));
                    session.sequence_evidence.insert(
                        sequence,
                        sequence_evidence.take().expect("planned sequence evidence"),
                    );
                    message_slot.insert(MessageEvidence { sequence, body_digest });
                    session.version = next_version;
                }
                Entry::Vacant(session_slot) => {
                    // A new session is assembled off-map; only the final
                    // VacantEntry insertion makes it visible.
                    let mut session = SessionState::default();
                    session.rows.insert(sequence, row.take().expect("planned row"));
                    session.sequence_evidence.insert(
                        sequence,
                        sequence_evidence.take().expect("planned sequence evidence"),
                    );
                    session
                        .message_evidence
                        .insert(message_id, MessageEvidence { sequence, body_digest });
                    session.version = next_version;
                    session_slot.insert(session);
                }
            }
            state.retained_bytes = next_total;
            Ok(TranscriptAppendDispositionV1::Applied)
        }));
        drop(state);
        // On any prepublication error, release the lease and retire client
        // values only after the store mutex is gone. Keep this cleanup outside
        // the catch boundary so a client Drop panic cannot unwind through it.
        if let Some(unpublished) = row.take() {
            retire_rows([unpublished]);
        }
        if let Some(unpublished) = sequence_evidence.take() {
            retire_client_value(unpublished.message_id);
        }
        match publication {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn load(
        &self,
        session_id: &SessionId,
    ) -> MemoryResult<Vec<TranscriptRecordV1<MessageId, Message>>>
    where
        MessageId: Clone,
        Message: Clone,
    {
        let state = self.lock_read()?;
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
        let _mutation = self.lifecycle.begin_mutation()?;
        self.retain_sequences_owned(session_id, retained_sequences)
    }

    fn truncate_to(&self, session_id: &SessionId, max_messages: usize) -> MemoryResult<()> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let state = self.lock_state()?;
        let retained_sequences = state.sessions.get(session_id).map_or_else(Vec::new, |session| {
            session.rows.keys().copied().take(max_messages).collect()
        });
        drop(state);
        self.retain_sequences_owned(session_id, &retained_sequences)
    }

    fn stats(&self) -> MemoryResult<StoreStats> {
        self.checked_stats()
    }

    fn close(&self) -> MemoryResult<()> {
        SimpleSessionTranscriptStore::close(self)
    }
}

/// Move non-clone lease authority out of rows while the mutation lane keeps
/// every row position stable. The returned indices are used to restore the
/// exact preimage if the broker atomically denies the release batch.
fn take_entry_charges<MessageId, Message>(
    entries: &mut [(u64, TranscriptEntry<MessageId, Message>)],
) -> (Vec<usize>, Vec<ManagedCharge>) {
    let mut slots = Vec::new();
    let mut charges = Vec::new();
    for (index, (_, entry)) in entries.iter_mut().enumerate() {
        if let Some(charge) = entry.charge.take() {
            slots.push(index);
            charges.push(charge);
        }
    }
    (slots, charges)
}

fn restore_entry_charges<MessageId, Message>(
    entries: &mut [(u64, TranscriptEntry<MessageId, Message>)],
    slots: &[usize],
    charges: Vec<ManagedCharge>,
) {
    for (slot, charge) in slots.iter().copied().zip(charges) {
        entries[slot].1.charge = Some(charge);
    }
}

fn take_session_charges<SessionId, MessageId, Message>(
    sessions: &mut BTreeMap<SessionId, SessionState<MessageId, Message>>,
) -> (Vec<(usize, usize)>, Vec<ManagedCharge>) {
    let mut slots = Vec::new();
    let mut charges = Vec::new();
    for (session_index, session) in sessions.values_mut().enumerate() {
        for (row_index, row) in session.rows.values_mut().enumerate() {
            if let Some(charge) = row.charge.take() {
                slots.push((session_index, row_index));
                charges.push(charge);
            }
        }
    }
    (slots, charges)
}

fn restore_session_charges<SessionId, MessageId, Message>(
    sessions: &mut BTreeMap<SessionId, SessionState<MessageId, Message>>,
    slots: &[(usize, usize)],
    charges: Vec<ManagedCharge>,
) {
    let mut slots_and_charges = slots.iter().copied().zip(charges).peekable();
    for (session_index, session) in sessions.values_mut().enumerate() {
        for (row_index, row) in session.rows.values_mut().enumerate() {
            if slots_and_charges
                .peek()
                .is_some_and(|((s, r), _)| (*s, *r) == (session_index, row_index))
            {
                row.charge = Some(slots_and_charges.next().expect("peeked charge").1);
            }
        }
    }
    debug_assert!(slots_and_charges.next().is_none());
}

/// Release every charge before invoking any client-owned destructor. Retire
/// individual generic fields so a panicking Drop cannot abort a concurrent
/// unwind or prevent later rows' cleanup.
fn retire_rows<MessageId, Message>(
    rows: impl IntoIterator<Item = TranscriptEntry<MessageId, Message>>,
) {
    for row in rows {
        let TranscriptEntry { message_id, body, charge, .. } = row;
        drop(charge);
        retire_client_value(message_id);
        retire_client_value(body);
    }
}

fn retire_sessions<SessionId, MessageId, Message>(
    mut sessions: BTreeMap<SessionId, SessionState<MessageId, Message>>,
) {
    for session in sessions.values_mut() {
        for row in session.rows.values_mut() {
            drop(row.charge.take());
        }
    }
    for (session_id, session) in sessions {
        retire_rows(session.rows.into_values());
        for (_, evidence) in session.sequence_evidence {
            retire_client_value(evidence.message_id);
        }
        for (message_id, _) in session.message_evidence {
            retire_client_value(message_id);
        }
        retire_client_value(session_id);
    }
}

impl<SessionId, MessageId, Message, Mode> Drop
    for SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode>
{
    fn drop(&mut self) {
        if self.lifecycle.transition_to_closed_for_drop() == DropTransition::AlreadyClosed {
            return;
        }
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        let sessions = std::mem::take(&mut state.sessions);
        state.retained_bytes = 0;
        // ManagedCharge drops perform exact release, or defer through the
        // issuing broker while its rebuild fence is active.
        retire_sessions(sessions);
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

    type Store = SimpleSessionTranscriptStore<String, String, String, TestUnbudgeted>;

    #[test]
    fn reentrant_load_stats_and_close_under_read_lock_are_typed_busy() {
        let store = store();
        store.append(request("s", 0, "m", "body")).expect("seed row");
        let held = store.state.lock().expect("simulated client clone holds read lock");
        assert!(matches!(store.load(&"s".into()), Err(MemoryError::OperationInProgress(_))));
        assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
        assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
        drop(held);
        assert_eq!(store.load(&"s".into()).expect("complete row").len(), 1);
        store.close().expect("retry close");
    }

    #[test]
    fn client_message_clone_during_load_reenters_as_typed_busy_without_ledger_drift() {
        use quanta_memory_platform_core::{MemoryPreset, ProcessMemoryBrokerBuilder};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::{OnceLock, Weak};

        type ReentrantStore = SimpleSessionTranscriptStore<String, String, ReentrantBody, Budgeted>;

        struct ReentrantBody {
            bytes: Vec<u8>,
            armed: Arc<AtomicBool>,
            busy_callbacks: Arc<AtomicUsize>,
            owner: Arc<OnceLock<Weak<ReentrantStore>>>,
        }

        impl Clone for ReentrantBody {
            fn clone(&self) -> Self {
                if self.armed.load(Ordering::SeqCst) {
                    let store = self.owner.get().expect("owner installed").upgrade().expect("live");
                    assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
                    assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
                    self.busy_callbacks.fetch_add(1, Ordering::SeqCst);
                }
                Self {
                    bytes: self.bytes.clone(),
                    armed: Arc::clone(&self.armed),
                    busy_callbacks: Arc::clone(&self.busy_callbacks),
                    owner: Arc::clone(&self.owner),
                }
            }
        }

        impl ByteSized for ReentrantBody {
            fn byte_size(&self) -> u64 {
                self.bytes.len() as u64
            }
        }

        impl CanonicalKey for ReentrantBody {
            fn canonical_key(&self) -> Vec<u8> {
                self.bytes.clone()
            }
        }

        let broker = ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("broker");
        let store = Arc::new(ReentrantStore::new_budgeted(
            StoreBindingHandle::new(MemoryClass::SessionTranscript, "clone-owner".into(), None),
            TranscriptRetentionPolicyV1::process_lifetime(),
            broker.clone(),
        ));
        let armed = Arc::new(AtomicBool::new(false));
        let busy_callbacks = Arc::new(AtomicUsize::new(0));
        let owner = Arc::new(OnceLock::new());
        owner.set(Arc::downgrade(&store)).expect("one owner");
        store
            .append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new("s".into(), 0, "m".into()),
                ReentrantBody {
                    bytes: vec![1, 2, 3],
                    armed: Arc::clone(&armed),
                    busy_callbacks: Arc::clone(&busy_callbacks),
                    owner,
                },
            ))
            .expect("append");
        armed.store(true, Ordering::SeqCst);
        assert_eq!(store.load(&"s".into()).expect("outer load").len(), 1);
        armed.store(false, Ordering::SeqCst);
        assert_eq!(busy_callbacks.load(Ordering::SeqCst), 1);
        assert_eq!(store.stats().expect("postimage").estimated_bytes, 3);
        assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 3);
        store.close().expect("close after callback");
        assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
    }

    fn store() -> Store {
        Store::new_test_unbudgeted(
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
                    loop {
                        match store.append(request("s", 0, "m0", "body")) {
                            Ok(disposition) => break disposition,
                            Err(TranscriptAppendErrorV1::Storage(
                                MemoryError::OperationInProgress(_),
                            )) => thread::yield_now(),
                            Err(error) => panic!("append: {error:?}"),
                        }
                    }
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
                    loop {
                        match store.append(request("s", 0, "m0", body)) {
                            Err(TranscriptAppendErrorV1::Storage(
                                MemoryError::OperationInProgress(_),
                            )) => thread::yield_now(),
                            result => break result,
                        }
                    }
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
        assert!(matches!(store.stats(), Err(MemoryError::InvalidState(_))));
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
        use quanta_memory_platform_core::{
            ExecutionBudget, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClassBudget,
            MemoryPreset, ReclaimPriority, ResidencyPolicy,
        };
        use std::cmp::Ordering;
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        use std::sync::{OnceLock, Weak};

        #[derive(Debug)]
        struct ComparisonTrap {
            calls: AtomicUsize,
            panic_at: AtomicUsize,
        }

        impl ComparisonTrap {
            fn new() -> Arc<Self> {
                Arc::new(Self {
                    calls: AtomicUsize::new(0),
                    panic_at: AtomicUsize::new(usize::MAX),
                })
            }

            fn arm_after_one_more_comparison(&self) -> usize {
                let before = self.calls.load(AtomicOrdering::SeqCst);
                self.panic_at.store(before + 2, AtomicOrdering::SeqCst);
                before
            }

            fn disarm(&self) {
                self.panic_at.store(usize::MAX, AtomicOrdering::SeqCst);
            }
        }

        #[derive(Clone, Debug)]
        struct OrderedSessionId {
            name: &'static str,
            trap: Arc<ComparisonTrap>,
        }

        impl PartialEq for OrderedSessionId {
            fn eq(&self, other: &Self) -> bool {
                self.name == other.name
            }
        }

        impl Eq for OrderedSessionId {}

        impl PartialOrd for OrderedSessionId {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for OrderedSessionId {
            fn cmp(&self, other: &Self) -> Ordering {
                let call = self.trap.calls.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                assert_ne!(
                    call,
                    self.trap.panic_at.load(AtomicOrdering::SeqCst),
                    "client SessionId::Ord must not run after retain planning",
                );
                self.name.cmp(other.name)
            }
        }

        #[derive(Debug)]
        struct CallbackTrap {
            calls: AtomicUsize,
            panic_at: AtomicUsize,
        }

        impl CallbackTrap {
            fn new() -> Arc<Self> {
                Arc::new(Self {
                    calls: AtomicUsize::new(0),
                    panic_at: AtomicUsize::new(usize::MAX),
                })
            }

            fn arm_after(&self, additional_calls: usize) {
                self.panic_at.store(
                    self.calls.load(AtomicOrdering::SeqCst) + additional_calls,
                    AtomicOrdering::SeqCst,
                );
            }

            fn disarm(&self) {
                self.panic_at.store(usize::MAX, AtomicOrdering::SeqCst);
            }

            fn hit(&self) {
                let call = self.calls.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                assert_ne!(
                    call,
                    self.panic_at.load(AtomicOrdering::SeqCst),
                    "client callback panic"
                );
            }
        }

        #[derive(Debug)]
        struct CallbackMessageId {
            name: &'static str,
            clones: Arc<CallbackTrap>,
            comparisons: Arc<CallbackTrap>,
        }

        impl Clone for CallbackMessageId {
            fn clone(&self) -> Self {
                self.clones.hit();
                Self {
                    name: self.name,
                    clones: Arc::clone(&self.clones),
                    comparisons: Arc::clone(&self.comparisons),
                }
            }
        }

        impl PartialEq for CallbackMessageId {
            fn eq(&self, other: &Self) -> bool {
                self.name == other.name
            }
        }

        impl Eq for CallbackMessageId {}

        impl PartialOrd for CallbackMessageId {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for CallbackMessageId {
            fn cmp(&self, other: &Self) -> Ordering {
                self.comparisons.hit();
                self.name.cmp(other.name)
            }
        }

        fn callback_id(
            name: &'static str,
            clones: &Arc<CallbackTrap>,
            comparisons: &Arc<CallbackTrap>,
        ) -> CallbackMessageId {
            CallbackMessageId {
                name,
                clones: Arc::clone(clones),
                comparisons: Arc::clone(comparisons),
            }
        }

        fn callback_store(
            broker: &LiveProcessMemoryBroker,
        ) -> SimpleSessionTranscriptStore<String, CallbackMessageId, String, Budgeted> {
            SimpleSessionTranscriptStore::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "transcript-append-callback-test".to_string(),
                    None,
                ),
                TranscriptRetentionPolicyV1::process_lifetime(),
                broker.clone(),
            )
        }

        type ReentrantIdStore =
            SimpleSessionTranscriptStore<String, ReentrantMessageId, String, Budgeted>;

        #[derive(Debug)]
        struct ReentrantMessageId {
            name: &'static str,
            owner: Arc<OnceLock<Weak<ReentrantIdStore>>>,
            busy_callbacks: Arc<AtomicUsize>,
        }

        impl ReentrantMessageId {
            fn callback(&self) {
                let store = self.owner.get().expect("owner installed").upgrade().expect("live");
                assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
                assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
                self.busy_callbacks.fetch_add(1, AtomicOrdering::SeqCst);
            }
        }

        impl Clone for ReentrantMessageId {
            fn clone(&self) -> Self {
                self.callback();
                Self {
                    name: self.name,
                    owner: Arc::clone(&self.owner),
                    busy_callbacks: Arc::clone(&self.busy_callbacks),
                }
            }
        }

        impl PartialEq for ReentrantMessageId {
            fn eq(&self, other: &Self) -> bool {
                self.name == other.name
            }
        }

        impl Eq for ReentrantMessageId {}

        impl PartialOrd for ReentrantMessageId {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for ReentrantMessageId {
            fn cmp(&self, other: &Self) -> Ordering {
                self.callback();
                self.name.cmp(other.name)
            }
        }

        #[test]
        fn append_message_id_clone_and_order_reentry_is_typed_busy() {
            let broker = broker(1_000_000);
            let store = Arc::new(ReentrantIdStore::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "transcript-append-reentry-test".to_string(),
                    None,
                ),
                TranscriptRetentionPolicyV1::process_lifetime(),
                broker.clone(),
            ));
            let owner = Arc::new(OnceLock::new());
            owner.set(Arc::downgrade(&store)).expect("one owner");
            let busy_callbacks = Arc::new(AtomicUsize::new(0));
            for (sequence, name, body) in [(0, "m0", "one"), (1, "m1", "four")] {
                store
                    .append(TranscriptAppendRequestV1::new(
                        TranscriptAppendIdentityV1::new(
                            "s".to_string(),
                            sequence,
                            ReentrantMessageId {
                                name,
                                owner: Arc::clone(&owner),
                                busy_callbacks: Arc::clone(&busy_callbacks),
                            },
                        ),
                        body.to_string(),
                    ))
                    .expect("append after typed busy callbacks");
            }
            assert!(busy_callbacks.load(AtomicOrdering::SeqCst) >= 4);
            assert_eq!(store.stats().expect("committed postimage").item_count, 2);
            assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 7);
            store.close().expect("close after callbacks");
            assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
        }

        fn assert_seed_only_after_append_panic(
            store: &SimpleSessionTranscriptStore<String, CallbackMessageId, String, Budgeted>,
            broker: &LiveProcessMemoryBroker,
        ) {
            let state = store.state.lock().unwrap_or_else(PoisonError::into_inner);
            let session = state.sessions.get("s").expect("seed session remains");
            assert_eq!(session.version, 1, "failed append advanced version");
            assert_eq!(session.rows.len(), 1, "failed append published a row");
            assert_eq!(
                session.sequence_evidence.len(),
                1,
                "failed append published sequence evidence"
            );
            assert_eq!(
                session.message_evidence.len(),
                1,
                "failed append published message evidence"
            );
            assert!(session
                .rows
                .get(&0)
                .expect("seed row")
                .charge
                .as_ref()
                .is_some_and(ManagedCharge::is_active));
            assert_eq!(state.retained_bytes, 3, "failed append changed retained-byte counter");
            drop(state);
            assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 3);
        }

        #[test]
        fn panicking_message_id_clone_at_each_publication_point_leaves_append_atomic() {
            for panic_after in [1, 2] {
                let broker = broker(1_000_000);
                let store = callback_store(&broker);
                let clones = CallbackTrap::new();
                let comparisons = CallbackTrap::new();
                store
                    .append(TranscriptAppendRequestV1::new(
                        TranscriptAppendIdentityV1::new(
                            "s".to_string(),
                            0,
                            callback_id("m0", &clones, &comparisons),
                        ),
                        "one".to_string(),
                    ))
                    .expect("seed row");
                clones.arm_after(panic_after);
                let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = store.append(TranscriptAppendRequestV1::new(
                        TranscriptAppendIdentityV1::new(
                            "s".to_string(),
                            1,
                            callback_id("m1", &clones, &comparisons),
                        ),
                        "four".to_string(),
                    ));
                }));
                assert!(panicked.is_err(), "clone trap must fire at point {panic_after}");
                clones.disarm();
                assert_seed_only_after_append_panic(&store, &broker);
                assert_eq!(
                    store.load(&"s".to_string()).expect("readable after callback panic").len(),
                    1
                );
                store.close().expect("close after callback panic");
                assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
            }
        }

        #[test]
        fn panicking_message_id_clone_cannot_publish_an_empty_new_session() {
            for panic_after in [1, 2] {
                let broker = broker(1_000_000);
                let store = callback_store(&broker);
                let clones = CallbackTrap::new();
                let comparisons = CallbackTrap::new();
                clones.arm_after(panic_after);
                let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = store.append(TranscriptAppendRequestV1::new(
                        TranscriptAppendIdentityV1::new(
                            "new".to_string(),
                            0,
                            callback_id("m0", &clones, &comparisons),
                        ),
                        "body".to_string(),
                    ));
                }));
                assert!(panicked.is_err(), "clone trap must fire at point {panic_after}");
                clones.disarm();
                assert!(store
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .sessions
                    .is_empty());
                assert_eq!(store.stats().expect("readable after callback panic").item_count, 0);
                assert_eq!(store.retained_bytes().expect("bytes"), 0);
                assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
                assert_eq!(
                    store
                        .append(TranscriptAppendRequestV1::new(
                            TranscriptAppendIdentityV1::new(
                                "new".to_string(),
                                0,
                                callback_id("m0", &clones, &comparisons),
                            ),
                            "body".to_string(),
                        ))
                        .expect("fresh retry"),
                    TranscriptAppendDispositionV1::Applied
                );
                store.close().expect("close after retry");
                assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
            }
        }

        #[test]
        fn panicking_new_session_order_after_charge_preserves_old_session_and_ledger() {
            let broker = broker(1_000_000);
            let trap = ComparisonTrap::new();
            let store = SimpleSessionTranscriptStore::<
                OrderedSessionId,
                String,
                String,
                Budgeted,
            >::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "transcript-new-session-order-panic".to_string(),
                    None,
                ),
                TranscriptRetentionPolicyV1::process_lifetime(),
                broker.clone(),
            );
            let old_id = OrderedSessionId { name: "old", trap: Arc::clone(&trap) };
            store
                .append(TranscriptAppendRequestV1::new(
                    TranscriptAppendIdentityV1::new(old_id.clone(), 0, "m0".to_string()),
                    "one".to_string(),
                ))
                .expect("seed session");
            let new_id = OrderedSessionId { name: "new", trap: Arc::clone(&trap) };
            // The preflight performs two lookups; the third comparison is the
            // charged publication entry lookup, still before insertion.
            let before = trap.calls.load(AtomicOrdering::SeqCst);
            trap.panic_at.store(before + 3, AtomicOrdering::SeqCst);
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.append(TranscriptAppendRequestV1::new(
                    TranscriptAppendIdentityV1::new(new_id.clone(), 0, "m1".to_string()),
                    "four".to_string(),
                ));
            }));
            assert!(panicked.is_err(), "new-session Ord trap must fire during publication");
            trap.disarm();
            assert_eq!(store.stats().expect("readable after callback panic").item_count, 1);
            assert_eq!(store.retained_bytes().expect("bytes"), 3);
            assert_eq!(store.load(&old_id).expect("old session").len(), 1);
            assert!(store.load(&new_id).expect("no new session").is_empty());
            assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 3);
            store.close().expect("close after callback panic");
            assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
        }

        #[test]
        fn panicking_message_id_order_at_publication_leaves_append_atomic() {
            let broker = broker(1_000_000);
            let store = callback_store(&broker);
            let clones = CallbackTrap::new();
            let comparisons = CallbackTrap::new();
            store
                .append(TranscriptAppendRequestV1::new(
                    TranscriptAppendIdentityV1::new(
                        "s".to_string(),
                        0,
                        callback_id("m0", &clones, &comparisons),
                    ),
                    "one".to_string(),
                ))
                .expect("seed row");
            // One comparison is the preflight lookup; the next is publication.
            comparisons.arm_after(2);
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.append(TranscriptAppendRequestV1::new(
                    TranscriptAppendIdentityV1::new(
                        "s".to_string(),
                        1,
                        callback_id("m1", &clones, &comparisons),
                    ),
                    "four".to_string(),
                ));
            }));
            assert!(panicked.is_err(), "order trap must fire at publication");
            comparisons.disarm();
            assert_seed_only_after_append_panic(&store, &broker);
            assert_eq!(
                store.load(&"s".to_string()).expect("readable after callback panic").len(),
                1
            );
            let before_retry = comparisons.calls.load(AtomicOrdering::SeqCst);
            comparisons.arm_after(3);
            assert_eq!(
                store
                    .append(TranscriptAppendRequestV1::new(
                        TranscriptAppendIdentityV1::new(
                            "s".to_string(),
                            1,
                            callback_id("m1", &clones, &comparisons),
                        ),
                        "four".to_string(),
                    ))
                    .expect("retry after callback panic"),
                TranscriptAppendDispositionV1::Applied
            );
            assert_eq!(
                comparisons.calls.load(AtomicOrdering::SeqCst),
                before_retry + 2,
                "append compared MessageId after its vacant slot was resolved"
            );
            comparisons.disarm();
            assert_eq!(store.retained_bytes().expect("committed bytes"), 7);
            assert_eq!(broker.ledger_snapshot().expect("committed ledger").total_owner_bytes(), 7);
            store.close().expect("close after callback panic");
            assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
        }

        #[derive(Clone, Debug)]
        struct PanicOnDropBody(Vec<u8>);

        impl ByteSized for PanicOnDropBody {
            fn byte_size(&self) -> u64 {
                self.0.len() as u64
            }
        }

        impl CanonicalKey for PanicOnDropBody {
            fn canonical_key(&self) -> Vec<u8> {
                self.0.clone()
            }
        }

        impl Drop for PanicOnDropBody {
            fn drop(&mut self) {
                panic!("client body destructor panic");
            }
        }

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
            quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
                .with_manifest(MemoryBudgetManifest::new(
                    MemoryPreset::Production,
                    ExecutionBudget::default(),
                    residency,
                ))
                .build()
                .expect("valid broker")
        }

        fn charged(
            broker: &LiveProcessMemoryBroker,
        ) -> SimpleSessionTranscriptStore<String, String, String, Budgeted> {
            SimpleSessionTranscriptStore::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "transcript-idempotency-tests".to_string(),
                    None,
                ),
                TranscriptRetentionPolicyV1::process_lifetime(),
                broker.clone(),
            )
        }

        #[test]
        fn retain_rollback_and_commit_do_not_compare_client_session_id_after_detach() {
            let broker = broker(1_000_000);
            let trap = ComparisonTrap::new();
            let id = OrderedSessionId { name: "session", trap: Arc::clone(&trap) };
            let store = SimpleSessionTranscriptStore::<OrderedSessionId, String, String, Budgeted>::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "transcript-ordinal-test".to_string(),
                    None,
                ),
                TranscriptRetentionPolicyV1::process_lifetime(),
                broker.clone(),
            );
            for (sequence, body) in [(0, "one"), (1, "four")] {
                store
                    .append(TranscriptAppendRequestV1::new(
                        TranscriptAppendIdentityV1::new(
                            id.clone(),
                            sequence,
                            format!("m{sequence}"),
                        ),
                        body.to_string(),
                    ))
                    .expect("seed row");
            }
            assert_eq!(broker.ledger_snapshot().expect("seed ledger").total_owner_bytes(), 7);

            broker.begin_rebuild().expect("fence release");
            let before = trap.arm_after_one_more_comparison();
            assert!(matches!(store.retain_sequences(&id, &[0]), Err(MemoryError::InvalidState(_))));
            assert_eq!(trap.calls.load(AtomicOrdering::SeqCst), before + 1);
            trap.disarm();
            assert_eq!(store.load(&id).expect("rollback rows").len(), 2);
            assert_eq!(store.retained_bytes().expect("rollback bytes"), 7);
            assert_eq!(broker.ledger_snapshot().expect("rollback ledger").total_owner_bytes(), 7);

            broker.finish_rebuild().expect("open fence");
            let before = trap.arm_after_one_more_comparison();
            store.retain_sequences(&id, &[0]).expect("retain after fence");
            assert_eq!(trap.calls.load(AtomicOrdering::SeqCst), before + 1);
            trap.disarm();
            assert_eq!(store.load(&id).expect("retained row").len(), 1);
            assert_eq!(store.retained_bytes().expect("retained bytes"), 3);
            assert_eq!(broker.ledger_snapshot().expect("retained ledger").total_owner_bytes(), 3);
        }

        #[test]
        fn panicking_session_order_before_detach_preserves_rows_and_ledger() {
            let broker = broker(1_000_000);
            let trap = ComparisonTrap::new();
            let id = OrderedSessionId { name: "session", trap: Arc::clone(&trap) };
            let store = SimpleSessionTranscriptStore::<OrderedSessionId, String, String, Budgeted>::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "transcript-order-panic-test".to_string(),
                    None,
                ),
                TranscriptRetentionPolicyV1::process_lifetime(),
                broker.clone(),
            );
            for (sequence, body) in [(0, "one"), (1, "four")] {
                store
                    .append(TranscriptAppendRequestV1::new(
                        TranscriptAppendIdentityV1::new(
                            id.clone(),
                            sequence,
                            format!("m{sequence}"),
                        ),
                        body.to_string(),
                    ))
                    .expect("seed row");
            }

            let before = trap.calls.load(AtomicOrdering::SeqCst);
            trap.panic_at.store(before + 1, AtomicOrdering::SeqCst);
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = store.retain_sequences(&id, &[0]);
            }));
            assert!(panicked.is_err(), "trap must execute before any row is detached");
            let state = store.state.lock().unwrap_or_else(PoisonError::into_inner);
            let session = state.sessions.values().next().expect("session remains");
            assert_eq!(session.rows.len(), 2);
            assert!(session
                .rows
                .values()
                .all(|row| row.charge.as_ref().is_some_and(ManagedCharge::is_active)));
            assert_eq!(state.retained_bytes, 7);
            drop(state);
            assert_eq!(broker.ledger_snapshot().expect("ledger unchanged").total_owner_bytes(), 7);
            drop(store);
            assert_eq!(broker.ledger_snapshot().expect("drop cleanup").total_owner_bytes(), 0);
        }

        #[test]
        fn drop_contains_multiple_client_destructor_panics_during_outer_unwind() {
            const CHILD_ENV: &str = "QMP_TRANSCRIPT_DROP_UNWIND_CHILD";
            if std::env::var_os(CHILD_ENV).is_some() {
                let broker = broker(1_000_000);
                let store = SimpleSessionTranscriptStore::<
                    String,
                    String,
                    PanicOnDropBody,
                    Budgeted,
                >::new_budgeted(
                    StoreBindingHandle::new(
                        MemoryClass::SessionTranscript,
                        "transcript-drop-panic-test".to_string(),
                        None,
                    ),
                    TranscriptRetentionPolicyV1::process_lifetime(),
                    broker.clone(),
                );
                for sequence in 0..2 {
                    store
                        .append(TranscriptAppendRequestV1::new(
                            TranscriptAppendIdentityV1::new(
                                "session".to_string(),
                                sequence,
                                format!("message-{sequence}"),
                            ),
                            PanicOnDropBody(vec![0; 3]),
                        ))
                        .expect("seed charged row");
                }
                assert_eq!(broker.ledger_snapshot().expect("live ledger").total_owner_bytes(), 6);
                let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _store = store;
                    panic!("outer unwind");
                }));
                assert!(outer.is_err());
                assert_eq!(broker.ledger_snapshot().expect("drop cleanup").total_owner_bytes(), 0);
                return;
            }

            let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
                .arg("--exact")
                .arg("transcript_store::service::tests::accounting::drop_contains_multiple_client_destructor_panics_during_outer_unwind")
                .env(CHILD_ENV, "1")
                .output()
                .expect("subprocess test run");
            assert!(
                output.status.success(),
                "store Drop must not abort on client panic: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        #[test]
        fn retain_retires_multiple_panicking_client_bodies_after_ledger_postimage() {
            let broker = broker(1_000_000);
            let store = SimpleSessionTranscriptStore::<
                String,
                String,
                PanicOnDropBody,
                Budgeted,
            >::new_budgeted(
                StoreBindingHandle::new(
                    MemoryClass::SessionTranscript,
                    "transcript-retain-panic-test".to_string(),
                    None,
                ),
                TranscriptRetentionPolicyV1::process_lifetime(),
                broker.clone(),
            );
            for sequence in 0..3 {
                store
                    .append(TranscriptAppendRequestV1::new(
                        TranscriptAppendIdentityV1::new(
                            "session".to_string(),
                            sequence,
                            format!("message-{sequence}"),
                        ),
                        PanicOnDropBody(vec![0; 3]),
                    ))
                    .expect("seed row");
            }
            assert_eq!(broker.ledger_snapshot().expect("live ledger").total_owner_bytes(), 9);
            store.retain_sequences(&"session".to_string(), &[0]).expect("retain");
            assert_eq!(store.stats().expect("postimage").item_count, 1);
            assert_eq!(store.retained_bytes().expect("retained bytes"), 3);
            assert_eq!(broker.ledger_snapshot().expect("retained ledger").total_owner_bytes(), 3);
            store.close().expect("close survivor without client panic");
            assert_eq!(broker.ledger_snapshot().expect("closed ledger").total_owner_bytes(), 0);
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

        #[test]
        fn close_refunds_all_sessions_exactly_once_and_exposes_terminal_empty_reads() {
            let broker = broker(1_000_000);
            let store = charged(&broker);
            store.append(request("a", 0, "m0", "one")).expect("first");
            store.append(request("a", 2, "m2", "four")).expect("second");
            store.append(request("b", 9, "m9", "xx")).expect("third");
            assert_eq!(broker.ledger_snapshot().expect("before").total_owner_bytes(), 9);

            SessionTranscriptStore::close(&store).expect("close");
            assert_eq!(broker.ledger_snapshot().expect("after").total_owner_bytes(), 0);
            assert_eq!(store.retained_bytes().expect("closed bytes"), 0);
            assert_eq!(store.stats().expect("closed stats").item_count, 0);
            assert!(store.load(&"a".to_string()).expect("closed load").is_empty());
            assert_eq!(
                store.load_versioned_snapshot(&"b".to_string()).expect("closed snapshot").version(),
                0
            );
            store.close().expect("idempotent double close");
            assert_eq!(broker.ledger_snapshot().expect("after repeat").total_owner_bytes(), 0);
            assert!(matches!(
                store.append(request("a", 0, "m0", "one")),
                Err(TranscriptAppendErrorV1::Storage(MemoryError::Closed(_)))
            ));
            assert!(matches!(store.truncate_to(&"a".to_string(), 0), Err(MemoryError::Closed(_))));
            assert!(matches!(
                store.retain_sequences_if_version(&"b".to_string(), 0, &[]),
                Err(MemoryError::Closed(_))
            ));
        }

        #[test]
        fn failed_close_preserves_rows_versions_tombstones_and_all_lease_authority() {
            let broker = broker(1_000_000);
            let store = charged(&broker);
            store.append(request("a", 0, "m0", "one")).expect("first");
            store.append(request("b", 1, "m1", "four")).expect("second");
            let version =
                store.load_versioned_snapshot(&"a".to_string()).expect("before").version();
            broker.begin_rebuild().expect("fence");
            assert!(matches!(store.close(), Err(MemoryError::InvalidState(_))));
            assert_eq!(broker.ledger_snapshot().expect("fenced ledger").total_owner_bytes(), 7);
            assert_eq!(store.stats().expect("restored stats").item_count, 2);
            assert_eq!(store.load(&"b".to_string()).expect("restored row")[0].body(), "four");
            assert_eq!(
                store.load_versioned_snapshot(&"a".to_string()).expect("version").version(),
                version
            );
            assert_eq!(
                store.append(request("a", 0, "m0", "one")).expect("restored evidence"),
                TranscriptAppendDispositionV1::Replayed
            );
            broker.finish_rebuild().expect("open fence");
            store.close().expect("retry close");
            assert_eq!(broker.ledger_snapshot().expect("after retry").total_owner_bytes(), 0);
        }

        #[test]
        fn drop_releases_all_live_rows_even_after_storage_poison() {
            let broker = broker(1_000_000);
            let store = Arc::new(charged(&broker));
            store.append(request("a", 0, "m0", "one")).expect("first");
            store.append(request("b", 1, "m1", "four")).expect("second");
            let poisoner = Arc::clone(&store);
            let _ = thread::spawn(move || {
                let _state = poisoner.state.lock().expect("state");
                panic!("poison store after committed charge");
            })
            .join();
            assert!(matches!(store.close(), Err(MemoryError::InvalidState(_))));
            assert!(matches!(store.stats(), Err(MemoryError::InvalidState(_))));
            assert_eq!(broker.ledger_snapshot().expect("charged").total_owner_bytes(), 7);
            drop(store);
            assert_eq!(broker.ledger_snapshot().expect("drop refund").total_owner_bytes(), 0);
        }

        #[test]
        fn drop_during_rebuild_defers_exact_release_until_finish() {
            let broker = broker(1_000_000);
            let store = charged(&broker);
            store.append(request("a", 0, "m0", "one")).expect("first");
            store.append(request("b", 1, "m1", "four")).expect("second");
            broker.begin_rebuild().expect("fence");
            drop(store);
            assert_eq!(broker.ledger_snapshot().expect("fenced").total_owner_bytes(), 7);
            broker.finish_rebuild().expect("drain deferred charges");
            assert_eq!(broker.ledger_snapshot().expect("drained").total_owner_bytes(), 0);
        }

        #[test]
        fn mutation_lane_rejects_reentry_and_reads_without_exposing_partial_images() {
            let broker = broker(1_000_000);
            let store = charged(&broker);
            store.append(request("s", 0, "m0", "one")).expect("seed");
            let permit = store.lifecycle.begin_mutation().expect("lane owner");
            assert!(matches!(
                store.append(request("s", 1, "m1", "two")),
                Err(TranscriptAppendErrorV1::Storage(MemoryError::OperationInProgress(_)))
            ));
            assert!(matches!(store.close(), Err(MemoryError::OperationInProgress(_))));
            assert!(matches!(
                store.load(&"s".to_string()),
                Err(MemoryError::OperationInProgress(_))
            ));
            assert!(matches!(store.stats(), Err(MemoryError::OperationInProgress(_))));
            drop(permit);
            assert_eq!(store.load(&"s".to_string()).expect("stable image").len(), 1);
            assert_eq!(broker.ledger_snapshot().expect("inert reentry").total_owner_bytes(), 3);
        }

        #[test]
        fn checked_byte_and_version_overflow_denials_leave_ledger_and_indexes_intact() {
            let broker = broker(1_000_000);
            let store = charged(&broker);
            {
                let mut state = store.state.lock().expect("state");
                state.retained_bytes = u64::MAX;
            }
            assert!(matches!(
                store.append(request("s", 0, "m0", "x")),
                Err(TranscriptAppendErrorV1::Accounting(MemoryError::Capacity(_)))
            ));
            assert!(store.lock_state().expect("state").sessions.is_empty());
            assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
            {
                let mut state = store.state.lock().expect("state");
                state.retained_bytes = 0;
                state.sessions.insert(
                    "s".to_string(),
                    SessionState { version: u64::MAX, ..SessionState::default() },
                );
            }
            assert!(matches!(
                store.append(request("s", 0, "m0", "x")),
                Err(TranscriptAppendErrorV1::Storage(MemoryError::Capacity(_)))
            ));
            assert_eq!(store.lock_state().expect("state").sessions["s"].version, u64::MAX);
            assert_eq!(broker.ledger_snapshot().expect("ledger").total_owner_bytes(), 0);
        }

        #[test]
        fn failed_multirow_retain_preserves_positions_tombstones_and_version() {
            let broker = broker(1_000_000);
            let store = charged(&broker);
            for (sequence, body) in [(0, "one"), (2, "two"), (4, "four"), (9, "nine")] {
                store.append(request("s", sequence, &format!("m{sequence}"), body)).expect("seed");
            }
            let before = store.load_versioned_snapshot(&"s".to_string()).expect("snapshot");
            let before_bytes = store.retained_bytes().expect("bytes");
            broker.begin_rebuild().expect("fence");
            assert!(matches!(
                store.retain_sequences_if_version(&"s".to_string(), before.version(), &[2, 9]),
                Err(MemoryError::InvalidState(_))
            ));
            assert_eq!(store.load_versioned_snapshot(&"s".to_string()).expect("unchanged"), before);
            assert_eq!(store.retained_bytes().expect("unchanged bytes"), before_bytes);
            assert_eq!(
                broker.ledger_snapshot().expect("unchanged ledger").total_owner_bytes(),
                before_bytes
            );
            broker.finish_rebuild().expect("finish");
            assert_eq!(
                store
                    .retain_sequences_if_version(&"s".to_string(), before.version(), &[2, 9])
                    .expect("retry"),
                TranscriptRetainDispositionV1::Applied { version: before.version() + 1 }
            );
            assert_eq!(
                store.append(request("s", 0, "m0", "one")).expect("evicted retry"),
                TranscriptAppendDispositionV1::Replayed
            );
            assert_eq!(store.retained_bytes().expect("after retain"), 7);
            assert_eq!(
                broker.ledger_snapshot().expect("after retain ledger").total_owner_bytes(),
                7
            );
        }

        #[test]
        fn racing_close_and_append_have_one_linearizable_terminal_image() {
            for _ in 0..16 {
                let broker = broker(1_000_000);
                let store = Arc::new(charged(&broker));
                let barrier = Arc::new(Barrier::new(2));
                let closer = Arc::clone(&store);
                let closer_barrier = Arc::clone(&barrier);
                let close = thread::spawn(move || {
                    closer_barrier.wait();
                    closer.close()
                });
                barrier.wait();
                let append = store.append(request("s", 0, "m0", "one"));
                let close = close.join().expect("join close");
                assert!(matches!(
                    append,
                    Ok(TranscriptAppendDispositionV1::Applied)
                        | Err(TranscriptAppendErrorV1::Storage(MemoryError::Closed(_)))
                        | Err(TranscriptAppendErrorV1::Storage(MemoryError::OperationInProgress(
                            _
                        )))
                ));
                assert!(matches!(close, Ok(()) | Err(MemoryError::OperationInProgress(_))));
                store.close().expect("terminal retry");
                assert!(store.load(&"s".to_string()).expect("terminal rows").is_empty());
                assert_eq!(store.retained_bytes().expect("terminal bytes"), 0);
                assert_eq!(
                    broker.ledger_snapshot().expect("terminal ledger").total_owner_bytes(),
                    0
                );
            }
        }
    }
}
