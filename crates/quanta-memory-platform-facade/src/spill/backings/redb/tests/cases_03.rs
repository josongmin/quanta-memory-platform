use super::*;

#[test]
fn sep23_fup002_stale_generation_rejected_after_claim_steal() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    let id = backing.reserve_record_id().expect("reserve");
    let _receipt = backing.commit_spill(id, b"owned").expect("commit");
    let first = backing.prepare_restore().expect("prepare").expect("candidate");
    assert_eq!(first.claim.generation(), 1);

    // A second recovery steals the live claim with a fresh generation.
    let stolen = backing.recover_active_restore().expect("recover").expect("recovery");
    assert_eq!(stolen.phase, DurableRestorePhase::Prepared);
    assert_eq!(stolen.candidate.sequence, 0);
    assert_eq!(stolen.candidate.bytes(), b"owned");
    assert_eq!(stolen.candidate.claim.generation(), 2);
    assert_eq!(stolen.candidate.claim.backend(), first.claim.backend());

    // The stale generation can no longer move the record; the payload stays
    // authoritative and the rejection is `Retained`, not ambiguous.
    assert!(matches!(backing.commit_restore(&first.claim), Err(DurableCommitFailure::Retained(_))));
    backing.commit_restore(&stolen.candidate.claim).expect("live claimant commits");
    assert!(
        matches!(backing.finalize_restore(&first.claim), Err(MemoryError::InvalidState(_))),
        "stale generation must not finalize"
    );
    backing.finalize_restore(&stolen.candidate.claim).expect("live claimant finalizes");
    assert_eq!(backing.resolve_record(id), Ok(DurableRecordResolution::Finalized));
    // Even after the record is gone, the OTHER claimant's finalize is
    // rejected: idempotent success requires the same generation's proof.
    assert!(matches!(backing.finalize_restore(&first.claim), Err(MemoryError::InvalidState(_))));
    // ...while the winner's own retry is exactly idempotent.
    backing.finalize_restore(&stolen.candidate.claim).expect("same-claimant finalize retry");
}
#[test]
fn sep23_fup002_finalize_proof_is_per_record_not_last_writer() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    for payload in [b"r0".as_slice(), b"r1".as_slice()] {
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, payload).expect("commit");
    }
    let first = backing.prepare_restore().expect("prepare 0").expect("candidate 0");
    backing.commit_restore(&first.claim).expect("commit 0");
    backing.finalize_restore(&first.claim).expect("finalize 0");
    let second = backing.prepare_restore().expect("prepare 1").expect("candidate 1");
    backing.commit_restore(&second.claim).expect("commit 1");
    backing.finalize_restore(&second.claim).expect("finalize 1");

    // A later record's finalize must not invalidate the first claimant's
    // proof: same-generation retries stay exactly idempotent per record.
    backing.finalize_restore(&first.claim).expect("retry first finalize");
    backing.finalize_restore(&second.claim).expect("retry second finalize");
    assert_eq!(
        backing.resolve_record(DurableRecordId::new(0)),
        Ok(DurableRecordResolution::Finalized)
    );
    assert_eq!(
        backing.resolve_record(DurableRecordId::new(1)),
        Ok(DurableRecordResolution::Finalized)
    );
}
#[test]
fn sep23_fup002_foreign_backend_claim_rejected_on_every_permission_change() {
    let db_a = test_db();
    let db_b = test_db();
    let backing_a = RedbSpillBacking::create(&db_a.path).expect("open a");
    let backing_b = RedbSpillBacking::create(&db_b.path).expect("open b");
    // Same numeric sequences in both files: sequence alone proves nothing.
    for backing in [&backing_a, &backing_b] {
        let id = backing.reserve_record_id().expect("reserve");
        assert_eq!(id.get(), 0);
        let _receipt = backing.commit_spill(id, b"payload").expect("commit");
    }
    let claim_a = backing_a.prepare_restore().expect("prepare a").expect("candidate a").claim;
    let claim_b = backing_b.prepare_restore().expect("prepare b").expect("candidate b").claim;
    assert_ne!(claim_a.backend(), claim_b.backend(), "distinct files, distinct identities");

    // Every authority-changing operation rejects the foreign claim before
    // touching the record; both records stay authoritative.
    assert!(matches!(backing_b.commit_restore(&claim_a), Err(DurableCommitFailure::Retained(_))));
    assert!(matches!(backing_b.finalize_restore(&claim_a), Err(MemoryError::InvalidState(_))));
    assert!(matches!(backing_b.abort_restore(&claim_a), Err(MemoryError::InvalidState(_))));
    assert!(matches!(backing_b.reconcile_restore(&claim_a), Err(MemoryError::InvalidState(_))));
    assert!(matches!(backing_a.commit_restore(&claim_b), Err(DurableCommitFailure::Retained(_))));
    for (backing, id) in [(&backing_a, 0u64), (&backing_b, 0u64)] {
        assert!(
            matches!(
                backing.resolve_record(DurableRecordId::new(id)),
                Ok(DurableRecordResolution::Present { .. })
            ),
            "foreign attempts must not move either record"
        );
    }
    // Each backend still serves its own claimant exactly once.
    backing_a.commit_restore(&claim_a).expect("commit a");
    backing_a.finalize_restore(&claim_a).expect("finalize a");
    backing_b.abort_restore(&claim_b).expect("abort b releases its own claim");
    assert_eq!(backing_b.active_restore(), Ok(None));
}
#[test]
fn sep23_fup002_backend_identity_stable_across_reopen() {
    let db = test_db();
    let before = {
        let backing = RedbSpillBacking::create(&db.path).expect("open 1");
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, b"stable").expect("commit");
        backing.prepare_restore().expect("prepare").expect("candidate").claim.backend()
    };
    let backing = RedbSpillBacking::create(&db.path).expect("open 2");
    let recovery = backing.recover_active_restore().expect("recover").expect("recovery");
    assert_eq!(recovery.candidate.claim.backend(), before, "identity survives reopen");
    assert_eq!(recovery.candidate.bytes(), b"stable");
    backing.commit_restore(&recovery.candidate.claim).expect("recovered claim commits");
    backing.finalize_restore(&recovery.candidate.claim).expect("recovered claim finalizes");
}
#[test]
fn backend_identity_and_pending_claim_survive_cold_rename() {
    let db = test_db();
    let renamed = db.directory.join("renamed.redb");
    let candidate = {
        let backing = RedbSpillBacking::create(&db.path).expect("open original");
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, b"rename").expect("commit");
        backing.prepare_restore().expect("prepare").expect("candidate")
    };
    std::fs::rename(&db.path, &renamed).expect("cold rename");

    let reopened = RedbSpillBacking::create(&renamed).expect("open renamed lineage");
    assert_eq!(reopened.backend_id, candidate.claim.backend());
    reopened.commit_restore(&candidate.claim).expect("pre-rename claim commits");
    reopened.finalize_restore(&candidate.claim).expect("pre-rename claim finalizes");
    assert_eq!(reopened.retained_count(), Ok(0));
}
#[test]
fn consistent_cold_copy_preserves_logical_lineage_identity() {
    let db = test_db();
    let copied = db.directory.join("copied.redb");
    let candidate = {
        let backing = RedbSpillBacking::create(&db.path).expect("open original");
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, b"snapshot").expect("commit");
        backing.prepare_restore().expect("prepare").expect("candidate")
    };
    std::fs::copy(&db.path, &copied).expect("copy quiesced database");

    let replacement = RedbSpillBacking::create(&copied).expect("open copied lineage");
    assert_eq!(replacement.backend_id, candidate.claim.backend());
    replacement
        .commit_restore(&candidate.claim)
        .expect("same-lineage claim commits on replacement");
    replacement
        .finalize_restore(&candidate.claim)
        .expect("same-lineage claim finalizes on replacement");
    assert_eq!(replacement.retained_count(), Ok(0));
}
#[test]
fn sep23_audit_finalize_retry_succeeds_while_unrelated_claim_is_live() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    for payload in [b"first".as_slice(), b"second".as_slice()] {
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, payload).expect("commit");
    }
    let first = backing.prepare_restore().expect("prepare 0").expect("candidate 0");
    assert_eq!(first.sequence, 0);
    backing.commit_restore(&first.claim).expect("commit 0");
    backing.finalize_restore(&first.claim).expect("finalize 0 lands");
    // A new restore makes an unrelated claim live.
    let second = backing.prepare_restore().expect("prepare 1").expect("candidate 1");
    assert_eq!(second.sequence, 1);
    // Same-claimant ACK-loss retry of the landed finalize: idempotent Ok,
    // not an error the caller must reconcile away.
    backing.finalize_restore(&first.claim).expect("landed finalize retry succeeds");
    assert_eq!(
        backing.active_restore().expect("active"),
        Some(DurableRecordId::new(1)),
        "the retry leaves the live claim untouched"
    );
    assert_eq!(second.bytes(), b"second", "the live candidate is intact");
    backing.abort_restore(&second.claim).expect("abort the live claim");
    backing
        .finalize_restore(&first.claim)
        .expect("landed proof still answers after the live claim aborts");
    assert_eq!(backing.retained_count().expect("count"), 1, "only record 1 remains");
    assert_eq!(
        backing.resolve_record(DurableRecordId::new(0)).expect("resolve"),
        DurableRecordResolution::Finalized,
        "record 0 stays exactly finalized"
    );
}
#[test]
fn ambiguous_finalize_probe_uses_own_record_proof_with_foreign_claim_live() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    for payload in [b"first".as_slice(), b"second".as_slice()] {
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, payload).expect("commit");
    }
    let first = backing.prepare_restore().expect("prepare first").expect("first");
    backing.commit_restore(&first.claim).expect("commit first");
    assert!(!backing.finalize_landed(first.sequence, first.claim.generation()).expect("preimage"));
    backing.finalize_restore(&first.claim).expect("finalize first");
    let second = backing.prepare_restore().expect("prepare second").expect("second");
    assert_eq!(backing.active_restore(), Ok(Some(DurableRecordId::new(second.sequence))));
    assert!(backing
        .finalize_landed(first.sequence, first.claim.generation())
        .expect("exact proof"));
    assert!(!backing
        .finalize_landed(first.sequence, second.claim.generation())
        .expect("foreign generation"));
    assert!(!backing
        .finalize_landed(second.sequence, second.claim.generation())
        .expect("unfinalized"));
}
#[test]
fn lost_finalize_and_abort_acks_allow_a_new_claim_before_reconciliation() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    for payload in [b"first".as_slice(), b"second".as_slice()] {
        let id = backing.reserve_record_id().expect("reserve");
        let _receipt = backing.commit_spill(id, payload).expect("commit");
    }
    let first = backing.prepare_restore().expect("prepare first").expect("first");
    backing.commit_restore(&first.claim).expect("commit first");
    backing.inject_lost_finalize_ack_with_next_claim();
    backing.finalize_restore(&first.claim).expect("exact finalize proof survives foreign claim");
    assert_eq!(backing.active_claim().expect("new claim").map(|claim| claim.0), Some(1));

    let second = backing.recover_active_restore().expect("recover second").expect("second");
    backing.inject_lost_abort_ack_with_next_claim();
    backing.abort_restore(&second.candidate.claim).expect("own claim was released");
    let next = backing.active_claim().expect("next claim").expect("active");
    assert_eq!(next.0, 1);
    assert_ne!(next.1, Some(second.candidate.claim.generation()));
    assert!(matches!(
        backing.abort_restore(&second.candidate.claim),
        Err(MemoryError::InvalidState(_))
    ));
}
#[test]
fn sep23_audit_finalize_proof_does_not_mask_reappeared_recovery_payload() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    let id = backing.reserve_record_id().expect("reserve");
    let _receipt = backing.commit_spill(id, b"payload").expect("commit");
    let candidate = backing.prepare_restore().expect("prepare").expect("candidate");
    backing.commit_restore(&candidate.claim).expect("commit restore");
    backing.finalize_restore(&candidate.claim).expect("finalize");

    let txn = backing.database.begin_write().expect("write");
    {
        let mut recovery = txn.open_table(SPILL_RECOVERY).expect("recovery");
        recovery.insert(id.get(), b"corrupt".as_slice()).expect("inject inconsistency");
    }
    txn.commit().expect("commit inconsistency");

    assert!(
        matches!(backing.finalize_restore(&candidate.claim), Err(MemoryError::InvalidState(_))),
        "a finalize proof must not hide a recovery payload that contradicts terminal state"
    );
}
#[test]
fn accept_at_exhausted_keyspace_fails_closed_without_overwriting() {
    let db = test_db();
    let backing = RedbSpillBacking::create(&db.path).expect("open");
    backing.seed_key_for_test(u64::MAX, b"last").expect("seed max key");

    let err = backing
        .reserve_record_id()
        .expect_err("reservation at an exhausted keyspace must be rejected");
    assert!(matches!(err, MemoryError::Capacity(_)), "got {err:?}");

    // The retained handle at u64::MAX was NOT overwritten or lost.
    assert_eq!(backing.retained_count().expect("len"), 1, "the seeded handle survives");
    assert_eq!(restore_backing(&backing).expect("restore"), Some(b"last".to_vec()));
}
