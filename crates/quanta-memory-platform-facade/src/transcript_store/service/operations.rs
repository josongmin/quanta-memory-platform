use super::*;

impl<SessionId, MessageId, Message, Mode> SessionTranscriptStore<SessionId, MessageId, Message>
    for SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode>
where
    SessionId: Clone + Ord + ByteSized,
    MessageId: Clone + Ord + ByteSized,
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
        // Caller-trait code runs while every owned caller value is guarded.
        let mut body = InFlight::new(request.body);
        let session_id = InFlight::new(request.identity.session_id);
        let message_id = InFlight::new(request.identity.message_id);
        let body_key = body.get().canonical_key();
        let body_digest = canonical_body_digest(&body_key);
        let sequence = request.identity.sequence;
        let message_id_bytes = message_id.get().byte_size();
        let active_bytes = active_row_cost(body.get().byte_size(), message_id_bytes)
            .map_err(TranscriptAppendErrorV1::Accounting)?;
        let evidence_bytes = identity_evidence_cost(message_id_bytes)
            .map_err(TranscriptAppendErrorV1::Accounting)?;
        let key_bytes = session_key_cost(session_id.get().byte_size())
            .map_err(TranscriptAppendErrorV1::Accounting)?;
        // Identity comparison and replay decoding invoke caller Ord and
        // CanonicalKey. Finish that read-only work and release the guard before
        // any panic can unwind; the mutation permit keeps its result stable.
        enum ExistingSession {
            New,
            Present { retained_bytes: u64, version: u64 },
            Replay,
        }
        let prior = invoke_client_read_while_locked(
            self.lock_state().map_err(TranscriptAppendErrorV1::Storage)?,
            |state| -> Result<ExistingSession, TranscriptAppendErrorV1> {
                let Some(session) = state.sessions.get(session_id.get()) else {
                    return Ok(ExistingSession::New);
                };
                if let Some(evidence) = session.sequence_evidence.get(&sequence) {
                    if evidence.message_id != *message_id.get() {
                        return Err(TranscriptAppendErrorV1::SequenceConflict { sequence });
                    }
                    let exact_replay = session.rows.get(&sequence).map_or_else(
                        || evidence.body_digest == body_digest,
                        |row| row.body.canonical_key() == body_key,
                    );
                    return if exact_replay {
                        Ok(ExistingSession::Replay)
                    } else {
                        Err(TranscriptAppendErrorV1::PayloadConflict { sequence })
                    };
                }
                if let Some(evidence) = session.message_evidence.get(message_id.get()) {
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
                Ok(ExistingSession::Present {
                    retained_bytes: session.retained_bytes,
                    version: session.version,
                })
            },
        )?;
        if matches!(prior, ExistingSession::Replay) {
            return Ok(TranscriptAppendDispositionV1::Replayed);
        }
        let (
            next_total,
            next_session_bytes,
            next_version,
            new_session,
            next_active_row_count,
            next_active_bytes,
            next_evidence_count,
            next_evidence_bytes,
            next_session_key_bytes,
        ) = {
            let state = self.lock_state().map_err(TranscriptAppendErrorV1::Storage)?;
            let next_evidence_count = state.evidence_count.checked_add(1).ok_or_else(|| {
                TranscriptAppendErrorV1::Accounting(MemoryError::capacity(
                    "transcript evidence count overflow",
                ))
            })?;
            let next_evidence_bytes =
                state.evidence_bytes.checked_add(evidence_bytes).ok_or_else(|| {
                    TranscriptAppendErrorV1::Accounting(MemoryError::capacity(
                        "transcript evidence-byte counter overflow",
                    ))
                })?;
            if next_evidence_count > state.retention.max_evidence_count()
                || next_evidence_bytes > state.retention.max_evidence_bytes()
            {
                let error = TranscriptAppendErrorV1::RetentionSaturated {
                    evidence_count: state.evidence_count,
                    max_evidence_count: state.retention.max_evidence_count(),
                    evidence_bytes: state.evidence_bytes,
                    requested_evidence_bytes: evidence_bytes,
                    max_evidence_bytes: state.retention.max_evidence_bytes(),
                };
                drop(state);
                self.accounting.emit(MemoryEvent::new(
                    MemoryOperation::TombstoneSaturation,
                    MemoryOutcome::CapacityDenied,
                    None,
                ));
                return Err(error);
            }

            let new_session = matches!(prior, ExistingSession::New);
            let increment = active_bytes
                .checked_add(evidence_bytes)
                .and_then(|sum| sum.checked_add(if new_session { key_bytes } else { 0 }))
                .ok_or_else(|| {
                    TranscriptAppendErrorV1::Accounting(MemoryError::capacity(
                        "transcript append cost overflow",
                    ))
                })?;
            let next_total = state.retained_bytes.checked_add(increment).ok_or_else(|| {
                TranscriptAppendErrorV1::Accounting(MemoryError::capacity(
                    "transcript retained-byte counter overflow",
                ))
            })?;
            let next_session_bytes = match prior {
                ExistingSession::New => 0,
                ExistingSession::Present { retained_bytes, .. } => retained_bytes,
                ExistingSession::Replay => unreachable!("replay returned before planning"),
            }
            .checked_add(increment)
            .ok_or_else(|| {
                TranscriptAppendErrorV1::Accounting(MemoryError::capacity(
                    "transcript session-byte counter overflow",
                ))
            })?;
            let next_version = match prior {
                ExistingSession::New => Ok(1),
                ExistingSession::Present { version, .. } => {
                    version.checked_add(1).ok_or_else(|| {
                        TranscriptAppendErrorV1::Storage(MemoryError::capacity(
                            "transcript session version overflow",
                        ))
                    })
                }
                ExistingSession::Replay => unreachable!("replay returned before planning"),
            }?;
            let next_active_row_count = state.active_row_count.checked_add(1).ok_or_else(|| {
                TranscriptAppendErrorV1::Accounting(MemoryError::capacity(
                    "transcript active-row count overflow",
                ))
            })?;
            let next_active_bytes =
                state.active_bytes.checked_add(active_bytes).ok_or_else(|| {
                    TranscriptAppendErrorV1::Accounting(MemoryError::capacity(
                        "transcript active-byte counter overflow",
                    ))
                })?;
            let next_session_key_bytes = state
                .session_key_bytes
                .checked_add(if new_session { key_bytes } else { 0 })
                .ok_or_else(|| {
                    TranscriptAppendErrorV1::Accounting(MemoryError::capacity(
                        "transcript session-key counter overflow",
                    ))
                })?;
            (
                next_total,
                next_session_bytes,
                next_version,
                new_session,
                next_active_row_count,
                next_active_bytes,
                next_evidence_count,
                next_evidence_bytes,
                next_session_key_bytes,
            )
        };

        // Client Clone may panic or reenter. Complete both copies before
        // acquiring a charge or mutating any store-owned state.
        let mut row_message_id = InFlight::new(message_id.get().clone());
        let mut evidence_message_id = InFlight::new(message_id.get().clone());
        // Finish all caller Clone operations before assembling a row or
        // acquiring charges. Both owned map keys contain hostile Drop during
        // their later Ord descents.
        let mut evidence_key = InFlight::new(message_id.get().clone());
        let mut new_session_key =
            if new_session { Some(InFlight::new(session_id.get().clone())) } else { None };

        let active_charge = self
            .accounting
            .charge(
                self.binding.class.clone(),
                self.binding.owner.clone(),
                MemoryContextKind::Session,
                ChargeKind::RetainedPayload,
                active_bytes,
            )
            .map_err(TranscriptAppendErrorV1::Accounting)?;
        let evidence_charge = self
            .accounting
            .charge(
                self.binding.class.clone(),
                self.binding.owner.clone(),
                MemoryContextKind::Session,
                ChargeKind::RetainedMetadata,
                evidence_bytes,
            )
            .map_err(TranscriptAppendErrorV1::Accounting)?;
        let key_charge = if new_session {
            self.accounting
                .charge(
                    self.binding.class.clone(),
                    self.binding.owner.clone(),
                    MemoryContextKind::Session,
                    ChargeKind::RetainedMetadata,
                    key_bytes,
                )
                .map_err(TranscriptAppendErrorV1::Accounting)?
        } else {
            None
        };

        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(error) => {
                drop(active_charge);
                drop(evidence_charge);
                drop(key_charge);
                return Err(TranscriptAppendErrorV1::Storage(error));
            }
        };
        let mut active_charge = active_charge;
        let mut evidence_charge = evidence_charge;
        let mut key_charge = key_charge;

        // Resolve every caller-keyed map descent before moving row values out
        // of their guards. The remaining inserts use vacant slots or u64 keys.
        let publication = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(session) = state.sessions.get_mut(session_id.get()) {
                if session.rows.contains_key(&sequence)
                    || session.sequence_evidence.contains_key(&sequence)
                {
                    return Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                        "transcript sequence index changed during append publication".to_string(),
                    )));
                }
                if session.message_evidence.contains_key(message_id.get()) {
                    return Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                        "transcript message-id index changed during append publication".to_string(),
                    )));
                }
                // Resolve the message-evidence slot FIRST: the only caller
                // trait between the guard releases and the counter commit is
                // the owned-key Ord descent here, before anything is
                // published. VacantEntry::insert below is trait-free.
                let Entry::Vacant(message_slot) =
                    session.message_evidence.entry(ClientKey::new(evidence_key.take()))
                else {
                    return Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                        "transcript message-id index changed during append publication".to_string(),
                    )));
                };
                let row = TranscriptEntry {
                    sequence,
                    message_id: row_message_id.take(),
                    body: body.take(),
                    active_bytes,
                    active_charge: active_charge.take(),
                };
                let sequence_evidence = SequenceEvidence {
                    message_id: evidence_message_id.take(),
                    body_digest,
                    evidence_bytes,
                    evidence_charge: evidence_charge.take(),
                };
                session.rows.insert(sequence, row);
                session.sequence_evidence.insert(sequence, sequence_evidence);
                message_slot.insert(MessageEvidence { sequence, body_digest });
                session.version = next_version;
                session.retained_bytes = next_session_bytes;
            } else {
                // Resolve the session's owned-key descent before assembling
                // any caller-owned row. The key itself contains hostile Drop
                // if its Ord panics during BTreeMap::entry.
                let Entry::Vacant(session_slot) = state.sessions.entry(ClientKey::new(
                    new_session_key.as_mut().expect("planned session key").take(),
                )) else {
                    return Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                        "transcript session index changed during append publication".to_string(),
                    )));
                };
                let mut session = SessionState {
                    key_bytes,
                    key_charge: key_charge.take(),
                    retained_bytes: next_session_bytes,
                    ..SessionState::default()
                };
                // Same discipline as the existing-session branch: resolve
                // the evidence slot before any caller value moves into the
                // session (on a fresh session the map is empty, so the
                // descent is trivially trait-free).
                let Entry::Vacant(message_slot) =
                    session.message_evidence.entry(ClientKey::new(evidence_key.take()))
                else {
                    return Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                        "transcript message-id index changed during append publication".to_string(),
                    )));
                };
                let row = TranscriptEntry {
                    sequence,
                    message_id: row_message_id.take(),
                    body: body.take(),
                    active_bytes,
                    active_charge: active_charge.take(),
                };
                let sequence_evidence = SequenceEvidence {
                    message_id: evidence_message_id.take(),
                    body_digest,
                    evidence_bytes,
                    evidence_charge: evidence_charge.take(),
                };
                session.rows.insert(sequence, row);
                session.sequence_evidence.insert(sequence, sequence_evidence);
                message_slot.insert(MessageEvidence { sequence, body_digest });
                session.version = next_version;
                session_slot.insert(session);
            }
            state.active_row_count = next_active_row_count;
            state.active_bytes = next_active_bytes;
            state.evidence_count = next_evidence_count;
            state.evidence_bytes = next_evidence_bytes;
            state.session_key_bytes = next_session_key_bytes;
            state.retained_bytes = next_total;
            Ok(TranscriptAppendDispositionV1::Applied)
        }));
        drop(state);
        // Prepublication failure leaves values in their in-flight guards.
        drop(active_charge);
        drop(evidence_charge);
        drop(key_charge);
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
        self.lifecycle.with_read_state(&self.state, |state| {
            state
                .sessions
                .get(session_id)
                .map_or_else(Vec::new, |session| clone_records(&session.rows))
        })
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
        let retained_sequences = invoke_client_read_while_locked(self.lock_state()?, |state| {
            state.sessions.get(session_id).map_or_else(Vec::new, |session| {
                session.rows.keys().copied().take(max_messages).collect()
            })
        });
        self.retain_sequences_owned(session_id, &retained_sequences)
    }

    fn stats(&self) -> MemoryResult<StoreStats> {
        self.checked_stats()
    }

    fn retention_stats(&self) -> MemoryResult<TranscriptRetentionStatsV2> {
        SimpleSessionTranscriptStore::retention_stats(self)
    }

    fn close(&self) -> MemoryResult<()> {
        SimpleSessionTranscriptStore::close(self)
    }
}
