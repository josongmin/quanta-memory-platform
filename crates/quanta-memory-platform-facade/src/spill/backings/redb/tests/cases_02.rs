use super::*;

#[test]
fn per_record_limit_is_checked_on_reopen_even_when_total_still_fits() {
    let db = test_db();
    let original = tiny_limits();
    {
        let backing = RedbSpillBacking::create_with_limits(&db.path, original).expect("open");
        let _ = spill_backing(&backing, b"four").expect("persist");
    }
    let tighter = RedbSpillLimits { max_record_bytes: 3, ..original };
    assert!(matches!(
        RedbSpillBacking::create_with_limits(&db.path, tighter),
        Err(MemoryError::Capacity(_))
    ));
    let reopened = RedbSpillBacking::create_with_limits(&db.path, original).expect("reopen");
    assert_eq!(restore_backing(&reopened).expect("payload survived"), Some(b"four".to_vec()));
}
#[test]
fn persisted_payload_usage_mismatch_fails_closed_without_erasing_record() {
    let db = test_db();
    let limits = tiny_limits();
    {
        let backing = RedbSpillBacking::create_with_limits(&db.path, limits).expect("open");
        let _ = spill_backing(&backing, b"four").expect("persist");
    }
    {
        let database = redb::Database::create(&db.path).expect("open raw fixture");
        let txn = database.begin_write().expect("begin metadata mutation");
        {
            let mut meta = txn.open_table(SPILL_META).expect("open metadata");
            meta.insert(RETAINED_PAYLOAD_BYTES, 0).expect("corrupt usage counter");
        }
        txn.commit().expect("commit fixture mutation");
    }
    assert!(matches!(
        RedbSpillBacking::create_with_limits(&db.path, limits),
        Err(MemoryError::InvalidState(_))
    ));
    {
        let database = redb::Database::create(&db.path).expect("open raw fixture");
        let txn = database.begin_write().expect("begin metadata repair");
        {
            let mut meta = txn.open_table(SPILL_META).expect("open metadata");
            meta.insert(RETAINED_PAYLOAD_BYTES, 4).expect("restore usage counter");
        }
        txn.commit().expect("commit fixture repair");
    }
    let reopened = RedbSpillBacking::create_with_limits(&db.path, limits).expect("reopen");
    assert_eq!(restore_backing(&reopened).expect("payload survived"), Some(b"four".to_vec()));
}
#[test]
fn every_invalid_limit_relation_rejects_before_creating_database_file() {
    let cases = [
        ("zero record cap", RedbSpillLimits { max_retained_records: 0, ..tiny_limits() }),
        ("zero file cap", RedbSpillLimits { max_database_file_bytes: 0, ..tiny_limits() }),
        (
            "one restore buffer",
            RedbSpillLimits { max_inflight_restore_buffers: 1, ..tiny_limits() },
        ),
        (
            "record exceeds retained bytes",
            RedbSpillLimits {
                max_record_bytes: 5,
                max_retained_payload_bytes: 4,
                max_inflight_restore_bytes: 10,
                ..tiny_limits()
            },
        ),
        (
            "two-buffer byte multiplication overflows",
            RedbSpillLimits {
                max_record_bytes: u64::MAX,
                max_retained_payload_bytes: u64::MAX,
                max_inflight_restore_bytes: u64::MAX,
                ..tiny_limits()
            },
        ),
        (
            "two-buffer byte budget is short",
            RedbSpillLimits { max_inflight_restore_bytes: 7, ..tiny_limits() },
        ),
    ];
    for (name, invalid) in cases {
        let db = test_db();
        assert!(
            matches!(
                RedbSpillBacking::create_with_limits(&db.path, invalid),
                Err(MemoryError::Config(_))
            ),
            "{name}"
        );
        assert!(!db.path.exists(), "{name} must not create a durable file");
    }
}
#[test]
fn capped_file_backend_rejects_extension_before_file_mutation() {
    let db = test_db();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&db.path)
        .expect("open fixture file");
    let backend = CappedFileBackend::new(file, 4).expect("cap fixture");
    assert!(backend.set_len(5).is_err());
    assert!(backend.write(0, b"12345").is_err());
    assert_eq!(backend.len().expect("unchanged length"), 0);
    backend.set_len(4).expect("exact-cap extension");
    backend.write(0, b"1234").expect("exact-cap write");
    assert_eq!(backend.len().expect("exact-cap length"), 4);
    assert_eq!(backend.read(0, 4).expect("exact-cap read"), b"1234");
    assert!(backend.write(4, b"x").is_err());
    assert_eq!(std::fs::metadata(&db.path).expect("file metadata").len(), 4);
}
#[test]
fn reopening_above_database_file_cap_fails_without_erasing_payload() {
    let db = test_db();
    let limits = tiny_limits();
    {
        let backing = RedbSpillBacking::create_with_limits(&db.path, limits).expect("open");
        let _ = spill_backing(&backing, b"four").expect("persist");
    }
    let file_bytes = std::fs::metadata(&db.path).expect("file metadata").len();
    assert!(file_bytes > 0);
    let tighter = RedbSpillLimits { max_database_file_bytes: file_bytes - 1, ..limits };
    assert!(matches!(
        RedbSpillBacking::create_with_limits(&db.path, tighter),
        Err(MemoryError::Capacity(_))
    ));
    assert_eq!(std::fs::metadata(&db.path).expect("file metadata").len(), file_bytes);
    let reopened = RedbSpillBacking::create_with_limits(&db.path, limits).expect("reopen");
    assert_eq!(restore_backing(&reopened).expect("record survived"), Some(b"four".to_vec()));
}
#[test]
fn database_file_cap_denial_preserves_prior_durable_record_across_reopen() {
    let db = test_db();
    let limits = RedbSpillLimits {
        max_record_bytes: 512 * 1024,
        max_retained_records: 64,
        max_retained_payload_bytes: 32 * 1024 * 1024,
        max_database_file_bytes: 4 * 1024 * 1024,
        cache_bytes: 4 * 1024 * 1024,
        max_inflight_restore_bytes: 1024 * 1024,
        max_inflight_restore_buffers: 2,
    };
    let backing = RedbSpillBacking::create_with_limits(&db.path, limits).expect("open");
    let anchor = spill_backing(&backing, b"anchor").expect("persist prior record");
    assert_eq!(anchor.inventory_after(), 1);
    let payload = vec![0xA5; 512 * 1024];
    let mut commit_denied = false;
    let mut accepted = 0;
    for _ in 0..63 {
        let id = backing.reserve_record_id().expect("reserve before file cap");
        match backing.commit_spill(id, &payload) {
            Ok(_) => accepted += 1,
            Err(failure) => {
                match &failure {
                    DurableWriteFailure::NotCommitted(error) => {
                        assert!(matches!(**error, MemoryError::Capacity(_)));
                    }
                    DurableWriteFailure::RecoveryRequired { commit_error, .. } => {
                        assert!(matches!(**commit_error, MemoryError::Capacity(_)));
                    }
                }
                assert!(
                    format!("{failure:?}").contains("database file"),
                    "expected the physical file cap, got {failure:?}"
                );
                commit_denied = true;
                break;
            }
        }
        assert!(
            std::fs::metadata(&db.path).expect("file metadata").len()
                <= limits.max_database_file_bytes
        );
    }
    assert!(commit_denied, "file cap must be reached before logical payload/record caps");
    assert!(
        std::fs::metadata(&db.path).expect("file metadata").len() <= limits.max_database_file_bytes
    );
    drop(backing);
    let reopened = RedbSpillBacking::create_with_limits(&db.path, limits).expect("reopen");
    assert_eq!(restore_backing(&reopened).expect("restore prior record"), Some(b"anchor".to_vec()));
    let mut restored = 0;
    let mut needs_capacity_for_restore = false;
    for _ in 0..accepted {
        match restore_backing(&reopened) {
            Ok(Some(bytes)) => {
                assert_eq!(bytes, payload);
                restored += 1;
            }
            Err(error) => {
                assert!(matches!(&error, MemoryError::Capacity(_)));
                assert!(
                    format!("{error}").contains("database file"),
                    "unexpected restore failure: {error}"
                );
                // Redb closes this handle to further I/O after a backend
                // error. Probe the persisted postimage only after reopen.
                needs_capacity_for_restore = true;
                break;
            }
            Ok(None) => panic!("accepted record vanished before restore"),
        }
    }
    drop(reopened);
    let larger = RedbSpillLimits { max_database_file_bytes: 64 * 1024 * 1024, ..limits };
    let reopened = RedbSpillBacking::create_with_limits(&db.path, larger).expect("reopen");
    if needs_capacity_for_restore {
        let active = reopened.active_restore().expect("active claim").expect("id");
        assert!(matches!(
            reopened.resolve_record(active),
            Ok(DurableRecordResolution::Present { .. })
        ));
        assert_eq!(reopened.retained_count(), Ok(accepted - restored));
        let recovered = reopened.recover_active_restore().expect("recover claim").expect("claim");
        assert_eq!(recovered.candidate.bytes(), payload);
        reopened.commit_restore(&recovered.candidate.claim).expect("commit after cap increase");
        reopened.finalize_restore(&recovered.candidate.claim).expect("finalize after cap increase");
        restored += 1;
    }
    for _ in restored..accepted {
        assert_eq!(
            restore_backing(&reopened).expect("restore remaining record"),
            Some(payload.clone())
        );
    }
    assert_eq!(reopened.retained_count(), Ok(0));
    assert_eq!(restore_backing(&reopened).expect("empty after drain"), None);
}
#[test]
fn partial_storage_full_write_reopens_to_whole_records_only() {
    let db = test_db();
    let limits = RedbSpillLimits { max_retained_payload_bytes: 8, ..tiny_limits() };
    let armed = Arc::new(AtomicBool::new(false));
    let observed = Arc::new(AtomicBool::new(false));
    let attempted_bytes = Arc::new(AtomicU64::new(0));
    let written_bytes = Arc::new(AtomicU64::new(0));
    let backing = open_with_partial_write_fault(
        &db.path,
        limits,
        Arc::clone(&armed),
        Arc::clone(&observed),
        Arc::clone(&attempted_bytes),
        Arc::clone(&written_bytes),
    );
    let anchor = backing.reserve_record_id().expect("reserve anchor");
    let _receipt = backing.commit_spill(anchor, b"old").expect("commit anchor");
    let uncertain = backing.reserve_record_id().expect("reserve uncertain record");
    armed.store(true, AtomicOrdering::SeqCst);
    let failure = backing.commit_spill(uncertain, b"new!").expect_err("partial I/O fault");
    assert!(observed.load(AtomicOrdering::SeqCst), "fault must hit a real write");
    let attempted = attempted_bytes.load(AtomicOrdering::SeqCst);
    let written = written_bytes.load(AtomicOrdering::SeqCst);
    assert!(written > 0 && written < attempted, "not a strict partial write");
    assert!(matches!(
        failure,
        DurableWriteFailure::NotCommitted(_) | DurableWriteFailure::RecoveryRequired { .. }
    ));
    drop(backing);

    let reopened = RedbSpillBacking::create_with_limits(&db.path, limits).expect("cold reopen");
    assert_eq!(restore_backing(&reopened).expect("prior record intact"), Some(b"old".to_vec()));
    match reopened.resolve_record(uncertain).expect("exact postimage") {
        DurableRecordResolution::Present { .. } => {
            assert_eq!(
                restore_backing(&reopened).expect("whole new record"),
                Some(b"new!".to_vec())
            );
        }
        DurableRecordResolution::Absent => {
            assert_eq!(restore_backing(&reopened).expect("no torn new record"), None);
        }
        other => panic!("uncommitted record has impossible disposition {other:?}"),
    }
    assert_eq!(reopened.retained_count(), Ok(0));
}
#[test]
fn partial_storage_full_during_restore_commit_preserves_one_whole_copy() {
    let db = test_db();
    let limits = RedbSpillLimits { max_retained_payload_bytes: 8, ..tiny_limits() };
    let armed = Arc::new(AtomicBool::new(false));
    let observed = Arc::new(AtomicBool::new(false));
    let attempted_bytes = Arc::new(AtomicU64::new(0));
    let written_bytes = Arc::new(AtomicU64::new(0));
    let backing = open_with_partial_write_fault(
        &db.path,
        limits,
        Arc::clone(&armed),
        Arc::clone(&observed),
        Arc::clone(&attempted_bytes),
        Arc::clone(&written_bytes),
    );
    let id = backing.reserve_record_id().expect("reserve");
    let _receipt = backing.commit_spill(id, b"four").expect("commit");
    let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
    armed.store(true, AtomicOrdering::SeqCst);
    let failure =
        backing.commit_restore(&candidate.claim).expect_err("partial restore-commit I/O fault");
    assert!(
        matches!(
            failure,
            DurableCommitFailure::Retained(ref error)
                | DurableCommitFailure::Ambiguous(ref error)
                if matches!(**error, MemoryError::Capacity(_))
        ),
        "storage-full restore error must be typed capacity: {failure:?}"
    );
    assert!(observed.load(AtomicOrdering::SeqCst));
    let attempted = attempted_bytes.load(AtomicOrdering::SeqCst);
    let written = written_bytes.load(AtomicOrdering::SeqCst);
    assert!(written > 0 && written < attempted, "not a strict partial write");
    drop(candidate);
    drop(backing);

    let reopened = RedbSpillBacking::create_with_limits(&db.path, limits).expect("cold reopen");
    assert_eq!(
        reopened.resolve_record(id),
        Ok(DurableRecordResolution::Present { inventory_after: 1 })
    );
    let recovery = reopened.recover_active_restore().expect("recover").expect("candidate");
    assert_eq!(recovery.candidate.bytes(), b"four");
    reopened
        .commit_restore(&recovery.candidate.claim)
        .expect("prepared or committed recovery converges");
    reopened.finalize_restore(&recovery.candidate.claim).expect("finalize recovered record");
    assert_eq!(reopened.retained_count(), Ok(0));
}
#[test]
fn partial_storage_full_during_finalize_reopens_present_or_finalized() {
    let db = test_db();
    let limits = RedbSpillLimits { max_retained_payload_bytes: 8, ..tiny_limits() };
    let armed = Arc::new(AtomicBool::new(false));
    let observed = Arc::new(AtomicBool::new(false));
    let attempted_bytes = Arc::new(AtomicU64::new(0));
    let written_bytes = Arc::new(AtomicU64::new(0));
    let backing = open_with_partial_write_fault(
        &db.path,
        limits,
        Arc::clone(&armed),
        Arc::clone(&observed),
        Arc::clone(&attempted_bytes),
        Arc::clone(&written_bytes),
    );
    let id = backing.reserve_record_id().expect("reserve");
    let _receipt = backing.commit_spill(id, b"four").expect("commit");
    let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
    backing.commit_restore(&candidate.claim).expect("commit restore");
    armed.store(true, AtomicOrdering::SeqCst);
    let _failure =
        backing.finalize_restore(&candidate.claim).expect_err("partial finalize I/O fault");
    assert!(observed.load(AtomicOrdering::SeqCst));
    let attempted = attempted_bytes.load(AtomicOrdering::SeqCst);
    let written = written_bytes.load(AtomicOrdering::SeqCst);
    assert!(written > 0 && written < attempted, "not a strict partial write");
    drop(candidate);
    drop(backing);

    let reopened = RedbSpillBacking::create_with_limits(&db.path, limits).expect("cold reopen");
    match reopened.resolve_record(id).expect("exact postimage") {
        DurableRecordResolution::Present { inventory_after: 1 } => {
            let recovery = reopened.recover_active_restore().expect("recover").expect("candidate");
            assert_eq!(recovery.phase, DurableRestorePhase::Committed);
            assert_eq!(recovery.candidate.bytes(), b"four");
            reopened.finalize_restore(&recovery.candidate.claim).expect("finish retained record");
        }
        DurableRecordResolution::Finalized => {}
        other => panic!("partial finalize produced impossible postimage {other:?}"),
    }
    assert_eq!(reopened.retained_count(), Ok(0));
}
#[test]
fn unsupported_explicit_schema_versions_fail_closed_before_v2_initialization() {
    for version in [0, 1, 3, u64::MAX] {
        let db = test_db();
        {
            let database = Database::create(&db.path).expect("create schema fixture");
            let txn = database.begin_write().expect("schema fixture txn");
            {
                let mut meta = txn.open_table(SPILL_META).expect("schema fixture metadata");
                meta.insert(SCHEMA_VERSION, version).expect("write unsupported schema");
                meta.insert("future_schema_sentinel", 97u64).expect("write sentinel");
            }
            txn.commit().expect("commit schema fixture");
        }

        let error = RedbSpillBacking::create(&db.path)
            .expect_err("unsupported explicit schema must fail closed");
        assert!(
            matches!(&error, MemoryError::Unsupported(message)
                if message.contains(&version.to_string())),
            "unexpected rejection for schema {version}: {error:?}"
        );

        let database = Database::create(&db.path).expect("reopen rejected schema fixture");
        assert_no_v2_initialization(&database);
        let read = database.begin_read().expect("read rejected schema fixture");
        let meta = read.open_table(SPILL_META).expect("schema fixture metadata survives");
        assert_eq!(
            meta.get(SCHEMA_VERSION).expect("read rejected schema").map(|value| value.value()),
            Some(version)
        );
        assert_eq!(
            meta.get("future_schema_sentinel")
                .expect("read future sentinel")
                .map(|value| value.value()),
            Some(97)
        );
        assert!(meta.get(BACKEND_ID_LO).expect("read backend low").is_none());
        assert!(meta.get(BACKEND_ID_HI).expect("read backend high").is_none());
    }
}
#[test]
fn missing_schema_marker_on_v2_shaped_database_fails_without_restamping() {
    let db = test_db();
    let backend_id = {
        let backing = RedbSpillBacking::create(&db.path).expect("initialize v2 fixture");
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, b"whole").expect("commit");
        backing.backend_id
    };
    {
        let database = Database::create(&db.path).expect("open v2 fixture");
        let txn = database.begin_write().expect("v2 fixture txn");
        {
            let mut meta = txn.open_table(SPILL_META).expect("open metadata");
            assert!(meta.remove(SCHEMA_VERSION).expect("remove schema marker").is_some());
        }
        txn.commit().expect("commit missing-marker fixture");
    }

    let error = RedbSpillBacking::create(&db.path)
        .expect_err("v2 evidence without marker must not migrate as legacy");
    assert!(matches!(error, MemoryError::InvalidState(_)), "got {error:?}");

    let database = Database::create(&db.path).expect("reopen rejected fixture");
    let read = database.begin_read().expect("read rejected fixture");
    let meta = read.open_table(SPILL_META).expect("metadata survives");
    assert!(meta.get(SCHEMA_VERSION).expect("read schema marker").is_none());
    let persisted_backend_id = u128::from(
        meta.get(BACKEND_ID_HI).expect("read high half").expect("high half survives").value(),
    ) << 64
        | u128::from(
            meta.get(BACKEND_ID_LO).expect("read low half").expect("low half survives").value(),
        );
    assert_eq!(persisted_backend_id, backend_id);
    let queue = read.open_table(SPILL_QUEUE).expect("queue survives");
    assert_eq!(
        queue.get(0).expect("read payload").map(|value| value.value().to_vec()),
        Some(b"whole".to_vec())
    );
    let states = read.open_table(SPILL_RECORD_STATE).expect("states survive");
    assert_eq!(
        states.get(0).expect("read state").map(|value| value.value()),
        Some(RecordDisposition::Queued as u8)
    );
}
#[test]
fn missing_metadata_table_on_v2_shaped_database_fails_without_recreation() {
    let db = test_db();
    {
        let backing = RedbSpillBacking::create(&db.path).expect("initialize v2 fixture");
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, b"whole").expect("commit");
    }
    {
        let database = Database::create(&db.path).expect("open v2 fixture");
        let txn = database.begin_write().expect("v2 fixture txn");
        assert!(txn.delete_table(SPILL_META).expect("delete metadata"));
        txn.commit().expect("commit missing-metadata fixture");
    }

    let error = RedbSpillBacking::create(&db.path)
        .expect_err("v2 tables without metadata must not initialize as legacy");
    assert!(matches!(error, MemoryError::InvalidState(_)), "got {error:?}");

    let database = Database::create(&db.path).expect("reopen rejected fixture");
    let read = database.begin_read().expect("read rejected fixture");
    assert!(matches!(read.open_table(SPILL_META), Err(redb::TableError::TableDoesNotExist(_))));
    let queue = read.open_table(SPILL_QUEUE).expect("queue survives");
    assert_eq!(
        queue.get(0).expect("read payload").map(|value| value.value().to_vec()),
        Some(b"whole".to_vec())
    );
    let states = read.open_table(SPILL_RECORD_STATE).expect("states survive");
    assert_eq!(
        states.get(0).expect("read state").map(|value| value.value()),
        Some(RecordDisposition::Queued as u8)
    );
}
#[test]
fn incomplete_backend_identity_fails_closed_without_replacing_surviving_half() {
    for (low, high) in [(Some(11u64), None), (None, Some(13u64)), (None, None)] {
        let db = test_db();
        {
            let database = Database::create(&db.path).expect("create identity fixture");
            let txn = database.begin_write().expect("identity fixture txn");
            {
                let mut meta = txn.open_table(SPILL_META).expect("identity fixture metadata");
                meta.insert(SCHEMA_VERSION, SCHEMA_VERSION_V2).expect("write current schema");
                if let Some(low) = low {
                    meta.insert(BACKEND_ID_LO, low).expect("write low half");
                }
                if let Some(high) = high {
                    meta.insert(BACKEND_ID_HI, high).expect("write high half");
                }
            }
            txn.commit().expect("commit identity fixture");
        }

        let error = RedbSpillBacking::create(&db.path)
            .expect_err("incomplete backend identity must fail closed");
        assert!(matches!(error, MemoryError::InvalidState(_)), "got {error:?}");

        let database = Database::create(&db.path).expect("reopen rejected identity fixture");
        assert_no_v2_initialization(&database);
        let read = database.begin_read().expect("read rejected identity fixture");
        let meta = read.open_table(SPILL_META).expect("identity fixture metadata survives");
        assert_eq!(meta.get(BACKEND_ID_LO).expect("read low half").map(|value| value.value()), low);
        assert_eq!(
            meta.get(BACKEND_ID_HI).expect("read high half").map(|value| value.value()),
            high
        );
        assert_eq!(
            meta.get(SCHEMA_VERSION).expect("read current schema").map(|value| value.value()),
            Some(SCHEMA_VERSION_V2)
        );
    }
}
#[test]
fn schema_v2_missing_required_authority_fails_without_recreating_it() {
    for missing in ["queue", "recovery", "state", "proof", "next_sequence"] {
        let db = test_db();
        drop(RedbSpillBacking::create(&db.path).expect("initialize v2 fixture"));
        {
            let database = Database::create(&db.path).expect("open v2 fixture");
            let txn = database.begin_write().expect("v2 fixture txn");
            match missing {
                "queue" => assert!(txn.delete_table(SPILL_QUEUE).expect("delete queue")),
                "recovery" => {
                    assert!(txn.delete_table(SPILL_RECOVERY).expect("delete recovery"));
                }
                "state" => {
                    assert!(txn.delete_table(SPILL_RECORD_STATE).expect("delete state"));
                }
                "proof" => {
                    assert!(txn.delete_table(SPILL_FINALIZED_BY).expect("delete proof"));
                }
                "next_sequence" => {
                    let mut meta = txn.open_table(SPILL_META).expect("open metadata");
                    assert!(meta.remove(NEXT_SEQUENCE).expect("remove frontier").is_some());
                }
                _ => unreachable!(),
            }
            txn.commit().expect("commit malformed fixture");
        }

        let error = RedbSpillBacking::create(&db.path)
            .expect_err("incomplete v2 authority must fail closed");
        assert!(matches!(error, MemoryError::InvalidState(_)), "{missing}: {error:?}");

        let database = Database::create(&db.path).expect("reopen rejected v2 fixture");
        let read = database.begin_read().expect("read rejected v2 fixture");
        match missing {
            "queue" => assert!(matches!(
                read.open_table(SPILL_QUEUE),
                Err(redb::TableError::TableDoesNotExist(_))
            )),
            "recovery" => assert!(matches!(
                read.open_table(SPILL_RECOVERY),
                Err(redb::TableError::TableDoesNotExist(_))
            )),
            "state" => assert!(matches!(
                read.open_table(SPILL_RECORD_STATE),
                Err(redb::TableError::TableDoesNotExist(_))
            )),
            "proof" => assert!(matches!(
                read.open_table(SPILL_FINALIZED_BY),
                Err(redb::TableError::TableDoesNotExist(_))
            )),
            "next_sequence" => {
                let meta = read.open_table(SPILL_META).expect("metadata survives");
                assert!(meta.get(NEXT_SEQUENCE).expect("read frontier").is_none());
            }
            _ => unreachable!(),
        }
    }
}
#[test]
fn schema_v2_payload_disposition_mismatch_fails_without_repair() {
    for corrupted in [None, Some(RecordDisposition::Recovery as u8), Some(u8::MAX)] {
        let db = test_db();
        {
            let backing = RedbSpillBacking::create(&db.path).expect("initialize v2 fixture");
            let id = backing.reserve_record_id().expect("reserve");
            let _receipt = backing.commit_spill(id, b"whole").expect("commit");
        }
        {
            let database = Database::create(&db.path).expect("open v2 fixture");
            let txn = database.begin_write().expect("v2 fixture txn");
            {
                let mut states = txn.open_table(SPILL_RECORD_STATE).expect("open states");
                match corrupted {
                    Some(raw) => {
                        states.insert(0, raw).expect("write corrupt disposition");
                    }
                    None => {
                        assert!(states.remove(0).expect("remove disposition").is_some());
                    }
                }
            }
            txn.commit().expect("commit malformed fixture");
        }

        let error = RedbSpillBacking::create(&db.path)
            .expect_err("payload/disposition mismatch must fail closed");
        assert!(matches!(error, MemoryError::InvalidState(_)), "got {error:?}");

        let database = Database::create(&db.path).expect("reopen rejected v2 fixture");
        let read = database.begin_read().expect("read rejected v2 fixture");
        let queue = read.open_table(SPILL_QUEUE).expect("queue survives");
        assert_eq!(
            queue.get(0).expect("read payload").map(|value| value.value().to_vec()),
            Some(b"whole".to_vec())
        );
        let states = read.open_table(SPILL_RECORD_STATE).expect("states survive");
        assert_eq!(states.get(0).expect("read disposition").map(|value| value.value()), corrupted);
    }
}
#[test]
fn sep23_fup001_legacy_watermark_file_migrates_payload_and_fails_closed() {
    let db = test_db();
    // Build a schema-1 (watermark-only) file by hand: queued payload 5,
    // recovery payload 3, allocator frontier 6, watermark claiming
    // "everything through 4 is consumed". No disposition rows exist.
    {
        let database = redb::Database::create(&db.path).expect("legacy fixture");
        let txn = database.begin_write().expect("legacy txn");
        {
            let mut queue = txn.open_table(SPILL_QUEUE).expect("legacy queue");
            queue.insert(5u64, b"five".as_slice()).expect("legacy queued row");
            let mut recovery = txn.open_table(SPILL_RECOVERY).expect("legacy recovery");
            recovery.insert(3u64, b"three".as_slice()).expect("legacy recovery row");
            let mut meta = txn.open_table(SPILL_META).expect("legacy meta");
            meta.insert(NEXT_SEQUENCE, 6u64).expect("legacy next");
            // nosemgrep: qmp-durable-no-watermark-resolution -- reason: obsolete fixture metadata proves recovery ignores the legacy watermark.
            meta.insert(FINALIZED_THROUGH, 4u64).expect("legacy watermark");
        }
        txn.commit().expect("legacy commit");
    }
    let backing = RedbSpillBacking::create(&db.path).expect("open migrates");
    // Explicit payload evidence converts exactly; bytes survive migration.
    assert!(
        matches!(
            backing.resolve_record(DurableRecordId::new(5)),
            Ok(DurableRecordResolution::Present { .. })
        ),
        "migrated queued payload must resolve present"
    );
    assert!(
        matches!(
            backing.resolve_record(DurableRecordId::new(3)),
            Ok(DurableRecordResolution::Present { .. })
        ),
        "migrated recovery payload must resolve present"
    );
    assert_eq!(backing.retained_count().expect("migrated inventory"), 2);
    let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
    assert_eq!(candidate.sequence, 5);
    assert_eq!(candidate.bytes(), b"five");
    backing.commit_restore(&candidate.claim).expect("commit migrated restore");
    backing.finalize_restore(&candidate.claim).expect("finalize migrated restore");
    assert_eq!(
        backing.resolve_record(DurableRecordId::new(5)),
        Ok(DurableRecordResolution::Finalized)
    );

    // Below-frontier ids with no payload evidence are genuinely ambiguous:
    // the legacy watermark claims 0..=4 were consumed, but a reserved hole
    // is indistinguishable from a consumed record. Fail closed — never
    // guess `Finalized` (wrong retire) or `Absent` (wrong resurrection).
    for ambiguous in [0u64, 1, 2, 4] {
        let probed = backing.resolve_record(DurableRecordId::new(ambiguous));
        assert!(
            matches!(&probed, Err(MemoryError::InvalidState(message))
                if message.contains("unresolved durable record")),
            "legacy id {ambiguous} must fail closed as unresolved, got {probed:?}"
        );
        assert!(
            matches!(
                backing.commit_spill(DurableRecordId::new(ambiguous), b"rewrite"),
                Err(DurableWriteFailure::NotCommitted(_))
            ),
            "legacy id {ambiguous} must not be rewritable"
        );
    }
    // Never-issued ids stay exactly `Absent` (not unresolved).
    for fresh in [6u64, 100] {
        assert_eq!(
            backing.resolve_record(DurableRecordId::new(fresh)),
            Ok(DurableRecordResolution::Absent)
        );
    }
    // The allocator frontier and exact states continue after migration.
    let next = backing.reserve_record_id().expect("post-migration reserve");
    assert_eq!(next.get(), 6);
    let _receipt = backing.commit_spill(next, b"six").expect("post-migration commit");
    assert!(matches!(backing.resolve_record(next), Ok(DurableRecordResolution::Present { .. })));
}
