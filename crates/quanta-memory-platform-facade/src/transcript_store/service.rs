use std::borrow::Borrow;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::sync::{Mutex, MutexGuard, PoisonError};

use quanta_memory_platform_core::{
    ChargeKind, LiveProcessMemoryBroker, ManagedCharge, MemoryContextKind, MemoryError,
    MemoryEvent, MemoryOperation, MemoryOutcome, MemoryResult, StoreBindingHandle, StoreStats,
};

#[cfg(feature = "unbudgeted-stores")]
use crate::shared::accounting::Unbudgeted;
use crate::shared::accounting::{Budgeted, StoreAccounting};
use crate::shared::lifecycle::{
    invoke_client_read_while_locked, retire_client_value, BeginClose, DropTransition, InFlight,
    StoreLifecycle,
};
use crate::shared::measure::{semantic_cost_v1, ByteSized, CanonicalKey, RetainedCostV1};
use crate::transcript_store::{
    SessionTranscriptStore, TranscriptAppendDispositionV1, TranscriptAppendErrorV1,
    TranscriptAppendRequestV1, TranscriptRecordV1, TranscriptRetainDispositionV1,
    TranscriptRetentionPolicyV2, TranscriptRetentionReconfigureErrorV2, TranscriptRetentionStatsV2,
    TranscriptSnapshotV1,
};

const BODY_DIGEST_DOMAIN: &[u8] = b"qmp.transcript-body.v1\0";

fn active_row_cost(body_bytes: u64, message_id_bytes: u64) -> MemoryResult<u64> {
    RetainedCostV1 {
        payload_bytes: body_bytes,
        key_identity_bytes: message_id_bytes,
        index_bytes: semantic_cost_v1::ORDERED_INDEX_BYTES,
        fixed_overhead_bytes: semantic_cost_v1::ENTRY_STATE_BYTES,
        ..RetainedCostV1::default()
    }
    .total()
}

fn identity_evidence_cost(message_id_bytes: u64) -> MemoryResult<u64> {
    let two = |value: u64| {
        value
            .checked_mul(2)
            .ok_or_else(|| MemoryError::capacity("transcript evidence cost overflow"))
    };
    RetainedCostV1 {
        key_identity_bytes: two(message_id_bytes)?,
        index_bytes: two(semantic_cost_v1::ORDERED_INDEX_BYTES)?,
        tombstone_bytes: two(semantic_cost_v1::DIGEST_BYTES)?,
        fixed_overhead_bytes: two(semantic_cost_v1::ENTRY_STATE_BYTES)?,
        ..RetainedCostV1::default()
    }
    .total()
}

fn session_key_cost(session_id_bytes: u64) -> MemoryResult<u64> {
    RetainedCostV1 {
        key_identity_bytes: session_id_bytes,
        index_bytes: semantic_cost_v1::ORDERED_INDEX_BYTES,
        fixed_overhead_bytes: semantic_cost_v1::ENTRY_STATE_BYTES,
        ..RetainedCostV1::default()
    }
    .total()
}

#[derive(Debug)]
struct TranscriptEntry<MessageId, Message> {
    sequence: u64,
    message_id: MessageId,
    body: Message,
    active_bytes: u64,
    active_charge: Option<ManagedCharge>,
}

fn clone_records<MessageId: Clone, Message: Clone>(
    rows: &BTreeMap<u64, TranscriptEntry<MessageId, Message>>,
) -> Vec<TranscriptRecordV1<MessageId, Message>> {
    // Complete every client Clone while earlier copies still have guarded
    // destructors. Only plain moves remain after the last Clone succeeds.
    let mut guarded: Vec<_> = rows
        .values()
        .map(|entry| {
            (
                entry.sequence,
                InFlight::new(entry.message_id.clone()),
                InFlight::new(entry.body.clone()),
            )
        })
        .collect();
    guarded
        .iter_mut()
        .map(|(sequence, message_id, body)| TranscriptRecordV1 {
            sequence: *sequence,
            message_id: message_id.take(),
            body: body.take(),
        })
        .collect()
}

#[derive(Debug)]
struct SequenceEvidence<MessageId> {
    message_id: MessageId,
    body_digest: [u8; 32],
    evidence_bytes: u64,
    evidence_charge: Option<ManagedCharge>,
}

/// Map-owned caller key whose destructor is contained if a caller `Ord`
/// panics during the map's owned-key descent.
struct ClientKey<T>(InFlight<T>);

impl<T> ClientKey<T> {
    fn new(value: T) -> Self {
        Self(InFlight::new(value))
    }
    fn get(&self) -> &T {
        self.0.get()
    }
    fn take(&mut self) -> T {
        self.0.take()
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for ClientKey<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(formatter)
    }
}
impl<T> Borrow<T> for ClientKey<T> {
    fn borrow(&self) -> &T {
        self.get()
    }
}
impl<T: Ord> PartialEq for ClientKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.get().eq(other.get())
    }
}
impl<T: Ord> Eq for ClientKey<T> {}
impl<T: Ord> PartialOrd for ClientKey<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T: Ord> Ord for ClientKey<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.get().cmp(other.get())
    }
}

#[derive(Clone, Debug)]
struct MessageEvidence {
    sequence: u64,
    body_digest: [u8; 32],
}

#[derive(Debug)]
struct SessionState<MessageId, Message> {
    version: u64,
    key_bytes: u64,
    key_charge: Option<ManagedCharge>,
    retained_bytes: u64,
    rows: BTreeMap<u64, TranscriptEntry<MessageId, Message>>,
    sequence_evidence: BTreeMap<u64, SequenceEvidence<MessageId>>,
    message_evidence: BTreeMap<ClientKey<MessageId>, MessageEvidence>,
}

impl<MessageId, Message> Default for SessionState<MessageId, Message> {
    fn default() -> Self {
        Self {
            version: 0,
            key_bytes: 0,
            key_charge: None,
            retained_bytes: 0,
            rows: BTreeMap::new(),
            sequence_evidence: BTreeMap::new(),
            message_evidence: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
struct TranscriptState<SessionId, MessageId, Message> {
    sessions: BTreeMap<ClientKey<SessionId>, SessionState<MessageId, Message>>,
    retention: TranscriptRetentionPolicyV2,
    active_row_count: usize,
    active_bytes: u64,
    evidence_count: usize,
    evidence_bytes: u64,
    session_key_bytes: u64,
    retained_bytes: u64,
}

fn semantic_session_bytes<MessageId, Message>(
    session: &SessionState<MessageId, Message>,
) -> MemoryResult<u64> {
    let with_rows = session.rows.values().try_fold(session.key_bytes, |total, row| {
        total
            .checked_add(row.active_bytes)
            .ok_or_else(|| MemoryError::capacity("transcript active-row sum overflow"))
    })?;
    session.sequence_evidence.values().try_fold(with_rows, |total, evidence| {
        total
            .checked_add(evidence.evidence_bytes)
            .ok_or_else(|| MemoryError::capacity("transcript evidence sum overflow"))
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TranscriptComponentTotals {
    active_row_count: usize,
    active_bytes: u64,
    evidence_count: usize,
    evidence_bytes: u64,
    session_key_bytes: u64,
    retained_bytes: u64,
}

fn checked_component_totals<SessionId, MessageId, Message>(
    state: &TranscriptState<SessionId, MessageId, Message>,
) -> MemoryResult<TranscriptComponentTotals> {
    let mut totals = TranscriptComponentTotals::default();
    for session in state.sessions.values() {
        totals.session_key_bytes = totals
            .session_key_bytes
            .checked_add(session.key_bytes)
            .ok_or_else(|| MemoryError::capacity("transcript session-key sum overflow"))?;
        totals.active_row_count = totals
            .active_row_count
            .checked_add(session.rows.len())
            .ok_or_else(|| MemoryError::capacity("transcript active-row count overflow"))?;
        totals.evidence_count = totals
            .evidence_count
            .checked_add(session.sequence_evidence.len())
            .ok_or_else(|| MemoryError::capacity("transcript evidence count overflow"))?;
        for row in session.rows.values() {
            totals.active_bytes = totals
                .active_bytes
                .checked_add(row.active_bytes)
                .ok_or_else(|| MemoryError::capacity("transcript active-byte sum overflow"))?;
        }
        for evidence in session.sequence_evidence.values() {
            totals.evidence_bytes = totals
                .evidence_bytes
                .checked_add(evidence.evidence_bytes)
                .ok_or_else(|| MemoryError::capacity("transcript evidence-byte sum overflow"))?;
        }
    }
    totals.retained_bytes = totals
        .active_bytes
        .checked_add(totals.evidence_bytes)
        .and_then(|sum| sum.checked_add(totals.session_key_bytes))
        .ok_or_else(|| MemoryError::capacity("transcript retained-byte sum overflow"))?;
    Ok(totals)
}

fn validate_component_counters<SessionId, MessageId, Message>(
    state: &TranscriptState<SessionId, MessageId, Message>,
) -> MemoryResult<()> {
    let checked = checked_component_totals(state)?;
    let recorded = TranscriptComponentTotals {
        active_row_count: state.active_row_count,
        active_bytes: state.active_bytes,
        evidence_count: state.evidence_count,
        evidence_bytes: state.evidence_bytes,
        session_key_bytes: state.session_key_bytes,
        retained_bytes: state.retained_bytes,
    };
    if checked == recorded {
        Ok(())
    } else {
        Err(MemoryError::InvalidState(format!(
            "transcript component counters disagree with owned state: recorded={recorded:?}, checked={checked:?}"
        )))
    }
}

/// Portable bounded transcript store with caller-owned append identity.
///
/// One mutex owns rows, both identity indexes, body evidence, retained-byte
/// accounting, and non-clone charge leases. Every append either commits all of
/// them or none. Truncation evicts active rows and refunds only their leases;
/// admitted identity evidence and the session key remain charged until close/Drop.
/// This in-memory implementation makes no
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
/// # use quanta_memory_platform_facade::{Budgeted, SimpleSessionTranscriptStore, TranscriptRetentionPolicyV2};
/// # use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};
/// let binding = StoreBindingHandle::new(MemoryClass::SessionTranscript, "owner".into(), None);
/// let _ = SimpleSessionTranscriptStore::<String, String, String, Budgeted>::new(
///     binding, TranscriptRetentionPolicyV2::default()
/// );
/// ```
///
/// ```compile_fail
/// # use quanta_memory_platform_facade::{Budgeted, SimpleSessionTranscriptStore};
/// let _ = SimpleSessionTranscriptStore::<String, String, String, Budgeted>::charged_against;
/// ```
#[derive(Debug)]
pub struct SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode> {
    binding: StoreBindingHandle,
    accounting: StoreAccounting,
    lifecycle: StoreLifecycle,
    state: Mutex<TranscriptState<SessionId, MessageId, Message>>,
    _mode: PhantomData<fn() -> Mode>,
}

impl<SessionId, MessageId, Message>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, Budgeted>
{
    pub(crate) fn new_budgeted(
        binding: StoreBindingHandle,
        retention: TranscriptRetentionPolicyV2,
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
        retention: TranscriptRetentionPolicyV2,
    ) -> Self {
        Self::with_accounting(binding, retention, StoreAccounting::Unbudgeted)
    }
}

impl<SessionId, MessageId, Message, Mode>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode>
{
    fn with_accounting(
        binding: StoreBindingHandle,
        retention: TranscriptRetentionPolicyV2,
        accounting: StoreAccounting,
    ) -> Self {
        Self {
            binding,
            accounting,
            lifecycle: StoreLifecycle::new("transcript store"),
            state: Mutex::new(TranscriptState {
                sessions: BTreeMap::new(),
                retention,
                active_row_count: 0,
                active_bytes: 0,
                evidence_count: 0,
                evidence_bytes: 0,
                session_key_bytes: 0,
                retained_bytes: 0,
            }),
            _mode: PhantomData,
        }
    }

    pub fn retention_policy(&self) -> MemoryResult<TranscriptRetentionPolicyV2> {
        self.lifecycle.with_read_state(&self.state, |state| state.retention)
    }

    fn lock_state(
        &self,
    ) -> MemoryResult<MutexGuard<'_, TranscriptState<SessionId, MessageId, Message>>> {
        self.lifecycle.try_lock_state(&self.state)
    }

    pub fn retained_bytes(&self) -> MemoryResult<u64> {
        self.lifecycle.with_read_state(&self.state, |state| state.retained_bytes)
    }

    pub fn retention_stats(&self) -> MemoryResult<TranscriptRetentionStatsV2> {
        self.lifecycle.with_read_state(&self.state, |state| {
            validate_component_counters(state)?;
            Ok(TranscriptRetentionStatsV2 {
                active_row_count: state.active_row_count,
                active_bytes: state.active_bytes,
                evidence_count: state.evidence_count,
                evidence_bytes: state.evidence_bytes,
                session_count: state.sessions.len(),
                session_key_bytes: state.session_key_bytes,
                total_retained_bytes: state.retained_bytes,
            })
        })?
    }

    pub fn try_reconfigure_retention(
        &self,
        retention: TranscriptRetentionPolicyV2,
    ) -> Result<(), TranscriptRetentionReconfigureErrorV2> {
        let _mutation = self
            .lifecycle
            .begin_mutation()
            .map_err(TranscriptRetentionReconfigureErrorV2::Storage)?;
        let mut state =
            self.lock_state().map_err(TranscriptRetentionReconfigureErrorV2::Storage)?;
        if state.evidence_count > retention.max_evidence_count()
            || state.evidence_bytes > retention.max_evidence_bytes()
        {
            return Err(TranscriptRetentionReconfigureErrorV2::BelowLiveEvidence {
                live_evidence_count: state.evidence_count,
                requested_max_evidence_count: retention.max_evidence_count(),
                live_evidence_bytes: state.evidence_bytes,
                requested_max_evidence_bytes: retention.max_evidence_bytes(),
            });
        }
        state.retention = retention;
        Ok(())
    }

    fn require_charge_shape(
        &self,
        charge: &Option<ManagedCharge>,
        operation: &str,
    ) -> MemoryResult<()> {
        match (self.accounting.broker().is_some(), charge.is_some()) {
            (true, false) => Err(MemoryError::InvalidState(format!(
                "budgeted transcript retention component has no managed charge during {operation}"
            ))),
            (false, true) => Err(MemoryError::InvalidState(format!(
                "unbudgeted transcript retention component owns a managed charge during {operation}"
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
            validate_component_counters(&state)?;
            let mut checked_total = 0u64;
            for session in state.sessions.values() {
                let checked_session = semantic_session_bytes(session)?;
                if checked_session != session.retained_bytes {
                    return Err(MemoryError::InvalidState(
                        "transcript session cost counter disagrees with owned leases".to_string(),
                    ));
                }
                checked_total = checked_total
                    .checked_add(checked_session)
                    .ok_or_else(|| MemoryError::capacity("transcript close total overflow"))?;
                self.require_charge_shape(&session.key_charge, "close session key")?;
                for entry in session.rows.values() {
                    self.require_charge_shape(&entry.active_charge, "close active row")?;
                }
                for evidence in session.sequence_evidence.values() {
                    self.require_charge_shape(
                        &evidence.evidence_charge,
                        "close identity evidence",
                    )?;
                }
            }
            if checked_total != state.retained_bytes {
                return Err(MemoryError::InvalidState(
                    "transcript total cost counter disagrees with sessions".to_string(),
                ));
            }
            std::mem::take(&mut state.sessions)
        };
        let (slots, mut charges) = take_session_charges(&mut sessions);
        if let Err(error) = self.accounting.release_many(&mut charges) {
            restore_session_charges(&mut sessions, &slots, charges);
            self.state.lock().unwrap_or_else(PoisonError::into_inner).sessions = sessions;
            return Err(error);
        }

        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active_row_count = 0;
        state.active_bytes = 0;
        state.evidence_count = 0;
        state.evidence_bytes = 0;
        state.session_key_bytes = 0;
        state.retained_bytes = 0;
        drop(state);
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
        retention: TranscriptRetentionPolicyV2,
    ) -> Self {
        Self::with_accounting(binding, retention, StoreAccounting::Unbudgeted)
    }
}

impl<SessionId, MessageId, Message, Mode>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode>
where
    SessionId: Clone + Ord + ByteSized,
{
    pub fn session_bytes(&self, session_id: &SessionId) -> MemoryResult<u64> {
        // The caller-implemented Ord probe runs inside the owned read
        // boundary: a comparison panic cannot poison the state mutex, and a
        // missing session still reports zero rather than an error.
        self.lifecycle.with_read_state(&self.state, |state| {
            state.sessions.get(session_id).map_or(Ok(0), |session| {
                let checked = semantic_session_bytes(session)?;
                if checked != session.retained_bytes {
                    return Err(MemoryError::InvalidState(
                        "transcript session cost counter disagrees with owned leases".to_string(),
                    ));
                }
                Ok(checked)
            })
        })?
    }

    pub fn checked_stats(&self) -> MemoryResult<StoreStats> {
        self.lifecycle.with_read_state(&self.state, |state| {
            validate_component_counters(state)?;
            Ok(StoreStats {
                item_count: state.active_row_count,
                estimated_bytes: state.retained_bytes,
            })
        })?
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

        // Resolve caller-controlled Ord before the state-mutation lock; the
        // positional ordinal keeps every later block caller-trait-free while
        // the plan is detached (pinned by the no-Ord-after-detach contract).
        // The scan runs inside the containment helper, and session count is
        // budget-bounded because every session owns a positive key charge.
        let session_ordinal = invoke_client_read_while_locked(self.lock_state()?, |state| {
            state.sessions.keys().position(|candidate| candidate.get().cmp(session_id).is_eq())
        });
        let Some(session_ordinal) = session_ordinal else {
            return if retained.is_empty() {
                Ok(())
            } else {
                Err(MemoryError::InvalidState(
                    "transcript retain selector references a missing session".to_string(),
                ))
            };
        };

        let (
            mut dropped,
            session_ordinal,
            next_version,
            next_retained_bytes,
            next_session_bytes,
            next_active_row_count,
            next_active_bytes,
        ) = {
            let mut state = self.lock_state()?;
            let session = state
                .sessions
                .values()
                .nth(session_ordinal)
                .expect("validated transcript session ordinal is present under state mutex");
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
                self.require_charge_shape(&entry.active_charge, "retain")?;
                dropped_bytes = dropped_bytes.checked_add(entry.active_bytes).ok_or_else(|| {
                    MemoryError::capacity("transcript dropped-byte counter overflow")
                })?;
            }
            let next_retained_bytes =
                state.retained_bytes.checked_sub(dropped_bytes).ok_or_else(|| {
                    MemoryError::InvalidState("transcript retained-byte underflow".to_string())
                })?;
            let next_session_bytes =
                session.retained_bytes.checked_sub(dropped_bytes).ok_or_else(|| {
                    MemoryError::InvalidState("transcript session-byte underflow".to_string())
                })?;
            let next_active_row_count =
                state.active_row_count.checked_sub(drop_sequences.len()).ok_or_else(|| {
                    MemoryError::InvalidState("transcript active-row count underflow".to_string())
                })?;
            let next_active_bytes =
                state.active_bytes.checked_sub(dropped_bytes).ok_or_else(|| {
                    MemoryError::InvalidState(
                        "transcript active-byte counter underflow".to_string(),
                    )
                })?;
            let session =
                state.sessions.values_mut().nth(session_ordinal).expect(
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
            (
                dropped,
                session_ordinal,
                next_version,
                next_retained_bytes,
                next_session_bytes,
                next_active_row_count,
                next_active_bytes,
            )
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
        session.retained_bytes = next_session_bytes;
        // sequence_evidence and message_evidence intentionally survive inside
        // the bounded fail-closed horizon. They are the late-retry safety owner.
        state.active_row_count = next_active_row_count;
        state.active_bytes = next_active_bytes;
        state.retained_bytes = next_retained_bytes;
        drop(state);
        retire_rows(dropped.into_iter().map(|(_, row)| row));
        Ok(())
    }
}

impl<SessionId, MessageId, Message, Mode>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode>
where
    SessionId: Clone + Ord + ByteSized,
    MessageId: Clone + Ord + ByteSized,
    Message: Clone + ByteSized + CanonicalKey,
{
    /// Copy active rows and their mutation version under the store mutex.
    /// Caller policy executes after this method releases the mutex.
    pub fn load_versioned_snapshot(
        &self,
        session_id: &SessionId,
    ) -> MemoryResult<TranscriptSnapshotV1<MessageId, Message>> {
        self.lifecycle.with_read_state(&self.state, |state| {
            let version = state.sessions.get(session_id).map_or(0, |session| session.version);
            let records = state
                .sessions
                .get(session_id)
                .map_or_else(Vec::new, |session| clone_records(&session.rows));
            TranscriptSnapshotV1 { version, records }
        })
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
        let actual_version = invoke_client_read_while_locked(self.lock_state()?, |state| {
            state.sessions.get(session_id).map_or(0, |session| session.version)
        });
        if actual_version != expected_version {
            return Ok(TranscriptRetainDispositionV1::StaleSnapshot {
                expected_version,
                actual_version,
            });
        }
        self.retain_sequences_owned(session_id, retained_sequences)?;
        let version = invoke_client_read_while_locked(self.lock_state()?, |state| {
            state.sessions.get(session_id).map_or(0, |session| session.version)
        });
        Ok(TranscriptRetainDispositionV1::Applied { version })
    }
}

mod operations;

/// Move non-clone lease authority out of rows while the mutation lane keeps
/// every row position stable. The returned indices are used to restore the
/// exact preimage if the broker atomically denies the release batch.
fn take_entry_charges<MessageId, Message>(
    entries: &mut [(u64, TranscriptEntry<MessageId, Message>)],
) -> (Vec<usize>, Vec<ManagedCharge>) {
    let mut slots = Vec::new();
    let mut charges = Vec::new();
    for (index, (_, entry)) in entries.iter_mut().enumerate() {
        if let Some(charge) = entry.active_charge.take() {
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
        entries[slot].1.active_charge = Some(charge);
    }
}

#[derive(Clone, Copy, Debug)]
enum SessionLeaseSlot {
    Key(usize),
    ActiveRow(usize, usize),
    Evidence(usize, usize),
}

fn take_session_charges<SessionId, MessageId, Message>(
    sessions: &mut BTreeMap<ClientKey<SessionId>, SessionState<MessageId, Message>>,
) -> (Vec<SessionLeaseSlot>, Vec<ManagedCharge>) {
    let mut slots = Vec::new();
    let mut charges = Vec::new();
    for (session_index, session) in sessions.values_mut().enumerate() {
        if let Some(charge) = session.key_charge.take() {
            slots.push(SessionLeaseSlot::Key(session_index));
            charges.push(charge);
        }
        for (row_index, row) in session.rows.values_mut().enumerate() {
            if let Some(charge) = row.active_charge.take() {
                slots.push(SessionLeaseSlot::ActiveRow(session_index, row_index));
                charges.push(charge);
            }
        }
        for (evidence_index, evidence) in session.sequence_evidence.values_mut().enumerate() {
            if let Some(charge) = evidence.evidence_charge.take() {
                slots.push(SessionLeaseSlot::Evidence(session_index, evidence_index));
                charges.push(charge);
            }
        }
    }
    (slots, charges)
}

fn restore_session_charges<SessionId, MessageId, Message>(
    sessions: &mut BTreeMap<ClientKey<SessionId>, SessionState<MessageId, Message>>,
    slots: &[SessionLeaseSlot],
    charges: Vec<ManagedCharge>,
) {
    for (slot, charge) in slots.iter().copied().zip(charges) {
        match slot {
            SessionLeaseSlot::Key(session_index) => {
                sessions
                    .values_mut()
                    .nth(session_index)
                    .expect("validated session slot")
                    .key_charge = Some(charge);
            }
            SessionLeaseSlot::ActiveRow(session_index, row_index) => {
                sessions
                    .values_mut()
                    .nth(session_index)
                    .expect("validated session slot")
                    .rows
                    .values_mut()
                    .nth(row_index)
                    .expect("validated row slot")
                    .active_charge = Some(charge);
            }
            SessionLeaseSlot::Evidence(session_index, evidence_index) => {
                sessions
                    .values_mut()
                    .nth(session_index)
                    .expect("validated session slot")
                    .sequence_evidence
                    .values_mut()
                    .nth(evidence_index)
                    .expect("validated evidence slot")
                    .evidence_charge = Some(charge);
            }
        }
    }
}

/// Release every charge before invoking any client-owned destructor. Retire
/// individual generic fields so a panicking Drop cannot abort a concurrent
/// unwind or prevent later rows' cleanup.
fn retire_rows<MessageId, Message>(
    rows: impl IntoIterator<Item = TranscriptEntry<MessageId, Message>>,
) {
    for row in rows {
        let TranscriptEntry { message_id, body, active_charge, .. } = row;
        drop(active_charge);
        retire_client_value(message_id);
        retire_client_value(body);
    }
}

fn retire_sessions<SessionId, MessageId, Message>(
    mut sessions: BTreeMap<ClientKey<SessionId>, SessionState<MessageId, Message>>,
) {
    for session in sessions.values_mut() {
        drop(session.key_charge.take());
        for row in session.rows.values_mut() {
            drop(row.active_charge.take());
        }
        for evidence in session.sequence_evidence.values_mut() {
            drop(evidence.evidence_charge.take());
        }
    }
    for (mut session_id, session) in sessions {
        retire_rows(session.rows.into_values());
        for (_, evidence) in session.sequence_evidence {
            drop(evidence.evidence_charge);
            retire_client_value(evidence.message_id);
        }
        for (mut message_id, _) in session.message_evidence {
            retire_client_value(message_id.take());
        }
        retire_client_value(session_id.take());
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
        state.active_row_count = 0;
        state.active_bytes = 0;
        state.evidence_count = 0;
        state.evidence_bytes = 0;
        state.session_key_bytes = 0;
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
mod tests;
