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

    fn open_session(&self, session_id: SessionId) -> MemoryResult<SessionHandle> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let mut session_id = InFlight::new(session_id);
        let key_bytes = session_key_cost(session_id.get().byte_size())?;
        let (handle, next_sequence, next_key_bytes, next_retained_bytes) = {
            let state = self.lock_state()?;
            let next_sequence = state
                .next_session_sequence
                .checked_add(1)
                .ok_or_else(|| MemoryError::capacity("transcript session sequence exhausted"))?;
            let next_key_bytes = state
                .session_key_bytes
                .checked_add(key_bytes)
                .ok_or_else(|| MemoryError::capacity("transcript session-key counter overflow"))?;
            if state.sessions.len() >= state.retention.max_session_count()
                || next_key_bytes > state.retention.max_session_key_bytes()
            {
                return Err(MemoryError::capacity("transcript open-session retention saturated"));
            }
            let next_retained_bytes =
                state.retained_bytes.checked_add(key_bytes).ok_or_else(|| {
                    MemoryError::capacity("transcript retained-byte counter overflow")
                })?;
            (
                SessionHandle::new(self.store_nonce, state.next_session_sequence),
                next_sequence,
                next_key_bytes,
                next_retained_bytes,
            )
        };
        let key_charge = self.accounting.charge(
            self.binding.class.clone(),
            self.binding.owner.clone(),
            MemoryContextKind::Session,
            ChargeKind::RetainedMetadata,
            key_bytes,
        )?;
        let mut state = self.lock_state()?;
        state.sessions.insert(
            handle,
            SessionState {
                session_id: session_id.take(),
                version: 0,
                key_bytes,
                key_charge,
                retained_bytes: key_bytes,
                rows: BTreeMap::new(),
                sequence_evidence: BTreeMap::new(),
                message_evidence: BTreeMap::new(),
            },
        );
        state.next_session_sequence = next_sequence;
        state.session_key_bytes = next_key_bytes;
        state.retained_bytes = next_retained_bytes;
        Ok(handle)
    }

    fn append(
        &self,
        request: TranscriptAppendRequestV1<SessionHandle, MessageId, Message>,
    ) -> Result<TranscriptAppendDispositionV1, TranscriptAppendErrorV1> {
        let _mutation =
            self.lifecycle.begin_mutation().map_err(TranscriptAppendErrorV1::Storage)?;
        // Caller-trait code runs while every owned caller value is guarded.
        let mut body = InFlight::new(request.body);
        let handle = request.identity.session_id;
        let message_id = InFlight::new(request.identity.message_id);
        let body_key = body.get().canonical_key();
        let body_digest = canonical_body_digest(&body_key);
        let sequence = request.identity.sequence;
        let message_id_bytes = message_id.get().byte_size();
        let active_bytes = active_row_cost(body.get().byte_size(), message_id_bytes)
            .map_err(TranscriptAppendErrorV1::Accounting)?;
        let evidence_bytes = identity_evidence_cost(message_id_bytes)
            .map_err(TranscriptAppendErrorV1::Accounting)?;
        // Identity comparison and replay decoding invoke caller Ord and
        // CanonicalKey. Finish that read-only work and release the guard before
        // any panic can unwind; the mutation permit keeps its result stable.
        enum ExistingSession {
            Present { retained_bytes: u64, version: u64 },
            Replay,
        }
        let prior = invoke_client_read_while_locked(
            self.lock_state().map_err(TranscriptAppendErrorV1::Storage)?,
            |state| -> Result<ExistingSession, TranscriptAppendErrorV1> {
                let Some(session) = state.sessions.get(&handle) else {
                    return Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                        "transcript session handle is not open in this store".to_string(),
                    )));
                };
                if let Some(evidence) = session.sequence_evidence.get(&sequence) {
                    if evidence.message_id != *message_id.get() {
                        return Err(TranscriptAppendErrorV1::SequenceConflict { sequence });
                    }
                    let exact_replay = session.rows.get(&sequence).map_or_else(
                        || evidence.body_digest == body_digest,
                        |row| row.data.body().canonical_key() == body_key,
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
            next_active_row_count,
            next_active_bytes,
            next_evidence_count,
            next_evidence_bytes,
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

            let increment = active_bytes.checked_add(evidence_bytes).ok_or_else(|| {
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
            (
                next_total,
                next_session_bytes,
                next_version,
                next_active_row_count,
                next_active_bytes,
                next_evidence_count,
                next_evidence_bytes,
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

        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(error) => {
                drop(active_charge);
                drop(evidence_charge);
                return Err(TranscriptAppendErrorV1::Storage(error));
            }
        };
        let mut active_charge = active_charge;
        let mut evidence_charge = evidence_charge;

        // Resolve every caller-keyed map descent before moving row values out
        // of their guards. The remaining inserts use vacant slots or u64 keys.
        let publication = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(session) = state.sessions.get_mut(&handle) {
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
                    data: Arc::new(TranscriptRowData {
                        message_id: Some(row_message_id.take()),
                        body: Some(body.take()),
                    }),
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
                return Err(TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                    "transcript session handle was retired during append".to_string(),
                )));
            }
            state.active_row_count = next_active_row_count;
            state.active_bytes = next_active_bytes;
            state.evidence_count = next_evidence_count;
            state.evidence_bytes = next_evidence_bytes;
            state.retained_bytes = next_total;
            Ok(TranscriptAppendDispositionV1::Applied)
        }));
        drop(state);
        // Prepublication failure leaves values in their in-flight guards.
        drop(active_charge);
        drop(evidence_charge);
        match publication {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn load(
        &self,
        handle: &SessionHandle,
    ) -> MemoryResult<Vec<TranscriptRecordV1<MessageId, Message>>>
    where
        MessageId: Clone,
        Message: Clone,
    {
        Ok(self.load_versioned_snapshot(handle)?.into_records())
    }

    fn read_page(
        &self,
        handle: &SessionHandle,
        cursor: Option<&TranscriptCursor>,
        max_rows: usize,
        max_semantic_bytes: u64,
    ) -> MemoryResult<TranscriptSnapshotV1<MessageId, Message>>
    where
        MessageId: Clone,
        Message: Clone,
    {
        SimpleSessionTranscriptStore::read_page(self, handle, cursor, max_rows, max_semantic_bytes)
    }

    fn retain_sequences(
        &self,
        handle: &SessionHandle,
        retained_sequences: &[u64],
    ) -> MemoryResult<()> {
        let _mutation = self.lifecycle.begin_mutation()?;
        self.retain_sequences_owned(handle, retained_sequences)
    }

    fn retain_sequences_if_version(
        &self,
        handle: &SessionHandle,
        expected_version: u64,
        retained_sequences: &[u64],
    ) -> MemoryResult<TranscriptRetainDispositionV1> {
        SimpleSessionTranscriptStore::retain_sequences_if_version(
            self,
            handle,
            expected_version,
            retained_sequences,
        )
    }

    fn truncate_to(&self, handle: &SessionHandle, max_messages: usize) -> MemoryResult<()> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let retained_sequences = self
            .lock_state()?
            .sessions
            .get(handle)
            .ok_or_else(|| {
                MemoryError::InvalidState(
                    "transcript session handle is not open in this store".to_string(),
                )
            })?
            .rows
            .keys()
            .copied()
            .take(max_messages)
            .collect::<Vec<_>>();
        self.retain_sequences_owned(handle, &retained_sequences)
    }

    fn retire_session(&self, handle: &SessionHandle) -> MemoryResult<()> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let (
            session,
            next_active_row_count,
            next_active_bytes,
            next_evidence_count,
            next_evidence_bytes,
            next_session_key_bytes,
            next_retained_bytes,
        ) = {
            let mut state = self.lock_state()?;
            let session = state.sessions.get(handle).ok_or_else(|| {
                MemoryError::InvalidState(
                    "transcript session handle is not open in this store".to_string(),
                )
            })?;
            if semantic_session_bytes(session)? != session.retained_bytes {
                return Err(MemoryError::InvalidState(
                    "transcript session cost counter disagrees with owned leases".to_string(),
                ));
            }
            self.require_charge_shape(&session.key_charge, "retire session key")?;
            for row in session.rows.values() {
                self.require_charge_shape(&row.active_charge, "retire active row")?;
                if Arc::strong_count(&row.data) != 1 {
                    return Err(MemoryError::operation_in_progress("transcript read"));
                }
            }
            for evidence in session.sequence_evidence.values() {
                self.require_charge_shape(&evidence.evidence_charge, "retire evidence")?;
            }
            let active_bytes = session.rows.values().try_fold(0u64, |sum, row| {
                sum.checked_add(row.active_bytes)
                    .ok_or_else(|| MemoryError::capacity("transcript retire active-byte overflow"))
            })?;
            let evidence_bytes =
                session.sequence_evidence.values().try_fold(0u64, |sum, evidence| {
                    sum.checked_add(evidence.evidence_bytes).ok_or_else(|| {
                        MemoryError::capacity("transcript retire evidence-byte overflow")
                    })
                })?;
            let next_active_row_count =
                state.active_row_count.checked_sub(session.rows.len()).ok_or_else(|| {
                    MemoryError::InvalidState("transcript active-row count underflow".to_string())
                })?;
            let next_active_bytes =
                state.active_bytes.checked_sub(active_bytes).ok_or_else(|| {
                    MemoryError::InvalidState("transcript active-byte underflow".to_string())
                })?;
            let next_evidence_count =
                state.evidence_count.checked_sub(session.sequence_evidence.len()).ok_or_else(
                    || MemoryError::InvalidState("transcript evidence-count underflow".to_string()),
                )?;
            let next_evidence_bytes =
                state.evidence_bytes.checked_sub(evidence_bytes).ok_or_else(|| {
                    MemoryError::InvalidState("transcript evidence-byte underflow".to_string())
                })?;
            let next_session_key_bytes =
                state.session_key_bytes.checked_sub(session.key_bytes).ok_or_else(|| {
                    MemoryError::InvalidState("transcript session-key underflow".to_string())
                })?;
            let next_retained_bytes =
                state.retained_bytes.checked_sub(session.retained_bytes).ok_or_else(|| {
                    MemoryError::InvalidState("transcript retained-byte underflow".to_string())
                })?;
            let session =
                state.sessions.remove(handle).expect("validated handle remains in mutation lane");
            (
                session,
                next_active_row_count,
                next_active_bytes,
                next_evidence_count,
                next_evidence_bytes,
                next_session_key_bytes,
                next_retained_bytes,
            )
        };
        let mut retired = BTreeMap::from([(*handle, session)]);
        let (slots, mut charges) = take_session_charges(&mut retired);
        if let Err(error) = self.accounting.release_many(&mut charges) {
            restore_session_charges(&mut retired, &slots, charges);
            let session = retired.remove(handle).expect("retire rollback owns session");
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .sessions
                .insert(*handle, session);
            return Err(error);
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active_row_count = next_active_row_count;
        state.active_bytes = next_active_bytes;
        state.evidence_count = next_evidence_count;
        state.evidence_bytes = next_evidence_bytes;
        state.session_key_bytes = next_session_key_bytes;
        state.retained_bytes = next_retained_bytes;
        drop(state);
        retire_sessions(retired);
        Ok(())
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
