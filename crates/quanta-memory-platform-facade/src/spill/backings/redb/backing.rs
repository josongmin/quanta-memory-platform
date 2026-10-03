use super::*;

impl RedbSpillBacking {
    /// Open (creating if absent) a redb-backed durable spill tier at `path`. A
    /// backend failure is surfaced as a neutral [`MemoryError`], never a leaked
    /// `redb` error type.
    ///
    /// A consistent byte copy/reflink/snapshot retains the persisted logical
    /// lineage and is suitable only as a quiesced replacement or backup. To
    /// create an independently writable backend, create a fresh database and
    /// transfer records through the public durable-spill protocol; do not mount
    /// copied lineages concurrently.
    pub fn create(path: impl AsRef<Path>) -> MemoryResult<Self> {
        Self::create_with_limits(path, RedbSpillLimits::default())
    }

    /// Open with explicit logical payload/record, database file-length and
    /// Redb cache limits. The file cap does not reserve volume capacity:
    /// deployments must still qualify ENOSPC behavior on their filesystem.
    pub fn create_with_limits(
        path: impl AsRef<Path>,
        limits: RedbSpillLimits,
    ) -> MemoryResult<Self> {
        let limits = limits.validate()?;
        let mut builder = redb::Builder::new();
        builder.set_cache_size(limits.cache_bytes);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|error| MemoryError::build(format!("redb spill file open failed: {error}")))?;
        let backend = CappedFileBackend::new(file, limits.max_database_file_bytes)?;
        let database = builder
            .create_with_backend(backend)
            .map_err(|e| MemoryError::build(format!("redb spill backing open failed: {e}")))?;
        let schema = Self::inspect_schema(&database)?;
        let backend_id = Self::inspect_backend_identity(&database, schema)?;
        Self::validate_current_schema_layout(&database, schema)?;
        Self::initialize(database, limits, schema, backend_id)
    }

    pub(super) fn inspect_schema(database: &Database) -> MemoryResult<PersistedSchema> {
        let read = database.begin_read().map_err(|e| {
            MemoryError::build(format!("redb schema inspection transaction failed: {e}"))
        })?;
        let meta = match read.open_table(SPILL_META) {
            Ok(meta) => meta,
            Err(redb::TableError::TableDoesNotExist(_)) => {
                let state_exists = match read.open_table(SPILL_RECORD_STATE) {
                    Ok(_) => true,
                    Err(redb::TableError::TableDoesNotExist(_)) => false,
                    Err(error) => {
                        return Err(MemoryError::build(format!(
                            "redb disposition evidence open failed: {error}"
                        )));
                    }
                };
                let proof_exists = match read.open_table(SPILL_FINALIZED_BY) {
                    Ok(_) => true,
                    Err(redb::TableError::TableDoesNotExist(_)) => false,
                    Err(error) => {
                        return Err(MemoryError::build(format!(
                            "redb finalize-proof evidence open failed: {error}"
                        )));
                    }
                };
                if state_exists || proof_exists {
                    return Err(MemoryError::InvalidState(
                        "redb metadata table is missing from a schema-2-shaped database".into(),
                    ));
                }
                return Ok(PersistedSchema::LegacyV1);
            }
            Err(error) => {
                return Err(MemoryError::build(format!(
                    "redb schema metadata open failed: {error}"
                )));
            }
        };
        let version = meta
            .get(SCHEMA_VERSION)
            .map_err(|e| MemoryError::build(format!("redb schema version read failed: {e}")))?
            .map(|value| value.value());
        match version {
            None => {
                let has_v2_metadata = [
                    BACKEND_ID_LO,
                    BACKEND_ID_HI,
                    RETAINED_PAYLOAD_BYTES,
                    ACTIVE_RESTORE_GENERATION,
                    RESTORE_CLAIM_GENERATION,
                ]
                .into_iter()
                .try_fold(false, |found, key| {
                    meta.get(key)
                        .map(|value| found || value.is_some())
                        .map_err(|e| {
                            MemoryError::build(format!(
                                "redb schema evidence read failed for {key}: {e}"
                            ))
                        })
                })?;
                let has_v2_state_table = match read.open_table(SPILL_RECORD_STATE) {
                    Ok(_) => true,
                    Err(redb::TableError::TableDoesNotExist(_)) => false,
                    Err(error) => {
                        return Err(MemoryError::build(format!(
                            "redb disposition evidence open failed: {error}"
                        )));
                    }
                };
                let has_v2_proof_table = match read.open_table(SPILL_FINALIZED_BY) {
                    Ok(_) => true,
                    Err(redb::TableError::TableDoesNotExist(_)) => false,
                    Err(error) => {
                        return Err(MemoryError::build(format!(
                            "redb finalize-proof evidence open failed: {error}"
                        )));
                    }
                };
                if has_v2_metadata || has_v2_state_table || has_v2_proof_table {
                    return Err(MemoryError::InvalidState(
                        "redb schema version is missing from a schema-2-shaped database"
                            .into(),
                    ));
                }
                Ok(PersistedSchema::LegacyV1)
            }
            Some(SCHEMA_VERSION_V2) => Ok(PersistedSchema::V2),
            Some(version) => Err(MemoryError::Unsupported(format!(
                "unsupported redb spill schema version {version}; expected {SCHEMA_VERSION_V2} or an absent legacy version"
            ))),
        }
    }

    pub(super) fn inspect_backend_identity(
        database: &Database,
        schema: PersistedSchema,
    ) -> MemoryResult<Option<u128>> {
        let read = database.begin_read().map_err(|e| {
            MemoryError::build(format!("redb backend identity transaction failed: {e}"))
        })?;
        let meta = match read.open_table(SPILL_META) {
            Ok(meta) => meta,
            Err(redb::TableError::TableDoesNotExist(_)) if schema == PersistedSchema::LegacyV1 => {
                return Ok(None);
            }
            Err(error) => {
                return Err(MemoryError::build(format!(
                    "redb backend identity metadata open failed: {error}"
                )));
            }
        };
        let low = meta
            .get(BACKEND_ID_LO)
            .map_err(|e| MemoryError::build(format!("redb backend identity read failed: {e}")))?
            .map(|value| value.value());
        let high = meta
            .get(BACKEND_ID_HI)
            .map_err(|e| MemoryError::build(format!("redb backend identity read failed: {e}")))?
            .map(|value| value.value());
        match (low, high, schema) {
            (Some(low), Some(high), _) => Ok(Some(u128::from(high) << 64 | u128::from(low))),
            (None, None, PersistedSchema::LegacyV1) => Ok(None),
            (None, None, PersistedSchema::V2) => {
                Err(MemoryError::InvalidState("schema-2 redb backend identity is missing".into()))
            }
            _ => Err(MemoryError::InvalidState(
                "redb backend identity is incomplete; refusing to replace a surviving half".into(),
            )),
        }
    }

    pub(super) fn validate_current_schema_layout(
        database: &Database,
        schema: PersistedSchema,
    ) -> MemoryResult<()> {
        if schema == PersistedSchema::LegacyV1 {
            return Ok(());
        }
        let read = database.begin_read().map_err(|e| {
            MemoryError::build(format!("redb schema layout transaction failed: {e}"))
        })?;
        let _queue = read.open_table(SPILL_QUEUE).map_err(|e| {
            MemoryError::InvalidState(format!("schema-2 redb spill queue is missing: {e}"))
        })?;
        let _recovery = read.open_table(SPILL_RECOVERY).map_err(|e| {
            MemoryError::InvalidState(format!("schema-2 redb recovery table is missing: {e}"))
        })?;
        let _states = read.open_table(SPILL_RECORD_STATE).map_err(|e| {
            MemoryError::InvalidState(format!("schema-2 redb disposition table is missing: {e}"))
        })?;
        let _proofs = read.open_table(SPILL_FINALIZED_BY).map_err(|e| {
            MemoryError::InvalidState(format!("schema-2 redb finalize-proof table is missing: {e}"))
        })?;
        let meta = read.open_table(SPILL_META).map_err(|e| {
            MemoryError::InvalidState(format!("schema-2 redb metadata table is missing: {e}"))
        })?;
        if meta
            .get(NEXT_SEQUENCE)
            .map_err(|e| MemoryError::build(format!("redb spill metadata read failed: {e}")))?
            .is_none()
        {
            return Err(MemoryError::InvalidState(
                "schema-2 redb sequence frontier is missing".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn initialize(
        database: Database,
        limits: RedbSpillLimits,
        schema: PersistedSchema,
        persisted_backend_id: Option<u128>,
    ) -> MemoryResult<Self> {
        let txn = database.begin_write().map_err(|e| {
            MemoryError::build(format!("redb spill metadata transaction failed: {e}"))
        })?;
        let backend_id = {
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::build(format!("redb spill metadata initialization failed: {e}"))
            })?;
            let queue = txn.open_table(SPILL_QUEUE).map_err(|e| {
                MemoryError::build(format!("redb spill queue initialization failed: {e}"))
            })?;
            let recovery = txn.open_table(SPILL_RECOVERY).map_err(|e| {
                MemoryError::build(format!("redb spill recovery initialization failed: {e}"))
            })?;
            // Ensure every table exists so later read transactions never meet
            // a missing table: the disposition table (schema 2) and the
            // finalize-proof table (single-owner claims) are created here and
            // populated by migration or by later writes.
            let mut states = txn.open_table(SPILL_RECORD_STATE).map_err(|e| {
                MemoryError::build(format!("redb record disposition initialization failed: {e}"))
            })?;
            let _finalized_by = txn.open_table(SPILL_FINALIZED_BY).map_err(|e| {
                MemoryError::build(format!("redb finalize proof initialization failed: {e}"))
            })?;
            let records = queue
                .len()
                .and_then(|queued| recovery.len().map(|recovering| (queued, recovering)))
                .map_err(|e| MemoryError::build(format!("redb retained count failed: {e}")))?;
            let count = records
                .0
                .checked_add(records.1)
                .ok_or_else(|| MemoryError::capacity("redb retained record count exceeds u64"))?;
            if count > limits.max_retained_records {
                return Err(MemoryError::capacity(format!(
                    "existing redb spill inventory has {count} records, above configured {}-record limit",
                    limits.max_retained_records
                )));
            }
            // Verify every persisted payload before adopting the configured
            // per-record limit. The scan is bounded by max_retained_records
            // above and does not copy payloads into process memory. It also
            // checks the transactional usage counter rather than trusting a
            // stale or corrupted value to authorize later writes.
            let mut payload_bytes = 0u64;
            for (table, expected_disposition) in
                [(&queue, RecordDisposition::Queued), (&recovery, RecordDisposition::Recovery)]
            {
                for entry in table.iter().map_err(|e| {
                    MemoryError::build(format!("redb payload usage scan failed: {e}"))
                })? {
                    let (key, value) = entry.map_err(|e| {
                        MemoryError::build(format!("redb payload usage scan failed: {e}"))
                    })?;
                    if schema == PersistedSchema::V2 {
                        let disposition = states
                            .get(key.value())
                            .map_err(|e| {
                                MemoryError::build(format!(
                                    "redb record disposition validation failed: {e}"
                                ))
                            })?
                            .map(|value| RecordDisposition::decode(value.value()))
                            .transpose()?;
                        if disposition != Some(expected_disposition) {
                            return Err(MemoryError::InvalidState(format!(
                                "schema-2 durable record {} has payload disposition {disposition:?}, expected {expected_disposition:?}",
                                key.value()
                            )));
                        }
                    }
                    let len = limits.check_record(value.value().len())?;
                    payload_bytes = payload_bytes.checked_add(len).ok_or_else(|| {
                        MemoryError::capacity("redb retained payload bytes exceed u64")
                    })?;
                    if payload_bytes > limits.max_retained_payload_bytes {
                        return Err(MemoryError::capacity(format!(
                            "existing redb spill inventory has {payload_bytes} payload bytes, above configured {}-byte limit",
                            limits.max_retained_payload_bytes
                        )));
                    }
                }
            }
            let recorded_payload_bytes = meta
                .get(RETAINED_PAYLOAD_BYTES)
                .map_err(|e| MemoryError::build(format!("redb payload usage read failed: {e}")))?
                .map(|value| value.value());
            match recorded_payload_bytes {
                Some(recorded) if recorded != payload_bytes => {
                    return Err(MemoryError::InvalidState(format!(
                        "redb retained payload usage {recorded} disagrees with scanned {payload_bytes} bytes"
                    )));
                }
                None => {
                    meta.insert(RETAINED_PAYLOAD_BYTES, payload_bytes).map_err(|e| {
                        MemoryError::build(format!("redb payload usage seed failed: {e}"))
                    })?;
                }
                _ => {}
            }
            if meta
                .get(NEXT_SEQUENCE)
                .map_err(|e| MemoryError::build(format!("redb spill metadata read failed: {e}")))?
                .is_none()
            {
                let next = match queue
                    .last()
                    .map_err(|e| {
                        MemoryError::build(format!("redb spill migration scan failed: {e}"))
                    })?
                    .map(|(key, _)| key.value())
                {
                    Some(last) => last.checked_add(1).ok_or_else(|| {
                        MemoryError::capacity("redb spill queue cannot migrate a u64::MAX sequence")
                    })?,
                    None => 0,
                };
                meta.insert(NEXT_SEQUENCE, next).map_err(|e| {
                    MemoryError::build(format!("redb spill metadata seed failed: {e}"))
                })?;
            }
            // Backend identity (FUP-002): mint once, persist in the same
            // transaction, then cache. Legacy files without an identity adopt
            // one here; it is stable from this open onward.
            let backend_id = if let Some(backend_id) = persisted_backend_id {
                backend_id
            } else {
                let backend_id = generate_backend_id();
                meta.insert(BACKEND_ID_LO, backend_id as u64).map_err(|e| {
                    MemoryError::build(format!("redb backend identity seed failed: {e}"))
                })?;
                meta.insert(BACKEND_ID_HI, (backend_id >> 64) as u64).map_err(|e| {
                    MemoryError::build(format!("redb backend identity seed failed: {e}"))
                })?;
                backend_id
            };
            // Schema-2 migration: convert only explicit payload evidence
            // (queue/recovery rows) into exact disposition rows, in the same
            // transaction that stamps the schema version. Ids below the next
            // sequence with no payload evidence are genuinely ambiguous
            // (reserved hole vs. consumed record) and are deliberately left
            // without a disposition row, so every later resolution fails closed
            // with an explicit unresolved error instead of guessing from the
            // legacy watermark. The legacy watermark itself is preserved
            // untouched and never consulted again.
            if schema == PersistedSchema::LegacyV1 {
                for entry in queue.iter().map_err(|e| {
                    MemoryError::build(format!("redb spill migration scan failed: {e}"))
                })? {
                    let (key, _) = entry.map_err(|e| {
                        MemoryError::build(format!("redb spill migration scan failed: {e}"))
                    })?;
                    states.insert(key.value(), RecordDisposition::Queued as u8).map_err(|e| {
                        MemoryError::build(format!("redb record disposition migration failed: {e}"))
                    })?;
                }
                for entry in recovery.iter().map_err(|e| {
                    MemoryError::build(format!("redb recovery migration scan failed: {e}"))
                })? {
                    let (key, _) = entry.map_err(|e| {
                        MemoryError::build(format!("redb recovery migration scan failed: {e}"))
                    })?;
                    states.insert(key.value(), RecordDisposition::Recovery as u8).map_err(|e| {
                        MemoryError::build(format!("redb record disposition migration failed: {e}"))
                    })?;
                }
                meta.insert(SCHEMA_VERSION, SCHEMA_VERSION_V2).map_err(|e| {
                    MemoryError::build(format!("redb schema version migration failed: {e}"))
                })?;
            }
            backend_id
        };
        txn.commit()
            .map_err(|e| MemoryError::build(format!("redb spill metadata commit failed: {e}")))?;
        Ok(Self {
            database,
            limits,
            restore_budget_used: Arc::new(AtomicU64::new(0)),
            restore_buffers_used: Arc::new(AtomicU64::new(0)),
            backend_id,
            #[cfg(test)]
            faults: RedbFaults::default(),
        })
    }

    pub(super) fn copy_restore_bytes(
        &self,
        bytes: &[u8],
    ) -> MemoryResult<(Vec<u8>, RestoreReadLease)> {
        let len = self.limits.check_record(bytes.len())?;
        let lease = RestoreReadLease::acquire(
            Arc::clone(&self.restore_budget_used),
            Arc::clone(&self.restore_buffers_used),
            len,
            self.limits.max_inflight_restore_bytes,
            self.limits.max_inflight_restore_buffers,
        )?;
        let mut copy = Vec::new();
        copy.try_reserve_exact(bytes.len()).map_err(|error| {
            MemoryError::capacity(format!("redb restore staging allocation failed: {error}"))
        })?;
        copy.extend_from_slice(bytes);
        Ok((copy, lease))
    }

    #[cfg(test)]
    pub(super) fn inject_fail_write_before_transaction(&self) {
        self.faults.fail_write_before_transaction.store(true, AtomicOrdering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn inject_lost_write_commit_ack(&self) {
        self.faults.lose_write_commit_ack.store(true, AtomicOrdering::SeqCst);
    }

    /// Test-only restore-commit ACK-loss injection, shared with in-crate store
    /// tests so the ambiguous-commit fence path runs against real claims.
    #[cfg(test)]
    pub(crate) fn inject_lost_restore_commit_ack(&self) {
        self.faults.lose_restore_commit_ack.store(true, AtomicOrdering::SeqCst);
    }

    /// Test-only finalize ACK-loss injection, shared with in-crate store tests
    /// so the full fence→reconcile→publish path runs against real claims.
    #[cfg(test)]
    pub(crate) fn inject_lost_finalize_commit_ack(&self) {
        self.faults.lose_finalize_commit_ack.store(true, AtomicOrdering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn inject_lost_finalize_ack_with_next_claim(&self) {
        self.inject_lost_finalize_commit_ack();
        self.faults.claim_after_lost_ack.store(true, AtomicOrdering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn inject_lost_abort_ack_with_next_claim(&self) {
        self.faults.lose_abort_commit_ack.store(true, AtomicOrdering::SeqCst);
        self.faults.claim_after_lost_ack.store(true, AtomicOrdering::SeqCst);
    }

    /// Reject a claim minted for a different backend file. Numeric sequences
    /// collide across files, so the backend identity is always compared first.
    pub(super) fn check_claim_backend(&self, claim: &DurableRestoreClaim) -> MemoryResult<()> {
        if claim.backend() != self.backend_id {
            return Err(MemoryError::InvalidState(format!(
                "durable restore claim targets backend {:032x} but this backend is \
                 {:032x}; numeric sequences do not transfer across files",
                claim.backend(),
                self.backend_id
            )));
        }
        Ok(())
    }

    /// Non-mutating probe of the persisted active claimant: the sequence plus
    /// the generation (a legacy claim predates generations and reports
    /// `None`, which matches no live claim). Used only to classify ambiguous
    /// commit acknowledgements — never as an authority grant.
    pub(super) fn active_claim(&self) -> MemoryResult<Option<(u64, Option<u64>)>> {
        let read = self.database.begin_read().map_err(|e| {
            MemoryError::report(format!("redb active claim probe transaction failed: {e}"))
        })?;
        let meta = read
            .open_table(SPILL_META)
            .map_err(|e| MemoryError::report(format!("redb spill metadata table failed: {e}")))?;
        let sequence = meta
            .get(ACTIVE_RESTORE_SEQUENCE)
            .map_err(|e| MemoryError::report(format!("redb active restore read failed: {e}")))?
            .map(|value| value.value());
        let generation = meta
            .get(ACTIVE_RESTORE_GENERATION)
            .map_err(|e| MemoryError::report(format!("redb active claim read failed: {e}")))?
            .map(|value| value.value());
        match (sequence, generation) {
            (Some(sequence), generation) => Ok(Some((sequence, generation))),
            (None, None) => Ok(None),
            (None, Some(_)) => Err(MemoryError::InvalidState(
                "durable restore generation without an active sequence".into(),
            )),
        }
    }

    /// A finalize ACK can be lost after another record acquires the global
    /// claim. Only this record's atomic proof and payload removal establish
    /// whether this claimant's finalize landed.
    pub(super) fn finalize_landed(&self, sequence: u64, generation: u64) -> MemoryResult<bool> {
        let read = self.database.begin_read().map_err(|e| {
            MemoryError::report(format!("redb finalize probe transaction failed: {e}"))
        })?;
        let proofs = read
            .open_table(SPILL_FINALIZED_BY)
            .map_err(|e| MemoryError::report(format!("redb finalize proof table failed: {e}")))?;
        let recovery = read
            .open_table(SPILL_RECOVERY)
            .map_err(|e| MemoryError::report(format!("redb recovery table failed: {e}")))?;
        let queue = read
            .open_table(SPILL_QUEUE)
            .map_err(|e| MemoryError::report(format!("redb spill table failed: {e}")))?;
        let states = read.open_table(SPILL_RECORD_STATE).map_err(|e| {
            MemoryError::report(format!("redb record disposition table failed: {e}"))
        })?;
        let proof = proofs
            .get(sequence)
            .map_err(|e| MemoryError::report(format!("redb finalize proof read failed: {e}")))?
            .map(|value| value.value());
        let recovery_present = recovery
            .get(sequence)
            .map_err(|e| MemoryError::report(format!("redb recovery read failed: {e}")))?
            .is_some();
        let queued = queue
            .get(sequence)
            .map_err(|e| MemoryError::report(format!("redb queued record read failed: {e}")))?
            .is_some();
        let disposition = states
            .get(sequence)
            .map_err(|e| MemoryError::report(format!("redb record disposition read failed: {e}")))?
            .map(|value| RecordDisposition::decode(value.value()))
            .transpose()?;
        if proof.is_some()
            && (recovery_present || queued || disposition != Some(RecordDisposition::Finalized))
        {
            return Err(MemoryError::InvalidState(format!(
                "restore {sequence} has finalize proof inconsistent with retained bytes or disposition"
            )));
        }
        Ok(proof == Some(generation))
    }

    /// Number of handles currently retained (restorable). For diagnostics/tests.
    pub fn retained_count(&self) -> MemoryResult<u64> {
        let read = self
            .database
            .begin_read()
            .map_err(|e| MemoryError::report(format!("redb spill read txn failed: {e}")))?;
        match read.open_table(SPILL_QUEUE) {
            Ok(table) => {
                let recovery = read.open_table(SPILL_RECOVERY).map_err(|e| {
                    MemoryError::report(format!("redb spill recovery table open failed: {e}"))
                })?;
                let queue_len = table
                    .len()
                    .map_err(|e| MemoryError::report(format!("redb spill len failed: {e}")))?;
                let recovery_len = recovery
                    .len()
                    .map_err(|e| MemoryError::report(format!("redb recovery len failed: {e}")))?;
                queue_len
                    .checked_add(recovery_len)
                    .ok_or_else(|| MemoryError::capacity("redb spill inventory exceeds u64"))
            }
            Err(redb::TableError::TableDoesNotExist(_)) => Ok(0),
            Err(e) => Err(MemoryError::report(format!("redb spill table open failed: {e}"))),
        }
    }

    /// Test-only seam: insert a handle under an explicit key, so a test can drive
    /// the (otherwise astronomically unreachable) exhausted-keyspace branch.
    #[cfg(test)]
    pub(super) fn seed_key_for_test(&self, key: u64, value: &[u8]) -> MemoryResult<()> {
        let txn = self
            .database
            .begin_write()
            .map_err(|e| MemoryError::report(format!("redb spill write txn failed: {e}")))?;
        {
            let mut table = txn
                .open_table(SPILL_QUEUE)
                .map_err(|e| MemoryError::report(format!("redb spill table open failed: {e}")))?;
            table
                .insert(key, value)
                .map_err(|e| MemoryError::report(format!("redb spill insert failed: {e}")))?;
            let mut meta = txn.open_table(SPILL_META).map_err(|e| {
                MemoryError::report(format!("redb spill metadata table failed: {e}"))
            })?;
            let current_bytes = meta
                .get(RETAINED_PAYLOAD_BYTES)
                .map_err(|e| MemoryError::report(format!("redb payload usage read failed: {e}")))?
                .map(|value| value.value())
                .ok_or_else(|| {
                    MemoryError::InvalidState(
                        "redb retained payload usage authority is missing".into(),
                    )
                })?;
            let added = u64::try_from(value.len())
                .map_err(|_| MemoryError::capacity("redb test payload length exceeds u64"))?;
            let after = current_bytes.checked_add(added).ok_or_else(|| {
                MemoryError::capacity("redb test retained payload bytes exceed u64")
            })?;
            meta.insert(RETAINED_PAYLOAD_BYTES, after)
                .map_err(|e| MemoryError::report(format!("redb payload usage seed failed: {e}")))?;
            meta.insert(NEXT_SEQUENCE, key).map_err(|e| {
                MemoryError::report(format!("redb spill metadata seed failed: {e}"))
            })?;
            let mut states = txn.open_table(SPILL_RECORD_STATE).map_err(|e| {
                MemoryError::report(format!("redb record disposition table failed: {e}"))
            })?;
            states.insert(key, RecordDisposition::Queued as u8).map_err(|e| {
                MemoryError::report(format!("redb record disposition seed failed: {e}"))
            })?;
        }
        txn.commit().map_err(|e| MemoryError::report(format!("redb spill commit failed: {e}")))?;
        Ok(())
    }
}
