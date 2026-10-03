use super::*;

#[test]
fn create_on_an_invalid_path_is_a_neutral_memory_error() {
    let bad = std::path::Path::new("/nonexistent-qmp-dir/redb/db.redb");
    let err = RedbSpillBacking::create(bad).expect_err("open under a missing dir must fail");
    assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");
}
#[test]
fn live_backing_excludes_second_open_and_drop_releases_persisted_file() {
    let db = test_db();
    let first = RedbSpillBacking::create(&db.path).expect("open first");
    let _receipt = spill_backing(&first, b"persisted").expect("seed");

    let err = RedbSpillBacking::create(&db.path).expect_err("second live open must be excluded");
    assert!(matches!(err, MemoryError::Build(_)), "got {err:?}");

    drop(first);
    let reopened = RedbSpillBacking::create(&db.path).expect("open after drop");
    assert_eq!(restore_backing(&reopened).expect("restore persisted"), Some(b"persisted".to_vec()));
}
#[test]
fn durable_spill_target_round_trips_fifo_through_the_trait() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");

    // Reached only through the SpillTarget abstraction — no redb types in sight.
    let first = spill_backing(&backing, b"first").expect("accept 1");
    let second = spill_backing(&backing, b"second").expect("accept 2");
    assert_eq!(first, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });
    assert_eq!(second, SpillReceipt::Durable { sequence: 1, inventory_after: 2 });
    assert_eq!(backing.retained_count().expect("len"), 2);

    assert_eq!(restore_backing(&backing).expect("r1"), Some(b"first".to_vec()));
    assert_eq!(restore_backing(&backing).expect("r2"), Some(b"second".to_vec()));
    assert_eq!(restore_backing(&backing).expect("drained"), None);
    assert_eq!(backing.retained_count().expect("len"), 0);
}
#[test]
fn reopened_backing_resumes_sequence_and_restores_persisted_handles() {
    let db = test_db();
    {
        let backing = RedbSpillBacking::create(&db.path).expect("open 1");
        let _receipt = spill_backing(&backing, b"persisted").expect("accept");
    }
    // Reopen: the persisted handle survives and the sequence does not collide.
    let backing = RedbSpillBacking::create(&db.path).expect("open 2");
    let receipt = spill_backing(&backing, b"after-restart").expect("accept again");
    assert_eq!(receipt, SpillReceipt::Durable { sequence: 1, inventory_after: 2 });
    assert_eq!(restore_backing(&backing).expect("r1"), Some(b"persisted".to_vec()));
    assert_eq!(restore_backing(&backing).expect("r2"), Some(b"after-restart".to_vec()));
}
#[test]
fn drained_queue_never_reuses_sequence_or_accepts_stale_restore_token() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");

    let first = spill_backing(&backing, b"first").expect("accept first");
    assert_eq!(first, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });
    let stale = backing.prepare_restore().expect("prepare").expect("candidate");
    backing.commit_restore(&stale.claim).expect("drain first");
    backing.finalize_restore(&stale.claim).expect("acknowledge first restore");

    let replacement = spill_backing(&backing, b"replacement").expect("accept after full drain");
    assert_eq!(replacement, SpillReceipt::Durable { sequence: 1, inventory_after: 1 });
    assert!(matches!(backing.commit_restore(&stale.claim), Err(DurableCommitFailure::Retained(_))));
    let current = backing.prepare_restore().expect("prepare current").expect("current");
    assert_eq!(current.sequence, 1);
    assert_eq!(current.bytes, b"replacement");
    assert_eq!(backing.retained_count(), Ok(1));

    let stale_write = backing
        .commit_spill(DurableRecordId::new(0), b"first")
        .expect_err("a finalized record ID must never be resurrected");
    assert!(matches!(stale_write, DurableWriteFailure::NotCommitted(_)));
    assert_eq!(backing.retained_count(), Ok(1));
}
#[test]
fn persisted_restore_claim_blocks_duplicate_prepare_and_recovers_after_reopen() {
    let db = test_db();
    {
        let backing = RedbSpillBacking::create(&db.path).expect("open 1");
        let _receipt = spill_backing(&backing, b"claimed").expect("seed");
        let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
        assert_eq!(candidate.sequence, 0);
        assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
    }

    let backing = RedbSpillBacking::create(&db.path).expect("open 2");
    assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
    assert!(matches!(backing.prepare_restore(), Err(MemoryError::OperationInProgress(_))));
    // After a restart the new owner claims the persisted restore before
    // reconciling; the recovered claim authorizes the abort-and-retain.
    let recovery =
        backing.recover_active_restore().expect("claim persisted restore").expect("recovery");
    assert_eq!(recovery.phase, DurableRestorePhase::Prepared);
    assert!(matches!(
        backing
            .reconcile_restore(&recovery.candidate.claim)
            .expect("retained claim reconciliation"),
        DurableRecordResolution::Present { inventory_after: 1 }
    ));
    assert_eq!(backing.active_restore(), Ok(None));
    assert_eq!(backing.prepare_restore().expect("prepare after recovery").unwrap().sequence, 0);
}
#[test]
fn committed_restore_payload_survives_reopen_until_resident_ack() {
    let db = test_db();
    {
        let backing = RedbSpillBacking::create(&db.path).expect("open 1");
        let _receipt = spill_backing(&backing, b"crash-safe").expect("seed");
        let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
        backing.commit_restore(&candidate.claim).expect("move to recovery");
        assert_eq!(backing.retained_count(), Ok(1));
        assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
    }

    let backing = RedbSpillBacking::create(&db.path).expect("open 2");
    let recovery =
        backing.recover_active_restore().expect("read recovery").expect("persisted recovery");
    assert_eq!(recovery.phase, DurableRestorePhase::Committed);
    assert_eq!(recovery.candidate.sequence, 0);
    assert_eq!(recovery.candidate.bytes(), b"crash-safe");
    assert!(matches!(backing.prepare_restore(), Err(MemoryError::OperationInProgress(_))));

    backing
        .finalize_restore(&recovery.candidate.claim)
        .expect("resident publication acknowledgement");
    assert_eq!(backing.active_restore(), Ok(None));
    assert_eq!(backing.retained_count(), Ok(0));
    assert_eq!(
        backing.resolve_record(DurableRecordId::new(0)),
        Ok(DurableRecordResolution::Finalized)
    );
}
#[test]
fn injected_write_crash_points_converge_without_duplicate_records() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");

    let first = backing.reserve_record_id().expect("reserve first");
    backing.inject_fail_write_before_transaction();
    assert!(matches!(
        backing.commit_spill(first, b"first"),
        Err(DurableWriteFailure::NotCommitted(_))
    ));
    assert_eq!(backing.resolve_record(first), Ok(DurableRecordResolution::Absent));

    let second = backing.reserve_record_id().expect("reserve second");
    backing.inject_lost_write_commit_ack();
    let receipt = backing
        .commit_spill(second, b"second")
        .expect("post-commit probe converts lost ack to committed");
    assert_eq!(receipt, SpillReceipt::Durable { sequence: 1, inventory_after: 1 });
    let retry =
        backing.commit_spill(second, b"second").expect("same record and payload are idempotent");
    assert_eq!(retry, receipt);
    assert_eq!(backing.retained_count(), Ok(1));
}
#[test]
fn injected_restore_and_finalize_ack_loss_preserve_recovery_authority() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    let _receipt = spill_backing(&backing, b"recoverable").expect("seed");
    let candidate = backing.prepare_restore().expect("prepare").expect("candidate");

    backing.inject_lost_restore_commit_ack();
    assert!(matches!(
        backing.commit_restore(&candidate.claim),
        Err(DurableCommitFailure::Ambiguous(_))
    ));
    // The live claimant reconciles with its OWN claim: recovery bytes plus
    // the still-live claim prove the commit landed. (`recover` would mint
    // a new generation and steal this claimant's authority, so it is not
    // a same-owner probe.)
    assert_eq!(backing.reconcile_restore(&candidate.claim), Ok(DurableRecordResolution::Finalized));

    backing.inject_lost_finalize_commit_ack();
    backing
        .finalize_restore(&candidate.claim)
        .expect("post-finalize probe converts lost ack to success");
    assert_eq!(backing.active_restore(), Ok(None));
    assert_eq!(backing.retained_count(), Ok(0));
}
#[test]
fn generic_restore_is_non_destructive_and_requires_owned_two_phase_path() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    let _receipt = spill_backing(&backing, b"only-copy").expect("accept");

    let error = SpillTarget::restore(&backing)
        .expect_err("generic destructive restore must be unavailable for durable records");
    assert!(matches!(error, MemoryError::Unsupported(_)));
    assert_eq!(backing.retained_count(), Ok(1));
    assert_eq!(
        backing.prepare_restore().expect("prepare").expect("candidate").bytes(),
        b"only-copy"
    );
}
#[test]
fn generic_write_is_non_mutating_and_returns_the_original_payload() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");

    let failure = SpillTarget::accept(&backing, b"only-copy".to_vec())
        .expect_err("generic write cannot represent an ambiguous durable commit");
    assert!(matches!(failure.error(), MemoryError::Unsupported(_)));
    assert_eq!(failure.into_parts().1, b"only-copy");
    assert_eq!(backing.retained_count(), Ok(0));
}
#[test]
fn reopened_redb_reconciles_controller_count_from_persisted_inventory() {
    let db = test_db();
    {
        let controller =
            RoutedSpillController::new(RedbSpillBacking::create(&db.path).expect("open 1"));
        let first = spill_controller(&controller, b"first").expect("first");
        let second = spill_controller(&controller, b"second").expect("second");
        assert_eq!(first, SpillReceipt::Durable { sequence: 0, inventory_after: 1 });
        assert_eq!(second, SpillReceipt::Durable { sequence: 1, inventory_after: 2 });
        assert_eq!(controller.spilled_count(), Ok(2));
    }

    let reopened = RoutedSpillController::new(RedbSpillBacking::create(&db.path).expect("open 2"));
    assert_eq!(reopened.spilled_count(), Ok(2));
    assert_eq!(restore_controller(&reopened).expect("restore first"), Some(b"first".to_vec()));
    assert_eq!(reopened.spilled_count(), Ok(1));
    assert_eq!(restore_controller(&reopened).expect("restore second"), Some(b"second".to_vec()));
    assert_eq!(reopened.spilled_count(), Ok(0));
}
#[test]
fn concurrent_accepts_lose_no_handle_and_assign_distinct_keys() {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, Barrier};

    const WRITERS: usize = 4;
    const WRITES_PER_WRITER: usize = 2;

    let db = test_db();
    let backing = Arc::new(RedbSpillBacking::create(&db.path).expect("open"));
    let start = Arc::new(Barrier::new(WRITERS + 1));
    let expected = WRITERS.checked_mul(WRITES_PER_WRITER).expect("bounded test record count");
    let expected_u64 = u64::try_from(expected).expect("record count fits u64");

    let mut handles = Vec::new();
    for writer in 0..WRITERS {
        let backing = Arc::clone(&backing);
        let start = Arc::clone(&start);
        handles.push(std::thread::spawn(move || -> Vec<(Vec<u8>, SpillReceipt)> {
            let mut accepted = Vec::with_capacity(WRITES_PER_WRITER);
            start.wait();
            for record in 0..WRITES_PER_WRITER {
                let payload = format!("writer-{writer}-record-{record}").into_bytes();
                let receipt = spill_backing(&backing, &payload).expect("concurrent accept");
                accepted.push((payload, receipt));
            }
            accepted
        }));
    }
    start.wait();

    let mut accepted = Vec::with_capacity(expected);
    for handle in handles {
        accepted.extend(handle.join().expect("writer thread panicked"));
    }
    assert_eq!(accepted.len(), expected, "every writer returned every acceptance receipt");

    let mut accepted_payloads = BTreeMap::new();
    let mut sequences = BTreeSet::new();
    for (payload, receipt) in accepted {
        *accepted_payloads.entry(payload).or_insert(0usize) += 1;
        let SpillReceipt::Durable { sequence, .. } = receipt else {
            panic!("redb backing must report durable acceptance: {receipt:?}");
        };
        assert!(sequences.insert(sequence), "duplicate durable sequence {sequence}");
    }
    let expected_sequences = (0..expected_u64).collect::<BTreeSet<_>>();
    assert_eq!(sequences, expected_sequences, "durable sequences are contiguous from zero");
    assert_eq!(backing.retained_count().expect("len"), expected_u64, "no accepted record was lost");

    let mut restored_payloads = BTreeMap::new();
    while let Some(payload) = restore_backing(&backing).expect("restore") {
        *restored_payloads.entry(payload).or_insert(0usize) += 1;
    }
    assert_eq!(restored_payloads, accepted_payloads, "exact payload multiset round-trips");
    assert_eq!(backing.retained_count().expect("len"), 0, "fully drained");
}
#[test]
fn sep23_fup001_reserved_hole_is_not_finalized_by_later_record() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    let hole = backing.reserve_record_id().expect("reserve hole id");
    assert_eq!(hole.get(), 0);
    let second = backing.reserve_record_id().expect("reserve second id");
    assert_eq!(second.get(), 1);
    assert_eq!(backing.resolve_record(hole), Ok(DurableRecordResolution::Absent));

    let receipt = backing.commit_spill(second, b"second").expect("commit id 1");
    assert_eq!(receipt, SpillReceipt::Durable { sequence: 1, inventory_after: 1 });
    assert_eq!(backing.retained_count().expect("one payload retained"), 1);

    // Restore and consume ONLY id 1; the queue/recovery bytes prove which
    // record actually existed.
    let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
    assert_eq!(candidate.sequence, 1);
    assert_eq!(candidate.bytes(), b"second");
    backing.commit_restore(&candidate.claim).expect("commit restore");
    assert!(
        matches!(backing.resolve_record(second), Ok(DurableRecordResolution::Present { .. })),
        "a recovery-phase record still exists and is not consumed"
    );
    backing.finalize_restore(&candidate.claim).expect("finalize");
    assert_eq!(backing.retained_count().expect("drained"), 0);
    assert_eq!(backing.resolve_record(second), Ok(DurableRecordResolution::Finalized));
    // The hole was never durable: it must NOT be misread as consumed.
    assert_eq!(backing.resolve_record(hole), Ok(DurableRecordResolution::Absent));
    drop(backing);

    let reopened = RedbSpillBacking::create(&db.path).expect("reopen");
    assert_eq!(
        reopened.resolve_record(hole),
        Ok(DurableRecordResolution::Absent),
        "hole disposition survives reopen"
    );
    assert_eq!(
        reopened.resolve_record(second),
        Ok(DurableRecordResolution::Finalized),
        "consumed disposition survives reopen"
    );
    assert_eq!(reopened.retained_count().expect("still drained"), 0);
}
#[test]
fn sep23_fup001_reserve_only_reopen_is_not_consumed() {
    let db = test_db();
    let reserved = {
        let backing = RedbSpillBacking::create(&db.path).expect("open 1");
        backing.reserve_record_id().expect("reserve only")
    };
    let backing = RedbSpillBacking::create(&db.path).expect("open 2");
    // An empty queue after a bare reserve is a hole, not a consumed record.
    assert_eq!(backing.retained_count().expect("empty"), 0);
    assert_eq!(backing.resolve_record(reserved), Ok(DurableRecordResolution::Absent));
    // The reservation itself survived: the same id still commits exactly once.
    let receipt = backing.commit_spill(reserved, b"late").expect("late commit");
    assert_eq!(receipt, SpillReceipt::Durable { sequence: reserved.get(), inventory_after: 1 });
    assert!(matches!(
        backing.resolve_record(reserved),
        Ok(DurableRecordResolution::Present { .. })
    ));
    assert_eq!(restore_backing(&backing).expect("restore"), Some(b"late".to_vec()));
    assert_eq!(backing.resolve_record(reserved), Ok(DurableRecordResolution::Finalized));
}
#[test]
fn sep23_fup001_out_of_order_commit_finalized_reinsert_rejected() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    let first = backing.reserve_record_id().expect("reserve 0");
    let second = backing.reserve_record_id().expect("reserve 1");
    // Commit in reverse allocation order: disposition follows the payload,
    // not the id order.
    let _receipt = backing.commit_spill(second, b"bbb").expect("commit 1 first");
    let _receipt = backing.commit_spill(first, b"aaa").expect("commit 0 second");
    assert!(matches!(backing.resolve_record(first), Ok(DurableRecordResolution::Present { .. })));
    assert!(matches!(backing.resolve_record(second), Ok(DurableRecordResolution::Present { .. })));

    // FIFO restores id 0 first; finalizing it must not affect id 1.
    let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
    assert_eq!(candidate.sequence, 0);
    backing.commit_restore(&candidate.claim).expect("commit restore 0");
    backing.finalize_restore(&candidate.claim).expect("finalize 0");
    assert_eq!(backing.resolve_record(first), Ok(DurableRecordResolution::Finalized));
    assert!(matches!(backing.resolve_record(second), Ok(DurableRecordResolution::Present { .. })));

    // A truly finalized id can never be recreated, with same or new bytes.
    assert!(matches!(
        backing.commit_spill(first, b"aaa"),
        Err(DurableWriteFailure::NotCommitted(_))
    ));
    assert!(matches!(
        backing.commit_spill(first, b"other"),
        Err(DurableWriteFailure::NotCommitted(_))
    ));
    // Conflicting bytes on a live id are rejected and preserve the payload.
    assert!(matches!(
        backing.commit_spill(second, b"different"),
        Err(DurableWriteFailure::NotCommitted(_))
    ));
    let survivor = backing.prepare_restore().expect("prepare").expect("candidate");
    assert_eq!(survivor.sequence, 1);
    assert_eq!(survivor.bytes, b"bbb", "rejected rewrite must preserve bytes");
    backing.commit_restore(&survivor.claim).expect("commit restore 1");
    backing.finalize_restore(&survivor.claim).expect("finalize 1");
    assert_eq!(backing.retained_count().expect("drained"), 0);
}
#[test]
fn sep23_fup001_same_bytes_retry_keeps_inventory_stable() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    let id = backing.reserve_record_id().expect("reserve");
    let first = backing.commit_spill(id, b"payload").expect("commit");
    let retry = backing.commit_spill(id, b"payload").expect("idempotent retry");
    assert_eq!(retry, first, "same id + same bytes is a duplicate insert-free retry");
    assert_eq!(backing.retained_count().expect("one record"), 1);
    assert!(matches!(backing.resolve_record(id), Ok(DurableRecordResolution::Present { .. })));
}
#[test]
fn limits_reject_oversized_record_before_commit_without_consuming_reserved_id() {
    let db = test_db();
    let backing = RedbSpillBacking::create_with_limits(&db.path, tiny_limits()).expect("open");
    let id = backing.reserve_record_id().expect("reserve");
    let failure = backing.commit_spill(id, b"12345").expect_err("record cap");
    assert!(matches!(failure, DurableWriteFailure::NotCommitted(error)
        if matches!(*error, MemoryError::Capacity(_))));
    assert_eq!(backing.resolve_record(id), Ok(DurableRecordResolution::Absent));
    assert_eq!(backing.retained_count(), Ok(0));
    let _ = backing.commit_spill(id, b"1234").expect("same reserved id remains writable");
    assert_eq!(restore_backing(&backing).expect("restore"), Some(b"1234".to_vec()));
    assert_eq!(backing.retained_count(), Ok(0));
}
#[test]
fn retained_limits_count_recovery_and_refund_only_after_finalize() {
    let db = test_db();
    let backing = RedbSpillBacking::create_with_limits(&db.path, tiny_limits()).expect("open");
    let _ = spill_backing(&backing, b"abc").expect("first");
    let _ = spill_backing(&backing, b"def").expect("exact total byte and record caps");
    let candidate = backing.prepare_restore().expect("prepare").expect("first");
    backing.commit_restore(&candidate.claim).expect("move to recovery");
    let denied_id = backing.reserve_record_id().expect("reserve third");
    let denial = backing.commit_spill(denied_id, b"x").expect_err("recovery still counts");
    assert!(matches!(denial, DurableWriteFailure::NotCommitted(error)
        if matches!(*error, MemoryError::Capacity(_))));
    assert_eq!(backing.resolve_record(denied_id), Ok(DurableRecordResolution::Absent));
    assert_eq!(backing.retained_count(), Ok(2));
    backing.finalize_restore(&candidate.claim).expect("finalize first");
    let _ = backing.commit_spill(denied_id, b"xy").expect("finalize refunds capacity");
    assert_eq!(backing.retained_count(), Ok(2));
    drop(backing);
    let reopened = RedbSpillBacking::create_with_limits(&db.path, tiny_limits()).expect("reopen");
    assert_eq!(restore_backing(&reopened).expect("second"), Some(b"def".to_vec()));
    assert_eq!(restore_backing(&reopened).expect("third"), Some(b"xy".to_vec()));
    assert_eq!(reopened.retained_count(), Ok(0));
}
#[test]
fn retained_payload_cap_denies_next_record_without_changing_durable_authority() {
    let db = test_db();
    let limits = RedbSpillLimits { max_retained_records: 3, ..tiny_limits() };
    let backing = RedbSpillBacking::create_with_limits(&db.path, limits).expect("open");
    let _ = spill_backing(&backing, b"abc").expect("first");
    let _ = spill_backing(&backing, b"def").expect("exact byte cap");
    let denied = backing.reserve_record_id().expect("reserve third");
    let failure = backing.commit_spill(denied, b"x").expect_err("byte cap");
    assert!(matches!(failure, DurableWriteFailure::NotCommitted(error)
        if matches!(*error, MemoryError::Capacity(_))));
    assert_eq!(backing.resolve_record(denied), Ok(DurableRecordResolution::Absent));
    assert_eq!(backing.retained_count(), Ok(2));
    assert_eq!(restore_backing(&backing).expect("first"), Some(b"abc".to_vec()));
    let _ = backing.commit_spill(denied, b"x").expect("finalize frees byte quota");
    assert_eq!(restore_backing(&backing).expect("second"), Some(b"def".to_vec()));
    assert_eq!(restore_backing(&backing).expect("third"), Some(b"x".to_vec()));
}
#[test]
fn spill_receipt_inventory_includes_other_records_in_recovery_slot() {
    let db = test_db();
    let backing = RedbSpillBacking::create_with_limits(&db.path, tiny_limits()).expect("open");
    let _ = spill_backing(&backing, b"a").expect("first");
    let candidate = backing.prepare_restore().expect("prepare").expect("first");
    backing.commit_restore(&candidate.claim).expect("recovery slot");
    let second = backing.reserve_record_id().expect("reserve second");
    let receipt = backing.commit_spill(second, b"b").expect("second");
    assert_eq!(receipt.inventory_after(), 2);
    let retry = backing.commit_spill(second, b"b").expect("idempotent retry");
    assert_eq!(retry.inventory_after(), 2);
    assert_eq!(backing.retained_count(), Ok(2));
    backing.finalize_restore(&candidate.claim).expect("first finalize");
    assert_eq!(backing.retained_count(), Ok(1));
}
#[test]
fn in_flight_restore_budget_denial_preserves_claim_and_releases_on_candidate_drop() {
    let db = test_db();
    let limits = RedbSpillLimits { max_inflight_restore_buffers: 3, ..tiny_limits() };
    let backing = RedbSpillBacking::create_with_limits(&db.path, limits).expect("open");
    let _ = spill_backing(&backing, b"four").expect("persist");
    let first = backing.prepare_restore().expect("prepare").expect("first candidate");
    let second = backing.recover_active_restore().expect("steal").expect("second candidate");
    assert_eq!(second.candidate.bytes(), b"four");
    assert!(matches!(backing.recover_active_restore(), Err(MemoryError::Capacity(_))));
    assert_eq!(backing.restore_budget_used.load(std::sync::atomic::Ordering::SeqCst), 8);
    assert_eq!(backing.restore_buffers_used.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
    assert_eq!(backing.retained_count(), Ok(1));
    drop(first);
    let third =
        backing.recover_active_restore().expect("retry after lease drop").expect("third candidate");
    drop(second);
    backing.commit_restore(&third.candidate.claim).expect("commit under bounded staging");
    backing.finalize_restore(&third.candidate.claim).expect("finalize");
    drop(third);
    assert_eq!(backing.retained_count(), Ok(0));
    assert_eq!(backing.restore_budget_used.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(backing.restore_buffers_used.load(std::sync::atomic::Ordering::SeqCst), 0);
}
#[test]
fn zero_byte_restore_candidates_are_bounded_by_buffer_count() {
    let db = test_db();
    let backing = RedbSpillBacking::create_with_limits(&db.path, tiny_limits()).expect("open");
    let _ = spill_backing(&backing, b"").expect("persist zero-byte record");
    let first = backing.prepare_restore().expect("prepare").expect("first candidate");
    let second = backing.recover_active_restore().expect("steal").expect("second candidate");
    assert!(first.bytes().is_empty() && second.candidate.bytes().is_empty());
    assert!(matches!(backing.recover_active_restore(), Err(MemoryError::Capacity(_))));
    assert_eq!(backing.retained_count(), Ok(1));
    drop(first);
    let third = backing.recover_active_restore().expect("retry").expect("third candidate");
    drop(second);
    backing.commit_restore(&third.candidate.claim).expect("commit under count limit");
    backing.finalize_restore(&third.candidate.claim).expect("finalize");
    drop(third);
    assert_eq!(backing.retained_count(), Ok(0));
    assert_eq!(backing.restore_buffers_used.load(std::sync::atomic::Ordering::SeqCst), 0);
}
#[test]
fn staging_denial_after_staged_remove_rolls_back_durable_payload() {
    let db = test_db();
    let backing = RedbSpillBacking::create_with_limits(&db.path, tiny_limits()).expect("open");
    let _ = spill_backing(&backing, b"four").expect("persist");
    let first = backing.prepare_restore().expect("prepare").expect("first candidate");
    let second = backing.recover_active_restore().expect("steal").expect("second candidate");
    assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
    let failure = backing
        .commit_restore(&second.candidate.claim)
        .expect_err("transactional copy needs a third buffer");
    assert!(matches!(failure, DurableCommitFailure::Retained(error)
        if matches!(*error, MemoryError::Capacity(_))));
    assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
    assert_eq!(
        backing.resolve_record(DurableRecordId::new(0)),
        Ok(DurableRecordResolution::Present { inventory_after: 1 }),
        "staged remove must roll back with the transaction"
    );
    assert_eq!(backing.retained_count(), Ok(1));
    assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
    drop(first);
    assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
    backing.commit_restore(&second.candidate.claim).expect("retry after capacity release");
    backing.finalize_restore(&second.candidate.claim).expect("finalize");
    drop(second);
    assert_eq!(backing.retained_count(), Ok(0));
    assert_eq!(backing.restore_buffers_used.load(std::sync::atomic::Ordering::SeqCst), 0);
}
#[test]
fn concurrent_writers_at_record_cap_admit_exactly_one_transaction() {
    use std::sync::{Arc, Barrier};

    let db = test_db();
    let limits = RedbSpillLimits { max_retained_records: 1, ..tiny_limits() };
    let backing = Arc::new(RedbSpillBacking::create_with_limits(&db.path, limits).expect("open"));
    let barrier = Arc::new(Barrier::new(3));
    let workers = (0..2)
        .map(|_| {
            let backing = Arc::clone(&backing);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let id = backing.reserve_record_id().expect("reserve");
                barrier.wait();
                (id, backing.commit_spill(id, b"xy"))
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let outcomes =
        workers.into_iter().map(|worker| worker.join().expect("writer")).collect::<Vec<_>>();
    let mut accepted = 0;
    for (id, outcome) in outcomes {
        match outcome {
            Ok(receipt) => {
                accepted += 1;
                assert_eq!(receipt.inventory_after(), 1);
                assert!(matches!(
                    backing.resolve_record(id),
                    Ok(DurableRecordResolution::Present { .. })
                ));
            }
            Err(DurableWriteFailure::NotCommitted(error)) => {
                assert!(matches!(*error, MemoryError::Capacity(_)));
                assert_eq!(backing.resolve_record(id), Ok(DurableRecordResolution::Absent));
            }
            Err(other) => panic!("unexpected write outcome: {other:?}"),
        }
    }
    assert_eq!(accepted, 1);
    assert_eq!(backing.retained_count(), Ok(1));
    assert_eq!(restore_backing(&backing).expect("drain"), Some(b"xy".to_vec()));
    assert_eq!(backing.retained_count(), Ok(0));
}
#[test]
fn concurrent_prepare_restore_issues_exactly_one_live_claim() {
    use std::sync::{Arc, Barrier};

    let db = test_db();
    let backing = Arc::new(RedbSpillBacking::create(&db.path).expect("open"));
    let _ = spill_backing(&backing, b"claim").expect("persist");
    let barrier = Arc::new(Barrier::new(3));
    let workers = (0..2)
        .map(|_| {
            let backing = Arc::clone(&backing);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                backing.prepare_restore()
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();

    let mut winner = None;
    let mut blocked = 0;
    for worker in workers {
        match worker.join().expect("preparer") {
            Ok(Some(candidate)) => {
                assert!(winner.replace(candidate).is_none(), "two claims escaped");
            }
            Err(MemoryError::OperationInProgress(_)) => blocked += 1,
            other => panic!("unexpected prepare outcome: {other:?}"),
        }
    }
    assert_eq!(blocked, 1);
    let winner = winner.expect("one live claim");
    assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(0))));
    backing.commit_restore(&winner.claim).expect("winner commits");
    backing.finalize_restore(&winner.claim).expect("winner finalizes");
    drop(winner);
    assert_eq!(backing.retained_count(), Ok(0));
    assert_eq!(backing.restore_buffers_used.load(AtomicOrdering::SeqCst), 0);
}
#[test]
fn concurrent_recovery_claims_are_generation_fenced_to_latest_owner() {
    use std::sync::{Arc, Barrier};

    let db = test_db();
    let limits = RedbSpillLimits {
        max_inflight_restore_bytes: 16,
        max_inflight_restore_buffers: 4,
        ..tiny_limits()
    };
    let backing = Arc::new(RedbSpillBacking::create_with_limits(&db.path, limits).expect("open"));
    let _ = spill_backing(&backing, b"four").expect("persist");
    let original = backing.prepare_restore().expect("prepare").expect("original claim");
    let barrier = Arc::new(Barrier::new(3));
    let workers = (0..2)
        .map(|_| {
            let backing = Arc::clone(&backing);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                backing.recover_active_restore().expect("recover").expect("active")
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let mut recovered =
        workers.into_iter().map(|worker| worker.join().expect("recoverer")).collect::<Vec<_>>();
    recovered.sort_by_key(|recovery| recovery.candidate.claim.generation());
    assert_eq!(recovered.len(), 2);
    assert!(recovered[0].candidate.claim.generation() < recovered[1].candidate.claim.generation());
    assert!(matches!(
        backing.commit_restore(&recovered[0].candidate.claim),
        Err(DurableCommitFailure::Retained(_))
    ));
    backing.commit_restore(&recovered[1].candidate.claim).expect("latest generation commits");
    backing.finalize_restore(&recovered[1].candidate.claim).expect("latest generation finalizes");
    drop(original);
    drop(recovered);
    assert_eq!(backing.retained_count(), Ok(0));
    assert_eq!(backing.restore_budget_used.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(backing.restore_buffers_used.load(AtomicOrdering::SeqCst), 0);
}
#[test]
fn tighter_reopen_rejects_existing_inventory_without_erasing_it() {
    let db = test_db();
    let original = tiny_limits();
    {
        let backing = RedbSpillBacking::create_with_limits(&db.path, original).expect("open");
        let _ = spill_backing(&backing, b"four").expect("persist");
    }
    let tightened =
        RedbSpillLimits { max_retained_payload_bytes: 3, max_record_bytes: 3, ..original };
    let failure = RedbSpillBacking::create_with_limits(&db.path, tightened)
        .expect_err("existing payload exceeds tighter policy");
    assert!(matches!(failure, MemoryError::Capacity(_)));
    let reopened = RedbSpillBacking::create_with_limits(&db.path, original).expect("reopen");
    assert_eq!(restore_backing(&reopened).expect("payload survived"), Some(b"four".to_vec()));
}
