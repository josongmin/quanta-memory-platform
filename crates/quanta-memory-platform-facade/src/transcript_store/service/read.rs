use super::*;

impl<SessionId, MessageId, Message, Mode>
    SimpleSessionTranscriptStore<SessionId, MessageId, Message, Mode>
where
    SessionId: Clone + Ord + ByteSized,
    MessageId: Clone + Ord + ByteSized,
    Message: Clone + ByteSized + CanonicalKey,
{
    /// Pin active rows under the mutex, then run caller Clone outside it.
    pub fn load_versioned_snapshot(
        &self,
        handle: &SessionHandle,
    ) -> MemoryResult<TranscriptSnapshotV1<MessageId, Message>> {
        let (version, selected) = self.lifecycle.with_read_state(&self.state, |state| {
            let session = state.sessions.get(handle).ok_or_else(|| {
                MemoryError::InvalidState(
                    "transcript session handle is not open in this store".to_string(),
                )
            })?;
            Ok::<_, MemoryError>((
                session.version,
                session
                    .rows
                    .iter()
                    .map(|(sequence, row)| (*sequence, Arc::clone(&row.data)))
                    .collect::<Vec<_>>(),
            ))
        })??;
        let records = clone_records(&selected);
        Ok(TranscriptSnapshotV1 { handle: *handle, version, records, next_cursor: None })
    }

    /// Bounded active-row page. The byte limit counts each selected row's
    /// semantic retained cost; caller-owned result copies are not charged.
    pub fn read_page(
        &self,
        handle: &SessionHandle,
        cursor: Option<&TranscriptCursor>,
        max_rows: usize,
        max_semantic_bytes: u64,
    ) -> MemoryResult<TranscriptSnapshotV1<MessageId, Message>> {
        if max_rows == 0 || max_semantic_bytes == 0 {
            return Err(MemoryError::capacity("transcript page limits must be positive"));
        }
        let (version, selected, next_cursor) =
            self.lifecycle.with_read_state(&self.state, |state| {
                let session = state.sessions.get(handle).ok_or_else(|| {
                    MemoryError::InvalidState(
                        "transcript session handle is not open in this store".to_string(),
                    )
                })?;
                if let Some(cursor) = cursor {
                    if cursor.handle != *handle || cursor.version != session.version {
                        return Err(MemoryError::InvalidState(
                            "foreign or stale transcript cursor".to_string(),
                        ));
                    }
                }
                let mut selected = Vec::new();
                let mut used_bytes = 0u64;
                let mut has_more = false;
                let mut visit = |sequence: &u64,
                                 row: &TranscriptEntry<MessageId, Message>|
                 -> MemoryResult<bool> {
                    if selected.len() >= max_rows {
                        has_more = true;
                        return Ok(false);
                    }
                    let next_bytes = used_bytes.checked_add(row.active_bytes).ok_or_else(|| {
                        MemoryError::capacity("transcript page semantic-byte overflow")
                    })?;
                    if next_bytes > max_semantic_bytes {
                        if selected.is_empty() {
                            return Err(MemoryError::capacity(
                                "transcript row exceeds page semantic-byte limit",
                            ));
                        }
                        has_more = true;
                        return Ok(false);
                    }
                    used_bytes = next_bytes;
                    selected.push((*sequence, Arc::clone(&row.data)));
                    Ok(true)
                };
                if let Some(cursor) = cursor {
                    for (sequence, row) in session.rows.range((
                        std::ops::Bound::Excluded(cursor.last_sequence),
                        std::ops::Bound::Unbounded,
                    )) {
                        if !visit(sequence, row)? {
                            break;
                        }
                    }
                } else {
                    for (sequence, row) in &session.rows {
                        if !visit(sequence, row)? {
                            break;
                        }
                    }
                }
                let next_cursor = if has_more {
                    selected.last().map(|(last_sequence, _)| TranscriptCursor {
                        handle: *handle,
                        version: session.version,
                        last_sequence: *last_sequence,
                    })
                } else {
                    None
                };
                Ok::<_, MemoryError>((session.version, selected, next_cursor))
            })??;
        let records = clone_records(&selected);
        Ok(TranscriptSnapshotV1 { handle: *handle, version, records, next_cursor })
    }

    /// Retain rows only if no append or retention mutation occurred after the
    /// supplied snapshot. Stale snapshots are reported without mutation.
    pub fn retain_sequences_if_version(
        &self,
        handle: &SessionHandle,
        expected_version: u64,
        retained_sequences: &[u64],
    ) -> MemoryResult<TranscriptRetainDispositionV1> {
        let _mutation = self.lifecycle.begin_mutation()?;
        let actual_version = self
            .lock_state()?
            .sessions
            .get(handle)
            .ok_or_else(|| {
                MemoryError::InvalidState(
                    "transcript session handle is not open in this store".to_string(),
                )
            })?
            .version;
        if actual_version != expected_version {
            return Ok(TranscriptRetainDispositionV1::StaleSnapshot {
                expected_version,
                actual_version,
            });
        }
        self.retain_sequences_owned(handle, retained_sequences)?;
        let version = self
            .lock_state()?
            .sessions
            .get(handle)
            .ok_or_else(|| {
                MemoryError::InvalidState(
                    "transcript session handle is not open in this store".to_string(),
                )
            })?
            .version;
        Ok(TranscriptRetainDispositionV1::Applied { version })
    }
}
