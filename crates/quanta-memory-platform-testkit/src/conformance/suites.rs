//! The reusable conformance suite runner.
//!
//! [`run_broker_accounting_suite`] drives any [`BrokerConformanceTarget`] through
//! a deterministic workload and asserts the host-independent accounting
//! invariants. Each check is attributed to a [`ContractFamily`], so a failure
//! localizes to a portable contract rather than a host package.
//!
//! The suite is itself test infrastructure: it MUST reject a broker that lies
//! about its accounting. The crate-level self-test feeds a deliberately broken
//! fake into this suite and asserts the verdict is `Failed`.

use crate::conformance::invariants::{AssertionVerdict, ContractFamily, InvariantAssertion};
use crate::conformance::target::{
    AccountingView, AllocatorObservation, BrokerConformanceTarget, ChargeAxis,
};
#[cfg(feature = "fixtures")]
use crate::fixtures::{MemoryFixture, MemoryTestKit};
use quanta_memory_platform_core::MemoryError;
#[cfg(test)]
use quanta_memory_platform_facade::TranscriptRetentionPolicyV2;
use quanta_memory_platform_facade::{
    AbortDispositionV1, Budgeted, PrepareDispositionV1, PreparedPublicationErrorV1,
    PreparedPublicationIdentityV1, PreparedPublicationRequestV1, PreparedPublicationService,
    PreparedPublicationStatsV1, PreparedPublicationTokenV1, SessionTranscriptStore,
    SimpleSessionTranscriptStore, TranscriptAppendDispositionV1, TranscriptAppendErrorV1,
    TranscriptAppendIdentityV1, TranscriptAppendRequestV1,
};

#[cfg(feature = "fixtures")]
use quanta_memory_platform_core::{
    LiveProcessMemoryBroker, MemoryClass, MemoryClassBudget, ReclaimPriority, StoreBindingHandle,
    SweepHint,
};
#[cfg(feature = "fixtures")]
use quanta_memory_platform_facade::{
    InMemorySpillTarget, LiveMemoryPlatform, MemoryPlatformBuilder, PayloadStore, RetainedMapStore,
    RevocableStore, RoutedSpillController, SimplePayloadStore, SimpleRetainedMapStore,
    SimpleRevocableStore, SpillController, SpillTarget,
};

/// Stable suite identifiers (kept for the public test-entry surface and for
/// host fixtures that select a named suite).
pub const CORE_ACCOUNTING_SUITE: &str = "core-accounting";
#[cfg(feature = "fixtures")]
pub const FACADE_STORE_SUITE: &str = "facade-store";

/// Backend-neutral target for prepared-publication replay and abort behavior.
pub trait PreparedPublicationConformanceTarget {
    type Token: Clone + std::fmt::Debug + PartialEq + Eq;

    fn prepare(
        &self,
        identity: &[u8],
        payload: &[u8],
    ) -> Result<(Self::Token, PrepareDispositionV1), PreparedPublicationErrorV1>;
    fn resolve(&self, token: &Self::Token) -> Result<Vec<u8>, PreparedPublicationErrorV1>;
    fn abort(&self, token: &Self::Token) -> Result<AbortDispositionV1, PreparedPublicationErrorV1>;
    fn orphan_tokens(
        &self,
        referenced_tokens: &[Self::Token],
    ) -> Result<Vec<Self::Token>, PreparedPublicationErrorV1>;
    fn gc_orphans(
        &self,
        referenced_tokens: &[Self::Token],
    ) -> Result<usize, PreparedPublicationErrorV1>;
    fn stats(&self) -> Result<PreparedPublicationStatsV1, PreparedPublicationErrorV1>;
}

impl PreparedPublicationConformanceTarget for PreparedPublicationService<Vec<u8>, Budgeted> {
    type Token = PreparedPublicationTokenV1;

    fn prepare(
        &self,
        identity: &[u8],
        payload: &[u8],
    ) -> Result<(Self::Token, PrepareDispositionV1), PreparedPublicationErrorV1> {
        let identity = PreparedPublicationIdentityV1::new(identity.to_vec())?;
        let receipt =
            self.prepare(PreparedPublicationRequestV1::new(identity, payload.to_vec()))?;
        Ok((receipt.token().clone(), receipt.disposition()))
    }

    fn resolve(&self, token: &Self::Token) -> Result<Vec<u8>, PreparedPublicationErrorV1> {
        self.resolve(token)
    }

    fn abort(&self, token: &Self::Token) -> Result<AbortDispositionV1, PreparedPublicationErrorV1> {
        self.abort(token)
    }

    fn orphan_tokens(
        &self,
        referenced_tokens: &[Self::Token],
    ) -> Result<Vec<Self::Token>, PreparedPublicationErrorV1> {
        self.orphan_tokens(referenced_tokens)
    }

    fn gc_orphans(
        &self,
        referenced_tokens: &[Self::Token],
    ) -> Result<usize, PreparedPublicationErrorV1> {
        self.gc_orphans(referenced_tokens)
    }

    fn stats(&self) -> Result<PreparedPublicationStatsV1, PreparedPublicationErrorV1> {
        self.stats()
    }
}

/// Exercise exact replay, payload-conflict, token resolution, and exact abort.
/// The returned verdicts deliberately compare both disposition and retained
/// accounting so a fake cannot pass by returning `Replayed` after mutating.
pub fn run_prepared_publication_suite<T>(target: &T) -> Vec<AssertionVerdict>
where
    T: PreparedPublicationConformanceTarget,
{
    // Independently specified V1 semantic cost; never ask the target to
    // calculate its own expected bytes. Vec<u8>::canonical_key retains a
    // second copy of the payload bytes while prepared.
    let evidence_cost = |identity_len: u64| identity_len + 128;
    let prepared_cost = |identity_len: u64, payload_len: u64| {
        evidence_cost(identity_len) + payload_len + payload_len + identity_len + 16
    };
    let initial_prepared = prepared_cost(8, 7);
    let first_evidence = evidence_cost(8);
    let final_evidence = first_evidence + evidence_cost(15) + evidence_cost(19);
    let replay = InvariantAssertion::new(
        ContractFamily::ReserveFailClosed,
        "prepared_publication_replay_is_inert",
    );
    let conflict = InvariantAssertion::new(
        ContractFamily::ReserveFailClosed,
        "prepared_publication_payload_drift_conflicts",
    );
    let abort = InvariantAssertion::new(
        ContractFamily::ReclaimAccounting,
        "prepared_publication_abort_reclaims_exactly_once",
    );
    let root_aware_gc = InvariantAssertion::new(
        ContractFamily::ReclaimAccounting,
        "prepared_publication_gc_preserves_root_references",
    );

    let first = target.prepare(b"identity", b"payload");
    let replayed = target.prepare(b"identity", b"payload");
    let before_conflict = target.stats();
    let drift = target.prepare(b"identity", b"different");
    let after_conflict = target.stats();

    let (resolution, abort_first, abort_replay, final_stats) = match &first {
        Ok((token, _)) => {
            (target.resolve(token), target.abort(token), target.abort(token), target.stats())
        }
        Err(error) => {
            let failure = PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(format!(
                "first prepare failed: {error}"
            )));
            (Err(failure.clone()), Err(failure.clone()), Err(failure), target.stats())
        }
    };

    let orphan = target.prepare(b"orphan-identity", b"orphan");
    let referenced = target.prepare(b"referenced-identity", b"rooted");
    let root_aware_gc_verdict = match (&orphan, &referenced) {
        (
            Ok((orphan_token, PrepareDispositionV1::Applied)),
            Ok((referenced_token, PrepareDispositionV1::Applied)),
        ) => {
            let roots = [referenced_token.clone()];
            let enumerated = target.orphan_tokens(&roots);
            let collected = target.gc_orphans(&roots);
            let orphan_after = target.resolve(orphan_token);
            let referenced_after = target.resolve(referenced_token);
            let cleanup = target.abort(referenced_token);
            let stats_after_cleanup = target.stats();

            root_aware_gc.assert_that(
                matches!(&enumerated, Ok(tokens) if tokens == std::slice::from_ref(orphan_token))
                    && matches!(collected, Ok(1))
                    && matches!(
                        &orphan_after,
                        Err(PreparedPublicationErrorV1::AbortedToken { .. })
                    )
                    && matches!(&referenced_after, Ok(payload) if payload == b"rooted")
                    && matches!(cleanup, Ok(AbortDispositionV1::Aborted))
                    && matches!(
                        stats_after_cleanup,
                        Ok(PreparedPublicationStatsV1 {
                            prepared_count: 0,
                            identity_evidence_count: 3,
                            identity_evidence_bytes,
                            charged_bytes,
                            tombstone_count: 3,
                            tombstone_bytes,
                        }) if charged_bytes == final_evidence && identity_evidence_bytes == final_evidence && tombstone_bytes == final_evidence
                    ),
                format!(
                    "orphan={orphan:?} referenced={referenced:?} enumerated={enumerated:?} collected={collected:?} orphan_after={orphan_after:?} referenced_after={referenced_after:?} cleanup={cleanup:?} final={stats_after_cleanup:?}"
                ),
            )
        }
        _ => {
            root_aware_gc.assert_that(false, format!("orphan={orphan:?} referenced={referenced:?}"))
        }
    };

    vec![
        replay.assert_that(
            matches!((&first, &replayed, &before_conflict),
                (Ok((first_token, PrepareDispositionV1::Applied)),
                 Ok((replay_token, PrepareDispositionV1::Replayed)),
                 Ok(PreparedPublicationStatsV1 { prepared_count: 1, identity_evidence_count: 1, identity_evidence_bytes, charged_bytes, tombstone_count: 0, tombstone_bytes: 0 }))
                 if first_token == replay_token && *charged_bytes == initial_prepared && *identity_evidence_bytes == first_evidence),
            format!("first={first:?} replay={replayed:?} stats={before_conflict:?}"),
        ),
        conflict.assert_that(
            matches!(&drift, Err(PreparedPublicationErrorV1::IdentityConflict { .. }))
                && before_conflict == after_conflict,
            format!(
                "drift={drift:?} before={before_conflict:?} after={after_conflict:?}"
            ),
        ),
        abort.assert_that(
            matches!(&resolution, Ok(bytes) if bytes == b"payload")
                && matches!(abort_first, Ok(AbortDispositionV1::Aborted))
                && matches!(abort_replay, Ok(AbortDispositionV1::Replayed))
                && matches!(
                    final_stats,
                    Ok(PreparedPublicationStatsV1 {
                        prepared_count: 0,
                        identity_evidence_count: 1,
                        identity_evidence_bytes,
                        charged_bytes,
                        tombstone_count: 1,
                        tombstone_bytes,
                    }) if charged_bytes == first_evidence && identity_evidence_bytes == first_evidence && tombstone_bytes == first_evidence
                ),
            format!(
                "resolution={resolution:?} abort={abort_first:?} replay={abort_replay:?} final={final_stats:?}"
            ),
        ),
        root_aware_gc_verdict,
    ]
}

/// A loaded transcript row: sequence, message identity, and body.
pub type TranscriptRow = (u64, Vec<u8>, Vec<u8>);

/// Backend-neutral target for transcript append idempotency conformance.
pub trait TranscriptAppendConformanceTarget {
    fn append(
        &self,
        session_id: &[u8],
        sequence: u64,
        message_id: &[u8],
        body: &[u8],
    ) -> Result<TranscriptAppendDispositionV1, TranscriptAppendErrorV1>;
    fn load(&self, session_id: &[u8]) -> Result<Vec<TranscriptRow>, MemoryError>;
    fn truncate_to(&self, session_id: &[u8], max_messages: usize) -> Result<(), MemoryError>;
    fn retained_bytes(&self) -> Result<u64, MemoryError>;
}

impl TranscriptAppendConformanceTarget
    for SimpleSessionTranscriptStore<Vec<u8>, Vec<u8>, Vec<u8>, Budgeted>
{
    fn append(
        &self,
        session_id: &[u8],
        sequence: u64,
        message_id: &[u8],
        body: &[u8],
    ) -> Result<TranscriptAppendDispositionV1, TranscriptAppendErrorV1> {
        SessionTranscriptStore::append(
            self,
            TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new(session_id.to_vec(), sequence, message_id.to_vec()),
                body.to_vec(),
            ),
        )
    }

    fn load(&self, session_id: &[u8]) -> Result<Vec<TranscriptRow>, MemoryError> {
        SessionTranscriptStore::load(self, &session_id.to_vec()).map(|rows| {
            rows.into_iter()
                .map(|row| (row.sequence(), row.message_id().clone(), row.into_body()))
                .collect()
        })
    }

    fn truncate_to(&self, session_id: &[u8], max_messages: usize) -> Result<(), MemoryError> {
        SessionTranscriptStore::truncate_to(self, &session_id.to_vec(), max_messages)
    }

    fn retained_bytes(&self) -> Result<u64, MemoryError> {
        self.retained_bytes()
    }
}

/// Exercise exact replay, all three collision axes, truncate-late-retry, and
/// fail-closed saturation against a target configured for two evidence entries.
pub fn run_transcript_append_suite<T>(target: &T) -> Vec<AssertionVerdict>
where
    T: TranscriptAppendConformanceTarget,
{
    // An independent, literal V1 semantic-cost oracle. The target may not
    // supply its own formula as the expected value.
    let session = 1 + 24 + 16;
    let evidence = 2 * 2 + 2 * 24 + 2 * 32 + 2 * 16;
    let first_active = 3 + 2 + 24 + 16;
    let first_expected = session + evidence + first_active;
    let after_truncate_expected = first_expected + evidence;
    let replay = InvariantAssertion::new(
        ContractFamily::ReserveFailClosed,
        "transcript_exact_replay_is_inert",
    );
    let collisions = InvariantAssertion::new(
        ContractFamily::ReserveFailClosed,
        "transcript_collision_axes_are_typed",
    );
    let retention = InvariantAssertion::new(
        ContractFamily::ReclaimAccounting,
        "transcript_truncate_preserves_retry_tombstone",
    );
    let saturation = InvariantAssertion::new(
        ContractFamily::ReserveFailClosed,
        "transcript_new_identity_fails_closed_at_retention_bound",
    );

    let first = target.append(b"s", 0, b"m0", b"one");
    let replayed = target.append(b"s", 0, b"m0", b"one");
    let rows_after_replay = target.load(b"s");
    let bytes_after_replay = target.retained_bytes();
    let payload_conflict = target.append(b"s", 0, b"m0", b"different");
    let sequence_conflict = target.append(b"s", 0, b"m1", b"one");
    let message_conflict = target.append(b"s", 1, b"m0", b"one");

    let second = target.append(b"s", 1, b"m1", b"four");
    let truncate = target.truncate_to(b"s", 1);
    let late_retry = target.append(b"s", 1, b"m1", b"four");
    let rows_after_late_retry = target.load(b"s");
    let bytes_after_late_retry = target.retained_bytes();
    let saturated = target.append(b"s", 2, b"m2", b"five");
    let rows_after_saturation = target.load(b"s");
    let bytes_after_saturation = target.retained_bytes();

    vec![
        replay.assert_that(
            matches!(first, Ok(TranscriptAppendDispositionV1::Applied))
                && matches!(replayed, Ok(TranscriptAppendDispositionV1::Replayed))
                && matches!(&rows_after_replay, Ok(rows) if rows.len() == 1)
                && matches!(bytes_after_replay, Ok(bytes) if bytes == first_expected),
            format!(
                "first={first:?} replay={replayed:?} rows={rows_after_replay:?} bytes={bytes_after_replay:?}"
            ),
        ),
        collisions.assert_that(
            matches!(
                payload_conflict,
                Err(TranscriptAppendErrorV1::PayloadConflict { sequence: 0 })
            ) && matches!(
                sequence_conflict,
                Err(TranscriptAppendErrorV1::SequenceConflict { sequence: 0 })
            ) && matches!(
                message_conflict,
                Err(TranscriptAppendErrorV1::MessageIdConflict)
            ),
            format!(
                "payload={payload_conflict:?} sequence={sequence_conflict:?} message={message_conflict:?}"
            ),
        ),
        retention.assert_that(
            matches!(second, Ok(TranscriptAppendDispositionV1::Applied))
                && truncate.is_ok()
                && matches!(late_retry, Ok(TranscriptAppendDispositionV1::Replayed))
                && matches!(&rows_after_late_retry, Ok(rows) if rows.len() == 1)
                && matches!(bytes_after_late_retry, Ok(bytes) if bytes == after_truncate_expected),
            format!(
                "second={second:?} truncate={truncate:?} late={late_retry:?} rows={rows_after_late_retry:?} bytes={bytes_after_late_retry:?}"
            ),
        ),
        saturation.assert_that(
            matches!(
                saturated,
                Err(TranscriptAppendErrorV1::RetentionSaturated {
                    evidence_count: 2,
                    max_evidence_count: 2,
                    ..
                })
            ) && rows_after_saturation == rows_after_late_retry
                && bytes_after_saturation == bytes_after_late_retry,
            format!(
                "saturated={saturated:?} rows_before={rows_after_late_retry:?} rows_after={rows_after_saturation:?} bytes_before={bytes_after_late_retry:?} bytes_after={bytes_after_saturation:?}"
            ),
        ),
    ]
}

/// Run the broker accounting conformance suite against a target.
///
/// Returns the full ordered list of [`AssertionVerdict`]s. The caller (the
/// [`ConformanceSuite`](crate::conformance::ConformanceSuite)) folds these into a
/// pass/fail verdict; returning the verdicts (not just a bool) keeps the evidence
/// for localization and reporting.
pub fn run_broker_accounting_suite<T>(target: &mut T) -> Vec<AssertionVerdict>
where
    T: BrokerConformanceTarget,
{
    let mut verdicts = Vec::new();
    let owner_a = "owner:conformance-a";
    let owner_b = "owner:conformance-b";
    let t1 = target.do_reserve(0, owner_a, ChargeAxis::RetainedPayload, 1_000);
    let t2 = target.do_reserve(1, owner_a, ChargeAxis::RetainedMetadata, 250);
    let t3 = target.do_reserve(0, owner_b, ChargeAxis::LiveScratch, 500);
    let reserve_ok = t1.is_ok() && t2.is_ok() && t3.is_ok();
    let reserve_assert =
        InvariantAssertion::new(ContractFamily::ReserveFailClosed, "in_cap_reserves_succeed");
    verdicts.push(reserve_assert.assert_that(
        reserve_ok,
        format!(
            "uncapped in-budget reserves must succeed: t1={:?} t2={:?} t3={:?}",
            t1.as_ref().map(|_| "ok").map_err(|e| e.clone()),
            t2.as_ref().map(|_| "ok").map_err(|e| e.clone()),
            t3.as_ref().map(|_| "ok").map_err(|e| e.clone()),
        ),
    ));
    let commits = match (&t1, &t2, &t3) {
        (Ok(t1), Ok(t2), Ok(t3)) => {
            [target.do_commit(t1), target.do_commit(t2), target.do_commit(t3)]
        }
        _ => [
            Err("reserve failed".into()),
            Err("reserve failed".into()),
            Err("reserve failed".into()),
        ],
    };
    let lifecycle =
        InvariantAssertion::new(ContractFamily::TicketLifecycle, "every_commit_result_checked");
    verdicts.push(
        lifecycle
            .assert_that(commits.iter().all(Result::is_ok), format!("commit_results={commits:?}")),
    );

    let expected_live = AccountingView {
        total_class_bytes: 1_750,
        total_owner_bytes: 1_750,
        total_context_bytes: 1_750,
        context_axis_bytes: [1_750, 0, 0, 0, 0],
        class_axis_bytes: [1_500, 250],
        owner_axis_bytes: [1_250, 500],
        group_axis_bytes: [0, 0],
        // t1: class0/A/payload/1000; t2: class1/A/metadata/250;
        // t3: class0/B/scratch/500. Every bucket carries its exact kind split.
        class_kind_bytes: [[1_000, 0, 500], [0, 250, 0]],
        owner_kind_bytes: [[1_000, 250, 0], [0, 0, 500]],
        context_kind_bytes: [[1_000, 250, 500], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0]],
        live_ticket_count: 3,
        total_retained_payload_bytes: 1_000,
        total_retained_metadata_bytes: 250,
        total_live_scratch_bytes: 500,
        rebuild_in_progress: false,
    };
    let observed_live = target.view();
    let exact = InvariantAssertion::new(
        ContractFamily::OwnerPartition,
        "independent_exact_accounting_model",
    );
    verdicts.push(exact.assert_that(
        observed_live.as_ref() == Ok(&expected_live),
        format!("expected={expected_live:?} observed={observed_live:?}"),
    ));
    let balance =
        InvariantAssertion::new(ContractFamily::ClassOwnerBalance, "observed_class_equals_owner");
    verdicts.push(balance.assert_that(
        matches!(&observed_live, Ok(view) if view.total_class_bytes == view.total_owner_bytes),
        format!("observed={observed_live:?}"),
    ));
    let partition =
        InvariantAssertion::new(ContractFamily::OwnerPartition, "observed_owner_partitions");
    verdicts.push(partition.assert_that(
        matches!(&observed_live, Ok(view) if Some(view.total_owner_bytes) == view.checked_partition_sum() && view.total_owner_bytes == view.total_context_bytes && Some(view.total_context_bytes) == view.checked_context_sum()),
        format!("observed={observed_live:?}"),
    ));
    // FUP-004: the kind split must agree on EVERY axis, not just the owner
    // axis. A payload<->metadata swap confined to one axis preserves every
    // scalar total; only the per-axis kind vectors catch it.
    let kinds = InvariantAssertion::new(
        ContractFamily::OwnerPartition,
        "observed_kind_split_agrees_across_axes",
    );
    verdicts.push(kinds.assert_that(
        matches!(&observed_live, Ok(view) if view.checked_class_kind_totals().is_some_and(|class| Some(class) == view.checked_owner_kind_totals() && Some(class) == view.checked_context_kind_totals())),
        format!("observed={observed_live:?}"),
    ));

    let duplicate_commit = t1.as_ref().ok().map(|ticket| target.do_commit(ticket));
    verdicts.push(lifecycle.assert_that(
        matches!(duplicate_commit, Some(Err(_))),
        format!("second commit must reject: {duplicate_commit:?}"),
    ));

    let fence = InvariantAssertion::new(ContractFamily::RebuildWriteFence, "rebuild_fences_writes");
    let begin = target.do_begin_rebuild();
    let begin_ok = begin.is_ok();
    verdicts.push(fence.assert_that(begin_ok, format!("begin_rebuild_ok={begin_ok}")));
    if let Ok(rebuild_permit) = begin {
        let before_denied = target.view();
        let during = target.do_reserve(0, owner_a, ChargeAxis::RetainedPayload, 1);
        verdicts.push(fence.assert_that(
            during.is_err(),
            format!("reserve during rebuild must be rejected, got {during:?}"),
        ));
        let after_denied = target.view();
        let fail_closed = InvariantAssertion::new(
            ContractFamily::ReserveFailClosed,
            "rebuild_denied_reserve_is_inert",
        );
        // The denied reserve must leave EVERY accounting axis untouched, not just
        // the grand total. Compare the full view so a compensating lie (e.g. one
        // axis up, another down) is still caught.
        verdicts.push(fail_closed.assert_that(
            before_denied == after_denied,
            format!(
                "denied reserve during rebuild mutated accounting: before={before_denied:?} after={after_denied:?}"
            ),
        ));
        let finish = target.do_finish_rebuild(rebuild_permit);
        verdicts.push(fence.assert_that(finish.is_ok(), format!("finish_rebuild={finish:?}")));
        let after = target.do_reserve(0, owner_a, ChargeAxis::RetainedPayload, 1);
        let after_ok = after.is_ok();
        let release_after = after.as_ref().map_or_else(
            |_| Err("post-rebuild reserve failed".into()),
            |ticket| target.do_release(ticket),
        );
        verdicts.push(
            fence.assert_that(after_ok, "reserve after rebuild window closes must be accepted"),
        );
        verdicts.push(
            lifecycle.assert_that(
                release_after.is_ok(),
                format!("post-rebuild release={release_after:?}"),
            ),
        );
    } else {
        verdicts.push(fence.assert_that(false, "target failed to enter rebuild window"));
    }

    let advisory = InvariantAssertion::new(
        ContractFamily::AllocatorAdvisory,
        "allocator_observation_is_inert",
    );
    let before_advisory = target.view();
    let semantic_bytes = before_advisory.as_ref().map(|view| view.total_owner_bytes);
    let mismatch = target.observe_allocator(&AllocatorObservation {
        resident_bytes: semantic_bytes.ok().and_then(|bytes| bytes.checked_add(7)),
    });
    let after_mismatch = target.view();
    verdicts.push(advisory.assert_that(
        matches!(
            (&semantic_bytes, &mismatch),
            (Ok(bytes), Ok(view))
                if view.semantic_bytes == *bytes
                    && view.allocator_resident_bytes
                        == bytes.checked_add(7)
                    && view.mismatch
                    && view.drift_bytes == Some(7)
        ) && before_advisory == after_mismatch,
        format!(
            "mismatch={mismatch:?} before_advisory={before_advisory:?} after_mismatch={after_mismatch:?}"
        ),
    ));
    let absent = target.observe_allocator(&AllocatorObservation { resident_bytes: None });
    let after_absent = target.view();
    verdicts.push(advisory.assert_that(
        matches!(
            (&semantic_bytes, &absent),
            (Ok(bytes), Ok(view))
                if view.semantic_bytes == *bytes
                    && view.allocator_resident_bytes.is_none()
                    && !view.mismatch
                    && view.drift_bytes.is_none()
        ) && before_advisory == after_absent,
        format!(
            "absent={absent:?} before_advisory={before_advisory:?} after_absent={after_absent:?}"
        ),
    ));

    let releases = match (&t1, &t2, &t3) {
        (Ok(t1), Ok(t2), Ok(t3)) => {
            [target.do_release(t1), target.do_release(t2), target.do_release(t3)]
        }
        _ => [
            Err("reserve failed".into()),
            Err("reserve failed".into()),
            Err("reserve failed".into()),
        ],
    };
    verdicts.push(lifecycle.assert_that(
        releases.iter().all(Result::is_ok),
        format!("terminal_releases={releases:?}"),
    ));
    let duplicate_release = t1.as_ref().ok().map(|ticket| target.do_release(ticket));
    verdicts.push(lifecycle.assert_that(
        matches!(duplicate_release, Some(Err(_))),
        format!("second release must reject: {duplicate_release:?}"),
    ));
    let final_view = target.view();
    verdicts.push(lifecycle.assert_that(
        final_view.as_ref() == Ok(&AccountingView::default()),
        format!("final exact zero expected, observed={final_view:?}"),
    ));

    verdicts
}

#[cfg(feature = "fixtures")]
mod facade_slices;
#[cfg(feature = "fixtures")]
use facade_slices::*;

/// Run the live broker-backed portable facade-store suite against `kit`.
///
/// The harness stays host-independent: it builds only portable facade slices and
/// a real `LiveProcessMemoryBroker` from the fixture manifest. Verdicts are
/// emitted as ordinary [`AssertionVerdict`] values so callers get the same
/// localized evidence shape as the accounting suite.
#[cfg(feature = "fixtures")]
pub fn run_facade_store_suite(kit: &MemoryTestKit) -> Vec<AssertionVerdict> {
    let mut verdicts = Vec::new();
    verdicts.extend(run_payload_store_slice(kit));
    verdicts.extend(run_retained_map_slice(kit));
    verdicts.extend(run_transcript_slice(kit));
    verdicts.extend(run_spill_slice());
    verdicts.extend(run_all_store_lifecycle_suites(kit));
    verdicts
}

/// A backend-neutral, destructive lifecycle target. Each scenario receives a
/// fresh target from the factory; the oracle never reaches into a store's
/// private accounting or implementation state.
#[cfg(feature = "fixtures")]
pub trait StoreLifecycleConformanceTarget {
    /// Permit coupling the rebuild window's open and close (production:
    /// the non-Clone epoch fence; fixtures: `()`).
    type RebuildFence;

    fn populate(&mut self) -> Result<(), String>;
    fn retained_bytes(&self) -> Result<u64, String>;
    fn ledger_bytes(&self) -> Result<u64, String>;
    fn close(&mut self) -> Result<(), String>;
    fn post_close_mutation_is_typed_closed(&mut self) -> bool;
    fn begin_rebuild(&mut self) -> Result<Self::RebuildFence, String>;
    fn finish_rebuild(&mut self, fence: Self::RebuildFence) -> Result<(), String>;
    fn drop_store(&mut self);
}

/// Public semantic oracle shared by honest five-store adapters and broken
/// lifecycle fakes. A passing result requires exact store/ledger agreement at
/// every observable transition, not merely a successful method return.
#[cfg(feature = "fixtures")]
pub fn run_store_lifecycle_suite<T, F>(name: &str, mut factory: F) -> Vec<AssertionVerdict>
where
    T: StoreLifecycleConformanceTarget,
    F: FnMut() -> Result<T, String>,
{
    let assertion = |suffix| {
        InvariantAssertion::new(ContractFamily::ReclaimAccounting, format!("{name}_{suffix}"))
    };
    let mut verdicts = Vec::new();

    let close_case = (|| {
        let mut target = factory()?;
        target.populate()?;
        let before = (target.retained_bytes()?, target.ledger_bytes()?);
        target.close()?;
        let after = (target.retained_bytes()?, target.ledger_bytes()?);
        let typed_closed = target.post_close_mutation_is_typed_closed();
        target.close()?;
        let twice = (target.retained_bytes()?, target.ledger_bytes()?);
        Ok::<_, String>((before, after, typed_closed, twice))
    })();
    verdicts.push(assertion("close_exactly_once").assert_that(
        matches!(&close_case, Ok(((store, ledger), (0, 0), true, (0, 0))) if *store > 0 && store == ledger),
        format!("{close_case:?}"),
    ));

    let rebuild_case = (|| {
        let mut target = factory()?;
        target.populate()?;
        let before = (target.retained_bytes()?, target.ledger_bytes()?);
        let rebuild_permit = target.begin_rebuild()?;
        let denied = target.close().is_err();
        let during = (target.retained_bytes()?, target.ledger_bytes()?);
        target.finish_rebuild(rebuild_permit)?;
        target.close()?;
        let after = (target.retained_bytes()?, target.ledger_bytes()?);
        Ok::<_, String>((before, denied, during, after))
    })();
    verdicts.push(assertion("rebuild_close_fail_closed").assert_that(
        matches!(&rebuild_case, Ok(((store, ledger), true, during, (0, 0)))
            if *store > 0 && store == ledger && during == &(*store, *ledger)),
        format!("{rebuild_case:?}"),
    ));

    let drop_case = (|| {
        let mut target = factory()?;
        target.populate()?;
        let before = (target.retained_bytes()?, target.ledger_bytes()?);
        target.drop_store();
        Ok::<_, String>((before, target.ledger_bytes()?))
    })();
    verdicts.push(assertion("last_owner_drop_releases").assert_that(
        matches!(&drop_case, Ok(((store, ledger), 0)) if *store > 0 && store == ledger),
        format!("{drop_case:?}"),
    ));

    let rebuild_drop_case = (|| {
        let mut target = factory()?;
        target.populate()?;
        let before = (target.retained_bytes()?, target.ledger_bytes()?);
        let rebuild_permit = target.begin_rebuild()?;
        target.drop_store();
        let during = target.ledger_bytes()?;
        target.finish_rebuild(rebuild_permit)?;
        Ok::<_, String>((before, during, target.ledger_bytes()?))
    })();
    verdicts.push(assertion("rebuild_drop_defers_until_finish").assert_that(
        matches!(&rebuild_drop_case, Ok(((store, ledger), during, 0))
                if *store > 0 && store == ledger && ledger == during),
        format!("{rebuild_drop_case:?}"),
    ));
    verdicts
}

#[cfg(feature = "fixtures")]
enum LifecycleStore {
    Payload(SimplePayloadStore<String, Vec<u8>, Budgeted>),
    Retained(SimpleRetainedMapStore<String, Vec<u8>, Budgeted>),
    Revocable(SimpleRevocableStore<String, Vec<u8>, Budgeted>),
    Transcript(SimpleSessionTranscriptStore<String, String, Vec<u8>, Budgeted>),
    Prepared(PreparedPublicationService<Vec<u8>, Budgeted>),
}

#[cfg(feature = "fixtures")]
struct FacadeLifecycleTarget {
    platform: LiveMemoryPlatform,
    store: Option<LifecycleStore>,
}

#[cfg(feature = "fixtures")]
impl FacadeLifecycleTarget {
    fn new(kit: &MemoryTestKit, name: &str) -> Result<Self, String> {
        let platform = make_platform(kit)?;
        let store = match name {
            "payload" => LifecycleStore::Payload(
                platform
                    .payload_store_builder::<String, Vec<u8>>()
                    .bind(binding(MemoryClass::ArtifactCache, "lifecycle:payload"))
                    .build()
                    .map_err(|error| error.to_string())?,
            ),
            "retained" => LifecycleStore::Retained(
                platform
                    .retained_map_store_builder::<String, Vec<u8>>()
                    .bind(binding(MemoryClass::MemoResidency, "lifecycle:retained"))
                    .build(),
            ),
            "revocable" => LifecycleStore::Revocable(
                platform
                    .revocable_store_builder::<String, Vec<u8>>()
                    .bind(binding(MemoryClass::MemoResidency, "lifecycle:revocable"))
                    .build()
                    .map_err(|error| error.to_string())?,
            ),
            "transcript" => LifecycleStore::Transcript(
                platform
                    .transcript_store_builder::<String, String, Vec<u8>>()
                    .bind(binding(MemoryClass::SessionTranscript, "lifecycle:transcript"))
                    .build()
                    .map_err(|error| error.to_string())?,
            ),
            "prepared" => LifecycleStore::Prepared(
                platform
                    .prepared_publication_builder::<Vec<u8>>()
                    .bind(binding(MemoryClass::ArtifactCache, "lifecycle:prepared"))
                    .build()
                    .map_err(|error| error.to_string())?,
            ),
            _ => return Err(format!("unknown lifecycle store: {name}")),
        };
        Ok(Self { platform, store: Some(store) })
    }

    fn store(&self) -> Result<&LifecycleStore, String> {
        self.store.as_ref().ok_or_else(|| "store already dropped".to_string())
    }
}

#[cfg(feature = "fixtures")]
impl StoreLifecycleConformanceTarget for FacadeLifecycleTarget {
    type RebuildFence = quanta_memory_platform_core::RebuildFence;

    fn populate(&mut self) -> Result<(), String> {
        match self.store()? {
            LifecycleStore::Payload(store) => {
                for bytes in [vec![1; 8], vec![2; 13]] {
                    store
                        .put_canonical(quanta_memory_platform_core::RetainedEnvelope::new(bytes))
                        .map_err(|error| error.to_string())?;
                }
            }
            LifecycleStore::Retained(store) => {
                for (key, bytes) in [("a", vec![1; 8]), ("b", vec![2; 13])] {
                    store
                        .insert_checked(key.to_string(), bytes)
                        .map_err(|error| error.to_string())?;
                }
            }
            LifecycleStore::Revocable(store) => {
                for (key, bytes) in [("a", vec![1; 8]), ("b", vec![2; 13])] {
                    store
                        .insert_checked(key.to_string(), bytes)
                        .map_err(|error| error.to_string())?;
                }
            }
            LifecycleStore::Transcript(store) => {
                for (sequence, bytes) in [(0, vec![1; 8]), (1, vec![2; 13])] {
                    store
                        .append(TranscriptAppendRequestV1::new(
                            TranscriptAppendIdentityV1::new(
                                "s".to_string(),
                                sequence,
                                format!("m{sequence}"),
                            ),
                            bytes,
                        ))
                        .map_err(|error| error.to_string())?;
                }
            }
            LifecycleStore::Prepared(store) => {
                for (identity, bytes) in
                    [(b"a".as_slice(), vec![1; 8]), (b"b".as_slice(), vec![2; 13])]
                {
                    store
                        .prepare(PreparedPublicationRequestV1::new(
                            PreparedPublicationIdentityV1::new(identity.to_vec())
                                .map_err(|error| error.to_string())?,
                            bytes,
                        ))
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        Ok(())
    }

    fn retained_bytes(&self) -> Result<u64, String> {
        match self.store()? {
            LifecycleStore::Payload(store) => {
                store.charged_bytes().map_err(|error| error.to_string())
            }
            LifecycleStore::Retained(store) => {
                store.committed_bytes().map_err(|error| error.to_string())
            }
            LifecycleStore::Revocable(store) => {
                store.committed_bytes().map_err(|error| error.to_string())
            }
            LifecycleStore::Transcript(store) => {
                store.retained_bytes().map_err(|error| error.to_string())
            }
            LifecycleStore::Prepared(store) => {
                store.stats().map(|stats| stats.charged_bytes).map_err(|error| error.to_string())
            }
        }
    }

    fn ledger_bytes(&self) -> Result<u64, String> {
        broker_bytes(self.platform.broker())
    }

    fn close(&mut self) -> Result<(), String> {
        match self.store()? {
            LifecycleStore::Payload(store) => store.close().map_err(|error| error.to_string()),
            LifecycleStore::Retained(store) => store.close().map_err(|error| error.to_string()),
            LifecycleStore::Revocable(store) => store.close().map_err(|error| error.to_string()),
            LifecycleStore::Transcript(store) => store.close().map_err(|error| error.to_string()),
            LifecycleStore::Prepared(store) => store.close().map_err(|error| error.to_string()),
        }
    }

    fn post_close_mutation_is_typed_closed(&mut self) -> bool {
        match self.store() {
            Ok(LifecycleStore::Payload(store)) => matches!(
                store.put_canonical(quanta_memory_platform_core::RetainedEnvelope::new(vec![3; 1])),
                Err(MemoryError::Closed(_))
            ),
            Ok(LifecycleStore::Retained(store)) => matches!(
                store.insert_checked("after".to_string(), vec![3; 1]),
                Err(MemoryError::Closed(_))
            ),
            Ok(LifecycleStore::Revocable(store)) => matches!(
                store.insert_checked("after".to_string(), vec![3; 1]),
                Err(MemoryError::Closed(_))
            ),
            Ok(LifecycleStore::Transcript(store)) => matches!(
                store.append(TranscriptAppendRequestV1::new(
                    TranscriptAppendIdentityV1::new("s".to_string(), 3, "after".to_string()),
                    vec![3; 1],
                )),
                Err(TranscriptAppendErrorV1::Storage(MemoryError::Closed(_)))
            ),
            Ok(LifecycleStore::Prepared(store)) => matches!(
                store.prepare(PreparedPublicationRequestV1::new(
                    PreparedPublicationIdentityV1::new(b"after".to_vec()).expect("valid identity"),
                    vec![3; 1],
                )),
                Err(PreparedPublicationErrorV1::Storage(MemoryError::Closed(_)))
            ),
            Err(_) => false,
        }
    }

    fn begin_rebuild(&mut self) -> Result<Self::RebuildFence, String> {
        self.platform.broker().begin_rebuild().map_err(|error| error.to_string())
    }

    fn finish_rebuild(&mut self, fence: Self::RebuildFence) -> Result<(), String> {
        fence.finish().map(|_| ()).map_err(|error| error.to_string())
    }

    fn drop_store(&mut self) {
        drop(self.store.take());
    }
}

#[cfg(feature = "fixtures")]
fn run_all_store_lifecycle_suites(kit: &MemoryTestKit) -> Vec<AssertionVerdict> {
    let mut verdicts = Vec::new();
    for name in ["payload", "retained", "revocable", "transcript", "prepared"] {
        verdicts.extend(run_store_lifecycle_suite(name, || FacadeLifecycleTarget::new(kit, name)));
    }
    verdicts
}

#[cfg(test)]
mod tests;
