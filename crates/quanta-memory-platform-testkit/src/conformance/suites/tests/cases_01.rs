use super::*;

#[test]
fn deterministic_trace_matches_independent_model() {
    run_independent_trace(&mut FakeProcessBroker::honest());
    run_independent_trace(&mut live_broker());
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
    assert_broken_target_rejected(BrokerFault::WrongContextAxis, &[ContractFamily::OwnerPartition]);
}
#[test]
fn suite_rejects_wrong_charge_kind_axis() {
    assert_broken_target_rejected(
        BrokerFault::WrongChargeKindAxis,
        &[ContractFamily::OwnerPartition],
    );
}
#[test]
fn sep23_fup004_suite_rejects_class_confined_kind_swap() {
    assert_broken_target_rejected(
        BrokerFault::WrongClassKindAxis,
        &[ContractFamily::OwnerPartition],
    );
    // The rejection must include the per-axis kind verdict itself — not
    // just the exact-model mismatch — so a checker that drops the class
    // axis goes red here.
    let report =
        run_broker_accounting_suite(&mut BrokenBrokerTarget::new(BrokerFault::WrongClassKindAxis));
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
    let _t2 =
        broken.do_reserve(1, "owner:conformance-a", ChargeAxis::RetainedMetadata, 250).expect("t2");
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
        verdict.name == "transcript_new_identity_fails_closed_at_retention_bound" && !verdict.held()
    }));
    assert!(verdicts
        .iter()
        .filter(|verdict| {
            verdict.name != "transcript_new_identity_fails_closed_at_retention_bound"
        })
        .all(AssertionVerdict::held));
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
    assert!(verdicts
        .iter()
        .any(|verdict| { verdict.name == "transcript_exact_replay_is_inert" && !verdict.held() }));
    assert!(verdicts.iter().any(|verdict| {
        verdict.name == "transcript_collision_axes_are_typed" && verdict.held()
    }));
}
#[test]
fn transcript_suite_rejects_swallowed_session_retirement() {
    let platform = MemoryPlatformBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("platform");
    let store = platform
        .transcript_store_builder::<Vec<u8>, Vec<u8>, Vec<u8>>()
        .bind(StoreBindingHandle::new(
            MemoryClass::SessionTranscript,
            "swallowed-retirement-transcript".to_string(),
            None,
        ))
        .with_retention_policy(TranscriptRetentionPolicyV2::fail_closed(2, u64::MAX))
        .build()
        .expect("bound transcript builder");
    let verdicts = run_transcript_append_suite(&SwallowedRetirementTranscriptTarget(&store));
    assert_eq!(
        verdicts
            .iter()
            .filter(|verdict| !verdict.held())
            .map(|verdict| verdict.name.as_str())
            .collect::<Vec<_>>(),
        vec!["transcript_retirement_invalidates_issued_handle"],
        "only the swallowed retirement contract should fail: {verdicts:?}"
    );
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
