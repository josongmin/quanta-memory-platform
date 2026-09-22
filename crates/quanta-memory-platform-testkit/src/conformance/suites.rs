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
                            charged_bytes,
                            tombstone_count: 3,
                            tombstone_bytes,
                        }) if charged_bytes == final_evidence && tombstone_bytes == final_evidence
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
                 Ok(PreparedPublicationStatsV1 { prepared_count: 1, charged_bytes, tombstone_count: 0, tombstone_bytes: 0 }))
                 if first_token == replay_token && *charged_bytes == initial_prepared),
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
                        charged_bytes,
                        tombstone_count: 1,
                        tombstone_bytes,
                    }) if charged_bytes == first_evidence && tombstone_bytes == first_evidence
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
    verdicts.push(fence.assert_that(begin.is_ok(), format!("begin_rebuild={begin:?}")));
    if begin.is_ok() {
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
        let finish = target.do_finish_rebuild();
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
fn make_platform(kit: &MemoryTestKit) -> Result<LiveMemoryPlatform, String> {
    MemoryPlatformBuilder::new()
        .with_manifest(kit.fixture.manifest.clone())
        .build()
        .map_err(|error| error.to_string())
}

#[cfg(feature = "fixtures")]
fn binding(class: MemoryClass, owner: &str) -> StoreBindingHandle {
    StoreBindingHandle::new(class, owner.to_string(), None)
}

#[cfg(feature = "fixtures")]
fn broker_bytes(broker: &LiveProcessMemoryBroker) -> Result<u64, String> {
    broker.ledger_snapshot().map(|snapshot| snapshot.total_owner_bytes()).map_err(|e| e.to_string())
}

#[cfg(feature = "fixtures")]
fn session_transcript_budget(base: &MemoryTestKit, hard_cap_bytes: u64) -> MemoryTestKit {
    let mut manifest = base.fixture.manifest.clone();
    let fallback = MemoryClassBudget {
        reserved_bytes: 0,
        soft_cap_bytes: hard_cap_bytes,
        hard_cap_bytes,
        borrow_from_shared_pool: false,
        reclaim_priority: ReclaimPriority::Last,
    };
    let prior = manifest
        .residency
        .classes
        .get(&MemoryClass::SessionTranscript)
        .cloned()
        .unwrap_or(fallback);
    manifest.residency = manifest.residency.clone().with_class_budget(
        MemoryClass::SessionTranscript,
        MemoryClassBudget { soft_cap_bytes: hard_cap_bytes, hard_cap_bytes, ..prior },
    );
    MemoryTestKit { fixture: MemoryFixture { manifest, fake_pressure: base.fixture.fake_pressure } }
}

/// A kit whose `class` is tightly capped at `hard_cap_bytes`, for driving
/// over-cap / atomic-recharge cases on a specific store class.
#[cfg(feature = "fixtures")]
fn class_capped_kit(
    base: &MemoryTestKit,
    class: MemoryClass,
    hard_cap_bytes: u64,
) -> MemoryTestKit {
    let mut manifest = base.fixture.manifest.clone();
    manifest.residency = manifest.residency.clone().with_class_budget(
        class,
        MemoryClassBudget {
            reserved_bytes: 0,
            soft_cap_bytes: hard_cap_bytes,
            hard_cap_bytes,
            borrow_from_shared_pool: false,
            reclaim_priority: ReclaimPriority::Last,
        },
    );
    MemoryTestKit { fixture: MemoryFixture { manifest, fake_pressure: base.fixture.fake_pressure } }
}

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
    fn populate(&mut self) -> Result<(), String>;
    fn retained_bytes(&self) -> Result<u64, String>;
    fn ledger_bytes(&self) -> Result<u64, String>;
    fn close(&mut self) -> Result<(), String>;
    fn post_close_mutation_is_typed_closed(&mut self) -> bool;
    fn begin_rebuild(&mut self) -> Result<(), String>;
    fn finish_rebuild(&mut self) -> Result<(), String>;
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
        target.begin_rebuild()?;
        let denied = target.close().is_err();
        let during = (target.retained_bytes()?, target.ledger_bytes()?);
        target.finish_rebuild()?;
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
        target.begin_rebuild()?;
        target.drop_store();
        let during = target.ledger_bytes()?;
        target.finish_rebuild()?;
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

    fn begin_rebuild(&mut self) -> Result<(), String> {
        self.platform.broker().begin_rebuild().map_err(|error| error.to_string())
    }

    fn finish_rebuild(&mut self) -> Result<(), String> {
        self.platform.broker().finish_rebuild().map_err(|error| error.to_string())
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

#[cfg(feature = "fixtures")]
fn run_payload_store_slice(kit: &MemoryTestKit) -> Vec<AssertionVerdict> {
    // 32 body + the retained "payload-0" reference (9) + two semantic index
    // slots (canonical digest and reference lookup, 32 each) + entry state
    // (16). This is independent of the store's calculator.
    let expected_entry_bytes = 32 + 9 + 64 + 16;
    let accounting =
        InvariantAssertion::new(ContractFamily::ReclaimAccounting, "payload_dedup_refund");
    let pin_discipline = InvariantAssertion::new(ContractFamily::PinDiscipline, "pin_safe_delete");
    let platform = match make_platform(kit) {
        Ok(platform) => platform,
        Err(err) => {
            return vec![
                accounting.assert_that(false, format!("payload broker setup failed: {err}")),
                pin_discipline
                    .assert_that(false, "payload pin-safe delete skipped: broker setup failed"),
            ];
        }
    };
    let broker = platform.broker().clone();
    let store = platform
        .payload_store_builder::<String, Vec<u8>>()
        .bind(binding(MemoryClass::ArtifactCache, "conformance:payload"))
        .build()
        .expect("bound payload builder");
    let payload = vec![7u8; 32];
    let first = match store
        .put_canonical(quanta_memory_platform_core::RetainedEnvelope::new(payload.clone()))
    {
        Ok(published) => published,
        Err(err) => {
            return vec![
                accounting.assert_that(false, format!("first payload put failed: {err:?}")),
                pin_discipline
                    .assert_that(false, "payload pin-safe delete skipped: initial put failed"),
            ];
        }
    };
    let ledger_after_first = broker_bytes(&broker);
    let second = store.put_canonical(quanta_memory_platform_core::RetainedEnvelope::new(payload));
    let ledger_after_dedup = broker_bytes(&broker);
    let charged_after_dedup = store.charged_bytes().map_err(|e| e.to_string());
    let pin = store.pin(first.payload_ref());
    let ledger_before_pinned_delete = broker_bytes(&broker);
    let pinned_delete = store.delete_if_unpinned(first.payload_ref());
    let ledger_after_pinned_delete = broker_bytes(&broker);
    let charged_after_pinned_delete = store.charged_bytes().map_err(|e| e.to_string());
    let unpin = store.unpin(first.payload_ref());
    let final_delete = store.delete_if_unpinned(first.payload_ref());
    let final_ledger = broker_bytes(&broker);
    let final_charged = store.charged_bytes().map_err(|e| e.to_string());

    vec![
        accounting.assert_that(
            matches!(&second, Ok(published) if published.payload_ref() == first.payload_ref())
                && matches!(&ledger_after_first, Ok(bytes) if *bytes == expected_entry_bytes)
                && matches!(&ledger_after_dedup, Ok(bytes) if *bytes == expected_entry_bytes)
                && matches!(&charged_after_dedup, Ok(bytes) if *bytes == expected_entry_bytes)
                && final_delete.is_ok()
                && matches!(&final_ledger, Ok(bytes) if *bytes == 0)
                && matches!(&final_charged, Ok(bytes) if *bytes == 0),
            format!(
                "first_ref={} second={second:?} ledger_after_first={ledger_after_first:?} ledger_after_dedup={ledger_after_dedup:?} charged_after_dedup={charged_after_dedup:?} final_delete={final_delete:?} final_ledger={final_ledger:?} final_charged={final_charged:?}",
                first.payload_ref()
            ),
        ),
        pin_discipline.assert_that(
            pin.is_ok()
                && pinned_delete.is_err()
                && matches!(
                    (
                        &ledger_before_pinned_delete,
                        &ledger_after_pinned_delete,
                        &charged_after_pinned_delete
                    ),
                    (Ok(before), Ok(after), Ok(charged)) if *before == *after && *charged == *before
                )
                && unpin.is_ok()
                // The full discipline cycle belongs to THIS family: after unpin,
                // the delete must SUCCEED and refund (so a store that refuses a
                // delete even after unpin is localized to PinDiscipline, not only
                // to the accounting family).
                && final_delete.is_ok()
                && matches!(&final_ledger, Ok(bytes) if *bytes == 0)
                && matches!(&final_charged, Ok(bytes) if *bytes == 0),
            format!(
                "pin={pin:?} pinned_delete={pinned_delete:?} ledger_before_pinned_delete={ledger_before_pinned_delete:?} ledger_after_pinned_delete={ledger_after_pinned_delete:?} charged_after_pinned_delete={charged_after_pinned_delete:?} unpin={unpin:?} final_delete={final_delete:?} final_ledger={final_ledger:?} final_charged={final_charged:?}",
            ),
        ),
    ]
}

#[cfg(feature = "fixtures")]
fn run_retained_map_slice(kit: &MemoryTestKit) -> Vec<AssertionVerdict> {
    let accounting =
        InvariantAssertion::new(ContractFamily::ReclaimAccounting, "retained_map_refunds");
    let platform = match make_platform(kit) {
        Ok(platform) => platform,
        Err(err) => {
            return vec![
                accounting.assert_that(false, format!("retained-map broker setup failed: {err}"))
            ]
        }
    };
    run_retained_map_slice_with_platform(kit, &platform)
}

#[cfg(feature = "fixtures")]
fn run_retained_map_slice_with_platform(
    kit: &MemoryTestKit,
    platform: &LiveMemoryPlatform,
) -> Vec<AssertionVerdict> {
    let accounting =
        InvariantAssertion::new(ContractFamily::ReclaimAccounting, "retained_map_refunds");
    let broker = platform.broker();
    let store = platform
        .retained_map_store_builder::<String, Vec<u8>>()
        .bind(binding(MemoryClass::MemoResidency, "conformance:map"))
        .build();

    let overwrite = store
        .insert_checked("k".to_string(), vec![0u8; 24])
        .and_then(|_| store.insert_checked("k".to_string(), vec![1u8; 10]));
    let after_overwrite = broker_bytes(broker);
    let committed_after_overwrite = store.committed_bytes().map_err(|e| e.to_string());
    let seed_sweep = store
        .insert_checked("b".to_string(), vec![2u8; 7])
        .and_then(|_| store.insert_checked("c".to_string(), vec![3u8; 5]));
    let sweep_stats = store.sweep(SweepHint { target_bytes: 11 });
    let after_sweep = broker_bytes(broker);
    let committed_after_sweep = store.committed_bytes().map_err(|e| e.to_string());

    // Tight-cap overwrite: an in-place replace whose NET fits the class cap but
    // whose (old + new) transiently exceeds it. An atomic recharge admits it; a
    // non-atomic charge-then-refund regression would be denied. This is the only
    // way to localize a non-atomic recharge bug to the accounting family.
    let recharge = InvariantAssertion::new(
        ContractFamily::ReclaimAccounting,
        "retained_atomic_recharge_overwrite",
    );
    let recharge_verdict =
        match make_platform(&class_capped_kit(kit, MemoryClass::MemoResidency, 70)) {
            Ok(tight_platform) => {
                let tight_broker = tight_platform.broker();
                let tight_store = tight_platform
                    .retained_map_store_builder::<String, Vec<u8>>()
                    .bind(binding(MemoryClass::MemoResidency, "conformance:map-tight"))
                    .build();
                // key 1 + hash slot 24 + state 16: old=65, new=61.
                // Combined transient 126 exceeds cap 70; net new 61 fits.
                let seed = tight_store.insert_checked("k".to_string(), vec![0u8; 24]);
                let net_fit = tight_store.insert_checked("k".to_string(), vec![0u8; 20]);
                let tight_ledger = broker_bytes(tight_broker);
                recharge.assert_that(
                    seed.is_ok()
                        && net_fit.is_ok()
                        && matches!(&tight_ledger, Ok(bytes) if *bytes == 61),
                    format!("seed={seed:?} net_fit={net_fit:?} tight_ledger={tight_ledger:?}"),
                )
            }
            Err(err) => {
                recharge.assert_that(false, format!("tight retained broker setup failed: {err}"))
            }
        };

    vec![
        accounting.assert_that(
            overwrite.is_ok()
                && matches!(&after_overwrite, Ok(bytes) if *bytes == 51)
                && matches!(&committed_after_overwrite, Ok(bytes) if *bytes == 51)
                && seed_sweep.is_ok()
                && matches!(&sweep_stats, Ok(stats) if stats.reclaimed_items == 1 && stats.reclaimed_bytes == 46)
                && matches!(&after_sweep, Ok(bytes) if *bytes == 99)
                && matches!(&committed_after_sweep, Ok(bytes) if *bytes == 99)
                && matches!(store.get_checked(&"b".to_string()), Ok(Some(value)) if value == vec![2u8; 7])
                && matches!(store.get_checked(&"c".to_string()), Ok(None))
                && matches!(store.get_checked(&"k".to_string()), Ok(Some(value)) if value == vec![1u8; 10]),
            format!(
                "overwrite={overwrite:?} after_overwrite={after_overwrite:?} committed_after_overwrite={committed_after_overwrite:?} seed_sweep={seed_sweep:?} sweep_stats={sweep_stats:?} after_sweep={after_sweep:?} committed_after_sweep={committed_after_sweep:?} remaining_keys={:?}",
                store.keys_checked()
            ),
        ),
        recharge_verdict,
    ]
}

#[cfg(feature = "fixtures")]
fn run_transcript_slice(kit: &MemoryTestKit) -> Vec<AssertionVerdict> {
    let session = 1 + 24 + 16;
    let evidence = 2 * 2 + 2 * 24 + 2 * 32 + 2 * 16;
    let first_active = 2 + 2 + 24 + 16;
    let second_active = 8 + 2 + 24 + 16;
    let before_expected = session + evidence + first_active + evidence + second_active;
    let after_expected = before_expected - second_active;
    let accounting =
        InvariantAssertion::new(ContractFamily::ReclaimAccounting, "transcript_truncate_refund");
    let fail_closed = InvariantAssertion::new(
        ContractFamily::ReserveFailClosed,
        "transcript_over_cap_fail_closed",
    );

    let platform = match make_platform(kit) {
        Ok(platform) => platform,
        Err(err) => {
            return vec![
                accounting.assert_that(false, format!("transcript broker setup failed: {err}")),
                fail_closed
                    .assert_that(false, "transcript over-cap check skipped: broker setup failed"),
            ];
        }
    };
    let broker = platform.broker().clone();
    let store = platform
        .transcript_store_builder::<String, String, String>()
        .bind(binding(MemoryClass::SessionTranscript, "conformance:transcript"))
        .build()
        .expect("bound transcript builder");
    let truncate = store
        .append(TranscriptAppendRequestV1::new(
            TranscriptAppendIdentityV1::new("s".to_string(), 0, "m0".to_string()),
            "aa".to_string(),
        ))
        .and_then(|_| {
            store.append(TranscriptAppendRequestV1::new(
                TranscriptAppendIdentityV1::new("s".to_string(), 1, "m1".to_string()),
                "bbbbbbbb".to_string(),
            ))
        });
    let before_truncate = broker_bytes(&broker);
    let truncate_result = store.truncate_to(&"s".to_string(), 1);
    let after_truncate = broker_bytes(&broker);
    let retained_after_truncate = store.retained_bytes().map_err(|e| e.to_string());

    let tiny_kit = session_transcript_budget(kit, 4);
    let tiny_platform = match make_platform(&tiny_kit) {
        Ok(platform) => platform,
        Err(err) => {
            return vec![
                accounting.assert_that(
                    truncate.is_ok()
                        && matches!(&before_truncate, Ok(bytes) if *bytes == before_expected)
                        && truncate_result.is_ok()
                        && matches!(&after_truncate, Ok(bytes) if *bytes == after_expected)
                        && matches!(&retained_after_truncate, Ok(bytes) if *bytes == after_expected),
                    format!(
                        "truncate={truncate:?} before_truncate={before_truncate:?} truncate_result={truncate_result:?} after_truncate={after_truncate:?} retained_after_truncate={retained_after_truncate:?}",
                    ),
                ),
                fail_closed.assert_that(false, format!("tiny transcript broker setup failed: {err}")),
            ];
        }
    };
    let tiny_broker = tiny_platform.broker().clone();
    let tiny_store = tiny_platform
        .transcript_store_builder::<String, String, String>()
        .bind(binding(MemoryClass::SessionTranscript, "conformance:transcript-tight"))
        .build()
        .expect("bound transcript builder");
    let over_cap = tiny_store.append(TranscriptAppendRequestV1::new(
        TranscriptAppendIdentityV1::new("s".to_string(), 0, "m0".to_string()),
        "toolong".to_string(),
    ));
    let tiny_ledger_after = broker_bytes(&tiny_broker);
    let tiny_retained_after = tiny_store.retained_bytes().map_err(|e| e.to_string());

    vec![
        accounting.assert_that(
            truncate.is_ok()
                && matches!(&before_truncate, Ok(bytes) if *bytes == before_expected)
                && truncate_result.is_ok()
                && matches!(&after_truncate, Ok(bytes) if *bytes == after_expected)
                && matches!(&retained_after_truncate, Ok(bytes) if *bytes == after_expected),
            format!(
                "truncate={truncate:?} before_truncate={before_truncate:?} truncate_result={truncate_result:?} after_truncate={after_truncate:?} retained_after_truncate={retained_after_truncate:?}",
            ),
        ),
        fail_closed.assert_that(
            matches!(
                &over_cap,
                Err(TranscriptAppendErrorV1::Accounting(MemoryError::Capacity(_)))
            )
                && matches!(&tiny_ledger_after, Ok(bytes) if *bytes == 0)
                && matches!(&tiny_retained_after, Ok(bytes) if *bytes == 0)
                && matches!(tiny_store.stats(), Ok(stats) if stats.item_count == 0),
            format!(
                "over_cap={over_cap:?} tiny_ledger_after={tiny_ledger_after:?} tiny_retained_after={tiny_retained_after:?} tiny_stats={:?}",
                tiny_store.stats()
            ),
        ),
    ]
}

/// The SpillRouting round-trip verdict over an INJECTED controller, so the suite
/// can be driven against a broken (handle-dropping) target and proven to BITE —
/// not merely accept the honest in-memory target. A spill then restore must hand
/// back the same handle (canonical target inventory 1 -> 0).
#[cfg(feature = "fixtures")]
fn spill_round_trip_verdict<T: SpillTarget<String>>(
    online: &RoutedSpillController<T>,
) -> AssertionVerdict {
    let spill = InvariantAssertion::new(ContractFamily::SpillRouting, "spill_round_trip");
    let spill_ok = online.spill("handle-a".to_string());
    // Capture inventory BETWEEN spill and restore: a target that drops the handle
    // would survive an "after == 0" check alone. The round-trip must go 1 -> 0.
    let before_restore = online.spilled_count();
    let restore = online.restore();
    let after = online.spilled_count();
    spill.assert_that(
        spill_ok.is_ok()
            && before_restore == Ok(1)
            && matches!(&restore, Ok(Some(handle)) if handle == "handle-a")
            && after == Ok(0),
        format!(
            "spill_ok={spill_ok:?} before_restore={before_restore:?} restore={restore:?} after={after:?}"
        ),
    )
}

#[cfg(feature = "fixtures")]
fn run_spill_slice() -> Vec<AssertionVerdict> {
    let fail_closed =
        InvariantAssertion::new(ContractFamily::SpillRouting, "spill_unavailable_fail_closed");

    // The round-trip check goes through the SAME injectable helper a broken-target
    // self-test drives, so weakening it would flip that self-test (it BITES).
    let online = RoutedSpillController::new(InMemorySpillTarget::available());
    let round_trip = spill_round_trip_verdict(&online);

    let offline = RoutedSpillController::new(InMemorySpillTarget::unavailable());
    let spill_denied = offline.spill("handle-b".to_string());
    let offline_spilled_after = offline.spilled_count();

    vec![
        round_trip,
        fail_closed.assert_that(
            matches!(&spill_denied, Err(failure) if matches!(failure.error(), MemoryError::Capacity(_)) && failure.handle() == "handle-b")
                && offline_spilled_after == Ok(0),
            format!("spill_denied={spill_denied:?} offline_spilled_after={offline_spilled_after:?}",),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::target::{AccountingView, AllocatorAdvisoryView};
    use crate::fake::{AccountingFidelity, FakeProcessBroker, FakeReservation};
    use proptest::prelude::*;
    use quanta_memory_platform_core::{
        ChargeKind, LiveProcessMemoryBroker, MemoryBudgetManifest, MemoryClass, MemoryClassBudget,
        MemoryContextKind, MemoryEvent, MemoryEventSink, MemoryPreset, ReclaimPriority,
        ResidencyPolicy, ResourceGroup, StoreBindingHandle,
    };
    use quanta_memory_platform_facade::MemoryPlatformBuilder;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    };

    fn all_held(verdicts: &[AssertionVerdict]) -> bool {
        verdicts.iter().all(AssertionVerdict::held)
    }

    /// Families that were VIOLATED in a verdict list (deduplicated, order kept).
    fn violated(verdicts: &[AssertionVerdict]) -> Vec<ContractFamily> {
        let mut out = Vec::new();
        for v in verdicts.iter().filter(|v| !v.held()) {
            if !out.contains(&v.family) {
                out.push(v.family);
            }
        }
        out
    }

    #[derive(Clone, Copy, Debug)]
    enum BrokerFault {
        BlackHoleAccounting,
        CommitSwallowing,
        ReleaseSwallowing,
        DuplicateCommitAccepting,
        WrongContextAxis,
        WrongChargeKindAxis,
        WrongClassKindAxis,
        WrongContextKindAxis,
        SwapKindAcrossClassBuckets,
        StaleTicketAccepting,
        ObservationError,
        TransferDoubleOwner,
    }

    struct BrokenBrokerTarget {
        inner: FakeProcessBroker,
        fault: BrokerFault,
    }

    impl BrokenBrokerTarget {
        fn new(fault: BrokerFault) -> Self {
            Self { inner: FakeProcessBroker::honest(), fault }
        }
    }

    impl BrokerConformanceTarget for BrokenBrokerTarget {
        type Ticket = FakeReservation;

        fn do_reserve(
            &mut self,
            class_axis: usize,
            owner: &str,
            kind: ChargeAxis,
            bytes: u64,
        ) -> Result<Self::Ticket, String> {
            BrokerConformanceTarget::do_reserve(&mut self.inner, class_axis, owner, kind, bytes)
        }

        fn do_commit(&mut self, ticket: &Self::Ticket) -> Result<(), String> {
            match self.fault {
                BrokerFault::CommitSwallowing => Ok(()),
                BrokerFault::DuplicateCommitAccepting => {
                    BrokerConformanceTarget::do_commit(&mut self.inner, ticket).or(Ok(()))
                }
                _ => BrokerConformanceTarget::do_commit(&mut self.inner, ticket),
            }
        }

        fn do_release(&mut self, ticket: &Self::Ticket) -> Result<(), String> {
            match self.fault {
                BrokerFault::ReleaseSwallowing => Ok(()),
                BrokerFault::StaleTicketAccepting => {
                    BrokerConformanceTarget::do_release(&mut self.inner, ticket).or(Ok(()))
                }
                _ => BrokerConformanceTarget::do_release(&mut self.inner, ticket),
            }
        }

        fn do_begin_rebuild(&mut self) -> Result<(), String> {
            BrokerConformanceTarget::do_begin_rebuild(&mut self.inner)
        }

        fn do_finish_rebuild(&mut self) -> Result<(), String> {
            BrokerConformanceTarget::do_finish_rebuild(&mut self.inner)
        }

        fn view(&self) -> Result<AccountingView, String> {
            if matches!(self.fault, BrokerFault::ObservationError) {
                return Err("injected snapshot failure".into());
            }
            if matches!(self.fault, BrokerFault::BlackHoleAccounting) {
                return Ok(AccountingView::default());
            }
            let mut view = BrokerConformanceTarget::view(&self.inner)?;
            match self.fault {
                BrokerFault::WrongContextAxis => {
                    view.context_axis_bytes = [0, view.total_context_bytes, 0, 0, 0];
                }
                BrokerFault::WrongChargeKindAxis => {
                    view.total_retained_metadata_bytes = view
                        .total_retained_metadata_bytes
                        .saturating_add(view.total_retained_payload_bytes);
                    view.total_retained_payload_bytes = 0;
                }
                // FUP-004: payload<->metadata swap confined to ONE class
                // bucket. Every scalar total still balances; only the per-axis
                // kind vectors diverge.
                BrokerFault::WrongClassKindAxis => {
                    view.class_kind_bytes[0].swap(0, 1);
                }
                BrokerFault::WrongContextKindAxis => {
                    view.context_kind_bytes[0].swap(0, 1);
                }
                // FUP-004: move payload across class buckets while preserving
                // every aggregate (axis totals AND per-kind global totals).
                // Aggregate checks are blind to this; only the bucket x kind
                // model comparison bites.
                BrokerFault::SwapKindAcrossClassBuckets => {
                    let moved = view.class_kind_bytes[0][0].min(1_000);
                    view.class_kind_bytes[0][0] -= moved;
                    view.class_kind_bytes[1][0] = view.class_kind_bytes[1][0].saturating_add(moved);
                    view.class_axis_bytes[0] -= moved;
                    view.class_axis_bytes[1] = view.class_axis_bytes[1].saturating_add(moved);
                }
                BrokerFault::TransferDoubleOwner => {
                    view.total_owner_bytes = view.total_owner_bytes.saturating_add(1_000);
                    view.owner_axis_bytes[1] = view.owner_axis_bytes[1].saturating_add(1_000);
                }
                _ => {}
            }
            Ok(view)
        }

        fn observe_allocator(
            &self,
            observation: &AllocatorObservation,
        ) -> Result<AllocatorAdvisoryView, String> {
            BrokerConformanceTarget::observe_allocator(&self.inner, observation)
        }
    }

    fn assert_broken_target_rejected(fault: BrokerFault, exact_families: &[ContractFamily]) {
        let mut target = BrokenBrokerTarget::new(fault);
        let verdicts = run_broker_accounting_suite(&mut target);
        assert!(!all_held(&verdicts), "{fault:?} must be rejected");
        assert_eq!(
            violated(&verdicts),
            exact_families,
            "{fault:?} must localize to its exact expected family set: {verdicts:?}"
        );
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ModelTicketState {
        Pending,
        Committed,
        Released,
    }

    struct ModelTicket<T> {
        target: T,
        class_axis: usize,
        owner_axis: usize,
        kind: ChargeAxis,
        bytes: u64,
        state: ModelTicketState,
    }

    #[derive(Default)]
    struct IndependentAccountingModel {
        class_axis_bytes: [u64; 2],
        owner_axis_bytes: [u64; 2],
        group_axis_bytes: [u64; 2],
        kind_bytes: [u64; 3],
        // FUP-004: the model tracks bucket x kind from the operation trace
        // itself — never by asking the target — so a cross-bucket kind swap
        // that preserves every aggregate still diverges from the view.
        class_axis_kind_bytes: [[u64; 3]; 2],
        owner_axis_kind_bytes: [[u64; 3]; 2],
        live_ticket_count: usize,
        rebuild_in_progress: bool,
        grouped: bool,
    }

    impl IndependentAccountingModel {
        fn grouped() -> Self {
            Self { grouped: true, ..Self::default() }
        }

        fn apply_reserve(
            &mut self,
            class_axis: usize,
            owner_axis: usize,
            kind: ChargeAxis,
            bytes: u64,
        ) {
            self.class_axis_bytes[class_axis] += bytes;
            self.owner_axis_bytes[owner_axis] += bytes;
            if self.grouped {
                self.group_axis_bytes[owner_axis] += bytes;
            }
            self.kind_bytes[kind_index(kind)] += bytes;
            self.class_axis_kind_bytes[class_axis][kind_index(kind)] += bytes;
            self.owner_axis_kind_bytes[owner_axis][kind_index(kind)] += bytes;
            self.live_ticket_count += 1;
        }

        fn apply_release<T>(&mut self, ticket: &ModelTicket<T>) {
            self.class_axis_bytes[ticket.class_axis] -= ticket.bytes;
            self.owner_axis_bytes[ticket.owner_axis] -= ticket.bytes;
            if self.grouped {
                self.group_axis_bytes[ticket.owner_axis] -= ticket.bytes;
            }
            self.kind_bytes[kind_index(ticket.kind)] -= ticket.bytes;
            self.class_axis_kind_bytes[ticket.class_axis][kind_index(ticket.kind)] -= ticket.bytes;
            self.owner_axis_kind_bytes[ticket.owner_axis][kind_index(ticket.kind)] -= ticket.bytes;
            self.live_ticket_count -= 1;
        }

        fn view(&self) -> AccountingView {
            let total = self.owner_axis_bytes.iter().sum();
            // Both targets charge every conformance reserve to the Request
            // context, so the model's whole kind vector sits in bucket 0.
            let context_kind_bytes = [self.kind_bytes, [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0]];
            AccountingView {
                total_class_bytes: self.class_axis_bytes.iter().sum(),
                total_owner_bytes: total,
                total_context_bytes: total,
                context_axis_bytes: [total, 0, 0, 0, 0],
                class_axis_bytes: self.class_axis_bytes,
                owner_axis_bytes: self.owner_axis_bytes,
                group_axis_bytes: self.group_axis_bytes,
                class_kind_bytes: self.class_axis_kind_bytes,
                owner_kind_bytes: self.owner_axis_kind_bytes,
                context_kind_bytes,
                live_ticket_count: self.live_ticket_count,
                total_retained_payload_bytes: self.kind_bytes[0],
                total_retained_metadata_bytes: self.kind_bytes[1],
                total_live_scratch_bytes: self.kind_bytes[2],
                rebuild_in_progress: self.rebuild_in_progress,
            }
        }
    }

    fn kind_index(kind: ChargeAxis) -> usize {
        match kind {
            ChargeAxis::RetainedPayload => 0,
            ChargeAxis::RetainedMetadata => 1,
            ChargeAxis::LiveScratch => 2,
        }
    }

    fn run_independent_trace<T: BrokerConformanceTarget>(target: &mut T) {
        let owners = ["owner:conformance-a", "owner:conformance-b"];
        let kinds =
            [ChargeAxis::RetainedPayload, ChargeAxis::RetainedMetadata, ChargeAxis::LiveScratch];
        let mut model = IndependentAccountingModel::default();
        let mut tickets: Vec<ModelTicket<T::Ticket>> = Vec::new();
        let mut seed = 0x9e37_79b9_u64;

        for step in 0..256u64 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let live =
                tickets.iter().filter(|ticket| ticket.state != ModelTicketState::Released).count();
            let action = if live < 8 { seed % 5 } else { 2 };
            match action {
                0 | 4 => {
                    let class_axis = ((seed >> 8) & 1) as usize;
                    let owner_axis = ((seed >> 9) & 1) as usize;
                    let kind = kinds[((seed >> 10) % 3) as usize];
                    let bytes = 1 + ((seed >> 16) % 31);
                    let ticket = target
                        .do_reserve(class_axis, owners[owner_axis], kind, bytes)
                        .unwrap_or_else(|error| panic!("step {step} reserve failed: {error}"));
                    model.apply_reserve(class_axis, owner_axis, kind, bytes);
                    tickets.push(ModelTicket {
                        target: ticket,
                        class_axis,
                        owner_axis,
                        kind,
                        bytes,
                        state: ModelTicketState::Pending,
                    });
                }
                1 => {
                    if let Some(ticket) =
                        tickets.iter_mut().find(|ticket| ticket.state == ModelTicketState::Pending)
                    {
                        target.do_commit(&ticket.target).expect("first commit");
                        ticket.state = ModelTicketState::Committed;
                        assert!(
                            target.do_commit(&ticket.target).is_err(),
                            "duplicate commit accepted"
                        );
                    }
                }
                2 | 3 => {
                    if let Some(index) =
                        tickets.iter().position(|ticket| ticket.state != ModelTicketState::Released)
                    {
                        target.do_release(&tickets[index].target).expect("terminal release");
                        model.apply_release(&tickets[index]);
                        tickets[index].state = ModelTicketState::Released;
                        assert!(
                            target.do_release(&tickets[index].target).is_err(),
                            "stale release accepted"
                        );
                    }
                }
                _ => unreachable!(),
            }
            assert_eq!(
                target.view(),
                Ok(model.view()),
                "independent model divergence after trace step {step}"
            );
        }

        for ticket in &mut tickets {
            if ticket.state != ModelTicketState::Released {
                target.do_release(&ticket.target).expect("final drain");
                model.apply_release(ticket);
                ticket.state = ModelTicketState::Released;
            }
        }
        assert_eq!(model.view(), AccountingView::default());
        assert_eq!(target.view(), Ok(AccountingView::default()));
    }

    #[test]
    fn deterministic_trace_matches_independent_model() {
        run_independent_trace(&mut FakeProcessBroker::honest());
        run_independent_trace(&mut live_broker());
    }

    #[derive(Clone, Debug)]
    enum ModelOperation {
        Reserve { class_axis: usize, owner_axis: usize, kind: ChargeAxis, bytes: u64 },
        Commit(usize),
        Release(usize),
        BeginRebuild,
        FinishRebuild,
        Observe(Option<u64>),
    }

    fn model_operation() -> impl Strategy<Value = ModelOperation> {
        let kind = prop_oneof![
            Just(ChargeAxis::RetainedPayload),
            Just(ChargeAxis::RetainedMetadata),
            Just(ChargeAxis::LiveScratch),
        ];
        prop_oneof![
            5 => (0usize..2, 0usize..2, kind, 0u64..=63).prop_map(
                |(class_axis, owner_axis, kind, bytes)| ModelOperation::Reserve {
                    class_axis,
                    owner_axis,
                    kind,
                    bytes,
                }
            ),
            1 => Just(ModelOperation::Reserve {
                class_axis: 0,
                owner_axis: 0,
                kind: ChargeAxis::RetainedPayload,
                bytes: u64::MAX,
            }),
            2 => (0usize..96).prop_map(ModelOperation::Commit),
            3 => (0usize..96).prop_map(ModelOperation::Release),
            1 => Just(ModelOperation::BeginRebuild),
            1 => Just(ModelOperation::FinishRebuild),
            2 => prop::option::of(0u64..=8_192).prop_map(ModelOperation::Observe),
        ]
    }

    fn run_generated_trace<T: BrokerConformanceTarget>(
        target: &mut T,
        operations: &[ModelOperation],
    ) {
        let owners = ["owner:conformance-a", "owner:conformance-b"];
        let mut model = IndependentAccountingModel::grouped();
        let mut tickets: Vec<ModelTicket<T::Ticket>> = Vec::new();

        for (step, operation) in operations.iter().enumerate() {
            match *operation {
                ModelOperation::Reserve { class_axis, owner_axis, kind, bytes } => {
                    let before = target.view().expect("pre-reserve observation");
                    let result = target.do_reserve(class_axis, owners[owner_axis], kind, bytes);
                    if model.rebuild_in_progress || bytes == u64::MAX {
                        assert!(result.is_err(), "step {step}: denied reserve unexpectedly passed");
                        assert_eq!(target.view(), Ok(before), "step {step}: denial mutated state");
                    } else {
                        let ticket = result.unwrap_or_else(|error| {
                            panic!("step {step}: admissible reserve failed: {error}")
                        });
                        model.apply_reserve(class_axis, owner_axis, kind, bytes);
                        tickets.push(ModelTicket {
                            target: ticket,
                            class_axis,
                            owner_axis,
                            kind,
                            bytes,
                            state: ModelTicketState::Pending,
                        });
                    }
                }
                ModelOperation::Commit(slot)
                    if !model.rebuild_in_progress && !tickets.is_empty() =>
                {
                    let index = slot % tickets.len();
                    let ticket = &mut tickets[index];
                    let result = target.do_commit(&ticket.target);
                    if ticket.state == ModelTicketState::Pending {
                        result.unwrap_or_else(|error| {
                            panic!("step {step}: first commit failed: {error}")
                        });
                        ticket.state = ModelTicketState::Committed;
                    } else {
                        assert!(result.is_err(), "step {step}: non-pending commit passed");
                    }
                }
                ModelOperation::Release(slot)
                    if !model.rebuild_in_progress && !tickets.is_empty() =>
                {
                    let index = slot % tickets.len();
                    let result = target.do_release(&tickets[index].target);
                    if tickets[index].state == ModelTicketState::Released {
                        assert!(result.is_err(), "step {step}: stale release passed");
                    } else {
                        result.unwrap_or_else(|error| {
                            panic!("step {step}: terminal release failed: {error}")
                        });
                        model.apply_release(&tickets[index]);
                        tickets[index].state = ModelTicketState::Released;
                    }
                }
                ModelOperation::BeginRebuild if !model.rebuild_in_progress => {
                    target
                        .do_begin_rebuild()
                        .unwrap_or_else(|error| panic!("step {step}: begin failed: {error}"));
                    model.rebuild_in_progress = true;
                }
                ModelOperation::FinishRebuild if model.rebuild_in_progress => {
                    target
                        .do_finish_rebuild()
                        .unwrap_or_else(|error| panic!("step {step}: finish failed: {error}"));
                    model.rebuild_in_progress = false;
                }
                ModelOperation::Observe(resident_bytes) => {
                    let advisory = target
                        .observe_allocator(&AllocatorObservation { resident_bytes })
                        .unwrap_or_else(|error| panic!("step {step}: observation failed: {error}"));
                    assert_eq!(advisory.semantic_bytes, model.view().total_owner_bytes);
                    assert_eq!(advisory.allocator_resident_bytes, resident_bytes);
                }
                _ => {}
            }
            assert_eq!(
                target.view(),
                Ok(model.view()),
                "independent model divergence after generated step {step}: {operation:?}"
            );
        }

        if model.rebuild_in_progress {
            target.do_finish_rebuild().expect("final rebuild close");
            model.rebuild_in_progress = false;
        }
        for ticket in &mut tickets {
            if ticket.state != ModelTicketState::Released {
                target.do_release(&ticket.target).expect("generated trace final drain");
                model.apply_release(ticket);
                ticket.state = ModelTicketState::Released;
            }
        }
        assert_eq!(model.view(), AccountingView::default());
        assert_eq!(target.view(), Ok(AccountingView::default()));
    }

    fn grouped_fake() -> FakeProcessBroker {
        FakeProcessBroker::honest()
            .with_resource_group(
                "conformance-group-a",
                6_000,
                vec!["owner:conformance-a".to_string()],
            )
            .with_resource_group(
                "conformance-group-b",
                6_000,
                vec!["owner:conformance-b".to_string()],
            )
    }

    fn grouped_live_broker() -> LiveProcessMemoryBroker {
        let group = ResourceGroup {
            reserved_bytes: 0,
            soft_cap_bytes: 4_000,
            hard_cap_bytes: 6_000,
            burst_percent: 0,
            reclaim_priority: ReclaimPriority::Normal,
            admission_weight: 1,
        };
        let residency = ResidencyPolicy::new(10_000_000)
            .with_class_budget(
                MemoryClass::ArtifactCache,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: 4_000,
                    hard_cap_bytes: 6_000,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            )
            .with_class_budget(
                MemoryClass::EvidenceCache,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: 4_000,
                    hard_cap_bytes: 6_000,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            )
            .with_resource_group("conformance-group-a", group.clone())
            .with_resource_group("conformance-group-b", group)
            .with_owner_group("owner:conformance-a", "conformance-group-a")
            .with_owner_group("owner:conformance-b", "conformance-group-b");
        quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
            .with_manifest(MemoryBudgetManifest::new(
                MemoryPreset::Ci,
                quanta_memory_platform_core::ExecutionBudget::default(),
                residency,
            ))
            .build()
            .expect("valid grouped residency")
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            max_shrink_iters: 8_192,
            ..ProptestConfig::default()
        })]

        #[test]
        fn property_trace_matches_independent_model(
            operations in prop::collection::vec(model_operation(), 1..96)
        ) {
            run_generated_trace(&mut grouped_fake(), &operations);
            run_generated_trace(&mut grouped_live_broker(), &operations);
        }
    }

    #[derive(Default)]
    struct ReentrantSink {
        broker: Mutex<Option<LiveProcessMemoryBroker>>,
        entered: AtomicBool,
        reentries: AtomicUsize,
    }

    impl MemoryEventSink for ReentrantSink {
        fn emit(&self, _event: MemoryEvent) {
            if self.entered.swap(true, Ordering::AcqRel) {
                return;
            }
            if let Some(broker) = self.broker.lock().expect("sink broker lock").clone() {
                broker.observe().expect("reentrant observation must not deadlock");
                self.reentries.fetch_add(1, Ordering::Release);
            }
        }
    }

    #[test]
    fn reentrant_sink_is_contained_outside_owner_lock() {
        let sink = Arc::new(ReentrantSink::default());
        let broker = quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
            .with_preset(MemoryPreset::Development)
            .with_event_sink(sink.clone())
            .build()
            .expect("broker");
        *sink.broker.lock().expect("install broker") = Some(broker.clone());

        let charge = broker
            .reserve_managed_charge(
                MemoryClass::ArtifactCache,
                "reentrant-sink".into(),
                MemoryContextKind::Request,
                ChargeKind::RetainedPayload,
                8,
            )
            .expect("reserve must survive reentrant advisory sink");
        assert_eq!(sink.reentries.load(Ordering::Acquire), 1);
        assert_eq!(broker.observe().expect("observation").semantic_bytes, 8);
        drop(charge);
        assert_eq!(broker.observe().expect("final observation").semantic_bytes, 0);
    }

    #[test]
    fn suite_rejects_black_hole_accounting() {
        assert_broken_target_rejected(
            BrokerFault::BlackHoleAccounting,
            &[ContractFamily::OwnerPartition, ContractFamily::AllocatorAdvisory],
        );
    }

    #[test]
    fn suite_rejects_commit_swallowing_target() {
        assert_broken_target_rejected(
            BrokerFault::CommitSwallowing,
            &[ContractFamily::TicketLifecycle],
        );
    }

    #[test]
    fn suite_rejects_release_swallowing_target() {
        assert_broken_target_rejected(
            BrokerFault::ReleaseSwallowing,
            &[ContractFamily::TicketLifecycle],
        );
    }

    #[test]
    fn suite_rejects_duplicate_commit_accepting_target() {
        assert_broken_target_rejected(
            BrokerFault::DuplicateCommitAccepting,
            &[ContractFamily::TicketLifecycle],
        );
    }

    #[test]
    fn suite_rejects_wrong_context_axis() {
        assert_broken_target_rejected(
            BrokerFault::WrongContextAxis,
            &[ContractFamily::OwnerPartition],
        );
    }

    #[test]
    fn suite_rejects_wrong_charge_kind_axis() {
        assert_broken_target_rejected(
            BrokerFault::WrongChargeKindAxis,
            &[ContractFamily::OwnerPartition],
        );
    }

    // --- FUP-004: per-axis kind validation ---------------------------------
    //
    // Aggregate totals can balance while one axis hides a kind swap. The suite
    // must reject axis-confined swaps (per-axis kind vectors) and
    // aggregate-preserving cross-bucket swaps (model bucket x kind comparison).

    #[test]
    fn sep23_fup004_suite_rejects_class_confined_kind_swap() {
        assert_broken_target_rejected(
            BrokerFault::WrongClassKindAxis,
            &[ContractFamily::OwnerPartition],
        );
        // The rejection must include the per-axis kind verdict itself — not
        // just the exact-model mismatch — so a checker that drops the class
        // axis goes red here.
        let report = run_broker_accounting_suite(&mut BrokenBrokerTarget::new(
            BrokerFault::WrongClassKindAxis,
        ));
        assert!(
            report.iter().any(|verdict| {
                verdict.name == "observed_kind_split_agrees_across_axes" && !verdict.held
            }),
            "class swap must violate the kind-axis verdict: {report:?}"
        );
    }

    #[test]
    fn sep23_fup004_suite_rejects_context_confined_kind_swap() {
        assert_broken_target_rejected(
            BrokerFault::WrongContextKindAxis,
            &[ContractFamily::OwnerPartition],
        );
        let report = run_broker_accounting_suite(&mut BrokenBrokerTarget::new(
            BrokerFault::WrongContextKindAxis,
        ));
        assert!(
            report.iter().any(|verdict| {
                verdict.name == "observed_kind_split_agrees_across_axes" && !verdict.held
            }),
            "context swap must violate the kind-axis verdict: {report:?}"
        );
    }

    #[test]
    fn sep23_fup004_suite_rejects_aggregate_preserving_bucket_swap() {
        // The swap preserves every aggregate the old checks could see, so the
        // rejection must come from the bucket x kind model comparison — prove
        // the distinction by checking the aggregates still agree.
        let mut broken = BrokenBrokerTarget::new(BrokerFault::SwapKindAcrossClassBuckets);
        let _t1 = broken
            .do_reserve(0, "owner:conformance-a", ChargeAxis::RetainedPayload, 1_000)
            .expect("t1");
        let _t2 = broken
            .do_reserve(1, "owner:conformance-a", ChargeAxis::RetainedMetadata, 250)
            .expect("t2");
        let _t3 =
            broken.do_reserve(0, "owner:conformance-b", ChargeAxis::LiveScratch, 500).expect("t3");
        let view = broken.view().expect("broken view");
        assert_eq!(view.total_class_bytes, 1_750, "scalar totals preserved");
        assert_eq!(view.total_owner_bytes, 1_750);
        assert_eq!(
            view.checked_class_kind_totals(),
            view.checked_owner_kind_totals(),
            "aggregate kind vectors still agree"
        );
        assert_eq!(view.checked_owner_kind_totals(), view.checked_context_kind_totals());
        assert_eq!(view.class_axis_bytes, [500, 1_250], "buckets moved");
        assert_eq!(view.class_kind_bytes, [[0, 0, 500], [1_000, 250, 0]]);
        assert_broken_target_rejected(
            BrokerFault::SwapKindAcrossClassBuckets,
            &[ContractFamily::OwnerPartition],
        );
    }

    #[test]
    fn sep23_fup004_mixed_kind_trace_agrees_on_every_axis_bucket() {
        // Fixed mixed-kind script asserting the FULL kind matrices on both
        // honest targets — not just the scalar projections — across a
        // reserve phase AND a release phase.
        let script = [
            (0usize, 0usize, ChargeAxis::RetainedPayload, 1_000u64),
            (1, 0, ChargeAxis::RetainedMetadata, 250),
            (0, 1, ChargeAxis::LiveScratch, 500),
            (1, 1, ChargeAxis::RetainedPayload, 100),
        ];
        let owners = ["owner:conformance-a", "owner:conformance-b"];
        let model_ticket = |(class_axis, owner_axis, kind, bytes)| ModelTicket {
            target: (),
            class_axis,
            owner_axis,
            kind,
            bytes,
            state: ModelTicketState::Committed,
        };
        let mut model = IndependentAccountingModel::default();
        let mut fake = FakeProcessBroker::honest();
        let mut fake_tickets = Vec::new();
        for (class, owner, kind, bytes) in script {
            let ticket = fake.do_reserve(class, owners[owner], kind, bytes).expect("reserve");
            fake.do_commit(&ticket).expect("commit");
            model.apply_reserve(class, owner, kind, bytes);
            // Hold the ticket guards: dropping them releases the charge.
            fake_tickets.push(ticket);
        }
        assert_eq!(fake.view().expect("fake view"), model.view(), "fake full view");
        // Release phase: drop the payload leg and the scratch leg; the kind
        // matrices must track the release exactly, per bucket.
        for index in [0, 2] {
            fake.do_release(&fake_tickets[index]).expect("fake release");
            model.apply_release(&model_ticket(script[index]));
        }
        assert_eq!(fake.view().expect("fake view"), model.view(), "fake view after release");

        let mut model = IndependentAccountingModel::default();
        let mut live = live_broker();
        let mut live_tickets = Vec::new();
        for (class, owner, kind, bytes) in script {
            let ticket = live.do_reserve(class, owners[owner], kind, bytes).expect("reserve");
            live.do_commit(&ticket).expect("commit");
            model.apply_reserve(class, owner, kind, bytes);
            live_tickets.push(ticket);
        }
        assert_eq!(live.view().expect("live view"), model.view(), "live full view");
        for index in [0, 2] {
            live.do_release(&live_tickets[index]).expect("live release");
            model.apply_release(&model_ticket(script[index]));
        }
        assert_eq!(live.view().expect("live view"), model.view(), "live view after release");

        // Only (1,0,metadata,250) and (1,1,payload,100) remain.
        let view = model.view();
        assert_eq!(view.class_kind_bytes, [[0, 0, 0], [100, 250, 0]]);
        assert_eq!(view.owner_kind_bytes, [[0, 250, 0], [100, 0, 0]]);
        assert_eq!(view.context_kind_bytes[0], [100, 250, 0], "every reserve lands in Request");
        assert_eq!(view.total_owner_bytes, 350);
    }

    #[test]
    fn suite_rejects_stale_ticket_accepting_target() {
        assert_broken_target_rejected(
            BrokerFault::StaleTicketAccepting,
            &[ContractFamily::TicketLifecycle],
        );
    }

    #[test]
    fn snapshot_error_is_failed_verdict_not_zero() {
        assert_broken_target_rejected(
            BrokerFault::ObservationError,
            &[
                ContractFamily::OwnerPartition,
                ContractFamily::ClassOwnerBalance,
                ContractFamily::AllocatorAdvisory,
                ContractFamily::TicketLifecycle,
            ],
        );
    }

    #[test]
    fn suite_rejects_transfer_double_owner() {
        assert_broken_target_rejected(
            BrokerFault::TransferDoubleOwner,
            &[
                ContractFamily::OwnerPartition,
                ContractFamily::ClassOwnerBalance,
                ContractFamily::AllocatorAdvisory,
                ContractFamily::TicketLifecycle,
            ],
        );
    }

    #[test]
    fn prepared_publication_suite_accepts_honest_service() {
        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("platform");
        let service = platform
            .prepared_publication_builder::<Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ManifestResidency,
                "prepared-conformance".to_string(),
                None,
            ))
            .build()
            .expect("bound prepared builder");
        let verdicts = run_prepared_publication_suite(&service);
        assert!(all_held(&verdicts), "honest prepared service failed: {verdicts:?}");
    }

    struct UnderbillingPreparedTarget<'a, T>(&'a T);

    impl<T: PreparedPublicationConformanceTarget> PreparedPublicationConformanceTarget
        for UnderbillingPreparedTarget<'_, T>
    {
        type Token = T::Token;

        fn prepare(
            &self,
            identity: &[u8],
            payload: &[u8],
        ) -> Result<(Self::Token, PrepareDispositionV1), PreparedPublicationErrorV1> {
            self.0.prepare(identity, payload)
        }

        fn resolve(&self, token: &Self::Token) -> Result<Vec<u8>, PreparedPublicationErrorV1> {
            self.0.resolve(token)
        }

        fn abort(
            &self,
            token: &Self::Token,
        ) -> Result<AbortDispositionV1, PreparedPublicationErrorV1> {
            self.0.abort(token)
        }

        fn orphan_tokens(
            &self,
            referenced_tokens: &[Self::Token],
        ) -> Result<Vec<Self::Token>, PreparedPublicationErrorV1> {
            self.0.orphan_tokens(referenced_tokens)
        }

        fn gc_orphans(
            &self,
            referenced_tokens: &[Self::Token],
        ) -> Result<usize, PreparedPublicationErrorV1> {
            self.0.gc_orphans(referenced_tokens)
        }

        fn stats(&self) -> Result<PreparedPublicationStatsV1, PreparedPublicationErrorV1> {
            self.0.stats().map(|mut stats| {
                stats.charged_bytes = 0;
                stats.tombstone_bytes = 0;
                stats
            })
        }
    }

    #[test]
    fn prepared_suite_rejects_correct_replay_with_unbilled_identity() {
        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("platform");
        let service = platform
            .prepared_publication_builder::<Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::ManifestResidency,
                "underbilling-prepared".to_string(),
                None,
            ))
            .build()
            .expect("service");
        let verdicts = run_prepared_publication_suite(&UnderbillingPreparedTarget(&service));
        assert!(!all_held(&verdicts), "zero-byte accounting fake must fail: {verdicts:?}");
        assert!(verdicts.iter().any(|verdict| {
            verdict.name == "prepared_publication_replay_is_inert" && !verdict.held()
        }));
        assert!(verdicts.iter().any(|verdict| {
            verdict.name == "prepared_publication_payload_drift_conflicts" && verdict.held()
        }));
    }

    #[derive(Debug, Default)]
    struct BrokenPreparedTarget {
        prepares: std::sync::Mutex<u64>,
    }

    impl PreparedPublicationConformanceTarget for BrokenPreparedTarget {
        type Token = u64;

        fn prepare(
            &self,
            _identity: &[u8],
            _payload: &[u8],
        ) -> Result<(Self::Token, PrepareDispositionV1), PreparedPublicationErrorV1> {
            let mut prepares = self.prepares.lock().map_err(|_| {
                PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                    "poisoned".to_string(),
                ))
            })?;
            *prepares = prepares.saturating_add(1);
            Ok((*prepares, PrepareDispositionV1::Applied))
        }

        fn resolve(&self, _token: &Self::Token) -> Result<Vec<u8>, PreparedPublicationErrorV1> {
            Ok(b"payload".to_vec())
        }

        fn abort(
            &self,
            _token: &Self::Token,
        ) -> Result<AbortDispositionV1, PreparedPublicationErrorV1> {
            Ok(AbortDispositionV1::Aborted)
        }

        fn orphan_tokens(
            &self,
            _referenced_tokens: &[Self::Token],
        ) -> Result<Vec<Self::Token>, PreparedPublicationErrorV1> {
            Ok(Vec::new())
        }

        fn gc_orphans(
            &self,
            _referenced_tokens: &[Self::Token],
        ) -> Result<usize, PreparedPublicationErrorV1> {
            Ok(0)
        }

        fn stats(&self) -> Result<PreparedPublicationStatsV1, PreparedPublicationErrorV1> {
            let prepares = *self.prepares.lock().map_err(|_| {
                PreparedPublicationErrorV1::Storage(MemoryError::InvalidState(
                    "poisoned".to_string(),
                ))
            })?;
            Ok(PreparedPublicationStatsV1 {
                prepared_count: prepares as usize,
                charged_bytes: prepares.saturating_mul(7),
                ..PreparedPublicationStatsV1::default()
            })
        }
    }

    #[test]
    fn prepared_publication_suite_rejects_duplicate_charging_fake() {
        let fake = BrokenPreparedTarget::default();
        let verdicts = run_prepared_publication_suite(&fake);
        assert!(!all_held(&verdicts), "duplicate-charging fake must be rejected");
        assert!(
            violated(&verdicts).contains(&ContractFamily::ReserveFailClosed),
            "duplicate mutation must localize to fail-closed: {verdicts:?}"
        );
    }

    #[derive(Debug)]
    struct RootBlindPreparedTarget {
        service: PreparedPublicationService<Vec<u8>, Budgeted>,
    }

    impl RootBlindPreparedTarget {
        fn new() -> Self {
            let platform = MemoryPlatformBuilder::new()
                .with_preset(MemoryPreset::Development)
                .build()
                .expect("platform");
            Self {
                service: platform
                    .prepared_publication_builder::<Vec<u8>>()
                    .bind(StoreBindingHandle::new(
                        MemoryClass::ManifestResidency,
                        "root-blind-prepared-conformance".to_string(),
                        None,
                    ))
                    .build()
                    .expect("bound prepared builder"),
            }
        }
    }

    impl PreparedPublicationConformanceTarget for RootBlindPreparedTarget {
        type Token = PreparedPublicationTokenV1;

        fn prepare(
            &self,
            identity: &[u8],
            payload: &[u8],
        ) -> Result<(Self::Token, PrepareDispositionV1), PreparedPublicationErrorV1> {
            PreparedPublicationConformanceTarget::prepare(&self.service, identity, payload)
        }

        fn resolve(&self, token: &Self::Token) -> Result<Vec<u8>, PreparedPublicationErrorV1> {
            self.service.resolve(token)
        }

        fn abort(
            &self,
            token: &Self::Token,
        ) -> Result<AbortDispositionV1, PreparedPublicationErrorV1> {
            self.service.abort(token)
        }

        fn orphan_tokens(
            &self,
            _referenced_tokens: &[Self::Token],
        ) -> Result<Vec<Self::Token>, PreparedPublicationErrorV1> {
            self.service.orphan_tokens(&[])
        }

        fn gc_orphans(
            &self,
            _referenced_tokens: &[Self::Token],
        ) -> Result<usize, PreparedPublicationErrorV1> {
            self.service.gc_orphans(&[])
        }

        fn stats(&self) -> Result<PreparedPublicationStatsV1, PreparedPublicationErrorV1> {
            self.service.stats()
        }
    }

    #[test]
    fn prepared_publication_suite_rejects_root_blind_gc_fake() {
        let fake = RootBlindPreparedTarget::new();
        let verdicts = run_prepared_publication_suite(&fake);
        assert!(!all_held(&verdicts), "root-blind GC fake must be rejected");
        assert!(
            violated(&verdicts).contains(&ContractFamily::ReclaimAccounting),
            "root-blind GC must localize to reclaim accounting: {verdicts:?}"
        );
    }

    #[test]
    fn transcript_append_suite_accepts_honest_service() {
        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("platform");
        let store = platform
            .transcript_store_builder::<Vec<u8>, Vec<u8>, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::SessionTranscript,
                "transcript-conformance".to_string(),
                None,
            ))
            .with_retention_policy(TranscriptRetentionPolicyV2::fail_closed(2, u64::MAX))
            .build()
            .expect("bound transcript builder");
        let verdicts = run_transcript_append_suite(&store);
        assert!(all_held(&verdicts), "honest transcript store failed: {verdicts:?}");
    }

    #[derive(Debug, Default)]
    struct BrokenTranscriptTarget {
        rows: std::sync::Mutex<Vec<TranscriptRow>>,
    }

    impl TranscriptAppendConformanceTarget for BrokenTranscriptTarget {
        fn append(
            &self,
            _session_id: &[u8],
            sequence: u64,
            message_id: &[u8],
            body: &[u8],
        ) -> Result<TranscriptAppendDispositionV1, TranscriptAppendErrorV1> {
            self.rows
                .lock()
                .map_err(|_| {
                    TranscriptAppendErrorV1::Storage(MemoryError::InvalidState(
                        "poisoned".to_string(),
                    ))
                })?
                .push((sequence, message_id.to_vec(), body.to_vec()));
            Ok(TranscriptAppendDispositionV1::Applied)
        }

        fn load(&self, _session_id: &[u8]) -> Result<Vec<TranscriptRow>, MemoryError> {
            self.rows
                .lock()
                .map(|rows| rows.clone())
                .map_err(|_| MemoryError::InvalidState("poisoned".to_string()))
        }

        fn truncate_to(&self, _session_id: &[u8], max_messages: usize) -> Result<(), MemoryError> {
            self.rows
                .lock()
                .map(|mut rows| rows.truncate(max_messages))
                .map_err(|_| MemoryError::InvalidState("poisoned".to_string()))
        }

        fn retained_bytes(&self) -> Result<u64, MemoryError> {
            self.rows
                .lock()
                .map(|rows| rows.iter().map(|(_, _, body)| body.len() as u64).sum())
                .map_err(|_| MemoryError::InvalidState("poisoned".to_string()))
        }
    }

    #[test]
    fn transcript_append_suite_rejects_non_idempotent_fake() {
        let fake = BrokenTranscriptTarget::default();
        let verdicts = run_transcript_append_suite(&fake);
        assert!(!all_held(&verdicts), "append-every-time fake must be rejected");
        assert!(
            violated(&verdicts).contains(&ContractFamily::ReserveFailClosed),
            "duplicate mutation must localize to fail-closed: {verdicts:?}"
        );
    }

    #[test]
    fn transcript_suite_rejects_target_that_ignores_declared_horizon() {
        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("platform");
        let wrong_horizon = platform
            .transcript_store_builder::<Vec<u8>, Vec<u8>, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::SessionTranscript,
                "wrong-horizon-transcript".to_string(),
                None,
            ))
            .with_retention_policy(TranscriptRetentionPolicyV2::fail_closed(3, u64::MAX))
            .build()
            .expect("target with wrong horizon");
        let verdicts = run_transcript_append_suite(&wrong_horizon);
        assert!(verdicts.iter().any(|verdict| {
            verdict.name == "transcript_new_identity_fails_closed_at_retention_bound"
                && !verdict.held()
        }));
        assert!(verdicts
            .iter()
            .filter(|verdict| {
                verdict.name != "transcript_new_identity_fails_closed_at_retention_bound"
            })
            .all(AssertionVerdict::held));
    }

    struct UnderbillingTranscriptTarget<'a, T>(&'a T);

    impl<T: TranscriptAppendConformanceTarget> TranscriptAppendConformanceTarget
        for UnderbillingTranscriptTarget<'_, T>
    {
        fn append(
            &self,
            session_id: &[u8],
            sequence: u64,
            message_id: &[u8],
            body: &[u8],
        ) -> Result<TranscriptAppendDispositionV1, TranscriptAppendErrorV1> {
            self.0.append(session_id, sequence, message_id, body)
        }

        fn load(&self, session_id: &[u8]) -> Result<Vec<TranscriptRow>, MemoryError> {
            self.0.load(session_id)
        }

        fn truncate_to(&self, session_id: &[u8], max_messages: usize) -> Result<(), MemoryError> {
            self.0.truncate_to(session_id, max_messages)
        }

        fn retained_bytes(&self) -> Result<u64, MemoryError> {
            Ok(0)
        }
    }

    #[test]
    fn transcript_suite_rejects_correct_replay_with_unbilled_identity() {
        let platform = MemoryPlatformBuilder::new()
            .with_preset(MemoryPreset::Development)
            .build()
            .expect("platform");
        let store = platform
            .transcript_store_builder::<Vec<u8>, Vec<u8>, Vec<u8>>()
            .bind(StoreBindingHandle::new(
                MemoryClass::SessionTranscript,
                "underbilling-transcript".to_string(),
                None,
            ))
            .with_retention_policy(TranscriptRetentionPolicyV2::fail_closed(2, u64::MAX))
            .build()
            .expect("store");
        let verdicts = run_transcript_append_suite(&UnderbillingTranscriptTarget(&store));
        assert!(!all_held(&verdicts), "zero-byte accounting fake must fail: {verdicts:?}");
        assert!(verdicts.iter().any(|verdict| {
            verdict.name == "transcript_exact_replay_is_inert" && !verdict.held()
        }));
        assert!(verdicts.iter().any(|verdict| {
            verdict.name == "transcript_collision_axes_are_typed" && verdict.held()
        }));
    }

    #[test]
    fn honest_fake_passes_every_accounting_invariant() {
        let mut fake = FakeProcessBroker::honest();
        let verdicts = run_broker_accounting_suite(&mut fake);
        assert!(!verdicts.is_empty(), "suite must run at least one check");
        assert!(
            all_held(&verdicts),
            "honest fake must satisfy every invariant: {:?}",
            verdicts.iter().filter(|v| !v.held()).map(|v| v.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn suite_rejects_fake_that_under_reports_owner_bytes() {
        // META-REQUIREMENT: a broker that under-reports owner usage must be
        // CAUGHT. The class total stays honest, so class != owner.
        let mut broken = FakeProcessBroker::broken(AccountingFidelity::UnderReportOwnerBytes(100));
        let verdicts = run_broker_accounting_suite(&mut broken);
        assert!(!all_held(&verdicts), "suite must not pass an under-reporting fake");
        // The failure must localize to the class/owner balance family.
        let violated_families: Vec<ContractFamily> =
            verdicts.iter().filter(|v| !v.held()).map(|v| v.family).collect();
        assert!(
            violated_families.contains(&ContractFamily::ClassOwnerBalance),
            "under-report must surface as a ClassOwnerBalance violation, got {violated_families:?}"
        );
    }

    #[test]
    fn suite_rejects_fake_whose_class_diverges_from_owner() {
        let mut broken = FakeProcessBroker::broken(AccountingFidelity::DivergeClassFromOwner(64));
        let verdicts = run_broker_accounting_suite(&mut broken);
        assert!(!all_held(&verdicts), "class-divergent fake must be rejected");
        let violated: Vec<ContractFamily> =
            verdicts.iter().filter(|v| !v.held()).map(|v| v.family).collect();
        assert!(violated.contains(&ContractFamily::ClassOwnerBalance), "got {violated:?}");
    }

    // -----------------------------------------------------------------------
    // META-REQUIREMENT: every AccountingFidelity break variant is rejected.
    // -----------------------------------------------------------------------

    /// Every non-honest [`AccountingFidelity`] the fake can model. Adding a new
    /// break variant to the enum and forgetting to list it here will leave a hole
    /// in the meta-coverage; this is the single place that enumerates them.
    fn all_break_variants() -> Vec<AccountingFidelity> {
        // Compile-time tie to the enum: this exhaustive match (no wildcard) fails
        // to compile when a new AccountingFidelity variant is added, forcing the
        // author to also list it below. Without this, a new break variant could
        // silently escape `suite_rejects_every_accounting_fidelity_break_variant`.
        fn _exhaustiveness_guard(v: AccountingFidelity) {
            match v {
                AccountingFidelity::Honest
                | AccountingFidelity::UnderReportOwnerBytes(_)
                | AccountingFidelity::DivergeClassFromOwner(_)
                | AccountingFidelity::DivergeOwnerPartition(_) => {}
            }
        }
        vec![
            AccountingFidelity::UnderReportOwnerBytes(100),
            AccountingFidelity::DivergeClassFromOwner(64),
            AccountingFidelity::DivergeOwnerPartition(48),
        ]
    }

    // Guards that the hand-maintained `all_break_variants()` list stays complete:
    // every entry is a distinct break variant (not Honest) and maps to a family.
    #[test]
    fn all_break_variants_are_distinct_non_honest_and_family_mapped() {
        let variants = all_break_variants();
        assert_eq!(variants.len(), 3, "update this count when a break variant is added/removed");
        for v in &variants {
            assert!(!matches!(v, AccountingFidelity::Honest), "Honest is not a break variant");
            // expected_family panics on Honest; calling it proves a mapping exists.
            let _ = expected_family(*v);
        }
    }

    /// The contract family each break variant must localize to. A variant that
    /// failed to localize here (or localized to the wrong family) would mean the
    /// suite caught "something" but not the SPECIFIC contract the lie violates.
    fn expected_family(variant: AccountingFidelity) -> ContractFamily {
        match variant {
            // Under-reporting owner bytes / inflating class both desync class from
            // owner -> invariant 1.
            AccountingFidelity::UnderReportOwnerBytes(_)
            | AccountingFidelity::DivergeClassFromOwner(_) => ContractFamily::ClassOwnerBalance,
            // Inflating the totals while leaving the partition honest keeps class
            // == owner but breaks owner == partition_sum -> invariant 2.
            AccountingFidelity::DivergeOwnerPartition(_) => ContractFamily::OwnerPartition,
            AccountingFidelity::Honest => unreachable!("Honest is not a break variant"),
        }
    }

    #[test]
    fn suite_rejects_every_accounting_fidelity_break_variant() {
        // scenario G-meta: the suite must BITE on EACH way the fake can lie about
        // its accounting AND localize the failure to the SPECIFIC contract family
        // that lie violates. A pass on any break variant — or a catch that
        // mislocalizes — means the suite is a rubber stamp for that contract.
        for variant in all_break_variants() {
            let mut broken = FakeProcessBroker::broken(variant);
            let verdicts = run_broker_accounting_suite(&mut broken);
            assert!(
                !all_held(&verdicts),
                "break variant {variant:?} must be REJECTED by the suite, but every check held"
            );
            let fams = violated(&verdicts);
            let expected = expected_family(variant);
            assert!(
                fams.contains(&expected),
                "break variant {variant:?} must localize to {expected:?}, got {fams:?}"
            );
        }
    }

    #[test]
    fn owner_partition_break_localizes_to_owner_partition_not_balance() {
        // The invariant-2 break must NOT masquerade as an invariant-1 (balance)
        // failure: class == owner still holds, so ONLY OwnerPartition violates.
        let mut broken = FakeProcessBroker::broken(AccountingFidelity::DivergeOwnerPartition(48));
        let verdicts = run_broker_accounting_suite(&mut broken);
        let fams = violated(&verdicts);
        assert!(
            fams.contains(&ContractFamily::OwnerPartition),
            "owner-partition divergence must violate OwnerPartition, got {fams:?}"
        );
        assert!(
            !fams.contains(&ContractFamily::ClassOwnerBalance),
            "class still equals owner, so ClassOwnerBalance must NOT be flagged, got {fams:?}"
        );
    }

    #[test]
    fn honest_fidelity_is_the_only_variant_the_suite_accepts() {
        // scenario G-meta (positive pole): the SAME suite that rejects every break
        // variant must ACCEPT the honest one, or it rejects everything (also
        // worthless). Pairs the negative enumeration above.
        let mut honest = FakeProcessBroker::broken(AccountingFidelity::Honest);
        let verdicts = run_broker_accounting_suite(&mut honest);
        assert!(all_held(&verdicts), "honest fidelity must pass: {:?}", violated(&verdicts));
    }

    // -----------------------------------------------------------------------
    // ReserveFailClosed: a target that MUTATES on a denied write is rejected.
    // -----------------------------------------------------------------------

    /// A conformance target that lies about fail-closed: it admits (mutates) a
    /// reserve even while the rebuild window is open. The new in-suite
    /// `ReserveFailClosed` check must catch this. Implemented locally because the
    /// stock fake is honest and the trait does not expose a "be dishonest" knob.
    struct MutateOnDenyTarget {
        owner_payload_bytes: u64,
        class_payload_bytes: u64,
        rebuild: bool,
        next_id: u64,
    }

    impl MutateOnDenyTarget {
        fn new() -> Self {
            Self { owner_payload_bytes: 0, class_payload_bytes: 0, rebuild: false, next_id: 1 }
        }
    }

    impl BrokerConformanceTarget for MutateOnDenyTarget {
        type Ticket = u64;

        fn do_reserve(
            &mut self,
            _class_axis: usize,
            _owner: &str,
            _kind: ChargeAxis,
            bytes: u64,
        ) -> Result<Self::Ticket, String> {
            // THE LIE: while rebuilding, return Err (so the RebuildWriteFence
            // check still sees a rejection) but mutate the counters first. A
            // correct broker validates and bails BEFORE touching state.
            if self.rebuild {
                self.owner_payload_bytes = self.owner_payload_bytes.saturating_add(bytes);
                self.class_payload_bytes = self.class_payload_bytes.saturating_add(bytes);
                return Err("rebuild in progress".to_string());
            }
            self.owner_payload_bytes = self.owner_payload_bytes.saturating_add(bytes);
            self.class_payload_bytes = self.class_payload_bytes.saturating_add(bytes);
            let id = self.next_id;
            self.next_id = self.next_id.saturating_add(1);
            Ok(id)
        }

        fn do_commit(&mut self, _ticket: &Self::Ticket) -> Result<(), String> {
            Ok(())
        }

        fn do_release(&mut self, _ticket: &Self::Ticket) -> Result<(), String> {
            self.owner_payload_bytes = 0;
            self.class_payload_bytes = 0;
            Ok(())
        }

        fn do_begin_rebuild(&mut self) -> Result<(), String> {
            self.rebuild = true;
            Ok(())
        }

        fn do_finish_rebuild(&mut self) -> Result<(), String> {
            self.rebuild = false;
            Ok(())
        }

        fn class_axes(&self) -> &'static [&'static str] {
            &["a", "b"]
        }

        fn view(&self) -> Result<AccountingView, String> {
            Ok(AccountingView {
                total_class_bytes: self.class_payload_bytes,
                total_owner_bytes: self.owner_payload_bytes,
                total_context_bytes: self.owner_payload_bytes,
                context_axis_bytes: [self.owner_payload_bytes, 0, 0, 0, 0],
                class_axis_bytes: [self.class_payload_bytes, 0],
                owner_axis_bytes: [self.owner_payload_bytes, 0],
                group_axis_bytes: [0, 0],
                class_kind_bytes: [[self.class_payload_bytes, 0, 0], [0, 0, 0]],
                owner_kind_bytes: [[self.owner_payload_bytes, 0, 0], [0, 0, 0]],
                context_kind_bytes: [
                    [self.owner_payload_bytes, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                live_ticket_count: 0,
                total_retained_payload_bytes: self.owner_payload_bytes,
                total_retained_metadata_bytes: 0,
                total_live_scratch_bytes: 0,
                rebuild_in_progress: self.rebuild,
            })
        }

        fn observe_allocator(
            &self,
            observation: &AllocatorObservation,
        ) -> Result<AllocatorAdvisoryView, String> {
            let semantic_bytes = self.owner_payload_bytes;
            Ok(AllocatorAdvisoryView {
                semantic_bytes,
                allocator_resident_bytes: observation.resident_bytes,
                mismatch: observation
                    .resident_bytes
                    .is_some_and(|resident| resident != semantic_bytes),
                drift_bytes: observation
                    .resident_bytes
                    .map(|resident| i128::from(resident) - i128::from(semantic_bytes)),
            })
        }
    }

    #[test]
    fn suite_rejects_target_that_mutates_accounting_on_a_denied_reserve() {
        // scenario G ReserveFailClosed: the denied reserve during rebuild DOES
        // return Err (so the rebuild fence looks satisfied), yet it moved the
        // counters. The suite's fail-closed check must surface this as a
        // ReserveFailClosed violation.
        let mut liar = MutateOnDenyTarget::new();
        let verdicts = run_broker_accounting_suite(&mut liar);
        assert!(!all_held(&verdicts), "a mutate-on-deny target must be rejected");
        let fams = violated(&verdicts);
        assert!(
            fams.contains(&ContractFamily::ReserveFailClosed),
            "mutating on a denied reserve must localize to ReserveFailClosed, got {fams:?}"
        );
    }

    #[test]
    fn honest_fake_passes_the_strengthened_fail_closed_check() {
        // scenario G ReserveFailClosed (positive pole): the new in-suite
        // before==after check must HOLD for an honest fake, proving the
        // strengthening did not introduce a false positive.
        let mut honest = FakeProcessBroker::honest();
        let verdicts = run_broker_accounting_suite(&mut honest);
        let fail_closed_checks: Vec<&AssertionVerdict> =
            verdicts.iter().filter(|v| v.family == ContractFamily::ReserveFailClosed).collect();
        assert!(
            !fail_closed_checks.is_empty(),
            "the suite must actually run a ReserveFailClosed check"
        );
        assert!(
            fail_closed_checks.iter().all(|v| v.held()),
            "honest fake must satisfy fail-closed: {:?}",
            fail_closed_checks.iter().map(|v| v.to_string()).collect::<Vec<_>>()
        );
    }

    struct ObserveMutatesTarget(std::cell::Cell<u64>);

    impl ObserveMutatesTarget {
        fn new() -> Self {
            Self(std::cell::Cell::new(0))
        }
    }

    impl BrokerConformanceTarget for ObserveMutatesTarget {
        type Ticket = ();

        fn do_reserve(
            &mut self,
            _class_axis: usize,
            _owner: &str,
            _kind: ChargeAxis,
            bytes: u64,
        ) -> Result<Self::Ticket, String> {
            self.0.set(self.0.get().saturating_add(bytes));
            Ok(())
        }

        fn do_commit(&mut self, _ticket: &Self::Ticket) -> Result<(), String> {
            Ok(())
        }

        fn do_release(&mut self, _ticket: &Self::Ticket) -> Result<(), String> {
            self.0.set(0);
            Ok(())
        }

        fn do_begin_rebuild(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn do_finish_rebuild(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn class_axes(&self) -> &'static [&'static str] {
            &["a", "b"]
        }

        fn view(&self) -> Result<AccountingView, String> {
            let bytes = self.0.get();
            Ok(AccountingView {
                total_class_bytes: bytes,
                total_owner_bytes: bytes,
                total_context_bytes: bytes,
                context_axis_bytes: [bytes, 0, 0, 0, 0],
                class_axis_bytes: [bytes, 0],
                owner_axis_bytes: [bytes, 0],
                group_axis_bytes: [0, 0],
                class_kind_bytes: [[bytes, 0, 0], [0, 0, 0]],
                owner_kind_bytes: [[bytes, 0, 0], [0, 0, 0]],
                context_kind_bytes: [[bytes, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0], [0, 0, 0]],
                live_ticket_count: 0,
                total_retained_payload_bytes: bytes,
                total_retained_metadata_bytes: 0,
                total_live_scratch_bytes: 0,
                rebuild_in_progress: false,
            })
        }

        fn observe_allocator(
            &self,
            observation: &AllocatorObservation,
        ) -> Result<AllocatorAdvisoryView, String> {
            let before = self.0.get();
            self.0.set(before.saturating_add(1));
            Ok(AllocatorAdvisoryView {
                semantic_bytes: before,
                allocator_resident_bytes: observation.resident_bytes,
                mismatch: observation.resident_bytes.is_some_and(|resident| resident != before),
                drift_bytes: observation
                    .resident_bytes
                    .map(|resident| i128::from(resident) - i128::from(before)),
            })
        }
    }

    #[test]
    fn suite_rejects_target_that_mutates_on_allocator_observation() {
        let mut liar = ObserveMutatesTarget::new();
        let verdicts = run_broker_accounting_suite(&mut liar);
        assert!(!all_held(&verdicts), "observation-mutating target must be rejected");
        let fams = violated(&verdicts);
        assert!(
            fams.contains(&ContractFamily::AllocatorAdvisory),
            "allocator observation mutation must localize to AllocatorAdvisory, got {fams:?}"
        );
    }

    // -----------------------------------------------------------------------
    // commit -> release returns the balance to zero (honest fake and real).
    // -----------------------------------------------------------------------

    #[test]
    fn commit_then_release_returns_balance_to_zero_on_honest_fake() {
        // scenario G commit->release: a full reserve+commit then release must
        // drive every accounting axis back to zero while keeping invariant 1/2.
        let mut fake = FakeProcessBroker::honest();
        let owner = "owner:cycle";
        let t = fake
            .do_reserve(0, owner, ChargeAxis::RetainedPayload, 4_096)
            .expect("in-budget reserve succeeds");
        fake.do_commit(&t).expect("commit succeeds");

        let mid = fake.view().expect("view");
        assert_eq!(mid.total_owner_bytes, 4_096, "committed bytes are accounted");
        assert_eq!(mid.total_class_bytes, mid.total_owner_bytes, "invariant 1 after commit");
        assert_eq!(
            mid.checked_partition_sum(),
            Some(mid.total_owner_bytes),
            "invariant 2 after commit"
        );

        fake.do_release(&t).expect("release succeeds");
        let end = fake.view().expect("view");
        assert_eq!(end.total_owner_bytes, 0, "release returns owner usage to zero");
        assert_eq!(end.total_class_bytes, 0, "release returns class usage to zero");
        assert_eq!(end, AccountingView::default(), "balance fully zeroed after release");
    }

    #[test]
    fn commit_then_release_returns_balance_to_zero_on_real_broker() {
        // scenario G commit->release: the SAME lifecycle on the real broker must
        // also return to a zero, balanced ledger.
        let mut broker = live_broker();
        let owner = "owner:cycle";
        let t = broker
            .do_reserve(0, owner, ChargeAxis::RetainedPayload, 4_096)
            .expect("in-budget reserve succeeds");
        broker.do_commit(&t).expect("commit succeeds");
        assert_eq!(broker.view().expect("view").total_owner_bytes, 4_096);

        broker.do_release(&t).expect("release succeeds");
        let end = broker.view().expect("view");
        assert_eq!(end.total_owner_bytes, 0, "release zeroes owner usage on the real broker");
        assert_eq!(end.total_class_bytes, end.total_owner_bytes, "invariant 1 holds at zero");
        assert_eq!(
            end.checked_partition_sum(),
            Some(end.total_owner_bytes),
            "invariant 2 holds at zero"
        );
    }

    // -----------------------------------------------------------------------
    // Cap-deny AGREEMENT: honest fake and real broker deny the same way.
    // -----------------------------------------------------------------------

    fn live_broker() -> LiveProcessMemoryBroker {
        let residency = ResidencyPolicy::new(10_000_000)
            .with_class_budget(
                MemoryClass::ArtifactCache,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: 4_000,
                    hard_cap_bytes: 6_000,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            )
            .with_class_budget(
                MemoryClass::EvidenceCache,
                MemoryClassBudget {
                    reserved_bytes: 0,
                    soft_cap_bytes: 4_000,
                    hard_cap_bytes: 6_000,
                    borrow_from_shared_pool: false,
                    reclaim_priority: ReclaimPriority::Normal,
                },
            );
        quanta_memory_platform_core::ProcessMemoryBrokerBuilder::new()
            .with_manifest(MemoryBudgetManifest::new(
                MemoryPreset::Ci,
                quanta_memory_platform_core::ExecutionBudget::default(),
                residency,
            ))
            .build()
            .expect("valid residency")
    }

    #[test]
    fn cap_deny_agrees_between_honest_fake_and_real_broker() {
        // scenario G cap-deny agreement: drive an over-cap reserve on BOTH the
        // honest fake (owner hard cap) and the real broker (class hard cap) and
        // assert they AGREE on three observable facts: (1) the reserve is denied
        // with a typed Capacity error, (2) a subsequent in-cap reserve still
        // succeeds, (3) the denied reserve was byte-for-byte inert (fail-closed).
        //
        // Both targets enforce a 6_000-byte cap on the artifact-cache axis.
        let owner = "owner:capped";

        // Honest fake with a 6_000-byte owner hard cap on this owner.
        let mut fake = FakeProcessBroker::honest().with_owner_hard_cap(owner, 6_000);
        let real = live_broker();

        // Capture state before the denied op on both, via the neutral view.
        let fake_before: AccountingView = fake.view().expect("fake view");
        let real_before: AccountingView = real.view().expect("real view");

        // Over-cap reserve: 7_000 > 6_000 cap on both.
        let fake_denied = fake.reserve(
            MemoryClass::ArtifactCache,
            owner.to_string(),
            quanta_memory_platform_core::ChargeKind::RetainedPayload,
            7_000,
        );
        let real_denied = real.reserve_owned(
            MemoryClass::ArtifactCache,
            owner.to_string(),
            quanta_memory_platform_core::MemoryContextKind::Request,
            quanta_memory_platform_core::ChargeKind::RetainedPayload,
            7_000,
        );

        // (1) AGREE: both deny with a typed Capacity error (variant, not message).
        assert!(
            matches!(fake_denied, Err(MemoryError::Capacity(_))),
            "fake must deny over-cap with Capacity, got {fake_denied:?}"
        );
        assert!(
            matches!(real_denied, Err(MemoryError::Capacity(_))),
            "real broker must deny over-cap with Capacity, got {real_denied:?}"
        );

        // (3) AGREE: fail-closed. Neither denied reserve mutated any axis.
        assert_eq!(fake.view(), Ok(fake_before), "fake denied reserve must be inert");
        assert_eq!(real.view(), Ok(real_before), "real denied reserve must be inert");

        // (2) AGREE: an in-cap reserve (under 6_000) still succeeds on both.
        let fake_ok = fake.reserve(
            MemoryClass::ArtifactCache,
            owner.to_string(),
            quanta_memory_platform_core::ChargeKind::RetainedPayload,
            5_000,
        );
        let real_ok = real.reserve_owned(
            MemoryClass::ArtifactCache,
            owner.to_string(),
            quanta_memory_platform_core::MemoryContextKind::Request,
            quanta_memory_platform_core::ChargeKind::RetainedPayload,
            5_000,
        );
        assert!(fake_ok.is_ok(), "fake in-cap reserve must succeed, got {fake_ok:?}");
        assert!(real_ok.is_ok(), "real in-cap reserve must succeed, got {real_ok:?}");

        // And the accounting moved identically (5_000 on the owner/class axis).
        assert_eq!(fake.view().expect("fake view").total_owner_bytes, 5_000);
        assert_eq!(real.view().expect("real view").total_owner_bytes, 5_000);
        assert_eq!(
            fake.view().expect("fake view").total_owner_bytes,
            real.view().expect("real view").total_owner_bytes,
            "fake and real agree on post-reserve owner usage"
        );
    }

    #[test]
    fn both_targets_still_pass_the_portable_suite_after_strengthening() {
        // scenario G regression guard: the strengthened suite must still PASS the
        // honest fake AND the real broker end-to-end (the existing self_tests
        // property, re-asserted against the new in-suite fail-closed check).
        let mut fake = FakeProcessBroker::honest();
        let fake_verdicts = run_broker_accounting_suite(&mut fake);
        assert!(all_held(&fake_verdicts), "honest fake: {:?}", violated(&fake_verdicts));

        let mut real = live_broker();
        let real_verdicts = run_broker_accounting_suite(&mut real);
        assert!(all_held(&real_verdicts), "real broker: {:?}", violated(&real_verdicts));
    }

    /// A spill target that CLAIMS to accept but retains nothing (drops the handle),
    /// so a restore can never bring it back — the SpillRouting fault.
    #[cfg(feature = "fixtures")]
    #[derive(Debug, Default)]
    struct DroppingSpillTarget;

    #[cfg(feature = "fixtures")]
    impl quanta_memory_platform_facade::SpillTarget<String> for DroppingSpillTarget {
        fn is_available(&self) -> bool {
            true
        }
        fn accept(
            &self,
            _handle: String,
        ) -> Result<
            quanta_memory_platform_facade::SpillReceipt,
            quanta_memory_platform_facade::SpillFailure<String>,
        > {
            Ok(quanta_memory_platform_facade::SpillReceipt::InProcess { inventory_after: 0 })
        }
        // restore uses the trait default -> always Ok(None): nothing comes back.
    }

    #[cfg(feature = "fixtures")]
    impl quanta_memory_platform_facade::SpillInventory for DroppingSpillTarget {
        fn retained_count(&self) -> quanta_memory_platform_core::MemoryResult<u64> {
            Ok(0)
        }
    }

    // The SpillRouting verdict must be able to VIOLATE: driving the SUITE's own
    // `spill_round_trip_verdict` against an honest in-memory target HOLDS, against a
    // handle-dropping target VIOLATES. A check that passed the dropping target —
    // or a weakened suite helper — would be a rubber stamp.
    #[cfg(feature = "fixtures")]
    #[test]
    fn spill_routing_verdict_holds_for_honest_and_violates_for_a_dropping_target() {
        let honest = RoutedSpillController::new(InMemorySpillTarget::available());
        assert!(
            spill_round_trip_verdict(&honest).held(),
            "honest in-memory target must hold SpillRouting via the suite helper"
        );

        let dropping = RoutedSpillController::new(DroppingSpillTarget);
        assert!(
            !spill_round_trip_verdict(&dropping).held(),
            "SpillRouting MUST violate for a handle-dropping target (suite helper bites)"
        );
    }
}
